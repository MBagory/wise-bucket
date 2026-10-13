//! Extracting one recording into a directory of Parquet files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use wb_core::error::{ErrorKind, Result, err};
use wb_core::roots::RecordingKind;

use crate::columns;
use crate::ros2::{self, Decoder};
use crate::sink::{StreamMeta, StreamSink};

/// Messages per batch (and per decoder run).
pub(crate) const BATCH: usize = 8192;

/// The streams extracted from one recording.
#[derive(Debug, Default)]
pub struct Extraction {
    pub streams: Vec<StreamMeta>,
    /// Remarks about the whole recording (truncated file, skipped parts…).
    pub notes: Vec<String>,
}

/// Extracts the recording at `path` into `out` (one Parquet file per stream under `out/data`).
pub async fn extract(path: &Path, kind: RecordingKind, out: &Path) -> Result<Extraction> {
    std::fs::create_dir_all(out.join("data")).map_err(crate::sink::io_err(out.display()))?;
    match kind {
        RecordingKind::Mcap => {
            let mut topics = Topics::new(out);
            crate::mcap_reader::read(&[path.to_path_buf()], &mut topics)?;
            topics.finish(out).await
        }
        RecordingKind::Db3 => {
            let mut topics = Topics::new(out);
            crate::db3::read(&[path.to_path_buf()], &mut topics)?;
            topics.finish(out).await
        }
        RecordingKind::Rosbag2 => {
            let splits = |ext: &str| -> Result<Vec<PathBuf>> {
                let mut v: Vec<PathBuf> = std::fs::read_dir(path)
                    .map_err(crate::sink::io_err(path.display()))?
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.extension()
                            .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case(ext))
                    })
                    .collect();
                v.sort();
                Ok(v)
            };
            let mut topics = Topics::new(out);
            let (mcap, db3) = (splits("mcap")?, splits("db3")?);
            if mcap.is_empty() && db3.is_empty() {
                return Err(err(
                    ErrorKind::RecordingUnreadable,
                    format!(
                        "{}: rosbag2 folder without .mcap or .db3 files",
                        path.display()
                    ),
                ));
            }
            crate::mcap_reader::read(&mcap, &mut topics)?;
            crate::db3::read(&db3, &mut topics)?;
            topics.finish(out).await
        }
        RecordingKind::Ulog => crate::ulog::read(path, out).await,
        RecordingKind::Csv
        | RecordingKind::JsonLines
        | RecordingKind::Parquet
        | RecordingKind::ArrowIpc => crate::tabular::read(path, kind, out).await,
        RecordingKind::Tlog => crate::tlog::read(path, out).await,
        RecordingKind::Ros1Bag
        | RecordingKind::ArdupilotBin
        | RecordingKind::CanAsc
        | RecordingKind::CanBlf
        | RecordingKind::Mdf4 => Err(err(
            ErrorKind::RecordingUnreadable,
            format!("{}: this format is not readable yet", path.display()),
        )),
    }
}

/// One topic of a message-based recording (MCAP, rosbag2).
struct Topic {
    sink: StreamSink,
    decoder: Option<Decoder>,
    t: Vec<i64>,
    bytes: Vec<u32>,
}

/// The topics of a message-based recording, decoded a batch at a time.
pub(crate) struct Topics {
    dir: PathBuf,
    topics: Vec<Topic>,
    by_key: HashMap<(String, String), usize>,
    names: HashMap<String, usize>,
}

impl Topics {
    fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            topics: Vec::new(),
            by_key: HashMap::new(),
            names: HashMap::new(),
        }
    }

    /// Finds or creates the stream of `topic` with message type `type_`.
    ///
    /// `definition` is the full `ros2msg` definition when the recording has one.
    pub fn topic(
        &mut self,
        topic: &str,
        type_: &str,
        encoding: &str,
        definition: Option<&str>,
    ) -> usize {
        let key = (topic.to_string(), type_.to_string());
        if let Some(&i) = self.by_key.get(&key) {
            return i;
        }
        // The same topic with another type (rare) becomes its own stream.
        let seen = self.names.entry(topic.to_string()).or_default();
        let name = if *seen == 0 {
            topic.to_string()
        } else {
            format!("{topic} ({type_})")
        };
        *seen += 1;
        let index = self.topics.len();
        let mut sink = StreamSink::new(&self.dir, index, &name, type_, encoding);
        let decoder = if !encoding.eq_ignore_ascii_case("cdr") {
            sink.note(format!(
                "payload not decoded: the {encoding} encoding is not supported yet"
            ));
            None
        } else if ros2::timing_only(type_) {
            sink.note("payload not decoded: images, point clouds and maps keep their timing and size only");
            None
        } else {
            let bundled;
            let def = match definition {
                Some(d) if !d.trim().is_empty() => Some(d),
                _ => {
                    bundled = ros2::bundled_definition(type_);
                    bundled.as_deref()
                }
            };
            match def {
                None => {
                    sink.note(
                        "payload not decoded: the recording has no definition for this custom type",
                    );
                    None
                }
                Some(def) => match Decoder::new(type_, def, BATCH) {
                    Ok(d) => Some(d),
                    Err(e) => {
                        sink.note(format!("payload not decoded: unusable definition: {e}"));
                        None
                    }
                },
            }
        };
        sink.meta.decoded = decoder.is_some();
        self.topics.push(Topic {
            sink,
            decoder,
            t: Vec::with_capacity(BATCH),
            bytes: Vec::with_capacity(BATCH),
        });
        self.by_key.insert(key, index);
        index
    }

    /// Adds one message (CDR payload with its encapsulation header).
    pub fn push(&mut self, index: usize, time_ns: i64, payload: &[u8]) -> Result<()> {
        let topic = &mut self.topics[index];
        if let Some(d) = topic.decoder.as_mut() {
            let accepted = d.push(payload).map_err(|e| {
                err(
                    ErrorKind::RecordingUnreadable,
                    format!("decoding {}: {e}", topic.sink.meta.name),
                )
            })?;
            if !accepted {
                return Ok(());
            }
        }
        topic.t.push(time_ns);
        topic
            .bytes
            .push(payload.len().try_into().unwrap_or(u32::MAX));
        if topic.t.len() >= BATCH {
            self.flush(index)?;
        }
        Ok(())
    }

    fn flush(&mut self, index: usize) -> Result<()> {
        let topic = &mut self.topics[index];
        if topic.t.is_empty() {
            return Ok(());
        }
        let t = std::mem::replace(&mut topic.t, Vec::with_capacity(BATCH));
        let bytes = std::mem::replace(&mut topic.bytes, Vec::with_capacity(BATCH));
        let decoded = topic.decoder.as_mut().map(|d| d.take(BATCH));
        let (batch, dropped) = columns::assemble(
            t,
            decoded.as_ref().and_then(|d| d.sensor_time.clone()),
            Some(bytes),
            decoded.as_ref().map(|d| &d.fields),
        );
        if !dropped.is_empty() {
            topic.sink.note(format!(
                "large byte arrays left out: {}",
                dropped.join(", ")
            ));
        }
        topic.sink.append(batch)
    }

    async fn finish(mut self, out: &Path) -> Result<Extraction> {
        for i in 0..self.topics.len() {
            self.flush(i)?;
        }
        let mut streams = Vec::new();
        for mut topic in self.topics {
            if let Some(d) = &topic.decoder
                && d.failures > 0
            {
                topic.sink.note(format!(
                    "{} message(s) could not be decoded and were skipped (first error: {})",
                    d.failures,
                    d.first_error.as_deref().unwrap_or("?")
                ));
            }
            if let Some(meta) = topic.sink.finish(out)? {
                streams.push(meta);
            }
        }
        Ok(Extraction {
            streams,
            notes: Vec::new(),
        })
    }
}
