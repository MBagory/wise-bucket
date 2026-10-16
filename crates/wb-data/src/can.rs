//! Vector CAN logs, text (`.asc`) and binary (`.blf`), read by `blf_asc`: every frame
//! goes to the stream `frames`. With DBC files the engineer confirmed, each DBC message
//! also gets a stream of its signals, `<dbc>/<message>`, decoded by `falcon_mdf`.

use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::path::Path;
use std::sync::Arc;

use blf_asc::{AscReader, BlfError, BlfReader, Message};
use datafusion::arrow::array::{
    ArrayRef, BooleanArray, Float64Array, ListBuilder, RecordBatch, StringArray,
    TimestampNanosecondArray, UInt8Array, UInt8Builder, UInt16Array, UInt32Array,
};
use datafusion::arrow::datatypes::{Field, Schema};
use falcon_mdf::{CanDatabase, DecodedSignal, MessageDef};
use wb_core::error::{ErrorKind, Result, err};
use wb_core::roots::{DbcChoice, DbcFile, RecordingKind};

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

/// What a recording note asks the engineer about a CAN log's DBC.
fn ask(files: &[DbcFile]) -> String {
    const UNTIL: &str = "until then, frames are not decoded";
    match files {
        [] => format!(
            "no DBC found next to this log or in its parent folders: ask the engineer where it is, then record it with note (key dbc: root:path or an absolute path, or none); {UNTIL}"
        ),
        [one] => format!(
            "DBC not confirmed: ask the engineer whether {} decodes this log, then record it with note (key dbc); {UNTIL}",
            one.shown
        ),
        several => format!(
            "DBC not chosen: ask the engineer which of {} decode this log, and on which channel, then record them with note (key dbc, e.g. \"1={}\"); {UNTIL}",
            several
                .iter()
                .map(|f| f.shown.as_str())
                .collect::<Vec<_>>()
                .join("; "),
            several[0].shown
        ),
    }
}

/// A DBC the engineer confirmed, ready to decode.
struct Dbc {
    db: CanDatabase,
    /// Its file name without `.dbc`, the first part of its stream names.
    stem: String,
    shown: String,
    channel: Option<u16>,
}

/// A signal column's name: a signal may not take the name of a column every stream has.
fn signal_column(name: &str) -> String {
    match name {
        T | "channel" | "t_sensor" | "message_bytes" | "recording" => format!("{name}_signal"),
        _ => name.to_string(),
    }
}

/// The signals of one DBC message, one batch at a time.
struct Signals {
    sink: StreamSink,
    columns: HashMap<String, usize>,
    names: Vec<String>,
    /// Signals with a value table also get a `<signal>_label` column.
    labeled: Vec<bool>,
    t: Vec<i64>,
    channel: Vec<u16>,
    values: Vec<Vec<Option<f64>>>,
    labels: Vec<Vec<Option<String>>>,
}

impl Signals {
    fn new(sink: StreamSink, def: &MessageDef) -> Self {
        let names: Vec<String> = def.signals.iter().map(|s| s.name.clone()).collect();
        Self {
            sink,
            columns: names
                .iter()
                .enumerate()
                .map(|(i, n)| (n.clone(), i))
                .collect(),
            labeled: def
                .signals
                .iter()
                .map(|s| !s.value_table.is_empty())
                .collect(),
            values: vec![Vec::new(); names.len()],
            labels: vec![Vec::new(); names.len()],
            names,
            t: Vec::new(),
            channel: Vec::new(),
        }
    }

    fn push(&mut self, t_ns: i64, channel: u16, decoded: &[DecodedSignal<'_>]) -> Result<()> {
        self.t.push(t_ns);
        self.channel.push(channel);
        for (v, l) in self.values.iter_mut().zip(&mut self.labels) {
            v.push(None);
            l.push(None);
        }
        for s in decoded {
            if let Some(&c) = self.columns.get(s.name) {
                *self.values[c].last_mut().expect("pushed above") = Some(s.value);
                *self.labels[c].last_mut().expect("pushed above") = s.text.map(str::to_string);
            }
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
        let mut fields = vec![
            Field::new(T, time_type(), true),
            Field::new(
                "channel",
                datafusion::arrow::datatypes::DataType::UInt16,
                true,
            ),
        ];
        let mut arrays: Vec<ArrayRef> = vec![
            Arc::new(TimestampNanosecondArray::from(std::mem::take(&mut self.t))),
            Arc::new(UInt16Array::from(std::mem::take(&mut self.channel))),
        ];
        for (i, name) in self.names.iter().enumerate() {
            let column = signal_column(name);
            fields.push(Field::new(
                &column,
                datafusion::arrow::datatypes::DataType::Float64,
                true,
            ));
            arrays.push(Arc::new(Float64Array::from(std::mem::take(
                &mut self.values[i],
            ))));
            let labels = std::mem::take(&mut self.labels[i]);
            if self.labeled[i] {
                fields.push(Field::new(
                    format!("{column}_label"),
                    datafusion::arrow::datatypes::DataType::Utf8,
                    true,
                ));
                arrays.push(Arc::new(StringArray::from(labels)));
            }
        }
        let batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays)
            .map_err(|e| err(ErrorKind::Internal, format!("CAN signals: {e}")))?;
        self.sink.append(batch)
    }
}

/// The signal streams of the DBC files the engineer confirmed for a log.
struct Decoders {
    dbcs: Vec<Dbc>,
    streams: Vec<Signals>,
    /// (DBC, message identifier) → stream.
    by_message: HashMap<(usize, u32), usize>,
    names: HashSet<String>,
    /// Data frames no DBC defines, by identifier.
    undecoded: HashMap<u32, u64>,
}

impl Decoders {
    fn new(choice: Option<&DbcChoice>, notes: &mut Vec<String>) -> Self {
        let mut dbcs = Vec::new();
        match choice {
            None => {}
            Some(DbcChoice::Nearby(files)) => notes.push(ask(files)),
            Some(DbcChoice::Confirmed(files)) if files.is_empty() => {
                notes.push("frames are not decoded: dbc = none".into())
            }
            Some(DbcChoice::Confirmed(files)) => {
                for f in files {
                    let db = std::fs::read(&f.path)
                        .map_err(|e| e.to_string())
                        .and_then(|b| CanDatabase::from_dbc(&b).map_err(|e| e.to_string()));
                    match db {
                        Ok(db) => dbcs.push(Dbc {
                            db,
                            stem: f
                                .path
                                .file_stem()
                                .map_or("dbc".into(), |s| s.to_string_lossy().into_owned()),
                            shown: f.shown.clone(),
                            channel: f.channel,
                        }),
                        Err(e) => notes.push(format!("DBC {} not read: {e}", f.shown)),
                    }
                }
            }
        }
        Self {
            dbcs,
            streams: Vec::new(),
            by_message: HashMap::new(),
            names: HashSet::new(),
            undecoded: HashMap::new(),
        }
    }

    /// Decodes a frame with every DBC that applies to its channel.
    fn push(
        &mut self,
        m: &Message,
        t_ns: i64,
        channel: u16,
        out: &Path,
        encoding: &str,
    ) -> Result<()> {
        if self.dbcs.is_empty() || m.is_error_frame || m.is_remote_frame {
            return Ok(());
        }
        let id = m.arbitration_id.0;
        let mut known = false;
        for (i, dbc) in self.dbcs.iter().enumerate() {
            if dbc.channel.is_some_and(|c| c != channel) {
                continue;
            }
            let Some(def) = dbc.db.message(id) else {
                continue;
            };
            known = true;
            let s = match self.by_message.get(&(i, def.id)) {
                Some(&s) => s,
                None => {
                    let mut name = format!("{}/{}", dbc.stem, def.name);
                    if !self.names.insert(name.clone()) {
                        name = format!("{}{i}/{}", dbc.stem, def.name);
                        self.names.insert(name.clone());
                    }
                    // `frames` is the recording's first stream.
                    let mut sink =
                        StreamSink::new(out, 1 + self.streams.len(), &name, "dbc", encoding);
                    sink.meta.decoded = true;
                    sink.note(format!("decoded with {}", dbc.shown));
                    let units: Vec<String> = def
                        .signals
                        .iter()
                        .filter(|s| !s.unit.is_empty())
                        .map(|s| format!("{} [{}]", s.name, s.unit))
                        .collect();
                    if !units.is_empty() {
                        sink.note(format!("units: {}", units.join(", ")));
                    }
                    self.streams.push(Signals::new(sink, def));
                    self.by_message.insert((i, def.id), self.streams.len() - 1);
                    self.streams.len() - 1
                }
            };
            self.streams[s].push(t_ns, channel, &dbc.db.decode(id, m.data.as_slice()))?;
        }
        if !known {
            *self.undecoded.entry(id).or_default() += 1;
        }
        Ok(())
    }

    fn finish(mut self, out: &Path, extraction: &mut Extraction) -> Result<()> {
        for mut s in self.streams.drain(..) {
            s.flush()?;
            if let Some(meta) = s.sink.finish(out)? {
                extraction.streams.push(meta);
            }
        }
        if !self.dbcs.is_empty() && !self.undecoded.is_empty() {
            let mut ids: Vec<(u32, u64)> = self.undecoded.into_iter().collect();
            ids.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let total: u64 = ids.iter().map(|(_, n)| n).sum();
            let first: Vec<String> = ids
                .iter()
                .take(5)
                .map(|(id, n)| format!("{id:#X} ({n})"))
                .collect();
            extraction.notes.push(format!(
                "{total} frames have an ID no DBC defines (stream {FRAMES}): {}{}",
                first.join(", "),
                if ids.len() > 5 { ", …" } else { "" }
            ));
        }
        Ok(())
    }
}

pub(crate) async fn read(
    path: &Path,
    kind: RecordingKind,
    dbc: Option<&DbcChoice>,
    out: &Path,
) -> Result<Extraction> {
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
    let mut decoders = Decoders::new(dbc, &mut extraction.notes);
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
                // Channels as the log numbers them (blf_asc counts from 0).
                decoders.push(&m, t_ns, m.channel.saturating_add(1), out, encoding)?;
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
    decoders.finish(out, &mut extraction)?;
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
        stream(out, x, FRAMES)
    }

    /// A stream of an extraction, read back.
    fn stream(out: &Path, x: &Extraction, name: &str) -> RecordBatch {
        let meta = x.streams.iter().find(|s| s.name == name).unwrap();
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
        let x = read(&path, RecordingKind::CanBlf, None, &out)
            .await
            .unwrap();
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
        let x = read(&cut, RecordingKind::CanBlf, None, &out).await.unwrap();
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
        let x = read(&path, RecordingKind::CanAsc, None, &out)
            .await
            .unwrap();
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

    const HEAD: &str = "VERSION \"\"\n\nNS_ :\n\nBS_:\n\nBU_: ECU\n\n";

    fn on(channel: u16, id: u32, data: &[u8], i: u32) -> Message {
        Message {
            timestamp: 100.0 + f64::from(i),
            arbitration_id: id.into(),
            dlc: data.len() as u8,
            data: data.to_vec().into(),
            channel: channel - 1,
            is_rx: true,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn signals_from_the_dbc_of_each_channel() {
        let dir = tempfile::tempdir().unwrap();
        let engine = dir.path().join("engine.dbc");
        std::fs::write(
            &engine,
            format!(
                "{HEAD}BO_ 256 Engine: 8 ECU\n SG_ Rpm : 0|16@1+ (0.25,0) [0|16383.75] \"rpm\" Vector__XXX\n SG_ Temp : 16|8@1- (1,-40) [-168|87] \"degC\" Vector__XXX\n SG_ Gear : 24|8@1+ (1,0) [0|5] \"\" Vector__XXX\n\nVAL_ 256 Gear 0 \"Park\" 1 \"Drive\" ;\n"
            ),
        )
        .unwrap();
        let brake = dir.path().join("brake.dbc");
        std::fs::write(
            &brake,
            format!("{HEAD}BO_ 256 Brake: 2 ECU\n SG_ Pressure : 0|16@1+ (0.1,0) [0|6553.5] \"bar\" Vector__XXX\n"),
        )
        .unwrap();
        let path = dir.path().join("bus.blf");
        let mut w = BlfWriter::create(&path).unwrap();
        // 0x100 means Engine on channel 1 and Brake on channel 2; 0x300 is in no DBC.
        w.on_message_received(&on(1, 0x100, &[0x10, 0x27, 0xF6, 0x01, 0, 0, 0, 0], 0))
            .unwrap();
        w.on_message_received(&on(2, 0x100, &[0xE8, 0x03], 1))
            .unwrap();
        w.on_message_received(&on(1, 0x300, &[1], 2)).unwrap();
        w.on_message_received(&on(1, 0x300, &[2], 3)).unwrap();
        w.finish().unwrap();
        let file = |path: &Path, shown: &str, channel| DbcFile {
            path: path.to_path_buf(),
            shown: shown.into(),
            channel,
        };
        let confirmed = DbcChoice::Confirmed(vec![
            file(&engine, "logs:engine.dbc", Some(1)),
            file(&brake, "logs:brake.dbc", Some(2)),
        ]);

        let out = out_dir(dir.path());
        let x = read(&path, RecordingKind::CanBlf, Some(&confirmed), &out)
            .await
            .unwrap();
        let names: Vec<&str> = x.streams.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["frames", "engine/Engine", "brake/Brake"]);
        assert_eq!(frames(&out, &x).num_rows(), 4);
        let e = stream(&out, &x, "engine/Engine");
        let f64s = |b: &RecordBatch, c: &str| -> Vec<f64> {
            b.column_by_name(c)
                .unwrap()
                .as_primitive::<datafusion::arrow::datatypes::Float64Type>()
                .values()
                .to_vec()
        };
        assert_eq!(e.num_rows(), 1);
        assert_eq!(
            (f64s(&e, "Rpm"), f64s(&e, "Temp")),
            (vec![2500.0], vec![-50.0])
        );
        assert_eq!(
            e.column_by_name("Gear_label")
                .unwrap()
                .as_string::<i32>()
                .value(0),
            "Drive"
        );
        let b = stream(&out, &x, "brake/Brake");
        assert_eq!(f64s(&b, "Pressure"), vec![100.0]);
        let meta = x.streams.iter().find(|s| s.name == "brake/Brake").unwrap();
        assert_eq!(
            meta.notes,
            ["decoded with logs:brake.dbc", "units: Pressure [bar]"]
        );
        assert_eq!(
            x.notes,
            ["2 frames have an ID no DBC defines (stream frames): 0x300 (2)"]
        );

        // Not confirmed yet: frames only, and a note saying what to ask.
        for (choice, start) in [
            (DbcChoice::Nearby(vec![]), "no DBC found next to this log"),
            (
                DbcChoice::Nearby(vec![file(&engine, "logs:engine.dbc", None)]),
                "DBC not confirmed: ask the engineer whether logs:engine.dbc decodes",
            ),
            (
                DbcChoice::Nearby(vec![
                    file(&brake, "logs:brake.dbc", None),
                    file(&engine, "logs:engine.dbc", None),
                ]),
                "DBC not chosen: ask the engineer which of logs:brake.dbc; logs:engine.dbc",
            ),
            (
                DbcChoice::Confirmed(vec![]),
                "frames are not decoded: dbc = none",
            ),
        ] {
            let out = out_dir(&dir.path().join(start.len().to_string()));
            let x = read(&path, RecordingKind::CanBlf, Some(&choice), &out)
                .await
                .unwrap();
            assert_eq!(x.streams.len(), 1, "{start}");
            assert!(x.notes[0].starts_with(start), "{:?}", x.notes);
        }
    }
}
