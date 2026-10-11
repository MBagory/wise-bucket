//! Writing one stream into the store: batches are spilled to a temporary Arrow IPC
//! file (memory stays bounded whatever the interleaving of streams in a recording),
//! then written in one go as a Lance dataset with a zone-map index on `t`.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use datafusion::arrow::array::{AsArray, RecordBatch};
use datafusion::arrow::datatypes::{SchemaRef, TimestampNanosecondType};
use datafusion::arrow::ipc::reader::FileReader;
use datafusion::arrow::ipc::writer::FileWriter;
use lance::Dataset;
use lance::dataset::{WriteMode, WriteParams};
use lance::index::DatasetIndexExt;
use lance_file::version::LanceFileVersion;
use lance_index::IndexType;
use lance_index::scalar::{BuiltinIndexType, ScalarIndexParams};
use serde::{Deserialize, Serialize};
use wb_core::error::{ErrorKind, Result, WbError, err};

use crate::columns::T;

/// The Lance file format written by Wise Bucket, fixed so a library upgrade never
/// changes the files on its own (ADR 0013).
const FILE_VERSION: LanceFileVersion = LanceFileVersion::V2_2;

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
    /// Dataset directory, relative to the recording's directory.
    pub dataset: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

pub fn io_err(what: impl std::fmt::Display) -> impl FnOnce(std::io::Error) -> WbError {
    move |e| err(ErrorKind::Io, format!("{what}: {e}"))
}

fn arrow_err(e: impl std::fmt::Display) -> WbError {
    err(ErrorKind::Internal, format!("arrow: {e}"))
}

fn lance_err(e: impl std::fmt::Display) -> WbError {
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
                dataset: format!("data/{index:04}.lance"),
                notes: Vec::new(),
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

    /// Writes the dataset under `dir` and returns the stream's metadata. A stream
    /// without rows produces no dataset.
    pub async fn finish(mut self, dir: &Path) -> Result<Option<StreamMeta>> {
        let Some(mut writer) = self.writer.take() else {
            return Ok(None);
        };
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
        let uri = dir.join(&self.meta.dataset);
        let params = WriteParams {
            mode: WriteMode::Create,
            data_storage_version: Some(FILE_VERSION),
            ..Default::default()
        };
        let mut ds = Dataset::write(reader, uri.to_string_lossy().as_ref(), Some(params))
            .await
            .map_err(lance_err)?;
        std::fs::remove_file(&self.spill).map_err(io_err(self.spill.display()))?;
        if self.meta.start_ns.is_some() {
            // Lets a narrow `WHERE t BETWEEN …` skip most of a long stream.
            let params = ScalarIndexParams::for_builtin(BuiltinIndexType::ZoneMap);
            if let Err(e) = ds
                .create_index(&[T], IndexType::ZoneMap, None, &params, false)
                .await
            {
                self.note(format!("no time index: {e}"));
            }
        }
        Ok(Some(self.meta))
    }
}

/// Opens a stream's dataset for reading.
pub async fn open(path: &Path) -> Result<Arc<Dataset>> {
    Dataset::open(path.to_string_lossy().as_ref())
        .await
        .map(Arc::new)
        .map_err(|e| err(ErrorKind::Io, format!("{}: {e}", path.display())))
}
