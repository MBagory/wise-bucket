//! NMEA 2000 captures in canboat's text formats (`.raw`: PLAIN/FAST, Actisense ASCII,
//! YDWG-02, candump…), read by `canboat`: every frame goes to the stream `frames`, and
//! each message it decodes (fast-packet messages reassembled) to one stream per PGN,
//! named as canboat names it (`windData`), in SI units.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use canboat::read::{Decoder, FrameSource, InputFormat};
use canboat::{Database, DecodedField, DecodedPgn, FieldValue, Frame, Units};
use chrono::{DateTime, NaiveDateTime};
use datafusion::arrow::array::{
    ArrayRef, ListBuilder, RecordBatch, TimestampNanosecondArray, UInt8Array, UInt8Builder,
    UInt32Array,
};
use datafusion::arrow::datatypes::{Field, Schema};
use serde_json::Value;
use wb_core::error::{ErrorKind, Result, err};

use crate::columns::{T, time_type};
use crate::extract::{BATCH, Extraction};
use crate::rows::Rows;
use crate::sink::StreamSink;

/// The stream holding every frame of a capture.
pub const FRAMES: &str = "frames";

/// A capture's timestamp in ns: ISO 8601 (`2020-08-22T13:52:36.950Z`) or canboat's older
/// `2011-11-24-22:42:04.388`, read as UTC.
fn timestamp_ns(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return t.timestamp_nanos_opt();
    }
    for format in [
        "%Y-%m-%d-%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
    ] {
        if let Ok(t) = NaiveDateTime::parse_from_str(s.trim_end_matches('Z'), format) {
            return t.and_utc().timestamp_nanos_opt();
        }
    }
    None
}

/// A field's value as a column value: numbers exact (canboat's JSON rounds them),
/// lookups by name, times in seconds, dates as `YYYY-MM-DD`; reserved, spare and
/// unavailable fields are left out.
fn value(v: &FieldValue) -> Option<Value> {
    Some(match v {
        FieldValue::Number(x) | FieldValue::Float(x) => Value::from(*x),
        FieldValue::Integer(x) => Value::from(*x),
        FieldValue::Lookup { value, name } => {
            name.map_or_else(|| Value::from(value.to_string()), Value::from)
        }
        FieldValue::BitField { value, bits } => {
            let names: Vec<&str> = bits.iter().filter_map(|(_, n)| *n).collect();
            if names.is_empty() {
                Value::from(value.to_string())
            } else {
                Value::from(names.join(", "))
            }
        }
        FieldValue::Decimal(s) | FieldValue::String(s) => Value::from(s.as_str()),
        FieldValue::Date(days) => Value::from(
            (chrono::NaiveDate::from_ymd_opt(1970, 1, 1)? + chrono::Days::new(u64::from(*days)))
                .to_string(),
        ),
        FieldValue::Time { seconds, .. } => Value::from(*seconds),
        FieldValue::Mmsi(x) => Value::from(*x),
        FieldValue::Pgn { value, .. } => Value::from(*value),
        FieldValue::IsoName { value, .. } => Value::from(format!("{value:#018x}")),
        _ => return None,
    })
}

/// The columns of a decoded message: its fields by canboat id (`windSpeed`), the
/// repeating sets as `list` and `list2` arrays, and `src`, `dst`, `prio`.
fn row(m: &DecodedPgn) -> serde_json::Map<String, Value> {
    let mut row = serde_json::Map::new();
    let mut sets: [Vec<serde_json::Map<String, Value>>; 2] = [Vec::new(), Vec::new()];
    for f in &m.fields {
        let Some(v) = value(&f.value) else { continue };
        match (f.repeat_set, f.repeat_index) {
            (s @ (1 | 2), Some(i)) => {
                let set = &mut sets[usize::from(s - 1)];
                let i = i as usize;
                if set.len() <= i {
                    set.resize_with(i + 1, serde_json::Map::new);
                }
                set[i].insert(f.info.id.to_string(), v);
            }
            _ => {
                row.insert(f.info.id.to_string(), v);
            }
        }
    }
    for (name, set) in ["list", "list2"].into_iter().zip(sets) {
        if !set.is_empty() {
            row.insert(
                name.into(),
                Value::Array(set.into_iter().map(Value::Object).collect()),
            );
        }
    }
    row.insert("src".into(), m.src.into());
    row.insert("dst".into(), m.dst.into());
    row.insert("prio".into(), m.prio.into());
    row
}

/// `windSpeed [m/s], windAngle [rad]…` for a message's top-level fields.
fn units(m: &DecodedPgn) -> Option<String> {
    let u: Vec<String> = m
        .fields
        .iter()
        .filter(|f: &&DecodedField| f.repeat_set == 0)
        .filter_map(|f| f.info.unit.map(|u| format!("{} [{u}]", f.info.id)))
        .collect();
    (!u.is_empty()).then(|| format!("units: {}", u.join(", ")))
}

/// The frames of a capture, already read, as a source for the decoder.
struct Replay(std::vec::IntoIter<Frame>);

impl FrameSource for Replay {
    fn read_frame(&mut self) -> std::io::Result<Option<Frame>> {
        Ok(self.0.next())
    }
}

/// The raw `frames` stream, a batch at a time.
struct Raw {
    sink: StreamSink,
    t: Vec<Option<i64>>,
    prio: Vec<u8>,
    pgn: Vec<u32>,
    src: Vec<u8>,
    dst: Vec<u8>,
    data: ListBuilder<UInt8Builder>,
}

impl Raw {
    fn push(&mut self, f: &Frame) -> Result<()> {
        self.t.push(f.timestamp.as_deref().and_then(timestamp_ns));
        self.prio.push(f.prio);
        self.pgn.push(f.pgn);
        self.src.push(f.src);
        self.dst.push(f.dst);
        self.data.values().append_slice(&f.data);
        self.data.append(true);
        if self.t.len() >= BATCH {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if self.t.is_empty() {
            return Ok(());
        }
        let arrays: Vec<(&str, ArrayRef)> = vec![
            (
                T,
                Arc::new(TimestampNanosecondArray::from(std::mem::take(&mut self.t))),
            ),
            (
                "prio",
                Arc::new(UInt8Array::from(std::mem::take(&mut self.prio))),
            ),
            (
                "pgn",
                Arc::new(UInt32Array::from(std::mem::take(&mut self.pgn))),
            ),
            (
                "src",
                Arc::new(UInt8Array::from(std::mem::take(&mut self.src))),
            ),
            (
                "dst",
                Arc::new(UInt8Array::from(std::mem::take(&mut self.dst))),
            ),
            ("data", Arc::new(self.data.finish())),
        ];
        let fields: Vec<Field> = arrays
            .iter()
            .map(|(n, a)| match *n {
                T => Field::new(T, time_type(), true),
                _ => Field::new(*n, a.data_type().clone(), true),
            })
            .collect();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(fields)),
            arrays.into_iter().map(|(_, a)| a).collect(),
        )
        .map_err(|e| err(ErrorKind::Internal, format!("NMEA 2000 frames: {e}")))?;
        self.sink.append(batch)
    }
}

pub(crate) async fn read(path: &Path, out: &Path) -> Result<Extraction> {
    let bytes = std::fs::read(path).map_err(crate::sink::io_err(path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    // The format of the first line that is not a comment, as canboat detects it.
    let mut format: Option<InputFormat> = None;
    let mut frames = Vec::new();
    let mut unread = 0u64;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f = *format.get_or_insert_with(|| {
            canboat::codec::line::detect(line).unwrap_or(InputFormat::Plain)
        });
        match canboat::codec::line::parse(f, line) {
            Ok(Some(frame)) => frames.push(frame),
            Ok(None) => {}
            Err(_) => unread += 1,
        }
    }
    if frames.is_empty() {
        return Err(err(
            ErrorKind::RecordingUnreadable,
            format!("{}: no NMEA 2000 frame in it", path.display()),
        ));
    }

    let mut raw = Raw {
        sink: StreamSink::new(out, 0, FRAMES, "nmea2000", "nmea2000"),
        t: Vec::new(),
        prio: Vec::new(),
        pgn: Vec::new(),
        src: Vec::new(),
        dst: Vec::new(),
        data: ListBuilder::new(UInt8Builder::new()),
    };
    raw.sink.meta.decoded = true;
    let mut pgns: BTreeMap<u32, u64> = BTreeMap::new();
    for f in &frames {
        raw.push(f)?;
        *pgns.entry(f.pgn).or_default() += 1;
    }
    raw.flush()?;

    let mut rows = Rows::new(out, 1);
    let mut decoded: BTreeSet<u32> = BTreeSet::new();
    let mut unit_notes: BTreeMap<&str, String> = BTreeMap::new();
    for message in Decoder::new(Replay(frames.into_iter()), Database::embedded(Units::Si)) {
        let Ok(m) = message else { continue };
        let t = m.timestamp.as_deref().and_then(timestamp_ns).unwrap_or(0);
        decoded.insert(m.pgn);
        if !unit_notes.contains_key(m.id)
            && let Some(u) = units(&m)
        {
            unit_notes.insert(m.id, u);
        }
        rows.push(m.id, &format!("PGN {}", m.pgn), "nmea2000", t, row(&m))?;
    }
    let mut extraction = rows.finish(out).await?;
    for s in &mut extraction.streams {
        if let Some(u) = unit_notes.get(s.name.as_str()) {
            s.notes.push(u.clone());
        }
    }
    if let Some(meta) = raw.sink.finish(out)? {
        extraction.streams.insert(0, meta);
    }
    let not_decoded: Vec<String> = pgns
        .iter()
        .filter(|(pgn, _)| !decoded.contains(pgn))
        .map(|(pgn, n)| format!("{pgn} ({n})"))
        .collect();
    if !not_decoded.is_empty() {
        extraction.notes.push(format!(
            "frames of PGNs not decoded, kept in stream {FRAMES}: {}",
            not_decoded.join(", ")
        ));
    }
    if unread > 0 {
        extraction
            .notes
            .push(format!("{unread} line(s) not read as frames"));
    }
    Ok(extraction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::{Array, AsArray};
    use datafusion::arrow::datatypes::Float64Type;

    fn stream(out: &Path, x: &Extraction, name: &str) -> RecordBatch {
        let meta = x.streams.iter().find(|s| s.name == name).unwrap();
        let file = std::fs::File::open(out.join(&meta.dataset)).unwrap();
        let reader =
            parquet::arrow::arrow_reader::ParquetRecordBatchReader::try_new(file, 1 << 20).unwrap();
        let batches: Vec<RecordBatch> = reader.map(|b| b.unwrap()).collect();
        datafusion::arrow::compute::concat_batches(&batches[0].schema(), &batches).unwrap()
    }

    #[test]
    fn timestamps() {
        assert_eq!(
            timestamp_ns("2020-08-22T13:52:36.950Z"),
            Some(1_598_104_356_950_000_000)
        );
        assert_eq!(
            timestamp_ns("2011-11-24-22:42:04.388"),
            Some(1_322_174_524_388_000_000)
        );
        assert_eq!(timestamp_ns("13:52:36.950"), None);
    }

    /// Lines of canboat's samples/susteranna2020.raw: a wind message, and a B&G
    /// fast-packet message in 7 frames.
    #[tokio::test]
    async fn frames_and_decoded_messages() {
        let capture = "# PLAIN capture\n\
            2020-08-22T13:52:36.950Z,2,130306,33,255,8,b7,3a,04,28,9e,fa,ff,ff\n\
            2020-08-22T13:52:36.950Z,3,130824,27,255,8,a0,2e,7d,99,37,21,fa,3f\n\
            2020-08-22T13:52:36.950Z,3,130824,27,255,8,a1,38,21,34,04,7e,20,a9\n\
            2020-08-22T13:52:36.950Z,3,130824,27,255,8,a2,01,7c,20,00,00,32,21\n\
            2020-08-22T13:52:36.950Z,3,130824,27,255,8,a3,c0,e5,33,21,c5,22,3a\n\
            2020-08-22T13:52:36.950Z,3,130824,27,255,8,a4,21,00,00,1d,21,00,00\n\
            2020-08-22T13:52:36.950Z,3,130824,27,255,8,a5,50,21,61,96,51,21,ce\n\
            2020-08-22T13:52:36.950Z,3,130824,27,255,8,a6,08,52,21,32,f7,ff,ff\n\
            not a frame\n";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("boat.raw");
        std::fs::write(&path, capture).unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir_all(out.join("data")).unwrap();
        let x = read(&path, &out).await.unwrap();
        let names: Vec<&str> = x.streams.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["frames", "windData", "bGKeyValueData"]);
        assert_eq!(stream(&out, &x, FRAMES).num_rows(), 8);

        let w = stream(&out, &x, "windData");
        let f = |c: &str| {
            w.column_by_name(c)
                .unwrap()
                .as_primitive::<Float64Type>()
                .value(0)
        };
        // 1082 × 0.01 m/s and 40488 × 0.0001 rad, exact (canboat's JSON rounds them).
        assert_eq!(
            (f("windSpeed"), f("windAngle")),
            (1082.0 * 0.01, 40488.0 * 0.0001)
        );
        assert_eq!(
            w.column_by_name("reference")
                .unwrap()
                .as_string::<i32>()
                .value(0),
            "Apparent"
        );
        let meta = x.streams.iter().find(|s| s.name == "windData").unwrap();
        assert_eq!(
            (meta.type_.as_str(), meta.notes[0].as_str()),
            ("PGN 130306", "units: windSpeed [m/s], windAngle [rad]")
        );

        let bg = stream(&out, &x, "bGKeyValueData");
        assert_eq!(bg.num_rows(), 1, "seven frames, one message");
        assert!(!bg.column_by_name("list").unwrap().is_null(0));
        assert_eq!(x.notes, ["1 line(s) not read as frames"]);
    }
}
