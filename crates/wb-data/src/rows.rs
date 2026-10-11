//! Streams built from JSON-like rows (MAVLink messages…): each stream's columns are
//! inferred from its first batch by Arrow's JSON decoder; arrays stay arrays.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::json::reader::{Decoder, ReaderBuilder, infer_json_schema_from_iterator};
use serde_json::{Map, Value};
use wb_core::error::{ErrorKind, Result, err};

use crate::columns::{T, time_type};
use crate::extract::{BATCH, Extraction};
use crate::sink::StreamSink;

struct Stream {
    sink: StreamSink,
    rows: Vec<Value>,
    decoder: Option<Decoder>,
}

pub(crate) struct Rows {
    dir: PathBuf,
    streams: Vec<Stream>,
    by_name: HashMap<String, usize>,
}

/// An object holding only `type` (how `serde` tags a unit enum) becomes its name.
fn simplify(v: Value) -> Value {
    match v {
        Value::Object(mut m) if m.len() == 1 && m.get("type").is_some_and(Value::is_string) => {
            m.remove("type").expect("checked above")
        }
        Value::Array(a) => Value::Array(a.into_iter().map(simplify).collect()),
        v => v,
    }
}

impl Rows {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            streams: Vec::new(),
            by_name: HashMap::new(),
        }
    }

    /// Adds a row to stream `name` (created with `type_` and `encoding` on first use).
    pub fn push(
        &mut self,
        name: &str,
        type_: &str,
        encoding: &str,
        t_ns: i64,
        fields: Map<String, Value>,
    ) -> Result<()> {
        let index = match self.by_name.get(name) {
            Some(&i) => i,
            None => {
                let i = self.streams.len();
                let mut sink = StreamSink::new(&self.dir, i, name, type_, encoding);
                sink.meta.decoded = true;
                self.streams.push(Stream {
                    sink,
                    rows: Vec::new(),
                    decoder: None,
                });
                self.by_name.insert(name.to_string(), i);
                i
            }
        };
        let mut row: Map<String, Value> = fields
            .into_iter()
            .filter(|(k, _)| k != T)
            .map(|(k, v)| (k, simplify(v)))
            .collect();
        row.insert(T.into(), t_ns.into());
        let stream = &mut self.streams[index];
        stream.rows.push(Value::Object(row));
        if stream.rows.len() >= BATCH {
            flush(stream)?;
        }
        Ok(())
    }

    pub async fn finish(self, out: &Path) -> Result<Extraction> {
        let mut extraction = Extraction::default();
        for mut stream in self.streams {
            flush(&mut stream)?;
            if let Some(meta) = stream.sink.finish(out).await? {
                extraction.streams.push(meta);
            }
        }
        Ok(extraction)
    }
}

fn flush(stream: &mut Stream) -> Result<()> {
    if stream.rows.is_empty() {
        return Ok(());
    }
    let unreadable = |e: &dyn std::fmt::Display| {
        err(
            ErrorKind::RecordingUnreadable,
            format!("{}: {e}", stream.sink.meta.name),
        )
    };
    if stream.decoder.is_none() {
        let inferred = infer_json_schema_from_iterator(stream.rows.iter().map(Ok))
            .map_err(|e| unreadable(&e))?;
        // `t` first; fields that were always null have no type to store.
        let mut fields = vec![Arc::new(Field::new(T, time_type(), true))];
        fields.extend(
            inferred
                .fields()
                .iter()
                .filter(|f| f.name() != T && *f.data_type() != DataType::Null)
                .cloned(),
        );
        let schema = Arc::new(Schema::new(fields));
        stream.decoder = Some(
            ReaderBuilder::new(schema)
                .with_batch_size(BATCH)
                .build_decoder()
                .map_err(|e| unreadable(&e))?,
        );
    }
    let decoder = stream.decoder.as_mut().expect("built above");
    decoder
        .serialize(&std::mem::take(&mut stream.rows))
        .map_err(|e| unreadable(&e))?;
    if let Some(batch) = decoder.flush().map_err(|e| unreadable(&e))? {
        stream.sink.append(batch)?;
    }
    Ok(())
}
