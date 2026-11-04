//! MAVLink telemetry logs (`.tlog`, as written by QGroundControl and MAVProxy): each
//! record is an 8-byte big-endian timestamp (µs since the Unix epoch) followed by one
//! MAVLink 1 or 2 frame. One stream per message name (`ATTITUDE`, `BATTERY_STATUS`…).

use std::path::Path;

use mavlink::dialects::ardupilotmega::MavMessage;
use mavlink::{MavlinkReader, Message};
use wb_core::error::{ErrorKind, Result, err};

use crate::extract::{Extraction, LogTally};
use crate::raw::{RAW, RawRecords};
use crate::rows::Rows;

/// Counts a `STATUSTEXT` of severity WARNING or worse (RFC 5424: 0 EMERGENCY … 4 WARNING).
fn tally_statustext(log: &mut LogTally, fields: &serde_json::Map<String, serde_json::Value>) {
    let severity = match fields.get("severity") {
        Some(serde_json::Value::Object(o)) => o.get("type").and_then(|v| v.as_str()),
        Some(v) => v.as_str(),
        None => None,
    };
    let Some(level) = severity.map(|s| s.trim_start_matches("MAV_SEVERITY_")) else {
        return;
    };
    let rank = match level {
        "EMERGENCY" => 0,
        "ALERT" => 1,
        "CRITICAL" => 2,
        "ERROR" => 3,
        "WARNING" => 4,
        _ => return,
    };
    let text = fields.get("text").and_then(|v| v.as_str()).unwrap_or("");
    log.add(rank, level, rank < 4, text);
}

/// Length of the MAVLink frame starting at `b[0]`, if `b` starts like one.
fn frame_len(b: &[u8]) -> Option<usize> {
    let payload = usize::from(*b.get(1)?);
    match b[0] {
        0xFE => Some(8 + payload),
        // Header 10 + checksum 2, plus a 13-byte signature when flagged.
        0xFD => Some(12 + payload + if b.get(2)? & 1 == 1 { 13 } else { 0 }),
        _ => None,
    }
}

pub(crate) async fn read(path: &Path, out: &Path) -> Result<Extraction> {
    // ponytail: the whole file is read in memory (tlogs are telemetry, MBs not GBs);
    // stream it if huge tlogs show up.
    let bytes = std::fs::read(path).map_err(crate::sink::io_err(path.display()))?;
    let mut rows = Rows::new(out, 0);
    let mut raw = RawRecords::default();
    let mut log = LogTally::default();
    let (mut pos, mut garbage, mut undecoded, mut truncated) = (0usize, 0u64, 0u64, false);
    let mut first_garbage = None;
    // Where the bytes that are no record, before `pos`, start.
    let mut junk: Option<usize> = None;
    while pos + 8 < bytes.len() {
        let frame = &bytes[pos + 8..];
        let Some(len) = frame_len(frame) else {
            first_garbage.get_or_insert(pos);
            junk.get_or_insert(pos);
            pos += 1;
            garbage += 1;
            continue;
        };
        if len > frame.len() {
            truncated = true;
            break;
        }
        if let Some(start) = junk.take() {
            raw.bytes(None, start as u64, "junk", &bytes[start..pos]);
        }
        let record = &bytes[pos..pos + 8 + len];
        let us = u64::from_be_bytes(bytes[pos..pos + 8].try_into().expect("8 bytes"));
        let t = i64::try_from(us).unwrap_or(i64::MAX).saturating_mul(1000);
        match MavlinkReader::new(&frame[..len]).read_any_message::<MavMessage>() {
            Ok((header, msg)) => {
                let serde_json::Value::Object(mut fields) =
                    serde_json::to_value(&msg).map_err(|e| {
                        err(ErrorKind::Internal, format!("{}: {e}", msg.message_name()))
                    })?
                else {
                    unreachable!("MAVLink messages serialize as objects")
                };
                fields.remove("type");
                if msg.message_name() == "STATUSTEXT" {
                    tally_statustext(&mut log, &fields);
                }
                fields.insert("system_id".into(), header.system_id.into());
                fields.insert("component_id".into(), header.component_id.into());
                let name = msg.message_name();
                rows.push_record(name, "mavlink", "mavlink", t, fields, Some(record))?;
            }
            // Unknown message or bad checksum: the whole record stays raw.
            Err(_) => {
                undecoded += 1;
                raw.bytes(Some(t), pos as u64, "frame", record);
            }
        }
        pos += 8 + len;
    }
    // Fewer bytes than a record holds, at the end of the file.
    if !truncated && pos < bytes.len() {
        first_garbage.get_or_insert(pos);
        junk.get_or_insert(pos);
        garbage += (bytes.len() - pos) as u64;
        pos = bytes.len();
    }
    if let Some(start) = junk {
        raw.bytes(None, start as u64, "junk", &bytes[start..pos]);
    }
    if truncated {
        raw.bytes(None, pos as u64, "cut", &bytes[pos..]);
    }
    let raw_index = rows.next_index();
    let mut extraction = rows.finish(out).await?;
    extraction
        .streams
        .extend(raw.finish(out, raw_index, "mavlink")?);
    extraction.notes.extend(log.note("STATUSTEXT"));
    if truncated {
        extraction.notes.push(format!(
            "the file ends mid-frame (cut while logging): its valid part was extracted, the cut record kept in stream {RAW}"
        ));
    }
    if undecoded > 0 {
        extraction.notes.push(format!(
            "{undecoded} frame(s) not decoded: unknown message (not in the ardupilotmega dialect) or bad checksum, kept in stream {RAW}"
        ));
    }
    if let Some(first) = first_garbage {
        extraction.notes.push(format!(
            "{garbage} byte(s) skipped: not MAVLink frames, the first at byte {first}, kept in stream {RAW}"
        ));
    }
    Ok(extraction)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn statustext_warnings_and_errors_are_noted() {
        use mavlink::dialects::ardupilotmega::{MavSeverity, STATUSTEXT_DATA};
        let dir = tempfile::tempdir().unwrap();
        let mut tlog = Vec::new();
        for (i, (severity, text)) in [
            (MavSeverity::MAV_SEVERITY_INFO, "Armed"),
            (MavSeverity::MAV_SEVERITY_WARNING, "Low battery"),
            (
                MavSeverity::MAV_SEVERITY_CRITICAL,
                "PreArm: Compass not calibrated",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            tlog.extend((1_700_000_000_000_000u64 + i as u64).to_be_bytes());
            let msg = MavMessage::STATUSTEXT(STATUSTEXT_DATA {
                severity,
                text: text.into(),
            });
            let header = mavlink::MavHeader {
                system_id: 1,
                component_id: 1,
                sequence: i as u8,
            };
            mavlink::write_v2_msg(&mut tlog, header, &msg).unwrap();
        }
        let path = dir.path().join("flight.tlog");
        std::fs::write(&path, tlog).unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir_all(out.join("data")).unwrap();
        let x = read(&path, &out).await.unwrap();
        assert_eq!(
            x.notes,
            vec![
                "1 CRITICAL, 1 WARNING log messages (stream STATUSTEXT), first: \"PreArm: Compass not calibrated\""
            ]
        );
    }

    /// A record, 5 bytes that are none, then a record cut short: the `raw` column and the
    /// stream `raw` hold every byte of the file.
    #[tokio::test]
    async fn every_byte_is_kept() {
        use datafusion::arrow::array::{AsArray, RecordBatch};
        use datafusion::arrow::datatypes::UInt64Type;
        use mavlink::dialects::ardupilotmega::HEARTBEAT_DATA;
        // Timestamps whose bytes hold no frame start (0xFD, 0xFE).
        let mut tlog = 1_000_000u64.to_be_bytes().to_vec();
        let heartbeat = MavMessage::HEARTBEAT(HEARTBEAT_DATA::default());
        mavlink::write_v2_msg(&mut tlog, mavlink::MavHeader::default(), &heartbeat).unwrap();
        let record = tlog.len();
        tlog.extend([0u8; 5]);
        let cut = tlog.len();
        tlog.extend(2_000_000u64.to_be_bytes());
        tlog.extend(tlog[8..14].to_vec());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cut.tlog");
        std::fs::write(&path, &tlog).unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir_all(out.join("data")).unwrap();
        let x = read(&path, &out).await.unwrap();
        let stream = |name: &str| -> RecordBatch {
            let meta = x.streams.iter().find(|s| s.name == name).unwrap();
            let file = std::fs::File::open(out.join(&meta.dataset)).unwrap();
            let mut reader =
                parquet::arrow::arrow_reader::ParquetRecordBatchReader::try_new(file, 1024)
                    .unwrap();
            reader.next().unwrap().unwrap()
        };
        let hb = stream("HEARTBEAT");
        let bytes = hb.column_by_name(RAW).unwrap().as_binary::<i64>();
        assert_eq!(bytes.value(0), &tlog[..record]);
        let r = stream(RAW);
        let kind = r.column_by_name("kind").unwrap().as_string::<i32>();
        let position = r
            .column_by_name("position")
            .unwrap()
            .as_primitive::<UInt64Type>();
        let bytes = r.column_by_name("bytes").unwrap().as_binary::<i64>();
        assert_eq!((kind.value(0), position.value(0)), ("junk", record as u64));
        assert_eq!(bytes.value(0), &tlog[record..cut]);
        assert_eq!((kind.value(1), position.value(1)), ("cut", cut as u64));
        assert_eq!(bytes.value(1), &tlog[cut..]);
        assert_eq!(r.num_rows(), 2);
    }

    #[test]
    fn frame_lengths() {
        assert_eq!(frame_len(&[0xFE, 9]), Some(17));
        assert_eq!(frame_len(&[0xFD, 9, 0]), Some(21));
        assert_eq!(frame_len(&[0xFD, 9, 1]), Some(34));
        assert_eq!(frame_len(&[0x00, 9, 0]), None);
        assert_eq!(frame_len(&[0xFD]), None);
    }
}
