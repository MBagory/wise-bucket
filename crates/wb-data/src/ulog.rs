//! PX4 ULog files: one stream per topic instance (`sensor_accel`, `sensor_accel.1`…).

use std::path::Path;
use std::sync::Arc;

use datafusion::arrow::array::{
    ArrayRef, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array,
    Int64Array, RecordBatch, TimestampNanosecondArray, UInt8Array, UInt16Array, UInt32Array,
    UInt64Array,
};
use datafusion::arrow::datatypes::{Field, Schema};
use px4_ulog::full_parser::{Completeness, SomeVec};
use wb_core::error::{ErrorKind, Result, err};

use crate::columns::{T, time_type};
use crate::extract::Extraction;
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
    for (index, (name, mut fields)) in streams.into_iter().enumerate() {
        let mut sink = StreamSink::new(out, index, &name, "ulog", "ulog");
        sink.meta.decoded = true;
        let Some(SomeVec::UInt64(stamps)) = fields.remove("timestamp") else {
            sink.note("no timestamp field: left out");
            continue;
        };
        let rows = stamps.len();
        let t: Vec<i64> = stamps
            .iter()
            .map(|us| (*us as i64).saturating_mul(1000))
            .collect();
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
    Ok(extraction)
}
