//! MCAP files (alone, or as rosbag2 splits), read as a stream: chunks are
//! decompressed on the way, and the file is never loaded whole.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;

use mcap::records::Record;
use mcap::sans_io::linear_reader::{LinearReadEvent, LinearReader};
use wb_core::error::{ErrorKind, Result, err};

use crate::extract::Topics;

fn unreadable(path: &std::path::Path, e: impl std::fmt::Display) -> wb_core::WbError {
    err(
        ErrorKind::RecordingUnreadable,
        format!("{}: {e}", path.display()),
    )
}

/// Reads every message of `paths`, in order, into `topics`.
pub(crate) fn read(paths: &[PathBuf], topics: &mut Topics) -> Result<()> {
    for path in paths {
        let mut file = std::fs::File::open(path).map_err(|e| unreadable(path, e))?;
        let mut reader = LinearReader::new();
        // Per file: schema id → (name, encoding, definition), channel id → stream index.
        let mut schemas: HashMap<u16, (String, String, Vec<u8>)> = HashMap::new();
        let mut channels: HashMap<u16, usize> = HashMap::new();
        while let Some(event) = reader.next_event() {
            match event.map_err(|e| unreadable(path, e))? {
                LinearReadEvent::ReadRequest(need) => {
                    let n = file
                        .read(reader.insert(need))
                        .map_err(|e| unreadable(path, e))?;
                    reader.notify_read(n);
                }
                LinearReadEvent::Record { opcode, data } => {
                    match mcap::parse_record(opcode, data).map_err(|e| unreadable(path, e))? {
                        Record::Schema { header, data } => {
                            schemas.insert(
                                header.id,
                                (header.name, header.encoding, data.into_owned()),
                            );
                        }
                        Record::Channel(ch) => {
                            let (type_, definition) = match schemas.get(&ch.schema_id) {
                                Some((name, encoding, data)) => (
                                    name.clone(),
                                    (encoding == "ros2msg")
                                        .then(|| String::from_utf8_lossy(data).into_owned()),
                                ),
                                None => (String::new(), None),
                            };
                            let index = topics.topic(
                                &ch.topic,
                                &type_,
                                &ch.message_encoding,
                                definition.as_deref(),
                            );
                            channels.insert(ch.id, index);
                        }
                        Record::Message { header, data } => {
                            if let Some(&index) = channels.get(&header.channel_id) {
                                topics.push(index, header.log_time as i64, &data)?;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(())
}
