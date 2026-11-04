//! Records as the file holds them. A decoded row keeps its own in a `raw` column; the
//! records that gave no decoded row (lines that are no frame, bytes that are no message)
//! go to the stream `raw`. Together they hold every line, or every byte, of the file.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use datafusion::arrow::array::{
    ArrayRef, LargeBinaryArray, RecordBatch, StringArray, TimestampNanosecondArray, UInt64Array,
};
use datafusion::arrow::datatypes::{Field, FieldRef, Schema};
use wb_core::error::{ErrorKind, Result, err};

use crate::columns::{T, time_type};
use crate::sink::{StreamMeta, StreamSink};

/// The stream of records with no decoded row, and the column of each decoded row's record.
pub const RAW: &str = "raw";

/// `batch` with each row's record appended as the column `raw`.
pub(crate) fn with_raw(batch: RecordBatch, raw: ArrayRef) -> Result<RecordBatch> {
    let mut fields: Vec<FieldRef> = batch.schema().fields().iter().cloned().collect();
    fields.push(Arc::new(Field::new(RAW, raw.data_type().clone(), true)));
    let mut columns = batch.columns().to_vec();
    columns.push(raw);
    RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)
        .map_err(|e| err(ErrorKind::Internal, format!("raw column: {e}")))
}

/// Records with no decoded row, written once the recording's other streams are numbered.
// ponytail: kept in memory, as such records are few; spill like `StreamSink` if files made
// mostly of unread records show up.
#[derive(Default)]
pub(crate) struct RawRecords {
    t: Vec<Option<i64>>,
    position: Vec<u64>,
    kind: Vec<String>,
    text: Vec<Option<String>>,
    bytes: Vec<Option<Vec<u8>>>,
}

impl RawRecords {
    /// Line `number` (from 1), with no decoded row.
    pub fn line(&mut self, t: Option<i64>, number: u64, kind: &str, line: &str) {
        self.push(t, number, kind, Some(line.to_string()), None);
    }

    /// Bytes at `offset` in the file, with no decoded row.
    pub fn bytes(&mut self, t: Option<i64>, offset: u64, kind: &str, bytes: &[u8]) {
        self.push(t, offset, kind, None, Some(bytes.to_vec()));
    }

    fn push(
        &mut self,
        t: Option<i64>,
        position: u64,
        kind: &str,
        text: Option<String>,
        bytes: Option<Vec<u8>>,
    ) {
        self.t.push(t);
        self.position.push(position);
        self.kind.push(kind.to_string());
        self.text.push(text);
        self.bytes.push(bytes);
    }

    /// `20 line(s) not read as frames, kept in stream raw: header 7, Statistic 6, …` (`what`
    /// is `line(s) not read as frames`), most frequent kind first.
    pub fn note(&self, what: &str) -> Option<String> {
        if self.kind.is_empty() {
            return None;
        }
        let mut counts: BTreeMap<&str, u64> = BTreeMap::new();
        for k in &self.kind {
            *counts.entry(k).or_default() += 1;
        }
        let mut counts: Vec<(&str, u64)> = counts.into_iter().collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let kinds: Vec<String> = counts.iter().map(|(k, n)| format!("{k} {n}")).collect();
        Some(format!(
            "{} {what}, kept in stream {RAW}: {}",
            self.kind.len(),
            kinds.join(", ")
        ))
    }

    /// Writes the stream as dataset `index` (`text` for lines, `bytes` for binary records).
    pub fn finish(self, out: &Path, index: usize, encoding: &str) -> Result<Option<StreamMeta>> {
        if self.kind.is_empty() {
            return Ok(None);
        }
        let mut sink = StreamSink::new(out, index, RAW, "raw", encoding);
        // Records of every kind, at no rate: the timing checks would only be noise.
        sink.checked = false;
        let mut arrays: Vec<(&str, ArrayRef)> = vec![
            (T, Arc::new(TimestampNanosecondArray::from(self.t))),
            ("position", Arc::new(UInt64Array::from(self.position))),
            ("kind", Arc::new(StringArray::from(self.kind))),
        ];
        if self.text.iter().any(Option::is_some) {
            arrays.push(("text", Arc::new(StringArray::from(self.text))));
        }
        if self.bytes.iter().any(Option::is_some) {
            arrays.push(("bytes", Arc::new(LargeBinaryArray::from_iter(self.bytes))));
        }
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
        .map_err(|e| err(ErrorKind::Internal, format!("raw records: {e}")))?;
        sink.append(batch)?;
        sink.finish(out)
    }
}
