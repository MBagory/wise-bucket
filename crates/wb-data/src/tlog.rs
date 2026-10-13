//! MAVLink telemetry logs (`.tlog`, as written by QGroundControl and MAVProxy): each
//! record is an 8-byte big-endian timestamp (µs since the Unix epoch) followed by one
//! MAVLink 1 or 2 frame. One stream per message name (`ATTITUDE`, `BATTERY_STATUS`…).

use std::path::Path;

use mavlink::dialects::ardupilotmega::MavMessage;
use mavlink::{MavlinkReader, Message};
use wb_core::error::{ErrorKind, Result, err};

use crate::extract::{Extraction, LogTally};
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
    let mut rows = Rows::new(out);
    let mut log = LogTally::default();
    let (mut pos, mut garbage, mut undecoded, mut truncated) = (0usize, 0u64, 0u64, false);
    while pos + 8 < bytes.len() {
        let frame = &bytes[pos + 8..];
        let Some(len) = frame_len(frame) else {
            pos += 1;
            garbage += 1;
            continue;
        };
        if len > frame.len() {
            truncated = true;
            break;
        }
        match MavlinkReader::new(&frame[..len]).read_any_message::<MavMessage>() {
            Ok((header, msg)) => {
                let us = u64::from_be_bytes(bytes[pos..pos + 8].try_into().expect("8 bytes"));
                let t = i64::try_from(us).unwrap_or(i64::MAX).saturating_mul(1000);
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
                rows.push(msg.message_name(), "mavlink", "mavlink", t, fields)?;
            }
            // Unknown message or bad checksum: skip the whole frame.
            Err(_) => undecoded += 1,
        }
        pos += 8 + len;
    }
    let mut extraction = rows.finish(out).await?;
    extraction.notes.extend(log.note("STATUSTEXT"));
    if truncated {
        extraction.notes.push(
            "the file ends mid-frame (cut while logging): its valid part was extracted".into(),
        );
    }
    if undecoded > 0 {
        extraction.notes.push(format!(
            "{undecoded} frame(s) skipped: unknown message (not in the ardupilotmega dialect) or bad checksum"
        ));
    }
    if garbage > 0 {
        extraction
            .notes
            .push(format!("{garbage} byte(s) skipped: not MAVLink frames"));
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

    #[test]
    fn frame_lengths() {
        assert_eq!(frame_len(&[0xFE, 9]), Some(17));
        assert_eq!(frame_len(&[0xFD, 9, 0]), Some(21));
        assert_eq!(frame_len(&[0xFD, 9, 1]), Some(34));
        assert_eq!(frame_len(&[0x00, 9, 0]), None);
        assert_eq!(frame_len(&[0xFD]), None);
    }
}
