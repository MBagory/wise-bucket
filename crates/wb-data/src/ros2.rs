//! ROS 2 messages: definitions (from the recording or bundled) and CDR decoding into Arrow.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock};

use datafusion::arrow::array::{Array, ArrayRef, StructArray, TimestampNanosecondArray};
use re_ros_msg::MessageSchema;
use re_ros_msg::reflection::{CdrArrowDecoder, CdrDecodeError, MessageDecodePlan};

/// Definitions bundled for rosbag2 SQLite bags that don't store their own (Humble and earlier).
const BUNDLED: &str = include_str!("ros2/humble.msgs");

/// Message types whose payloads are not decoded (only their timing and size are kept):
/// images, point clouds, maps and video are large and not time series.
const TIMING_ONLY: &[&str] = &[
    "sensor_msgs/Image",
    "sensor_msgs/CompressedImage",
    "sensor_msgs/PointCloud",
    "sensor_msgs/PointCloud2",
    "nav_msgs/OccupancyGrid",
    "foxglove_msgs/CompressedVideo",
    "foxglove_msgs/RawImage",
    "foxglove_msgs/CompressedImage",
    "foxglove_msgs/PointCloud",
    "ffmpeg_image_transport_msgs/FFMPEGPacket",
];

/// `pkg/msg/Type` → `pkg/Type`.
pub fn short_name(type_name: &str) -> String {
    match type_name.split('/').collect::<Vec<_>>().as_slice() {
        [pkg, "msg", name] => format!("{pkg}/{name}"),
        _ => type_name.to_string(),
    }
}

/// Whether payloads of this type are skipped.
pub fn timing_only(type_name: &str) -> bool {
    TIMING_ONLY.contains(&short_name(type_name).as_str())
}

static BUNDLE: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    let mut current: Option<(String, Vec<&str>)> = None;
    for line in BUNDLED.lines() {
        if line.starts_with('#') || re_ros_msg::is_schema_separator(line) {
            continue;
        }
        if let Some(name) = line.strip_prefix("MSG: ") {
            if let Some((n, body)) = current.take() {
                map.insert(n, body.join("\n"));
            }
            current = Some((name.trim().to_string(), Vec::new()));
        } else if let Some((_, body)) = current.as_mut() {
            body.push(line);
        }
    }
    if let Some((n, body)) = current {
        map.insert(n, body.join("\n"));
    }
    map
});

const BUILTINS: &[&str] = &[
    "bool", "byte", "char", "float32", "float64", "int8", "uint8", "int16", "uint16", "int32",
    "uint32", "int64", "uint64", "string", "wstring",
];

/// Message types referenced by a definition body, as `pkg/Type`.
fn references(package: &str, body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in body.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some(ty) = line.split_whitespace().next() else {
            continue;
        };
        if line
            .split_whitespace()
            .nth(1)
            .is_some_and(|f| f.contains('='))
        {
            continue; // constant
        }
        let ty = ty.split('[').next().unwrap_or(ty);
        let ty = ty.split("<=").next().unwrap_or(ty);
        if BUILTINS.contains(&ty) {
            continue;
        }
        out.push(if ty.contains('/') {
            short_name(ty)
        } else {
            format!("{package}/{ty}")
        });
    }
    out
}

/// The full `ros2msg` definition of a bundled type (the type followed by its dependencies,
/// in the format MCAP and rosbag2 use), or `None` if it is not bundled.
pub fn bundled_definition(type_name: &str) -> Option<String> {
    let root = short_name(type_name);
    let main = BUNDLE.get(&root)?;
    let mut deps: BTreeMap<String, &String> = BTreeMap::new();
    let mut todo = vec![root.clone()];
    while let Some(name) = todo.pop() {
        let body = BUNDLE.get(&name)?;
        let package = name.split('/').next().unwrap_or("");
        for dep in references(package, body) {
            if dep != root && !deps.contains_key(&dep) {
                deps.insert(dep.clone(), BUNDLE.get(&dep)?);
                todo.push(dep);
            }
        }
    }
    let mut text = main.clone();
    for (name, body) in deps {
        text.push_str(&format!("\n{}\nMSG: {name}\n{body}", "=".repeat(80)));
    }
    Some(text)
}

/// Decodes the CDR messages of one stream into Arrow, a batch at a time.
pub struct Decoder {
    plan: Arc<MessageDecodePlan>,
    current: CdrArrowDecoder,
    /// Messages that could not be decoded.
    pub failures: u64,
    /// The first decoding error, for the stream's notes.
    pub first_error: Option<String>,
}

/// One decoded batch.
pub struct Decoded {
    /// The message fields, one struct row per message.
    pub fields: StructArray,
    /// `header.stamp` (or a top-level `stamp`) in ns, when the type has one.
    pub sensor_time: Option<ArrayRef>,
}

impl Decoder {
    /// Builds a decoder from a type name and its full definition.
    pub fn new(type_name: &str, definition: &str, capacity: usize) -> Result<Self, String> {
        let schema = MessageSchema::parse(type_name, definition).map_err(|e| format!("{e:#}"))?;
        let plan = Arc::new(MessageDecodePlan::from_schema(&schema).map_err(|e| format!("{e:#}"))?);
        Ok(Self {
            current: CdrArrowDecoder::new(plan.clone(), capacity),
            plan,
            failures: 0,
            first_error: None,
        })
    }

    /// Decodes one message. `false` if it was rejected (its row is dropped).
    pub fn push(&mut self, cdr: &[u8]) -> Result<bool, String> {
        match self.current.decode_message(cdr) {
            Ok(()) => Ok(true),
            Err(CdrDecodeError::Message(e)) => {
                self.failures += 1;
                self.first_error.get_or_insert_with(|| format!("{e:#}"));
                Ok(false)
            }
            Err(CdrDecodeError::Unrecoverable(e)) => Err(e.to_string()),
        }
    }

    /// The messages decoded since the last call.
    pub fn take(&mut self, capacity: usize) -> Decoded {
        let mut finished = std::mem::replace(
            &mut self.current,
            CdrArrowDecoder::new(self.plan.clone(), capacity),
        );
        let messages = finished.finish();
        let sensor_time = match self.plan.timestamp_nanos(&messages) {
            Ok(Some(ns)) => Some(Arc::new(TimestampNanosecondArray::from(
                ns.into_iter().map(|v| v as i64).collect::<Vec<_>>(),
            )) as ArrayRef),
            _ => None,
        };
        let fields = messages
            .values()
            .as_any()
            .downcast_ref::<StructArray>()
            .cloned()
            .unwrap_or_else(|| StructArray::new_empty_fields(messages.len(), None));
        Decoded {
            fields,
            sensor_time,
        }
    }
}

/// A topic's offered QoS profiles (rosbag2 YAML, one profile per publisher) in short:
/// `reliable, volatile, depth 10`. Older bags write the rmw enum values as numbers.
pub fn compact_qos(yaml: &str) -> Option<String> {
    // rmw enum values, as older bags write them.
    let name = |key: &str, v: &str| -> String {
        match (key, v) {
            ("reliability", "1") => "reliable",
            ("reliability", "2") => "best_effort",
            ("durability", "1") => "transient_local",
            ("durability", "2") => "volatile",
            ("history", "1") => "keep_last",
            ("history", "2") => "keep_all",
            (_, "0") => "system_default",
            (_, "3") => "unknown",
            (_, "4") => "best_available",
            _ => v,
        }
        .to_string()
    };
    let mut profiles: Vec<Vec<(String, String)>> = Vec::new();
    for line in yaml.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("- ") {
            profiles.push(Vec::new());
            if let Some((k, v)) = rest.split_once(':') {
                profiles
                    .last_mut()?
                    .push((k.trim().into(), v.trim().into()));
            }
        } else if let (Some(p), Some((k, v))) = (profiles.last_mut(), trimmed.split_once(':')) {
            p.push((k.trim().into(), v.trim().into()));
        }
    }
    let short: Vec<String> = profiles
        .iter()
        .filter_map(|p| {
            let get = |k: &str| p.iter().find(|(key, _)| key == k).map(|(_, v)| name(k, v));
            let mut parts = vec![get("reliability")?, get("durability")?];
            let depth: u64 = p
                .iter()
                .find(|(k, _)| k == "depth")
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(0);
            if get("history").as_deref() == Some("keep_all") {
                parts.push("keep_all".into());
            } else if depth > 0 {
                parts.push(format!("depth {depth}"));
            }
            Some(parts.join(", "))
        })
        .collect();
    (!short.is_empty()).then(|| short.join(" | "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qos_in_short() {
        // The demo talker bag (rmw enum values as numbers).
        let topic = "- history: 3\n  depth: 0\n  reliability: 1\n  durability: 2\n  deadline:\n    sec: 0\n    nsec: 0\n  lifespan:\n    sec: 0\n    nsec: 0\n  liveliness: 1\n  liveliness_lease_duration:\n    sec: 0\n    nsec: 0\n  avoid_ros_namespace_conventions: false";
        assert_eq!(compact_qos(topic).as_deref(), Some("reliable, volatile"));
        let rosout = topic.replace("durability: 2", "durability: 1");
        assert_eq!(
            compact_qos(&rosout).as_deref(),
            Some("reliable, transient_local")
        );
        // Newer bags write names; two publishers.
        let named = "- history: keep_last\n  depth: 10\n  reliability: reliable\n  durability: volatile\n- history: keep_all\n  depth: 0\n  reliability: best_effort\n  durability: volatile";
        assert_eq!(
            compact_qos(named).as_deref(),
            Some("reliable, volatile, depth 10 | best_effort, volatile, keep_all")
        );
        assert_eq!(compact_qos(""), None);
    }

    #[test]
    fn bundled_definitions_resolve_and_parse() {
        let imu = bundled_definition("sensor_msgs/msg/Imu").expect("Imu is bundled");
        for dep in [
            "MSG: std_msgs/Header",
            "MSG: builtin_interfaces/Time",
            "MSG: geometry_msgs/Quaternion",
            "MSG: geometry_msgs/Vector3",
        ] {
            assert!(imu.contains(dep), "{dep} missing in:\n{imu}");
        }
        assert!(bundled_definition("my_pkg/msg/Custom").is_none());
        // Every bundled type yields a definition re_ros_msg can plan.
        for name in BUNDLE.keys() {
            let def = bundled_definition(name).unwrap();
            Decoder::new(name, &def, 1).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn heavy_types_are_timing_only() {
        assert!(timing_only("sensor_msgs/msg/Image"));
        assert!(timing_only("sensor_msgs/PointCloud2"));
        assert!(!timing_only("sensor_msgs/msg/Imu"));
    }
}
