//! What the engineer said about recordings, beyond what their files say: key/values per
//! target, kept in `context.toml` next to the user configuration.
//!
//! ```toml
//! ["flights:"]                       # every recording of the root `flights`
//! hardware = "drone-2"
//!
//! ["flights:2026-10/"]               # every recording under this folder
//! propellers = "9x4.5 carbon"
//!
//! ["flights:2026-10/flight3.ulg"]    # one recording
//! wind = "gusts 8 m/s"
//! ```
//!
//! For each key, the most specific target wins.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::config::EffectiveConfig;
use crate::error::{ErrorKind, Result, WbError, err};
use crate::fsutil::{self, FileLock};

const HEADER: &str = "# What the engineer said about recordings (`wisebucket note`, the agent's `note` tool).\n\
# One table per target: a recording (\"root:path\"), a folder (\"root:folder/\") or a root (\"root:\").\n\
# For each key, the most specific target wins.\n";

/// Where the context is kept: `context.toml`, next to the user configuration.
pub fn path(cfg: &EffectiveConfig) -> PathBuf {
    cfg.user_config_path.value.with_file_name("context.toml")
}

/// Whether `target` covers `recording` (`root:path`): the recording itself, a folder
/// above it (`root:folder/`) or its root (`root:`).
pub fn covers(target: &str, recording: &str) -> bool {
    target == recording
        || ((target.ends_with('/') || target.ends_with(':'))
            && format!("{recording}/").starts_with(target))
}

fn invalid(path: &Path, message: impl std::fmt::Display) -> WbError {
    err(
        ErrorKind::ContextInvalid,
        format!("{}: {message}", path.display()),
    )
}

/// The context of every target: target → key → value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context(pub BTreeMap<String, BTreeMap<String, String>>);

impl Context {
    /// Reads `path`; a missing file is an empty context.
    pub fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(invalid(path, e)),
        };
        let doc: toml::Table = toml::from_str(&text).map_err(|e| invalid(path, e.message()))?;
        let mut targets = BTreeMap::new();
        for (target, keys) in doc {
            let toml::Value::Table(keys) = keys else {
                return Err(invalid(
                    path,
                    format!(
                        "{target:?} must be a table: [\"{target}\"], then key = \"value\" lines"
                    ),
                ));
            };
            let values = keys
                .into_iter()
                .map(|(k, v)| {
                    let v = match v {
                        toml::Value::String(s) => s,
                        other => other.to_string(),
                    };
                    (k, v)
                })
                .collect();
            targets.insert(target, values);
        }
        Ok(Self(targets))
    }

    /// The context of one recording (`root:path`): for each key, the value of the most
    /// specific target covering it.
    pub fn of(&self, recording: &str) -> BTreeMap<String, String> {
        let mut covering: Vec<_> = self
            .0
            .iter()
            .filter(|(target, _)| covers(target, recording))
            .collect();
        covering.sort_by_key(|(target, _)| target.len());
        let mut out = BTreeMap::new();
        for (_, keys) in covering {
            out.extend(keys.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        out
    }
}

/// Sets the keys of `target` in the context file at `path` (comments and formatting are
/// kept); an empty or missing value removes the key. Returns the target's keys.
pub fn update(
    path: &Path,
    target: &str,
    changes: &BTreeMap<String, Option<String>>,
) -> Result<BTreeMap<String, String>> {
    for key in changes.keys() {
        if key.trim().is_empty() || key.chars().any(char::is_control) {
            return Err(invalid(path, format!("invalid key {key:?}")));
        }
    }
    // Two agent windows may write at the same time.
    let _lock = FileLock::acquire(&path.with_extension("lock"))?;
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(invalid(path, e)),
    };
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| invalid(path, e))?;
    if !changes.is_empty() {
        let table = doc
            .entry(target)
            .or_insert(toml_edit::table())
            .as_table_mut()
            .ok_or_else(|| invalid(path, format!("{target:?} must be a table")))?;
        for (key, value) in changes {
            match value.as_deref().map(str::trim) {
                Some(v) if !v.is_empty() => table[key.trim()] = toml_edit::value(v),
                _ => {
                    table.remove(key.trim());
                }
            }
        }
        if table.is_empty() {
            doc.remove(target);
        }
        let mut out = doc.to_string();
        if !out.starts_with('#') {
            out = format!("{HEADER}\n{out}");
        }
        fsutil::write_atomic(path, out.as_bytes())?;
    }
    Ok(Context::load(path)?.0.remove(target).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_most_specific_target_wins() {
        assert!(covers("flights:", "flights:a/b.ulg"));
        assert!(covers("flights:a/", "flights:a/b.ulg"));
        assert!(covers("flights:a/b.ulg", "flights:a/b.ulg"));
        // A rosbag2 recording is a folder: its folder target covers it.
        assert!(covers("bags:run3/", "bags:run3"));
        assert!(!covers("flights:", "flights2:a.ulg"));
        assert!(!covers("flights:a/", "flights:ab/c.ulg"));
        assert!(!covers("flights:a", "flights:a/b.ulg"));

        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("context.toml");
        let set = |target: &str, k: &str, v: &str| {
            update(&p, target, &BTreeMap::from([(k.into(), Some(v.into()))])).unwrap()
        };
        set("flights:", "hardware", "drone-1");
        set("flights:", "site", "field B");
        set("flights:2026-10/", "hardware", "drone-2");
        let own = set("flights:2026-10/f3.ulg", "wind", "gusts 8 m/s");
        assert_eq!(own, BTreeMap::from([("wind".into(), "gusts 8 m/s".into())]));
        let ctx = Context::load(&p).unwrap();
        assert_eq!(
            ctx.of("flights:2026-10/f3.ulg"),
            BTreeMap::from([
                ("hardware".into(), "drone-2".into()),
                ("site".into(), "field B".into()),
                ("wind".into(), "gusts 8 m/s".into()),
            ])
        );
        assert_eq!(ctx.of("flights:2025/f1.ulg")["hardware"], "drone-1");
        assert!(ctx.of("bags:run3").is_empty());

        // An empty value removes the key, and an empty target disappears.
        update(
            &p,
            "flights:2026-10/f3.ulg",
            &BTreeMap::from([("wind".into(), Some(String::new()))]),
        )
        .unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("# What the engineer said"), "{text}");
        assert!(!text.contains("f3.ulg"), "{text}");
    }

    #[test]
    fn hand_edits_are_read_and_kept() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("context.toml");
        std::fs::write(
            &p,
            "# my notes\n[\"bench:\"]\npayload_kg = 1.2\ncalibrated = true\n",
        )
        .unwrap();
        assert_eq!(
            Context::load(&p).unwrap().of("bench:run.csv"),
            BTreeMap::from([
                ("calibrated".into(), "true".into()),
                ("payload_kg".into(), "1.2".into()),
            ])
        );
        update(
            &p,
            "bench:",
            &BTreeMap::from([("rig".into(), Some("R-17".into()))]),
        )
        .unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("# my notes"));

        std::fs::write(&p, "hardware = \"drone-2\"\n").unwrap();
        assert_eq!(
            Context::load(&p).unwrap_err().kind(),
            ErrorKind::ContextInvalid
        );
        std::fs::write(&p, "[oops\n").unwrap();
        assert_eq!(
            Context::load(&p).unwrap_err().kind(),
            ErrorKind::ContextInvalid
        );
    }
}
