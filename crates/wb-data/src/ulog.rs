//! PX4 ULog files: one stream per topic instance (`sensor_accel`, `sensor_accel.1`…),
//! plus the log's own records, named and timed as pyulog reads them: `parameters`,
//! `logged_messages`, `dropouts`, and `texts` (boot console, crash dump, performance
//! counters). Info messages become `properties`.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use datafusion::arrow::array::{
    ArrayRef, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array,
    Int64Array, RecordBatch, TimestampNanosecondArray, UInt8Array, UInt16Array, UInt32Array,
    UInt64Array,
};
use datafusion::arrow::datatypes::{Field, Schema};
use px4_ulog::full_parser::{Completeness, SomeVec};
use px4_ulog::stream_parser::model::ParameterDefaultMessage;
use px4_ulog::stream_parser::{
    LogStage, LoggedStringMessage, Message, ParameterMessage, SimpleCallbackResult,
    read_file_with_simple_callback,
};
use serde_json::{Map, Value};
use wb_core::error::{ErrorKind, Result, err};

use crate::columns::{T, time_type};
use crate::extract::{Extraction, LogTally};
use crate::rows::Rows;
use crate::sink::StreamSink;

fn array(v: SomeVec) -> Option<ArrayRef> {
    Some(match v {
        SomeVec::Int8(v) => Arc::new(Int8Array::from(v)),
        SomeVec::UInt8(v) => Arc::new(UInt8Array::from(v)),
        SomeVec::Int16(v) => Arc::new(Int16Array::from(v)),
        SomeVec::UInt16(v) => Arc::new(UInt16Array::from(v)),
        SomeVec::Int32(v) => Arc::new(Int32Array::from(v)),
        SomeVec::UInt32(v) => Arc::new(UInt32Array::from(v)),
        SomeVec::Int64(v) => Arc::new(Int64Array::from(v)),
        SomeVec::UInt64(v) => Arc::new(UInt64Array::from(v)),
        SomeVec::Float(v) => Arc::new(Float32Array::from(v)),
        SomeVec::Double(v) => Arc::new(Float64Array::from(v)),
        SomeVec::Bool(v) => Arc::new(BooleanArray::from(v)),
        // Text is logged one character per field: not a signal.
        SomeVec::Char(_) => return None,
    })
}

/// Microseconds as nanoseconds.
fn ns(us: u64) -> i64 {
    i64::try_from(us).unwrap_or(i64::MAX).saturating_mul(1000)
}

/// A float parameter as it is written (`0.09`, not `0.0900000035…`).
fn short_f64(v: f32) -> f64 {
    format!("{v}").parse().unwrap_or(f64::from(v))
}

/// An info value as text, decoded by its type (`char[40]`, `uint32_t`, `float`…).
fn info_text(type_str: &str, value: &[u8]) -> Option<String> {
    if type_str.starts_with("char") {
        let end = value.iter().position(|&b| b == 0).unwrap_or(value.len());
        return Some(String::from_utf8_lossy(&value[..end]).into_owned());
    }
    fn le<const N: usize>(v: &[u8]) -> Option<[u8; N]> {
        v.get(..N)?.try_into().ok()
    }
    Some(match type_str {
        "bool" => (*value.first()? != 0).to_string(),
        "int8_t" => (*value.first()? as i8).to_string(),
        "uint8_t" => value.first()?.to_string(),
        "int16_t" => i16::from_le_bytes(le(value)?).to_string(),
        "uint16_t" => u16::from_le_bytes(le(value)?).to_string(),
        "int32_t" => i32::from_le_bytes(le(value)?).to_string(),
        "uint32_t" => u32::from_le_bytes(le(value)?).to_string(),
        "int64_t" => i64::from_le_bytes(le(value)?).to_string(),
        "uint64_t" => u64::from_le_bytes(le(value)?).to_string(),
        "float" => f32::from_le_bytes(le(value)?).to_string(),
        "double" => f64::from_le_bytes(le(value)?).to_string(),
        _ => return None,
    })
}

/// `perf_counter_preflight-07` → (`perf_counter_preflight`, line 8): older logs split a
/// text into numbered info keys.
fn numbered(key: &str) -> Option<(&str, usize)> {
    let (name, n) = key.rsplit_once('-')?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((name, n.parse::<usize>().ok()? + 1))
}

/// A multi-part text: fragments that carry newlines are chunks of one text (boot
/// console, crash dump), others are one line each (performance counters).
fn text_of(entries: &[Vec<String>]) -> String {
    entries
        .iter()
        .map(|fragments| {
            if fragments.iter().any(|f| f.contains('\n')) {
                fragments.concat()
            } else {
                fragments.join("\n")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The log's own records, in file order.
#[derive(Default)]
struct Records {
    /// The header start, then the largest data timestamp so far (µs): the time given
    /// to records that carry none, as pyulog does.
    last: u64,
    /// `(t, name, value, initial)`.
    parameters: Vec<(u64, String, f64, bool)>,
    /// Name → (system default, airframe default).
    defaults: HashMap<String, (Option<f64>, Option<f64>)>,
    /// `(t, rank, level, message, tag)`; rank 0 is EMERGENCY, 7 DEBUG.
    logged: Vec<(u64, u8, &'static str, String, Option<u16>)>,
    /// `(t, duration_ms)`.
    dropouts: Vec<(u64, u16)>,
    /// Multi-part texts: `(key, t, entries)`, each entry a list of fragments.
    texts: Vec<(String, u64, Vec<Vec<String>>)>,
    /// Texts split into numbered info keys: `(name, line, text, t)`.
    text_lines: Vec<(String, usize, String, u64)>,
    properties: Vec<(String, String)>,
    /// Multi-part info that is not text (`metadata_events`).
    binary: Vec<String>,
}

impl Records {
    fn on(&mut self, m: &Message) {
        match m {
            Message::Data(d) => {
                if let Some(ts) = &d.flattened_format.timestamp_field {
                    self.last = self.last.max(ts.parse_timestamp(d.data));
                }
            }
            Message::ParameterMessage(p) => {
                let (name, value, stage) = match p {
                    ParameterMessage::Float(n, v, s) => (n, short_f64(*v), s),
                    ParameterMessage::Int32(n, v, s) => (n, f64::from(*v), s),
                };
                let initial = *stage == LogStage::Definitions;
                self.parameters
                    .push((self.last, name.to_string(), value, initial));
            }
            Message::ParameterDefaultMessage(p) => {
                let (name, value, types) = match p {
                    ParameterDefaultMessage::Float(n, v, t) => (n, short_f64(*v), *t),
                    ParameterDefaultMessage::Int32(n, v, t) => (n, f64::from(*v), *t),
                };
                let entry = self.defaults.entry(name.to_string()).or_default();
                if types & 1 != 0 {
                    entry.0 = Some(value);
                }
                if types & 2 != 0 {
                    entry.1 = Some(value);
                }
            }
            Message::LoggedMessage(l) => self.log(l, None),
            Message::TaggedLoggedMessage(l) => {
                let plain = LoggedStringMessage {
                    log_level: l.log_level,
                    timestamp: l.timestamp,
                    logged_message: l.logged_message,
                };
                self.log(&plain, Some(l.tag));
            }
            Message::InfoMessage(i) => {
                let Some(text) = info_text(i.type_str, i.value) else {
                    return;
                };
                match numbered(i.key) {
                    Some((name, line)) => {
                        self.text_lines
                            .push((name.to_string(), line, text, self.last))
                    }
                    None => self.properties.push((i.key.to_string(), text)),
                }
            }
            Message::MultiInfoMessage(m) => {
                if !m.type_str.starts_with("char") {
                    if !self.binary.iter().any(|k| k == m.key) {
                        self.binary.push(m.key.to_string());
                    }
                    return;
                }
                let text = String::from_utf8_lossy(m.value)
                    .trim_end_matches('\0')
                    .to_string();
                // `is_continued`: this fragment continues the key's previous entry.
                match self.texts.iter_mut().find(|(k, ..)| k == m.key) {
                    Some((.., entries)) if m.is_continued && !entries.is_empty() => {
                        entries.last_mut().expect("not empty").push(text)
                    }
                    Some((.., entries)) => entries.push(vec![text]),
                    None => self
                        .texts
                        .push((m.key.to_string(), self.last, vec![vec![text]])),
                }
            }
            Message::DropoutMessage(d) => self.dropouts.push((self.last, d.duration_ms)),
            _ => {}
        }
    }

    fn log(&mut self, l: &LoggedStringMessage, tag: Option<u16>) {
        let rank = (l.log_level as char).to_digit(10).unwrap_or(7) as u8;
        self.logged.push((
            l.timestamp,
            rank,
            l.human_readable_log_level(),
            l.logged_message.to_string(),
            tag,
        ));
    }
}

pub(crate) async fn read(path: &Path, out: &Path) -> Result<Extraction> {
    let unreadable = |e: &dyn std::fmt::Display| {
        err(
            ErrorKind::RecordingUnreadable,
            format!("{}: {e}", path.display()),
        )
    };
    // ponytail: the whole file is parsed in memory (ULog files are usually small);
    // switch to px4_ulog's stream parser if multi-GB logs show up.
    let parsed =
        px4_ulog::full_parser::read_file(&path.to_string_lossy()).map_err(|e| unreadable(&e))?;
    let mut extraction = Extraction::default();
    match &parsed.completeness {
        Completeness::Complete => {}
        Completeness::Truncated => extraction.notes.push(
            "the file ends mid-record (cut while logging): its valid part was extracted".into(),
        ),
        Completeness::MalformedRecord(m) => extraction.notes.push(format!(
            "a damaged record stopped the reading ({m}): the part before it was extracted"
        )),
    }
    let mut streams: Vec<(String, std::collections::HashMap<String, SomeVec>)> = Vec::new();
    for (topic, instances) in parsed.messages {
        for (multi_id, fields) in instances {
            let name = match multi_id.value() {
                0 => topic.clone(),
                n => format!("{topic}.{n}"),
            };
            streams.push((name, fields));
        }
    }
    streams.sort_by(|a, b| a.0.cmp(&b.0));
    let first_free = streams.len();
    for (index, (name, mut fields)) in streams.into_iter().enumerate() {
        let mut sink = StreamSink::new(out, index, &name, "ulog", "ulog");
        sink.meta.decoded = true;
        let Some(SomeVec::UInt64(stamps)) = fields.remove("timestamp") else {
            sink.note("no timestamp field: left out");
            continue;
        };
        let rows = stamps.len();
        match stamps.iter().filter(|&&us| us == 0).count() {
            0 => {}
            n if n == rows => sink.note("timestamp 0 (not set) on every row"),
            n => sink.note(format!("{n} rows have timestamp 0 (not set)")),
        }
        let t: Vec<i64> = stamps.iter().map(|us| ns(*us)).collect();
        let mut cols: Vec<(Field, ArrayRef)> = vec![(
            Field::new(T, time_type(), true),
            Arc::new(TimestampNanosecondArray::from(t)),
        )];
        let mut names: Vec<String> = fields.keys().cloned().collect();
        names.sort();
        let mut skipped_text = false;
        for n in names {
            if n.starts_with("_padding") {
                continue;
            }
            let v = fields.remove(&n).expect("key from the map");
            match array(v) {
                Some(a) if a.len() == rows => {
                    cols.push((Field::new(n, a.data_type().clone(), true), a));
                }
                Some(_) => sink.note(format!("field {n} has a different length: left out")),
                None => skipped_text = true,
            }
        }
        if skipped_text {
            sink.note("character fields left out");
        }
        let (fields, arrays): (Vec<_>, Vec<_>) = cols.into_iter().unzip();
        let batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays)
            .map_err(|e| unreadable(&e))?;
        sink.append(batch)?;
        if let Some(meta) = sink.finish(out)? {
            extraction.streams.push(meta);
        }
    }

    // The log's own records. ponytail: a second pass over the file, because
    // full_parser keeps only the data; one LogParser with both callbacks if large logs
    // make this slow.
    let mut header = [0u8; 16];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut header))
        .map_err(|e| unreadable(&e))?;
    let start = u64::from_le_bytes(header[8..16].try_into().expect("8 bytes"));
    let mut records = Records {
        last: start,
        ..Default::default()
    };
    read_file_with_simple_callback(&path.to_string_lossy(), &mut |m: &Message| {
        records.on(m);
        SimpleCallbackResult::KeepReading
    })
    .map_err(|e| unreadable(&e))?;
    extraction.declared_start_ns = Some(ns(start));
    extraction.properties = std::mem::take(&mut records.properties);
    extraction.notes.extend(notes(&records));
    extraction
        .streams
        .extend(record_streams(records, out, first_free).await?.streams);
    Ok(extraction)
}

/// Recording notes that point to the log's problems.
fn notes(r: &Records) -> Vec<String> {
    let mut notes = Vec::new();
    let mut tally = LogTally::default();
    for (_, rank, level, message, _) in &r.logged {
        if *rank <= 4 {
            tally.add(*rank, level, *rank <= 3, message);
        }
    }
    notes.extend(tally.note("logged_messages"));
    if !r.dropouts.is_empty() {
        let total: u64 = r.dropouts.iter().map(|(_, ms)| u64::from(*ms)).sum();
        notes.push(format!(
            "{} logger dropouts, {total} ms in total (stream dropouts)",
            r.dropouts.len()
        ));
    }
    for (key, _, _) in &r.texts {
        if key.starts_with("hardfault") {
            notes.push(format!(
                "crash dump from an earlier crash (stream texts, name {key})"
            ));
        }
    }
    if !r.binary.is_empty() {
        notes.push(format!("binary info left out: {}", r.binary.join(", ")));
    }
    notes
}

/// The `parameters`, `logged_messages`, `dropouts` and `texts` streams.
async fn record_streams(r: Records, out: &Path, first_index: usize) -> Result<Extraction> {
    let mut rows = Rows::new(out, first_index);
    for (t, name, value, initial) in r.parameters {
        let mut f = Map::new();
        if let Some((system, airframe)) = r.defaults.get(&name) {
            if let Some(v) = system {
                f.insert("system_default".into(), (*v).into());
            }
            if let Some(v) = airframe {
                f.insert("airframe_default".into(), (*v).into());
            }
        }
        f.insert("name".into(), name.into());
        f.insert("value".into(), value.into());
        f.insert("initial".into(), initial.into());
        rows.push("parameters", "ulog parameters", "ulog", ns(t), f)?;
    }
    for (t, _, level, message, tag) in r.logged {
        let mut f = Map::new();
        f.insert("level".into(), level.into());
        f.insert("message".into(), message.into());
        if let Some(tag) = tag {
            f.insert("tag".into(), tag.into());
        }
        rows.push("logged_messages", "ulog logged messages", "ulog", ns(t), f)?;
    }
    for (t, duration_ms) in r.dropouts {
        let f = Map::from_iter([("duration_ms".to_string(), Value::from(duration_ms))]);
        rows.push("dropouts", "ulog dropouts", "ulog", ns(t), f)?;
    }
    let lines = r
        .texts
        .iter()
        .flat_map(|(key, t, entries)| {
            text_of(entries)
                .lines()
                .enumerate()
                .map(|(i, line)| (key.clone(), i + 1, line.to_string(), *t))
                .collect::<Vec<_>>()
        })
        .chain(r.text_lines);
    for (name, line, text, t) in lines {
        let f = Map::from_iter([
            ("name".to_string(), Value::from(name)),
            ("line".to_string(), Value::from(line)),
            ("text".to_string(), Value::from(text)),
        ]);
        rows.push("texts", "ulog texts", "ulog", ns(t), f)?;
    }
    rows.finish(out).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_values_and_numbered_keys() {
        assert_eq!(
            info_text("char[10]", b"AUAV_X21\0\0").as_deref(),
            Some("AUAV_X21")
        );
        assert_eq!(
            info_text("int32_t", &(-3600i32).to_le_bytes()).as_deref(),
            Some("-3600")
        );
        assert_eq!(
            info_text("uint32_t", &17_170_432u32.to_le_bytes()).as_deref(),
            Some("17170432")
        );
        assert_eq!(info_text("uint8_t[16]", &[1; 16]), None);
        assert_eq!(
            numbered("perf_counter_preflight-07"),
            Some(("perf_counter_preflight", 8))
        );
        assert_eq!(numbered("ver_sw_release"), None);
        assert_eq!(numbered("sys-"), None);
        assert_eq!(short_f64(0.09), 0.09);
        let chunks = vec![vec![
            "boot: ok\nbaro: ".to_string(),
            "not found\n".to_string(),
        ]];
        assert_eq!(text_of(&chunks), "boot: ok\nbaro: not found\n");
        let counters = vec![vec![
            "imu: 3 events".to_string(),
            "gps: 1 events".to_string(),
        ]];
        assert_eq!(text_of(&counters), "imu: 3 events\ngps: 1 events");
    }
}
