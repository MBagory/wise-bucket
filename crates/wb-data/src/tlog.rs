//! MAVLink telemetry logs (`.tlog`, as written by QGroundControl and MAVProxy): each
//! record is an 8-byte big-endian timestamp (µs since the Unix epoch) followed by one
//! MAVLink 1 or 2 frame. One stream per message name (`ATTITUDE`, `BATTERY_STATUS`…).

use std::path::Path;

use mavlink::dialects::ardupilotmega::MavMessage;
use mavlink::{MavlinkReader, Message};
use wb_core::error::{ErrorKind, Result, err};

use crate::extract::Extraction;
use crate::rows::Rows;

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

    #[test]
    fn frame_lengths() {
        assert_eq!(frame_len(&[0xFE, 9]), Some(17));
        assert_eq!(frame_len(&[0xFD, 9, 0]), Some(21));
        assert_eq!(frame_len(&[0xFD, 9, 1]), Some(34));
        assert_eq!(frame_len(&[0x00, 9, 0]), None);
        assert_eq!(frame_len(&[0xFD]), None);
    }
}
