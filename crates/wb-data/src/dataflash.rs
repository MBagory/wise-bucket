//! ArduPilot DataFlash logs (`.bin`), read by `dflog`: one stream per message type the
//! log defines (`ATT`, `IMU`, `GPS`, `PARM`, `MSG`…), values scaled as pymavlink does.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use datafusion::arrow::array::{
    ArrayRef, Float64Array, Int16Builder, Int64Array, ListBuilder, RecordBatch, StringArray,
    TimestampNanosecondArray, UInt64Array,
};
use datafusion::arrow::datatypes::{Field, Schema};
use dflog::access::Value;
use wb_core::error::{ErrorKind, Result, err};

use crate::columns::{T, time_type};
use crate::extract::{BATCH, Extraction, LogTally};
use crate::sink::StreamSink;

/// The microseconds since boot that time a record.
const TIME_US: &str = "TimeUS";

/// One column of a message type, typed by its first value.
enum Column {
    I64(Vec<Option<i64>>),
    U64(Vec<Option<u64>>),
    F64(Vec<Option<f64>>),
    Str(Vec<Option<String>>),
    Shorts(ListBuilder<Int16Builder>),
}

impl Column {
    fn new(v: &Value) -> Self {
        match v {
            Value::I64(_) => Self::I64(Vec::new()),
            Value::U64(_) => Self::U64(Vec::new()),
            Value::F64(_) => Self::F64(Vec::new()),
            Value::Str(_) => Self::Str(Vec::new()),
            Value::Shorts(_) => Self::Shorts(ListBuilder::new(Int16Builder::new())),
        }
    }

    /// Appends `v`, or a null when it is missing or of another type.
    fn push(&mut self, v: Option<Value>) {
        match (self, v) {
            (Self::I64(c), Some(Value::I64(x))) => c.push(Some(x)),
            (Self::U64(c), Some(Value::U64(x))) => c.push(Some(x)),
            (Self::F64(c), Some(Value::F64(x))) => c.push(Some(x)),
            (Self::Str(c), Some(Value::Str(x))) => c.push(Some(x)),
            (Self::Shorts(c), Some(Value::Shorts(x))) => {
                c.values().append_slice(&x);
                c.append(true);
            }
            (Self::I64(c), _) => c.push(None),
            (Self::U64(c), _) => c.push(None),
            (Self::F64(c), _) => c.push(None),
            (Self::Str(c), _) => c.push(None),
            (Self::Shorts(c), _) => c.append(false),
        }
    }

    fn finish(&mut self) -> ArrayRef {
        match self {
            Self::I64(c) => Arc::new(Int64Array::from(std::mem::take(c))),
            Self::U64(c) => Arc::new(UInt64Array::from(std::mem::take(c))),
            Self::F64(c) => Arc::new(Float64Array::from(std::mem::take(c))),
            Self::Str(c) => Arc::new(StringArray::from(std::mem::take(c))),
            Self::Shorts(c) => Arc::new(c.finish()),
        }
    }
}

/// The rows of one message type, one batch at a time.
struct Messages {
    sink: StreamSink,
    labels: Vec<String>,
    /// Created from the first record's values.
    columns: Vec<Option<Column>>,
    t: Vec<i64>,
}

impl Messages {
    fn push(&mut self, t_ns: i64, values: Vec<(&str, Value)>) -> Result<()> {
        self.t.push(t_ns);
        let mut by_label: HashMap<&str, Value> = values.into_iter().collect();
        for (i, label) in self.labels.iter().enumerate() {
            let v = by_label.remove(label.as_str());
            let column = self.columns[i].get_or_insert_with(|| {
                let mut c = v.as_ref().map_or(Column::F64(Vec::new()), Column::new);
                // Rows before this one had no value here.
                for _ in 1..self.t.len() {
                    c.push(None);
                }
                c
            });
            column.push(v);
        }
        if self.t.len() >= BATCH {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if self.t.is_empty() {
            return Ok(());
        }
        let mut fields = vec![Field::new(T, time_type(), true)];
        let mut arrays: Vec<ArrayRef> = vec![Arc::new(TimestampNanosecondArray::from(
            std::mem::take(&mut self.t),
        ))];
        for (label, column) in self.labels.iter().zip(&mut self.columns) {
            if let Some(c) = column {
                let a = c.finish();
                fields.push(Field::new(label, a.data_type().clone(), true));
                arrays.push(a);
            }
        }
        let batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays)
            .map_err(|e| err(ErrorKind::Internal, format!("{}: {e}", self.sink.meta.name)))?;
        self.sink.append(batch)
    }
}

pub(crate) async fn read(path: &Path, out: &Path) -> Result<Extraction> {
    let unreadable = |e: &dyn std::fmt::Display| {
        err(
            ErrorKind::RecordingUnreadable,
            format!("{}: {e}", path.display()),
        )
    };
    // ponytail: the whole log is read in memory (dflog without its memory map, which a
    // log still being written would break); stream it if logs of several GB show up.
    let log = dflog::Log::open(path).map_err(|e| unreadable(&e))?;
    let units = log.units();
    let mut extraction = Extraction::default();
    let mut streams: Vec<Messages> = Vec::new();
    let mut by_type: HashMap<String, usize> = HashMap::new();
    let mut errors = LogTally::default();
    // With a GPS fix, `t` is the UTC time of boot plus `TimeUS`; without one, time since boot.
    // Whole microseconds first: nanoseconds since 1970 are beyond an f64's exact integers.
    let boot_ns = log
        .time_base()
        .map(|base| ((base.wall_clock_unix_ms(0.0) * 1e3).round() as i64).saturating_mul(1000));
    if let Some(ns) = boot_ns {
        let boot = chrono::DateTime::from_timestamp_nanos(ns);
        extraction.properties.push((
            "boot_utc".into(),
            boot.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        ));
    }
    // Records without a time take the last time before them, as pymavlink does.
    let mut last_us: u64 = 0;
    // The recording starts at its first timed record, not at boot: format and
    // parameter records written before it carry no time.
    let mut first_us: Option<u64> = None;
    for record in log.records() {
        let name = record.type_name().to_string();
        let values = record.values();
        if let Some(us) = values
            .iter()
            .find(|(f, _)| *f == TIME_US)
            .and_then(|(_, v)| match v {
                Value::U64(x) => Some(*x),
                Value::I64(x) => u64::try_from(*x).ok(),
                _ => None,
            })
        {
            last_us = us;
            first_us.get_or_insert(us);
        }
        let t_ns = i64::try_from(last_us)
            .unwrap_or(i64::MAX)
            .saturating_mul(1000)
            .saturating_add(boot_ns.unwrap_or(0));
        match name.as_str() {
            // ECode 0 means the error was resolved.
            "ERR" => {
                let get = |f: &str| {
                    values
                        .iter()
                        .find(|(l, _)| *l == f)
                        .and_then(|(_, v)| v.as_f64())
                };
                if let (Some(subsys), Some(code)) = (get("Subsys"), get("ECode"))
                    && code != 0.0
                {
                    errors.add(3, "ERR", true, &format!("Subsys {subsys} ECode {code}"));
                }
            }
            "VER" => {
                for (f, v) in &values {
                    if *f != TIME_US {
                        let v = v.as_str().map_or_else(
                            || v.as_f64().map_or(String::new(), |x| x.to_string()),
                            str::to_string,
                        );
                        extraction.properties.push((format!("VER.{f}"), v));
                    }
                }
            }
            "MSG" if !extraction.properties.iter().any(|(k, _)| k == "firmware") => {
                // The first MSG lines after boot name the vehicle and firmware.
                if let Some(text) = values.iter().find_map(|(_, v)| v.as_str())
                    && text.starts_with("Ardu")
                {
                    extraction
                        .properties
                        .push(("firmware".into(), text.to_string()));
                }
            }
            _ => {}
        }
        let i = match by_type.get(&name) {
            Some(&i) => i,
            None => {
                let type_id = log.name_to_id.get(&name).copied();
                let fmt = type_id.and_then(|id| log.fmts.get(&id));
                let labels: Vec<String> = fmt
                    .map(|f| f.labels.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|l| l != TIME_US)
                    .collect();
                let mut sink = StreamSink::new(out, streams.len(), &name, "dataflash", "dataflash");
                sink.meta.decoded = true;
                if let (Some(id), Some(f)) = (type_id, fmt) {
                    let described: Vec<String> = f
                        .labels
                        .iter()
                        .enumerate()
                        .filter(|(_, l)| l.as_str() != TIME_US)
                        .filter_map(|(i, l)| {
                            let meta = units.field_meta(id, i);
                            let unit = meta.unit?;
                            // Format-scaled fields (c, C, e, E, L) are already in their unit.
                            let scaled = matches!(
                                f.format.as_bytes().get(i),
                                Some(b'c' | b'C' | b'e' | b'E' | b'L')
                            );
                            Some(match meta.multiplier {
                                Some(m) if m != 1.0 && !scaled => format!("{l} [{unit} ×{m}]"),
                                _ => format!("{l} [{unit}]"),
                            })
                        })
                        .collect();
                    if !described.is_empty() {
                        sink.note(format!("units: {}", described.join(", ")));
                    }
                }
                let n = labels.len();
                streams.push(Messages {
                    sink,
                    labels,
                    columns: (0..n).map(|_| None).collect(),
                    t: Vec::new(),
                });
                by_type.insert(name.clone(), streams.len() - 1);
                streams.len() - 1
            }
        };
        streams[i].push(t_ns, values)?;
    }
    extraction.declared_start_ns = first_us.map(|us| {
        i64::try_from(us)
            .unwrap_or(i64::MAX)
            .saturating_mul(1000)
            .saturating_add(boot_ns.unwrap_or(0))
    });
    for mut s in streams {
        s.flush()?;
        if let Some(meta) = s.sink.finish(out)? {
            extraction.streams.push(meta);
        }
    }
    extraction.notes.extend(errors.note("ERR"));
    if log.stats.truncated {
        extraction
            .notes
            .push("the file is cut or damaged: its valid part was extracted".into());
    }
    if log.stats.unknown_type_records > 0 {
        extraction.notes.push(format!(
            "{} record(s) of a type the log does not define were skipped",
            log.stats.unknown_type_records
        ));
    }
    Ok(extraction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::AsArray;
    use datafusion::arrow::datatypes::{Float64Type, TimestampNanosecondType};
    use dflog::write::LogWriter;

    fn log() -> Vec<u8> {
        let mut w = LogWriter::new();
        w.define(10, "ATT", "QccC", &["TimeUS", "Roll", "Pitch", "Yaw"])
            .unwrap();
        w.define(11, "ERR", "QBB", &["TimeUS", "Subsys", "ECode"])
            .unwrap();
        w.define(12, "MSG", "QZ", &["TimeUS", "Message"]).unwrap();
        w.record(
            "MSG",
            &[
                Value::U64(1000),
                Value::Str("ArduCopter V4.5.0 (abc)".into()),
            ],
        )
        .unwrap();
        for i in 0..2000u64 {
            let at = Value::U64(2000 + i * 100);
            w.record(
                "ATT",
                &[at, Value::F64(1.25), Value::F64(-2.5), Value::F64(90.0)],
            )
            .unwrap();
        }
        // ECode 0: the error was resolved.
        w.record("ERR", &[Value::U64(300_000), Value::I64(16), Value::I64(2)])
            .unwrap();
        w.record("ERR", &[Value::U64(300_100), Value::I64(16), Value::I64(0)])
            .unwrap();
        w.into_bytes()
    }

    fn stream(out: &Path, x: &Extraction, name: &str) -> RecordBatch {
        let meta = x.streams.iter().find(|s| s.name == name).unwrap();
        let file = std::fs::File::open(out.join(&meta.dataset)).unwrap();
        let reader =
            parquet::arrow::arrow_reader::ParquetRecordBatchReader::try_new(file, 1 << 20).unwrap();
        let batches: Vec<RecordBatch> = reader.map(|b| b.unwrap()).collect();
        datafusion::arrow::compute::concat_batches(&batches[0].schema(), &batches).unwrap()
    }

    #[tokio::test]
    async fn messages_errors_and_a_cut_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flight.bin");
        let bytes = log();
        std::fs::write(&path, &bytes).unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir_all(out.join("data")).unwrap();
        let x = read(&path, &out).await.unwrap();
        let att = stream(&out, &x, "ATT");
        assert_eq!(att.num_rows(), 2000);
        let t = att
            .column_by_name(T)
            .unwrap()
            .as_primitive::<TimestampNanosecondType>();
        assert_eq!(t.value(1), 2_100_000);
        let roll = att
            .column_by_name("Roll")
            .unwrap()
            .as_primitive::<Float64Type>();
        assert_eq!(roll.value(0), 1.25);
        assert!(att.column_by_name("TimeUS").is_none());
        assert_eq!(
            x.properties,
            [(
                "firmware".to_string(),
                "ArduCopter V4.5.0 (abc)".to_string()
            )]
        );
        assert_eq!(
            x.notes,
            ["1 ERR log messages (stream ERR), first: \"Subsys 16 ECode 2\""]
        );

        // Cut in the middle of the 1001st ATT record (17 bytes each, then two ERR of 13), as
        // by a power loss while logging.
        let att_start = bytes.len() - 2000 * 17 - 2 * 13;
        let cut = dir.path().join("cut.bin");
        std::fs::write(&cut, &bytes[..att_start + 1000 * 17 + 5]).unwrap();
        let out = dir.path().join("cut");
        std::fs::create_dir_all(out.join("data")).unwrap();
        let x = read(&cut, &out).await.unwrap();
        assert_eq!(stream(&out, &x, "ATT").num_rows(), 1000);
        assert_eq!(
            x.notes,
            ["the file is cut or damaged: its valid part was extracted"]
        );
    }

    #[tokio::test]
    async fn a_gps_fix_puts_t_in_utc() {
        let mut w = LogWriter::new();
        w.define(10, "ATT", "QccC", &["TimeUS", "Roll", "Pitch", "Yaw"])
            .unwrap();
        w.define(13, "GPS", "QBIH", &["TimeUS", "Status", "GMS", "GWk"])
            .unwrap();
        let zero = Value::F64(0.0);
        w.record(
            "ATT",
            &[Value::U64(1_000_000), zero.clone(), zero.clone(), zero],
        )
        .unwrap();
        // A 3D fix 2 s after boot: GPS week 2300, 1 s into the week.
        w.record(
            "GPS",
            &[
                Value::U64(2_000_000),
                Value::I64(3),
                Value::I64(1000),
                Value::I64(2300),
            ],
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gps.bin");
        std::fs::write(&path, w.into_bytes()).unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir_all(out.join("data")).unwrap();
        let x = read(&path, &out).await.unwrap();
        // The GPS epoch (1980-01-06) in Unix ms, 18 leap seconds in 2300, 2 s since boot.
        let boot_ms: i64 = 315_964_800_000 + 2300 * 604_800_000 + 1000 - 18_000 - 2000;
        // The log starts at its first timed record (ATT, 1 s after boot); boot is a property.
        assert_eq!(x.declared_start_ns, Some((boot_ms + 1000) * 1_000_000));
        assert!(
            x.properties
                .iter()
                .any(|(k, v)| k == "boot_utc" && v.ends_with('Z')),
            "{:?}",
            x.properties
        );
        let att = stream(&out, &x, "ATT");
        let t = att
            .column_by_name(T)
            .unwrap()
            .as_primitive::<TimestampNanosecondType>();
        assert_eq!(t.value(0), (boot_ms + 1000) * 1_000_000);
    }
}
