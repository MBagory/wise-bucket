//! Writing one stream into the store: batches are spilled to a temporary Arrow IPC
//! file (memory stays bounded whatever the interleaving of streams in a recording),
//! then written in one go as a Parquet file whose page statistics let a time range
//! skip the rest of the stream.

use std::fs::File;
use std::path::{Path, PathBuf};

use datafusion::arrow::array::{AsArray, RecordBatch};
use datafusion::arrow::datatypes::{SchemaRef, TimestampNanosecondType};
use datafusion::arrow::ipc::reader::FileReader;
use datafusion::arrow::ipc::writer::FileWriter;
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::{EnabledStatistics, WriterProperties};
use serde::{Deserialize, Serialize};
use wb_core::error::{ErrorKind, Result, WbError, err};

use crate::columns::T;

/// What the store knows about one extracted stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamMeta {
    /// Topic, ULog message name, or `table` for table-like files.
    pub name: String,
    /// Message type (`sensor_msgs/msg/Imu`) or file format.
    #[serde(rename = "type")]
    pub type_: String,
    pub encoding: String,
    pub rows: u64,
    pub start_ns: Option<i64>,
    pub end_ns: Option<i64>,
    /// `false` when only the timing (and size) of messages was kept.
    pub decoded: bool,
    pub columns: Vec<String>,
    /// Parquet file, relative to the recording's directory.
    pub dataset: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// The publishers' QoS in short (ROS 2 bags): `reliable, volatile, depth 10`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qos: Option<String>,
}

pub fn io_err(what: impl std::fmt::Display) -> impl FnOnce(std::io::Error) -> WbError {
    move |e| err(ErrorKind::Io, format!("{what}: {e}"))
}

fn arrow_err(e: impl std::fmt::Display) -> WbError {
    err(ErrorKind::Internal, format!("arrow: {e}"))
}

fn parquet_err(e: impl std::fmt::Display) -> WbError {
    err(ErrorKind::Io, format!("writing the store: {e}"))
}

/// A stream being extracted.
pub struct StreamSink {
    pub meta: StreamMeta,
    spill: PathBuf,
    writer: Option<FileWriter<File>>,
    schema: Option<SchemaRef>,
}

impl StreamSink {
    /// `dir` is the recording's temporary directory; `index` numbers the dataset.
    pub fn new(dir: &Path, index: usize, name: &str, type_: &str, encoding: &str) -> Self {
        Self {
            meta: StreamMeta {
                name: name.to_string(),
                type_: type_.to_string(),
                encoding: encoding.to_string(),
                rows: 0,
                start_ns: None,
                end_ns: None,
                decoded: false,
                columns: Vec::new(),
                dataset: format!("data/{index:04}.parquet"),
                notes: Vec::new(),
                qos: None,
            },
            spill: dir.join(format!("spill-{index:04}.arrow")),
            writer: None,
            schema: None,
        }
    }

    pub fn note(&mut self, note: impl Into<String>) {
        let note = note.into();
        if !self.meta.notes.contains(&note) {
            self.meta.notes.push(note);
        }
    }

    /// Appends a batch (all batches of a stream share one schema).
    pub fn append(&mut self, batch: RecordBatch) -> Result<()> {
        if batch.num_rows() == 0 {
            return Ok(());
        }
        if self.writer.is_none() {
            let file = File::create(&self.spill).map_err(io_err(self.spill.display()))?;
            self.writer = Some(FileWriter::try_new(file, &batch.schema()).map_err(arrow_err)?);
            self.schema = Some(batch.schema());
        }
        if self.schema.as_ref() != Some(&batch.schema()) {
            return Err(err(
                ErrorKind::RecordingUnreadable,
                format!("stream {:?} changes its columns midway", self.meta.name),
            ));
        }
        if let Some(t) = batch.column_by_name(T) {
            let t = t.as_primitive::<TimestampNanosecondType>();
            for v in t.iter().flatten() {
                self.meta.start_ns = Some(self.meta.start_ns.map_or(v, |s| s.min(v)));
                self.meta.end_ns = Some(self.meta.end_ns.map_or(v, |e| e.max(v)));
            }
        }
        self.meta.rows += batch.num_rows() as u64;
        self.writer
            .as_mut()
            .expect("writer created above")
            .write(&batch)
            .map_err(arrow_err)
    }

    /// Writes the stream's Parquet file under `dir` and returns its metadata. A stream
    /// without rows produces no file.
    pub fn finish(mut self, dir: &Path) -> Result<Option<StreamMeta>> {
        let Some(mut writer) = self.writer.take() else {
            return Ok(None);
        };
        // A reader may already have said why (ULog: "timestamp 0 (not set) on every row").
        let explained = self.meta.notes.iter().any(|n| n.ends_with("on every row"));
        if self.meta.rows > 1
            && self.meta.start_ns.is_some()
            && self.meta.start_ns == self.meta.end_ns
            && !explained
        {
            self.note("t is the same on every row");
        }
        writer.finish().map_err(arrow_err)?;
        drop(writer);
        let reader = FileReader::try_new(
            File::open(&self.spill).map_err(io_err(self.spill.display()))?,
            None,
        )
        .map_err(arrow_err)?;
        self.meta.columns = crate::columns::flat_columns(&reader.schema())
            .into_iter()
            .map(|c| c.name)
            .collect();
        let path = dir.join(&self.meta.dataset);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io_err(parent.display()))?;
        }
        let props = WriterProperties::builder()
            .set_compression(Compression::ZSTD(ZstdLevel::default()))
            // Min and max per page: a narrow `WHERE t BETWEEN …` skips most of a long stream.
            .set_statistics_enabled(EnabledStatistics::Page)
            .set_data_page_row_count_limit(8192)
            .build();
        let file = File::create(&path).map_err(io_err(path.display()))?;
        let mut out =
            ArrowWriter::try_new(file, reader.schema(), Some(props)).map_err(parquet_err)?;
        for batch in reader {
            out.write(&batch.map_err(arrow_err)?).map_err(parquet_err)?;
        }
        out.close().map_err(parquet_err)?;
        std::fs::remove_file(&self.spill).map_err(io_err(self.spill.display()))?;
        Ok(Some(self.meta))
    }
}
