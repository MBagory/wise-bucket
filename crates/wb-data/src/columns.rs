//! Column shaping shared by every reader: the time columns, nested message fields,
//! dropped byte arrays, and the dotted names under which nested fields are queried.
//!
//! Streams are stored with their structs nested (Lance column names cannot contain
//! dots); `stream()` flattens them at query time into `linear_acceleration.z`.

use std::sync::Arc;

use datafusion::arrow::array::{
    Array, ArrayRef, AsArray, RecordBatch, RecordBatchOptions, StructArray,
    TimestampNanosecondArray, UInt32Array,
};
use datafusion::arrow::datatypes::{DataType, Field, FieldRef, Fields, Schema, TimeUnit};

/// Receive (or main) time of each row, nanoseconds since the Unix epoch (or since boot).
pub const T: &str = "t";
/// `header.stamp` of ROS 2 messages, when present.
pub const T_SENSOR: &str = "t_sensor";
/// Size of each message on the wire, in bytes (bag and MCAP recordings).
pub const MESSAGE_BYTES: &str = "message_bytes";

const RESERVED: &[&str] = &[T, T_SENSOR, MESSAGE_BYTES, "recording"];

/// Byte arrays longer than this on average (images, blobs) are dropped from decoded messages.
const MAX_BYTE_ARRAY_LEN: usize = 64;

/// The Arrow type of time columns.
pub fn time_type() -> DataType {
    DataType::Timestamp(TimeUnit::Nanosecond, None)
}

/// Whether a column holds byte arrays longer than [`MAX_BYTE_ARRAY_LEN`] on average.
fn is_large_byte_array(array: &ArrayRef) -> bool {
    let rows = array.len().max(1);
    match array.data_type() {
        DataType::Binary | DataType::LargeBinary => true,
        DataType::List(f) | DataType::LargeList(f) | DataType::FixedSizeList(f, _)
            if matches!(f.data_type(), DataType::UInt8 | DataType::Int8) =>
        {
            let values = match array.data_type() {
                DataType::List(_) => array.as_list::<i32>().values().len(),
                DataType::LargeList(_) => array.as_list::<i64>().values().len(),
                _ => array.as_fixed_size_list().values().len(),
            };
            values / rows > MAX_BYTE_ARRAY_LEN
        }
        _ => false,
    }
}

/// Drops large byte arrays, inside structs too. `None` if nothing is left.
fn prune(
    path: &str,
    field: &FieldRef,
    array: &ArrayRef,
    dropped: &mut Vec<String>,
) -> Option<(FieldRef, ArrayRef)> {
    if is_large_byte_array(array) {
        dropped.push(path.to_string());
        return None;
    }
    let Some(s) = array.as_struct_opt() else {
        return Some((field.clone(), array.clone()));
    };
    let mut fields = Vec::new();
    let mut columns = Vec::new();
    for (f, c) in s.fields().iter().zip(s.columns()) {
        if let Some((f, c)) = prune(&format!("{path}.{}", f.name()), f, c, dropped) {
            fields.push(f);
            columns.push(c);
        }
    }
    if fields.is_empty() {
        return None;
    }
    let fields = Fields::from(fields);
    let pruned = StructArray::new(fields.clone(), columns, s.nulls().cloned());
    Some((
        Arc::new(Field::new(field.name(), DataType::Struct(fields), true)),
        Arc::new(pruned),
    ))
}

/// Builds a stream batch: `t`, optional `t_sensor` and `message_bytes`, then the message
/// fields (structs kept nested). Large byte arrays are dropped; their names are returned.
pub fn assemble(
    t: Vec<i64>,
    t_sensor: Option<ArrayRef>,
    message_bytes: Option<Vec<u32>>,
    fields: Option<&StructArray>,
) -> (RecordBatch, Vec<String>) {
    let rows = t.len();
    let mut cols: Vec<(FieldRef, ArrayRef)> = vec![(
        Arc::new(Field::new(T, time_type(), true)),
        Arc::new(TimestampNanosecondArray::from(t)),
    )];
    if let Some(ts) = t_sensor {
        cols.push((Arc::new(Field::new(T_SENSOR, time_type(), true)), ts));
    }
    if let Some(sizes) = message_bytes {
        cols.push((
            Arc::new(Field::new(MESSAGE_BYTES, DataType::UInt32, true)),
            Arc::new(UInt32Array::from(sizes)),
        ));
    }
    let mut dropped = Vec::new();
    if let Some(s) = fields {
        for (f, c) in s.fields().iter().zip(s.columns()) {
            let Some((f, c)) = prune(f.name(), f, c, &mut dropped) else {
                continue;
            };
            let f = if RESERVED.contains(&f.name().as_str()) {
                Arc::new(Field::new(
                    format!("{}_field", f.name()),
                    f.data_type().clone(),
                    true,
                ))
            } else {
                f
            };
            cols.push((f, c));
        }
    }
    let (fields, arrays): (Vec<_>, Vec<_>) = cols.into_iter().unzip();
    let batch = RecordBatch::try_new_with_options(
        Arc::new(Schema::new(fields)),
        arrays,
        &RecordBatchOptions::new().with_row_count(Some(rows)),
    )
    .expect("columns built with the same row count");
    (batch, dropped)
}

/// A leaf column as queried: its dotted name, the path of struct fields leading to it,
/// and its type.
#[derive(Debug, Clone, PartialEq)]
pub struct FlatColumn {
    pub name: String,
    pub path: Vec<String>,
    pub data_type: DataType,
}

/// The leaf columns of a schema, structs flattened (`linear_acceleration.z`).
pub fn flat_columns(schema: &Schema) -> Vec<FlatColumn> {
    fn walk(path: &mut Vec<String>, field: &Field, out: &mut Vec<FlatColumn>) {
        path.push(field.name().clone());
        match field.data_type() {
            DataType::Struct(children) => {
                for c in children {
                    walk(path, c, out);
                }
            }
            dt => out.push(FlatColumn {
                name: path.join("."),
                path: path.clone(),
                data_type: dt.clone(),
            }),
        }
        path.pop();
    }
    let mut out = Vec::new();
    for f in schema.fields() {
        walk(&mut Vec::new(), f, &mut out);
    }
    out
}

/// How a column becomes the time column `t`.
#[derive(Debug, Clone, PartialEq)]
pub enum TimeSource {
    /// An Arrow timestamp (or date) column.
    Timestamp(String),
    /// A numeric column with `per_second` units per second (1 for s, 1e3 for ms…).
    Numeric { column: String, per_second: f64 },
    /// A text column holding dates and times.
    Text(String),
}

impl TimeSource {
    pub fn column(&self) -> &str {
        match self {
            Self::Timestamp(c) | Self::Text(c) => c,
            Self::Numeric { column, .. } => column,
        }
    }
}

/// Finds the time column of a table: the first timestamp column, else a column named
/// `time`, `timestamp`, `t` or `stamp` (any case), optionally suffixed with its unit
/// (`_s`, `_ms`, `_us`, `_ns`; seconds by default).
pub fn detect_time(schema: &Schema) -> Option<TimeSource> {
    if let Some(f) = schema
        .fields()
        .iter()
        .find(|f| matches!(f.data_type(), DataType::Timestamp(..) | DataType::Date64))
    {
        return Some(TimeSource::Timestamp(f.name().clone()));
    }
    const BASES: [&str; 4] = ["time", "timestamp", "t", "stamp"];
    const UNITS: [(&str, f64); 4] = [("s", 1.0), ("ms", 1e3), ("us", 1e6), ("ns", 1e9)];
    for f in schema.fields() {
        let lower = f.name().to_ascii_lowercase();
        let (base, per_second) = match lower.rsplit_once('_') {
            Some((b, u)) => match UNITS.iter().find(|(n, _)| *n == u) {
                Some((_, s)) => (b, *s),
                None => (lower.as_str(), 1.0),
            },
            None => (lower.as_str(), 1.0),
        };
        if !BASES.contains(&base) {
            continue;
        }
        if f.data_type().is_numeric() {
            return Some(TimeSource::Numeric {
                column: f.name().clone(),
                per_second,
            });
        }
        if matches!(
            f.data_type(),
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
        ) {
            return Some(TimeSource::Text(f.name().clone()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::{Float64Array, Int32Array};

    #[test]
    fn keeps_structs_drops_byte_arrays_and_flattens_names() {
        let inner = StructArray::from(vec![(
            Arc::new(Field::new("z", DataType::Float64, false)),
            Arc::new(Float64Array::from(vec![1.0, 2.0])) as ArrayRef,
        )]);
        let bytes = datafusion::arrow::array::ListArray::from_iter_primitive::<
            datafusion::arrow::datatypes::UInt8Type,
            _,
            _,
        >(vec![Some(vec![Some(0u8); 100]), Some(vec![Some(1u8); 100])]);
        let imu = StructArray::from(vec![
            (
                Arc::new(Field::new("acc", inner.data_type().clone(), true)),
                Arc::new(inner) as ArrayRef,
            ),
            (
                Arc::new(Field::new("blob", bytes.data_type().clone(), true)),
                Arc::new(bytes) as ArrayRef,
            ),
        ]);
        let msg = StructArray::from(vec![
            (
                Arc::new(Field::new("imu", imu.data_type().clone(), true)),
                Arc::new(imu) as ArrayRef,
            ),
            (
                Arc::new(Field::new("t", DataType::Int32, false)),
                Arc::new(Int32Array::from(vec![7, 8])) as ArrayRef,
            ),
        ]);
        let (batch, dropped) = assemble(vec![10, 20], None, Some(vec![3, 4]), Some(&msg));
        assert_eq!(dropped, ["imu.blob"]);
        let names: Vec<_> = flat_columns(&batch.schema())
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["t", "message_bytes", "imu.acc.z", "t_field"]);
        let top: Vec<_> = batch
            .schema()
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .collect();
        assert_eq!(top, ["t", "message_bytes", "imu", "t_field"]);
    }

    #[test]
    fn time_columns_are_found() {
        let s = |fields: Vec<Field>| Schema::new(fields);
        let num = |n: &str| Field::new(n, DataType::Float64, true);
        assert_eq!(
            detect_time(&s(vec![num("x"), num("Time_ms")])),
            Some(TimeSource::Numeric {
                column: "Time_ms".into(),
                per_second: 1e3
            })
        );
        assert_eq!(
            detect_time(&s(vec![num("t")])),
            Some(TimeSource::Numeric {
                column: "t".into(),
                per_second: 1.0
            })
        );
        assert_eq!(
            detect_time(&s(vec![Field::new("timestamp", DataType::Utf8, true)])),
            Some(TimeSource::Text("timestamp".into()))
        );
        assert_eq!(
            detect_time(&s(vec![num("x"), Field::new("when", time_type(), true)])),
            Some(TimeSource::Timestamp("when".into()))
        );
        assert_eq!(detect_time(&s(vec![num("x"), num("start_time")])), None);
    }
}
