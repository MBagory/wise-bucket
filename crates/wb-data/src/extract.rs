//! Extracting one recording into a directory of Parquet files.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use datafusion::arrow::array::{Array, AsArray};
use datafusion::arrow::compute::cast;
use datafusion::arrow::datatypes::{DataType, UInt8Type};

use wb_core::error::{ErrorKind, Result, err};
use wb_core::roots::{DbcChoice, RecordingKind};

use crate::columns;
use crate::payload::JsonRows;
use crate::raw::{self, RawRecords};
use crate::ros2::{self, Decoded, Decoder};
use crate::sink::{StreamMeta, StreamSink};

/// Messages per batch (and per decoder run).
pub(crate) const BATCH: usize = 8192;

/// Larger messages keep their size only, not their `raw` bytes.
const RAW_MAX_BYTES: usize = 64 * 1024;

/// `raw` bytes held per topic before its batch is written.
const RAW_BATCH_BYTES: usize = 32 << 20;

/// The streams extracted from one recording.
#[derive(Debug, Default)]
pub struct Extraction {
    pub streams: Vec<StreamMeta>,
    /// Remarks about the whole recording (truncated file, skipped parts…).
    pub notes: Vec<String>,
    /// Short facts the recording states about itself (`key`, `value`).
    pub properties: Vec<(String, String)>,
    /// When the recording says it starts (the ULog header), in nanoseconds.
    pub declared_start_ns: Option<i64>,
}

/// Extracts the recording at `path` into `out` (one Parquet file per stream under `out/data`).
/// `dbc` is the DBC choice of a CAN log.
pub async fn extract(
    path: &Path,
    kind: RecordingKind,
    dbc: Option<&DbcChoice>,
    out: &Path,
) -> Result<Extraction> {
    std::fs::create_dir_all(out.join("data")).map_err(crate::sink::io_err(out.display()))?;
    match kind {
        RecordingKind::Mcap => {
            let mut topics = Topics::new(out);
            crate::mcap_reader::read(&[path.to_path_buf()], &mut topics)?;
            topics.finish(out).await
        }
        RecordingKind::Db3 => {
            let mut topics = Topics::new(out);
            crate::db3::read(&[path.to_path_buf()], &mut topics)?;
            topics.finish(out).await
        }
        RecordingKind::Rosbag2 => {
            let splits = |ext: &str| -> Result<Vec<PathBuf>> {
                let mut v: Vec<PathBuf> = std::fs::read_dir(path)
                    .map_err(crate::sink::io_err(path.display()))?
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.extension()
                            .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case(ext))
                    })
                    .collect();
                v.sort();
                Ok(v)
            };
            let mut topics = Topics::new(out);
            let (mcap, db3) = (splits("mcap")?, splits("db3")?);
            if mcap.is_empty() && db3.is_empty() {
                return Err(err(
                    ErrorKind::RecordingUnreadable,
                    format!(
                        "{}: rosbag2 folder without .mcap or .db3 files",
                        path.display()
                    ),
                ));
            }
            bag_facts(path, &mut topics);
            crate::mcap_reader::read(&mcap, &mut topics)?;
            crate::db3::read(&db3, &mut topics)?;
            topics.finish(out).await
        }
        RecordingKind::Ulog => crate::ulog::read(path, out).await,
        RecordingKind::Csv
        | RecordingKind::JsonLines
        | RecordingKind::Parquet
        | RecordingKind::ArrowIpc => crate::tabular::read(path, kind, out).await,
        RecordingKind::Tlog => crate::tlog::read(path, out).await,
        RecordingKind::CanAsc | RecordingKind::CanBlf => {
            crate::can::read(path, kind, dbc, out).await
        }
        RecordingKind::ArdupilotBin => crate::dataflash::read(path, out).await,
        RecordingKind::Nmea0183 => crate::nmea0183::read(path, out).await,
        RecordingKind::Nmea2000 => crate::nmea2000::read(path, out).await,
        RecordingKind::Mdf4 => crate::mdf4::read(path, out).await,
        RecordingKind::Ros1Bag => Err(err(
            ErrorKind::RecordingUnreadable,
            format!("{}: this format is not readable yet", path.display()),
        )),
    }
}

/// The warnings and errors of a log stream, for a recording note that points to them.
#[derive(Debug, Default)]
pub(crate) struct LogTally {
    /// `(rank, level)` → count; a lower rank is more severe.
    counts: BTreeMap<(u8, String), u64>,
    first_error: Option<String>,
    first_warning: Option<String>,
}

/// Longest message quoted in a note, in characters.
const QUOTE: usize = 100;

impl LogTally {
    /// Counts one message at a warning level or worse; `rank` orders the levels
    /// (0 = most severe).
    pub fn add(&mut self, rank: u8, level: &str, is_error: bool, message: &str) {
        *self.counts.entry((rank, level.to_string())).or_default() += 1;
        let first = if is_error {
            &mut self.first_error
        } else {
            &mut self.first_warning
        };
        if first.is_none() {
            let one_line = message.split_whitespace().collect::<Vec<_>>().join(" ");
            *first = Some(match one_line.char_indices().nth(QUOTE) {
                Some((cut, _)) => format!("{}…", &one_line[..cut]),
                None => one_line,
            });
        }
    }

    /// `4 ERROR, 1 WARNING log messages (stream /rosout), first: "…"`, or nothing.
    pub fn note(&self, stream: &str) -> Option<String> {
        if self.counts.is_empty() {
            return None;
        }
        let counts: Vec<String> = self
            .counts
            .iter()
            .map(|((_, level), n)| format!("{n} {level}"))
            .collect();
        let mut note = format!("{} log messages (stream {stream})", counts.join(", "));
        if let Some(first) = self.first_error.as_ref().or(self.first_warning.as_ref()) {
            note.push_str(&format!(", first: \"{first}\""));
        }
        Some(note)
    }
}

/// What a recording says about a topic's messages.
pub(crate) enum Schema<'a> {
    /// A full `ros2msg` definition.
    Ros2(&'a str),
    /// A protobuf `FileDescriptorSet`.
    Protobuf(&'a [u8]),
    None,
}

/// How a topic's payloads are decoded.
enum Payload {
    Cdr(Decoder),
    /// Protobuf or JSON, as rows.
    Rows(JsonRows),
}

impl Payload {
    fn push(&mut self, payload: &[u8]) -> Result<bool, String> {
        match self {
            Self::Cdr(d) => d.push(payload),
            Self::Rows(r) => r.push(payload),
        }
    }

    fn take(&mut self) -> Result<Decoded, String> {
        match self {
            Self::Cdr(d) => Ok(d.take(BATCH)),
            Self::Rows(r) => Ok(Decoded {
                fields: r.take()?,
                sensor_time: None,
            }),
        }
    }

    fn failures(&self) -> (u64, Option<&str>) {
        match self {
            Self::Cdr(d) => (d.failures, d.first_error.as_deref()),
            Self::Rows(r) => (r.failures, r.first_error.as_deref()),
        }
    }
}

/// What a rosbag2 folder's `metadata.yaml` says about the bag (`rosbag2.version`,
/// `rosbag2.storage_identifier`, its compression and ROS distribution, when set).
fn bag_facts(path: &Path, topics: &mut Topics) {
    let Ok(text) = std::fs::read_to_string(path.join("metadata.yaml")) else {
        return;
    };
    // Keys of `rosbag2_bagfile_information`, two spaces in.
    let keys = [
        "version",
        "storage_identifier",
        "compression_format",
        "compression_mode",
        "ros_distro",
    ];
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("  ").filter(|r| !r.starts_with(' ')) else {
            continue;
        };
        let Some((key, value)) = rest.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches(['"', '\'']);
        if keys.contains(&key) && !value.is_empty() {
            topics.property(format!("rosbag2.{key}"), value.to_string());
        }
    }
}

/// One topic of a message-based recording (MCAP, rosbag2).
struct Topic {
    sink: StreamSink,
    decoder: Option<Payload>,
    t: Vec<i64>,
    bytes: Vec<u32>,
    /// Each message's payload, as the file holds it (`None` above [`RAW_MAX_BYTES`]); none
    /// for images and point clouds, which keep their timing and size only.
    raw: Option<Vec<Option<Vec<u8>>>>,
    raw_len: usize,
    raw_left_out: u64,
    /// For ROS 2 log topics (`rcl_interfaces/msg/Log`): their warnings and errors.
    log: Option<LogTally>,
}

/// The topics of a message-based recording, decoded a batch at a time.
pub(crate) struct Topics {
    dir: PathBuf,
    topics: Vec<Topic>,
    by_key: HashMap<(String, String), usize>,
    names: HashMap<String, usize>,
    /// Facts the recording states about itself (MCAP metadata records).
    pub properties: Vec<(String, String)>,
    /// Messages read so far, and those the decoder rejected.
    messages: u64,
    rejected: RawRecords,
}

impl Topics {
    fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            topics: Vec::new(),
            by_key: HashMap::new(),
            names: HashMap::new(),
            properties: Vec::new(),
            messages: 0,
            rejected: RawRecords::default(),
        }
    }

    /// Adds a fact the recording states about itself, once (the splits of a bag repeat them).
    pub fn property(&mut self, key: String, value: String) {
        if !self.properties.iter().any(|(k, _)| *k == key) {
            self.properties.push((key, value));
        }
    }

    /// Finds or creates the stream of `topic` with message type `type_`.
    ///
    /// `schema` is what the recording says about the messages, and `qos` the
    /// publishers' `offered_qos_profiles` (rosbag2 YAML).
    pub fn topic(
        &mut self,
        topic: &str,
        type_: &str,
        encoding: &str,
        schema: Schema<'_>,
        qos: Option<&str>,
    ) -> usize {
        let key = (topic.to_string(), type_.to_string());
        if let Some(&i) = self.by_key.get(&key) {
            return i;
        }
        // The same topic with another type (rare) becomes its own stream.
        let seen = self.names.entry(topic.to_string()).or_default();
        let name = if *seen == 0 {
            topic.to_string()
        } else {
            format!("{topic} ({type_})")
        };
        *seen += 1;
        let index = self.topics.len();
        let mut sink = StreamSink::new(&self.dir, index, &name, type_, encoding);
        sink.meta.qos = qos.and_then(ros2::compact_qos);
        let decoder = if encoding.eq_ignore_ascii_case("protobuf") {
            match schema {
                Schema::Protobuf(set) => match JsonRows::protobuf(type_, set) {
                    Ok(rows) => Some(Payload::Rows(rows)),
                    Err(e) => {
                        sink.note(format!("payload not decoded: unusable schema: {e}"));
                        None
                    }
                },
                _ => {
                    sink.note("payload not decoded: the recording has no protobuf schema for it");
                    None
                }
            }
        } else if encoding.eq_ignore_ascii_case("json") {
            Some(Payload::Rows(JsonRows::json()))
        } else if !encoding.eq_ignore_ascii_case("cdr") {
            sink.note(format!(
                "payload not decoded: the {encoding} encoding is not supported yet"
            ));
            None
        } else if ros2::timing_only(type_) {
            sink.note("payload not decoded: images, point clouds and maps keep their timing and size only");
            None
        } else {
            let bundled;
            let def = match schema {
                Schema::Ros2(d) if !d.trim().is_empty() => Some(d),
                _ => {
                    bundled = ros2::bundled_definition(type_);
                    bundled.as_deref()
                }
            };
            match def {
                None => {
                    sink.note(
                        "payload not decoded: the recording has no definition for this custom type",
                    );
                    None
                }
                Some(def) => match Decoder::new(type_, def, BATCH) {
                    Ok(d) => Some(Payload::Cdr(d)),
                    Err(e) => {
                        sink.note(format!("payload not decoded: unusable definition: {e}"));
                        None
                    }
                },
            }
        };
        sink.meta.decoded = decoder.is_some();
        let log = (decoder.is_some() && ros2::short_name(type_) == "rcl_interfaces/Log")
            .then(LogTally::default);
        self.topics.push(Topic {
            sink,
            decoder,
            t: Vec::with_capacity(BATCH),
            bytes: Vec::with_capacity(BATCH),
            raw: (!ros2::timing_only(type_)).then(|| Vec::with_capacity(BATCH)),
            raw_len: 0,
            raw_left_out: 0,
            log,
        });
        self.by_key.insert(key, index);
        index
    }

    /// Adds one message (CDR payload with its encapsulation header).
    pub fn push(&mut self, index: usize, time_ns: i64, payload: &[u8]) -> Result<()> {
        self.messages += 1;
        let topic = &mut self.topics[index];
        if let Some(d) = topic.decoder.as_mut() {
            let accepted = d.push(payload).map_err(|e| {
                err(
                    ErrorKind::RecordingUnreadable,
                    format!("decoding {}: {e}", topic.sink.meta.name),
                )
            })?;
            if !accepted {
                // Its position is its number in the recording.
                let name = &topic.sink.meta.name;
                self.rejected
                    .bytes(Some(time_ns), self.messages, name, payload);
                return Ok(());
            }
        }
        topic.t.push(time_ns);
        topic
            .bytes
            .push(payload.len().try_into().unwrap_or(u32::MAX));
        if let Some(raw) = topic.raw.as_mut() {
            if payload.len() <= RAW_MAX_BYTES {
                topic.raw_len += payload.len();
                raw.push(Some(payload.to_vec()));
            } else {
                topic.raw_left_out += 1;
                raw.push(None);
            }
        }
        if topic.t.len() >= BATCH || topic.raw_len >= RAW_BATCH_BYTES {
            self.flush(index)?;
        }
        Ok(())
    }

    fn flush(&mut self, index: usize) -> Result<()> {
        let topic = &mut self.topics[index];
        if topic.t.is_empty() {
            return Ok(());
        }
        let t = std::mem::replace(&mut topic.t, Vec::with_capacity(BATCH));
        let bytes = std::mem::replace(&mut topic.bytes, Vec::with_capacity(BATCH));
        let payloads = topic
            .raw
            .as_mut()
            .map(|r| std::mem::replace(r, Vec::with_capacity(BATCH)));
        topic.raw_len = 0;
        let decoded = match topic.decoder.as_mut().map(Payload::take).transpose() {
            Ok(d) => d,
            Err(e) => {
                return Err(err(
                    ErrorKind::RecordingUnreadable,
                    format!("decoding {}: {e}", topic.sink.meta.name),
                ));
            }
        };
        let (batch, dropped) = columns::assemble(
            t,
            decoded.as_ref().and_then(|d| d.sensor_time.clone()),
            Some(bytes),
            decoded.as_ref().map(|d| &d.fields),
        );
        if !dropped.is_empty() {
            topic.sink.note(format!(
                "large byte arrays left out: {}",
                dropped.join(", ")
            ));
        }
        if let Some(tally) = topic.log.as_mut() {
            tally_ros_log(tally, &batch);
        }
        let batch = match payloads {
            Some(p) => raw::with_raw(
                batch,
                std::sync::Arc::new(datafusion::arrow::array::LargeBinaryArray::from_iter(p)),
            )?,
            None => batch,
        };
        topic.sink.append(batch)
    }

    async fn finish(mut self, out: &Path) -> Result<Extraction> {
        for i in 0..self.topics.len() {
            self.flush(i)?;
        }
        let raw_index = self.topics.len();
        let mut streams = Vec::new();
        let mut notes = Vec::new();
        for mut topic in self.topics {
            if let Some(note) = topic
                .log
                .as_ref()
                .and_then(|l| l.note(&topic.sink.meta.name))
            {
                notes.push(note);
            }
            if let Some((failures, first)) = topic.decoder.as_ref().map(Payload::failures)
                && failures > 0
            {
                topic.sink.note(format!(
                    "{failures} message(s) could not be decoded, kept in stream {} (first error: {})",
                    raw::RAW,
                    first.unwrap_or("?")
                ));
            }
            if topic.raw_left_out > 0 {
                topic.sink.note(format!(
                    "{} message(s) over 64 KiB without their raw bytes (message_bytes keeps their size)",
                    topic.raw_left_out
                ));
            }
            if let Some(meta) = topic.sink.finish(out)? {
                streams.push(meta);
            }
        }
        if let Some(meta) = self.rejected.finish(out, raw_index, "messages")? {
            streams.push(meta);
        }
        Ok(Extraction {
            streams,
            notes,
            properties: self.properties,
            declared_start_ns: None,
        })
    }
}

/// Counts the WARN, ERROR and FATAL messages of a decoded `rcl_interfaces/msg/Log` batch.
fn tally_ros_log(tally: &mut LogTally, batch: &datafusion::arrow::array::RecordBatch) {
    let column =
        |name: &str, ty: &DataType| batch.column_by_name(name).and_then(|c| cast(c, ty).ok());
    let (Some(levels), Some(msgs)) = (
        column("level", &DataType::UInt8),
        column("msg", &DataType::Utf8),
    ) else {
        return;
    };
    let (levels, msgs) = (levels.as_primitive::<UInt8Type>(), msgs.as_string::<i32>());
    for i in 0..levels.len() {
        let (rank, name) = match levels.value(i) {
            50.. => (0, "FATAL"),
            40.. => (1, "ERROR"),
            30.. => (2, "WARN"),
            _ => continue,
        };
        let msg = if msgs.is_null(i) { "" } else { msgs.value(i) };
        tally.add(rank, name, rank < 2, msg);
    }
}

#[cfg(test)]
mod tests {
    use super::{LogTally, Schema, Topics};

    /// A decoded message keeps its payload in `raw`; one the decoder rejects goes to the
    /// stream `raw`, with its number in the recording.
    #[tokio::test]
    async fn every_message_is_kept_raw() {
        use datafusion::arrow::array::{AsArray, RecordBatch};
        use datafusion::arrow::datatypes::UInt64Type;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        let mut topics = Topics::new(dir.path());
        let i = topics.topic("/chatter", "std_msgs/msg/String", "cdr", Schema::None, None);
        // CDR, little endian: the header, then "hi" with its length and terminating zero.
        let hi = [0u8, 1, 0, 0, 3, 0, 0, 0, b'h', b'i', 0, 0];
        topics.push(i, 1, &hi).unwrap();
        topics.push(i, 2, &[0, 1]).unwrap();
        let x = topics.finish(dir.path()).await.unwrap();
        let stream = |name: &str| -> RecordBatch {
            let meta = x.streams.iter().find(|s| s.name == name).unwrap();
            let file = std::fs::File::open(dir.path().join(&meta.dataset)).unwrap();
            let mut reader =
                parquet::arrow::arrow_reader::ParquetRecordBatchReader::try_new(file, 1024)
                    .unwrap();
            reader.next().unwrap().unwrap()
        };
        let chatter = stream("/chatter");
        assert_eq!(chatter.num_rows(), 1);
        let raw = chatter.column_by_name("raw").unwrap().as_binary::<i64>();
        assert_eq!(raw.value(0), &hi);
        let rejected = stream("raw");
        let kind = rejected.column_by_name("kind").unwrap().as_string::<i32>();
        let position = rejected.column_by_name("position").unwrap();
        let bytes = rejected.column_by_name("bytes").unwrap().as_binary::<i64>();
        assert_eq!(kind.value(0), "/chatter");
        assert_eq!(position.as_primitive::<UInt64Type>().value(0), 2);
        assert_eq!(bytes.value(0), &[0, 1]);
    }

    #[test]
    fn log_notes_point_to_the_worst_first() {
        let mut t = LogTally::default();
        assert_eq!(t.note("/rosout"), None);
        t.add(2, "WARN", false, "battery low");
        t.add(1, "ERROR", true, "lost\nlink");
        t.add(1, "ERROR", true, "second error");
        assert_eq!(
            t.note("/rosout").unwrap(),
            "2 ERROR, 1 WARN log messages (stream /rosout), first: \"lost link\""
        );
        let mut t = LogTally::default();
        t.add(4, "WARNING", false, &"é".repeat(150));
        assert_eq!(
            t.note("x").unwrap(),
            format!(
                "1 WARNING log messages (stream x), first: \"{}…\"",
                "é".repeat(100)
            )
        );
    }
}
