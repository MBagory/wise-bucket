//! ASAM MDF 4 files (`.mf4`), read by `falcon_mdf`: one stream per channel group, with
//! physical values; `t` is the file's start time plus the group's master channel.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use datafusion::arrow::array::{
    ArrayRef, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array,
    RecordBatch, StringArray, TimestampNanosecondArray, UInt8Array, UInt16Array, UInt32Array,
    UInt64Array,
};
use datafusion::arrow::datatypes::{Field, Schema};
use falcon_mdf::blocks::channel::SyncType;
use falcon_mdf::{Mf4File, SignalValues};
use wb_core::error::{ErrorKind, Result, err};

use crate::columns::{T, time_type};
use crate::extract::Extraction;
use crate::sink::StreamSink;

/// A column of physical values; `None` for byte channels (bus frames, blobs).
fn array(v: SignalValues) -> Option<ArrayRef> {
    Some(match v {
        SignalValues::U8(x) => Arc::new(UInt8Array::from(x)),
        SignalValues::U16(x) => Arc::new(UInt16Array::from(x)),
        SignalValues::U32(x) => Arc::new(UInt32Array::from(x)),
        SignalValues::U64(x) => Arc::new(UInt64Array::from(x)),
        SignalValues::I8(x) => Arc::new(Int8Array::from(x)),
        SignalValues::I16(x) => Arc::new(Int16Array::from(x)),
        SignalValues::I32(x) => Arc::new(Int32Array::from(x)),
        SignalValues::I64(x) => Arc::new(Int64Array::from(x)),
        SignalValues::F32(x) => Arc::new(Float32Array::from(x)),
        SignalValues::F64(x) => Arc::new(Float64Array::from(x)),
        SignalValues::Str(x) => Arc::new(StringArray::from(x)),
        _ => return None,
    })
}

/// Seconds as f64 values, for a master channel.
fn seconds(v: &SignalValues) -> Option<Vec<f64>> {
    Some(match v {
        SignalValues::F64(x) => x.clone(),
        SignalValues::F32(x) => x.iter().map(|&s| f64::from(s)).collect(),
        SignalValues::U64(x) => x.iter().map(|&s| s as f64).collect(),
        SignalValues::U32(x) => x.iter().map(|&s| f64::from(s)).collect(),
        SignalValues::I64(x) => x.iter().map(|&s| s as f64).collect(),
        SignalValues::I32(x) => x.iter().map(|&s| f64::from(s)).collect(),
        _ => return None,
    })
}

pub(crate) async fn read(path: &Path, out: &Path) -> Result<Extraction> {
    let unreadable = |e: &dyn std::fmt::Display| {
        err(
            ErrorKind::RecordingUnreadable,
            format!("{}: {e}", path.display()),
        )
    };
    // Buffered reads: no memory map of a file a logger may still be writing.
    let file = Mf4File::open_buffered(path).map_err(|e| unreadable(&e))?;
    let start_ns = file.start_time().timestamp_ns;
    let mut extraction = Extraction::default();
    let meta = file.metadata();
    if !meta.text().is_empty() {
        extraction
            .properties
            .push(("comment".into(), meta.text().to_string()));
    }
    for (k, v) in meta.properties() {
        extraction
            .properties
            .push((format!("HD.{k}"), v.to_string()));
    }

    let mut names: HashSet<String> = HashSet::new();
    let mut index = 0;
    for dg in file.data_groups() {
        for cg in &dg.channel_groups {
            if cg.sample_count == 0 {
                continue;
            }
            let mut name = if cg.acquisition_name.is_empty() {
                format!("group_{}", index + 1)
            } else {
                cg.acquisition_name.clone()
            };
            if !names.insert(name.clone()) {
                name = format!("{name} ({})", index + 1);
                names.insert(name.clone());
            }
            let mut sink = StreamSink::new(out, index, &name, "mdf4", "mdf4");
            sink.meta.decoded = true;
            let master = file.master_channel(dg.index, cg.index);
            let signals = file.signals(&cg.channels).map_err(|e| unreadable(&e))?;
            let rows = cg.sample_count as usize;
            let mut t: Vec<Option<i64>> = vec![None; rows];
            let mut fields = vec![Field::new(T, time_type(), true)];
            let mut arrays: Vec<ArrayRef> = Vec::new();
            let (mut skipped, mut units) = (Vec::new(), Vec::new());
            // A group may hold two channels of one name: the second gets ` (2)`.
            let mut columns_seen: HashSet<String> = HashSet::new();
            for (channel, signal) in cg.channels.iter().zip(signals) {
                let values = signal.values().map_err(|e| unreadable(&e))?;
                // A time master gives t; an angle, distance or index master stays a column.
                if master.is_some_and(|m| m.id == channel.id) {
                    if channel.sync_type == SyncType::Time
                        && let Some(s) = seconds(&values).filter(|s| s.len() == rows)
                    {
                        for (slot, s) in t.iter_mut().zip(s) {
                            *slot = Some(start_ns.saturating_add((s * 1e9).round() as i64));
                        }
                        continue;
                    }
                    sink.note(format!(
                        "the master channel {} is not a time ({:?}): t is empty, the master is a column",
                        channel.name, channel.sync_type
                    ));
                }
                match array(values) {
                    Some(a) if a.len() == rows => {
                        let mut column = channel.name.clone();
                        let mut n = 1;
                        while !columns_seen.insert(column.clone()) {
                            n += 1;
                            column = format!("{} ({n})", channel.name);
                        }
                        if !channel.unit.is_empty() {
                            units.push(format!("{column} [{}]", channel.unit));
                        }
                        fields.push(Field::new(&column, a.data_type().clone(), true));
                        arrays.push(a);
                    }
                    _ => skipped.push(channel.name.clone()),
                }
            }
            if master.is_none() {
                sink.note("no master channel: t is empty");
            }
            if !units.is_empty() {
                sink.note(format!("units: {}", units.join(", ")));
            }
            if !skipped.is_empty() {
                sink.note(format!(
                    "byte channels left out (bus frames are not decoded): {}",
                    skipped.join(", ")
                ));
            }
            let mut columns: Vec<ArrayRef> = vec![Arc::new(TimestampNanosecondArray::from(t))];
            columns.extend(arrays);
            let batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)
                .map_err(|e| unreadable(&e))?;
            sink.append(batch)?;
            if let Some(meta) = sink.finish(out)? {
                extraction.streams.push(meta);
            }
            index += 1;
        }
    }
    Ok(extraction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::AsArray;
    use datafusion::arrow::datatypes::{Float64Type, TimestampNanosecondType};

    #[tokio::test]
    async fn groups_times_and_units() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bench.mf4");
        let start = 1_700_000_000_000_000_000;
        let mut w = falcon_mdf::Mf4Writer::with_start_time_ns(start);
        let g = w.add_group(&[0.0, 0.5, 1.0]).unwrap();
        g.add_channel("Speed", "km/h", &[0.0, 5.0, 10.0]).unwrap();
        g.add_channel("Speed", "km/h", &[1.0, 6.0, 11.0]).unwrap();
        g.add_channel("Torque", "Nm", &[1.5, 2.5, 3.5]).unwrap();
        w.write_to_file(&path).unwrap();

        let out = dir.path().join("out");
        std::fs::create_dir_all(out.join("data")).unwrap();
        let x = read(&path, &out).await.unwrap();
        assert_eq!(x.streams.len(), 1);
        let meta = &x.streams[0];
        assert_eq!(meta.name, "group_1");
        assert_eq!(
            meta.notes,
            ["units: Speed [km/h], Speed (2) [km/h], Torque [Nm]"]
        );
        let file = std::fs::File::open(out.join(&meta.dataset)).unwrap();
        let b = parquet::arrow::arrow_reader::ParquetRecordBatchReader::try_new(file, 1024)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let t = b
            .column_by_name(T)
            .unwrap()
            .as_primitive::<TimestampNanosecondType>();
        assert_eq!(
            t.values(),
            &[start, start + 500_000_000, start + 1_000_000_000]
        );
        let f = |c: &str| {
            b.column_by_name(c)
                .unwrap()
                .as_primitive::<Float64Type>()
                .values()
                .to_vec()
        };
        assert_eq!(
            (f("Speed"), f("Speed (2)")),
            (vec![0.0, 5.0, 10.0], vec![1.0, 6.0, 11.0])
        );
        assert_eq!(f("Torque"), [1.5, 2.5, 3.5]);
    }
}
