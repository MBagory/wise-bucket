//! Vector CAN logs, text (`.asc`) and binary (`.blf`), read by `blf_asc`: every frame
//! goes to one stream, `frames`.

use std::io::BufRead;
use std::path::Path;
use std::sync::Arc;

use blf_asc::{AscReader, BlfError, BlfReader, Message};
use datafusion::arrow::array::{
    ArrayRef, BooleanArray, ListBuilder, RecordBatch, TimestampNanosecondArray, UInt8Array,
    UInt8Builder, UInt16Array, UInt32Array,
};
use datafusion::arrow::datatypes::{Field, Schema};
use wb_core::error::{ErrorKind, Result, err};
use wb_core::roots::RecordingKind;

use crate::columns::{T, time_type};
use crate::extract::{BATCH, Extraction};
use crate::sink::StreamSink;

/// The stream holding every frame of a CAN log.
pub const FRAMES: &str = "frames";

enum Reader {
    Asc(AscReader),
    Blf(BlfReader),
}

impl Reader {
    fn next(&mut self) -> std::result::Result<Option<Message>, BlfError> {
        match self {
            Self::Asc(r) => r.next_message(),
            Self::Blf(r) => r.next_message(),
        }
    }
}

/// The columns of `frames`, one batch at a time.
struct Frames {
    t: Vec<i64>,
    channel: Vec<u16>,
    id: Vec<Option<u32>>,
    extended: Vec<Option<bool>>,
    fd: Vec<bool>,
    remote: Vec<bool>,
    error: Vec<bool>,
    rx: Vec<bool>,
    dlc: Vec<u8>,
    data: ListBuilder<UInt8Builder>,
}

impl Frames {
    fn new() -> Self {
        Self {
            t: Vec::new(),
            channel: Vec::new(),
            id: Vec::new(),
            extended: Vec::new(),
            fd: Vec::new(),
            remote: Vec::new(),
            error: Vec::new(),
            rx: Vec::new(),
            dlc: Vec::new(),
            data: ListBuilder::new(UInt8Builder::new()),
        }
    }

    fn push(&mut self, m: &Message, t_ns: i64) {
        self.t.push(t_ns);
        // blf_asc numbers channels from 0, as python-can does; logs and CANoe from 1.
        self.channel.push(m.channel.saturating_add(1));
        // An error frame has no identifier.
        let id = (!m.is_error_frame).then_some(m.arbitration_id.0);
        self.id.push(id);
        self.extended.push(id.map(|_| m.is_extended_id));
        self.fd.push(m.is_fd);
        self.remote.push(m.is_remote_frame);
        self.error.push(m.is_error_frame);
        self.rx.push(m.is_rx);
        self.dlc.push(m.dlc);
        self.data.values().append_slice(m.data.as_slice());
        self.data.append(true);
    }

    fn len(&self) -> usize {
        self.t.len()
    }

    fn batch(&mut self) -> Result<RecordBatch> {
        let arrays: Vec<(&str, ArrayRef)> = vec![
            (
                T,
                Arc::new(TimestampNanosecondArray::from(std::mem::take(&mut self.t))),
            ),
            (
                "channel",
                Arc::new(UInt16Array::from(std::mem::take(&mut self.channel))),
            ),
            (
                "id",
                Arc::new(UInt32Array::from(std::mem::take(&mut self.id))),
            ),
            (
                "extended",
                Arc::new(BooleanArray::from(std::mem::take(&mut self.extended))),
            ),
            (
                "fd",
                Arc::new(BooleanArray::from(std::mem::take(&mut self.fd))),
            ),
            (
                "remote",
                Arc::new(BooleanArray::from(std::mem::take(&mut self.remote))),
            ),
            (
                "error",
                Arc::new(BooleanArray::from(std::mem::take(&mut self.error))),
            ),
            (
                "rx",
                Arc::new(BooleanArray::from(std::mem::take(&mut self.rx))),
            ),
            (
                "dlc",
                Arc::new(UInt8Array::from(std::mem::take(&mut self.dlc))),
            ),
            ("data", Arc::new(self.data.finish())),
        ];
        let fields: Vec<Field> = arrays
            .iter()
            .map(|(name, a)| match *name {
                T => Field::new(T, time_type(), true),
                _ => Field::new(*name, a.data_type().clone(), true),
            })
            .collect();
        RecordBatch::try_new(
            Arc::new(Schema::new(fields)),
            arrays.into_iter().map(|(_, a)| a).collect(),
        )
        .map_err(|e| err(ErrorKind::Internal, format!("CAN frames: {e}")))
    }
}

/// The text after `date` on the first line of an ASC log, as written.
fn asc_date(path: &Path) -> Option<String> {
    let mut line = String::new();
    std::io::BufReader::new(std::fs::File::open(path).ok()?)
        .read_line(&mut line)
        .ok()?;
    let date = line.trim().get(5..)?.trim();
    (line.get(..5)?.eq_ignore_ascii_case("date ") && !date.is_empty()).then(|| date.to_string())
}

pub(crate) async fn read(path: &Path, kind: RecordingKind, out: &Path) -> Result<Extraction> {
    let unreadable = |e: &dyn std::fmt::Display| {
        err(
            ErrorKind::RecordingUnreadable,
            format!("{}: {e}", path.display()),
        )
    };
    let mut extraction = Extraction::default();
    let (mut reader, encoding) = if kind == RecordingKind::CanAsc {
        // Relative times: the `date` header has no time zone.
        let reader = AscReader::open(path).map_err(|e| unreadable(&e))?;
        (Reader::Asc(reader), "asc")
    } else {
        let reader = BlfReader::open(path).map_err(|e| unreadable(&e))?;
        (Reader::Blf(reader), "blf")
    };
    let mut sink = StreamSink::new(out, 0, FRAMES, "can", encoding);
    sink.meta.decoded = true;
    if let Some(date) = asc_date(path) {
        sink.note("t counts from the start of the measurement; it started at the property date");
        extraction.properties.push(("date".into(), date));
    } else if kind == RecordingKind::CanBlf {
        sink.note("t is the start time in the file header, read as UTC, plus each frame's offset, to the microsecond");
    }
    let mut frames = Frames::new();
    let (mut error_frames, mut skipped, mut cut) = (0u64, 0u64, false);
    loop {
        match reader.next() {
            Ok(Some(m)) => {
                error_frames += u64::from(m.is_error_frame);
                let t_ns = match reader {
                    Reader::Asc(_) => (m.timestamp * 1e9).round() as i64,
                    // Seconds since 1970 in an f64 are precise to about 0.24 µs.
                    Reader::Blf(_) => (m.timestamp * 1e6).round() as i64 * 1000,
                };
                frames.push(&m, t_ns);
                if frames.len() >= BATCH {
                    sink.append(frames.batch()?)?;
                }
            }
            Ok(None) => break,
            // An ASC line that is not a frame (statistics, J1939 transport…): blf_asc has
            // consumed it, so reading goes on.
            Err(BlfError::Parse(_)) if matches!(reader, Reader::Asc(_)) => skipped += 1,
            // A BLF file cut while logging ends mid-object.
            Err(BlfError::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                cut = true;
                break;
            }
            Err(e @ BlfError::Parse(_)) if frames.len() + sink.meta.rows as usize > 0 => {
                tracing::debug!("{}: {e}", path.display());
                cut = true;
                break;
            }
            Err(e) => return Err(unreadable(&e)),
        }
    }
    if frames.len() > 0 {
        sink.append(frames.batch()?)?;
    }
    if let Some(meta) = sink.finish(out)? {
        extraction.streams.push(meta);
    }
    if error_frames > 0 {
        extraction.notes.push(format!(
            "{error_frames} error frames (stream {FRAMES}, error = true)"
        ));
    }
    if skipped > 0 {
        extraction.notes.push(format!(
            "{skipped} line(s) not read as frames (statistics, J1939 transport…)"
        ));
    }
    if cut {
        extraction
            .notes
            .push("the file is cut or damaged: its valid part was extracted".into());
    }
    Ok(extraction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use blf_asc::{AscWriter, BlfWriter};
    use datafusion::arrow::array::{Array, AsArray};
    use datafusion::arrow::datatypes::{TimestampNanosecondType, UInt32Type};

    fn frame(i: u32) -> Message {
        Message {
            timestamp: 1_700_000_000.0 + f64::from(i) / 1000.0,
            arbitration_id: (0x100 + i % 3).into(),
            dlc: 2,
            data: vec![i as u8, 0xAB].into(),
            channel: 1,
            is_rx: true,
            ..Default::default()
        }
    }

    /// The `frames` stream of an extraction, read back.
    fn frames(out: &Path, x: &Extraction) -> RecordBatch {
        let meta = x.streams.iter().find(|s| s.name == FRAMES).unwrap();
        let file = std::fs::File::open(out.join(&meta.dataset)).unwrap();
        let reader =
            parquet::arrow::arrow_reader::ParquetRecordBatchReader::try_new(file, 1 << 20).unwrap();
        let batches: Vec<RecordBatch> = reader.map(|b| b.unwrap()).collect();
        datafusion::arrow::compute::concat_batches(&batches[0].schema(), &batches).unwrap()
    }

    fn out_dir(dir: &Path) -> std::path::PathBuf {
        let out = dir.join("out");
        std::fs::create_dir_all(out.join("data")).unwrap();
        out
    }

    #[tokio::test]
    async fn blf_frames_and_a_cut_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bus.blf");
        let mut w = BlfWriter::create(&path).unwrap();
        for i in 0..10_000 {
            w.on_message_received(&frame(i)).unwrap();
        }
        let mut error = frame(10_000);
        error.is_error_frame = true;
        w.on_message_received(&error).unwrap();
        w.finish().unwrap();

        let out = out_dir(dir.path());
        let x = read(&path, RecordingKind::CanBlf, &out).await.unwrap();
        let b = frames(&out, &x);
        assert_eq!(b.num_rows(), 10_001);
        let t = b
            .column_by_name(T)
            .unwrap()
            .as_primitive::<TimestampNanosecondType>();
        assert_eq!(t.value(1), 1_700_000_000_001_000_000);
        let id = b.column_by_name("id").unwrap().as_primitive::<UInt32Type>();
        assert_eq!((id.value(1), id.is_null(10_000)), (0x101, true));
        let channel = b.column_by_name("channel").unwrap();
        assert_eq!(
            channel
                .as_primitive::<datafusion::arrow::datatypes::UInt16Type>()
                .value(0),
            2
        );
        let data = b.column_by_name("data").unwrap().as_list::<i32>().value(1);
        assert_eq!(
            data.as_primitive::<datafusion::arrow::datatypes::UInt8Type>()
                .values(),
            &[1, 0xAB]
        );
        assert_eq!(
            x.notes,
            vec!["1 error frames (stream frames, error = true)"]
        );

        // Cut in half, as by a power loss while logging.
        let bytes = std::fs::read(&path).unwrap();
        let cut = dir.path().join("cut.blf");
        std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
        let out = out_dir(&dir.path().join("cut"));
        let x = read(&cut, RecordingKind::CanBlf, &out).await.unwrap();
        assert!(frames(&out, &x).num_rows() > 1000);
        assert_eq!(
            x.notes,
            vec!["the file is cut or damaged: its valid part was extracted"]
        );
    }

    #[tokio::test]
    async fn asc_frames_skip_other_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bus.asc");
        let mut w = AscWriter::create(&path).unwrap();
        for i in 0..3 {
            w.on_message_received(&frame(i)).unwrap();
        }
        w.finish().unwrap();
        // A statistics line, as CANoe writes them, before the end of the trigger block.
        let text = std::fs::read_to_string(&path).unwrap().replace(
            "End TriggerBlock",
            "   2.000000 1  Statistic: D 0 R 0 XD 0 XR 0 E 0 O 0 B 0.00%\nEnd TriggerBlock",
        );
        std::fs::write(&path, text).unwrap();

        let out = out_dir(dir.path());
        let x = read(&path, RecordingKind::CanAsc, &out).await.unwrap();
        let b = frames(&out, &x);
        assert_eq!(b.num_rows(), 3);
        let id = b.column_by_name("id").unwrap().as_primitive::<UInt32Type>();
        assert_eq!(id.values(), &[0x100, 0x101, 0x102]);
        assert_eq!(
            x.notes,
            vec!["1 line(s) not read as frames (statistics, J1939 transport…)"]
        );
        assert_eq!(x.properties[0].0, "date");
    }
}
