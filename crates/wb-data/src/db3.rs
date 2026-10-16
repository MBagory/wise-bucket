//! rosbag2 SQLite files (`.db3`), opened read-only and immutable: the recording is
//! never written, not even a journal.

use std::collections::HashMap;
use std::path::PathBuf;

use rusqlite::{Connection, OpenFlags};
use wb_core::error::{ErrorKind, Result, err};

use crate::extract::{Schema, Topics};

fn unreadable(path: &std::path::Path, e: impl std::fmt::Display) -> wb_core::WbError {
    err(
        ErrorKind::RecordingUnreadable,
        format!("{}: {e}", path.display()),
    )
}

/// Reads every message of `paths`, in recording order, into `topics`.
pub(crate) fn read(paths: &[PathBuf], topics: &mut Topics) -> Result<()> {
    for path in paths {
        // SQLite URIs decode %HH escapes: escape the characters that would be misread.
        let mut escaped = String::new();
        for c in path.to_string_lossy().chars() {
            match c {
                '%' | '?' | '#' | ' ' => escaped.push_str(&format!("%{:02X}", c as u32)),
                c => escaped.push(c),
            }
        }
        let uri = format!("file:{escaped}?immutable=1");
        let db = Connection::open_with_flags(
            uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .map_err(|e| unreadable(path, e))?;
        let fail = |e: rusqlite::Error| unreadable(path, e);

        // Newer rosbag2 versions store the definitions; Humble does not.
        let mut definitions: HashMap<String, String> = HashMap::new();
        let has_definitions: bool = db
            .query_row(
                "SELECT count(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'message_definitions'",
                [],
                |r| r.get(0),
            )
            .map_err(fail)?;
        if has_definitions {
            let mut stmt = db
                .prepare("SELECT topic_type, encoding, encoded_message_definition FROM message_definitions")
                .map_err(fail)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(fail)?;
            for row in rows {
                let (ty, encoding, def) = row.map_err(fail)?;
                if encoding == "ros2msg" {
                    definitions.insert(ty, def);
                }
            }
        }

        // Bags since Foxy store each topic's offered QoS.
        let has_qos: bool = db
            .query_row(
                "SELECT count(*) > 0 FROM pragma_table_info('topics') WHERE name = 'offered_qos_profiles'",
                [],
                |r| r.get(0),
            )
            .map_err(fail)?;
        let mut ids: HashMap<i64, usize> = HashMap::new();
        {
            let mut stmt = db
                .prepare(if has_qos {
                    "SELECT id, name, type, serialization_format, offered_qos_profiles FROM topics"
                } else {
                    "SELECT id, name, type, serialization_format, NULL FROM topics"
                })
                .map_err(fail)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                })
                .map_err(fail)?;
            for row in rows {
                let (id, name, ty, format, qos) = row.map_err(fail)?;
                let index = topics.topic(
                    &name,
                    &ty,
                    &format,
                    definitions
                        .get(&ty)
                        .map_or(Schema::None, |d| Schema::Ros2(d)),
                    qos.as_deref(),
                );
                ids.insert(id, index);
            }
        }

        // Insertion order is recording order.
        let mut stmt = db
            .prepare("SELECT topic_id, timestamp, data FROM messages ORDER BY id")
            .map_err(fail)?;
        let mut rows = stmt.query([]).map_err(fail)?;
        while let Some(row) = rows.next().map_err(fail)? {
            let topic: i64 = row.get(0).map_err(fail)?;
            let t: i64 = row.get(1).map_err(fail)?;
            let data = row
                .get_ref(2)
                .map_err(fail)?
                .as_blob()
                .map_err(|e| unreadable(path, e))?;
            if let Some(&index) = ids.get(&topic) {
                topics.push(index, t, data)?;
            }
        }
    }
    Ok(())
}
