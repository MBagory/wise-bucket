//! The store: a decoded, columnar copy of every recording, in `<state>/store`.
//!
//! ```text
//! <state>/store/
//!   candidates.json           recordings found in the roots (written after each discovery)
//!   <key>.lock                held while a recording is extracted; freed if the process dies
//!   <key>/<fingerprint>/      written once in .tmp-<pid>/, then published by a rename
//!     recording.json          the recording and its streams
//!     data/NNNN.parquet       one Parquet file per stream
//!     error.txt               instead of the above, when extraction failed
//! ```
//!
//! The store is derived data: deleting it is safe, everything is extracted again.
//! Several processes (agent windows) share it; locks and atomic renames keep them
//! from extracting a recording twice or reading a half-written one.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wb_core::config::EffectiveConfig;
use wb_core::error::{ErrorKind, Result, err};
use wb_core::fsutil::{self, FileLock};
use wb_core::roots::{self, Candidate, RecordingKind, Root};

use crate::EXTRACTOR_VERSION;
use crate::extract;
use crate::sink::{StreamMeta, io_err};

/// Free space below which extraction pauses.
const MIN_FREE_BYTES: u64 = 10 * 1024 * 1024 * 1024;

/// What the store knows about one extracted recording (`recording.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordingMeta {
    /// `root:path`, the identifier used in SQL.
    pub recording: String,
    pub root: String,
    pub path: PathBuf,
    pub kind: RecordingKind,
    pub hardware: Option<String>,
    pub size_bytes: u64,
    pub mtime_ns: i64,
    pub extractor_version: u32,
    pub streams: Vec<StreamMeta>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl RecordingMeta {
    pub fn start_ns(&self) -> Option<i64> {
        self.streams.iter().filter_map(|s| s.start_ns).min()
    }
    pub fn end_ns(&self) -> Option<i64> {
        self.streams.iter().filter_map(|s| s.end_ns).max()
    }
}

/// A recording found in a root, with the root's hardware (`candidates.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    #[serde(flatten)]
    pub candidate: Candidate,
    pub hardware: Option<String>,
}

/// Where a recording stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Waiting to be extracted.
    Pending,
    /// Being extracted by an agent session.
    Extracting,
    Done,
    Failed,
    /// Waiting for free disk space.
    Paused,
    /// No longer in the roots (its data stays queryable).
    Missing,
    /// A format Wise Bucket cannot read yet.
    Unsupported,
}

/// One row of the `recordings` table.
#[derive(Debug, Clone)]
pub struct Row {
    pub recording: String,
    pub root: String,
    pub path: PathBuf,
    pub kind: RecordingKind,
    pub hardware: Option<String>,
    pub status: Status,
    pub size_bytes: u64,
    pub mtime_ns: i64,
    pub error: Option<String>,
    /// Set when extracted (done or missing).
    pub meta: Option<RecordingMeta>,
    /// Directory holding `recording.json` and `data/`.
    pub dir: Option<PathBuf>,
}

fn short_hash(parts: &[&[u8]]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    hex::encode(&h.finalize()[..8])
}

/// The identifier of a recording in SQL: `root:path`.
pub fn recording_id(c: &Candidate) -> String {
    format!("{}:{}", c.root, c.path.display())
}

/// The store of one state directory.
#[derive(Debug, Clone)]
pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            dir: state_dir.join("store"),
        }
    }

    pub fn for_config(cfg: &EffectiveConfig) -> Self {
        Self::new(&cfg.state_dir.value)
    }

    fn key(c: &Candidate) -> String {
        short_hash(&[c.root.as_bytes(), c.path.to_string_lossy().as_bytes()])
    }

    fn fingerprint(c: &Candidate) -> String {
        short_hash(&[
            &c.size.to_le_bytes(),
            &c.mtime_ns.to_le_bytes(),
            &EXTRACTOR_VERSION.to_le_bytes(),
        ])
    }

    fn lock_path(&self, c: &Candidate) -> PathBuf {
        self.dir.join(format!("{}.lock", Self::key(c)))
    }

    /// Where the current extraction of `c` lives (once published).
    pub fn current_dir(&self, c: &Candidate) -> PathBuf {
        self.dir.join(Self::key(c)).join(Self::fingerprint(c))
    }

    fn candidates_path(&self) -> PathBuf {
        self.dir.join("candidates.json")
    }

    /// Records the recordings found in the roots.
    pub fn write_candidates(&self, entries: &[Entry]) -> Result<()> {
        let json = serde_json::to_vec_pretty(entries)
            .map_err(|e| err(ErrorKind::Internal, e.to_string()))?;
        fsutil::write_atomic(&self.candidates_path(), &json)
    }

    /// The recordings found by the last discovery (empty before the first one).
    pub fn read_candidates(&self) -> Vec<Entry> {
        std::fs::read(self.candidates_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Whether extraction must wait for free disk space.
    pub fn paused(&self) -> bool {
        fsutil::available_space(&self.dir).is_some_and(|free| free < MIN_FREE_BYTES)
    }

    fn read_meta(dir: &Path) -> Option<RecordingMeta> {
        serde_json::from_slice(&std::fs::read(dir.join("recording.json")).ok()?).ok()
    }

    /// Every recording, with its status: the current candidates, plus extracted
    /// recordings that are no longer in the roots (`missing`).
    ///
    /// ponytail: reads every `recording.json` on each call (each query and `server_info`);
    /// keep an index file updated at publish time if thousands of recordings make it slow.
    pub fn rows(&self) -> Vec<Row> {
        let paused = self.paused();
        let mut keys = HashSet::new();
        let mut rows = Vec::new();
        for e in self.read_candidates() {
            let c = &e.candidate;
            keys.insert(Self::key(c));
            let dir = self.current_dir(c);
            let meta = Self::read_meta(&dir);
            let error = std::fs::read_to_string(dir.join("error.txt")).ok();
            let status = if !c.kind.supported() {
                Status::Unsupported
            } else if meta.is_some() {
                Status::Done
            } else if error.is_some() {
                Status::Failed
            } else if FileLock::is_held(&self.lock_path(c)) {
                Status::Extracting
            } else if paused {
                Status::Paused
            } else {
                Status::Pending
            };
            rows.push(Row {
                recording: recording_id(c),
                root: c.root.clone(),
                path: c.path.clone(),
                kind: c.kind,
                hardware: e.hardware.clone(),
                status,
                size_bytes: c.size,
                mtime_ns: c.mtime_ns,
                error: error.map(|e| e.trim().to_string()),
                dir: meta.is_some().then_some(dir),
                meta,
            });
        }
        // Extracted recordings whose source is gone stay visible and queryable.
        for (key, dir) in self.extracted_dirs() {
            if keys.contains(&key) {
                continue;
            }
            if let Some(meta) = Self::read_meta(&dir) {
                rows.push(Row {
                    recording: meta.recording.clone(),
                    root: meta.root.clone(),
                    path: meta.path.clone(),
                    kind: meta.kind,
                    hardware: meta.hardware.clone(),
                    status: Status::Missing,
                    size_bytes: meta.size_bytes,
                    mtime_ns: meta.mtime_ns,
                    error: None,
                    dir: Some(dir),
                    meta: Some(meta),
                });
            }
        }
        rows.sort_by(|a, b| a.recording.cmp(&b.recording));
        rows
    }

    /// `(key, newest published directory)` of every extracted recording.
    fn extracted_dirs(&self) -> Vec<(String, PathBuf)> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return out;
        };
        for e in entries.flatten() {
            let path = e.path();
            if !path.is_dir() {
                continue;
            }
            let key = e.file_name().to_string_lossy().to_string();
            let newest = std::fs::read_dir(&path)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|d| !d.file_name().to_string_lossy().starts_with('.'))
                .filter(|d| d.path().join("recording.json").is_file())
                .max_by_key(|d| d.metadata().and_then(|m| m.modified()).ok());
            if let Some(d) = newest {
                out.push((key, d.path()));
            }
        }
        out
    }

    /// Count of recordings per status.
    pub fn counts(&self) -> BTreeMap<Status, u64> {
        let mut counts = BTreeMap::new();
        for r in self.rows() {
            *counts.entry(r.status).or_default() += 1;
        }
        counts
    }

    /// Bytes used by the store.
    pub fn size(&self) -> u64 {
        fn walk(p: &Path) -> u64 {
            let Ok(entries) = std::fs::read_dir(p) else {
                return 0;
            };
            entries
                .flatten()
                .map(|e| match e.file_type() {
                    Ok(t) if t.is_dir() => walk(&e.path()),
                    Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
                    Err(_) => 0,
                })
                .sum()
        }
        walk(&self.dir)
    }

    /// Recordings waiting for extraction, newest first. Files changed in the last few
    /// seconds are left for later: they may still be being written.
    pub fn pending(&self, settle: Duration) -> Vec<Entry> {
        let now_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        let mut out: Vec<Entry> = self
            .read_candidates()
            .into_iter()
            .filter(|e| {
                let c = &e.candidate;
                let dir = self.current_dir(c);
                c.kind.supported()
                    && now_ns - c.mtime_ns >= settle.as_nanos() as i64
                    && !dir.exists()
                    && !FileLock::is_held(&self.lock_path(c))
            })
            .collect();
        out.sort_by_key(|e| std::cmp::Reverse(e.candidate.mtime_ns));
        out
    }

    /// Records a failure for `c` (used when the extracting process crashed).
    pub fn record_failure(&self, c: &Candidate, message: &str) -> Result<()> {
        let dir = self.current_dir(c);
        if dir.exists() {
            return Ok(());
        }
        let tmp = dir.with_file_name(format!(".tmp-fail-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).map_err(io_err(tmp.display()))?;
        std::fs::write(tmp.join("error.txt"), message).map_err(io_err(tmp.display()))?;
        std::fs::rename(&tmp, &dir).map_err(io_err(dir.display()))
    }
}

/// What [`ingest`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Extracted and published.
    Extracted { streams: usize },
    /// Already extracted (or failed) for this version of the file.
    UpToDate,
    /// Another process is extracting it.
    Busy,
    /// The extraction failed; the error was recorded in the store.
    Failed(String),
}

/// Extracts one recording into the store, unless another process is doing it or it
/// is already there.
pub async fn ingest(store: &Store, root: &Root, candidate: &Candidate) -> Result<Outcome> {
    std::fs::create_dir_all(&store.dir).map_err(io_err(store.dir.display()))?;
    let Some(_lock) = FileLock::try_acquire(&store.lock_path(candidate))? else {
        return Ok(Outcome::Busy);
    };
    let dest = store.current_dir(candidate);
    if dest.exists() {
        return Ok(Outcome::UpToDate);
    }
    let parent = dest
        .parent()
        .expect("dest is <store>/<key>/<fp>")
        .to_path_buf();
    // Only the lock holder writes here, so any leftover temporary directory is stale.
    for e in std::fs::read_dir(&parent).into_iter().flatten().flatten() {
        if e.file_name().to_string_lossy().starts_with(".tmp-") {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    let tmp = parent.join(format!(".tmp-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(io_err(tmp.display()))?;

    let source = root.path.join(&candidate.path);
    let outcome = match extract::extract(&source, candidate.kind, &tmp).await {
        Ok(x) => {
            let meta = RecordingMeta {
                recording: recording_id(candidate),
                root: candidate.root.clone(),
                path: candidate.path.clone(),
                kind: candidate.kind,
                hardware: root.hardware.clone(),
                size_bytes: candidate.size,
                mtime_ns: candidate.mtime_ns,
                extractor_version: EXTRACTOR_VERSION,
                streams: x.streams,
                notes: x.notes,
            };
            let json = serde_json::to_vec_pretty(&meta)
                .map_err(|e| err(ErrorKind::Internal, e.to_string()))?;
            std::fs::write(tmp.join("recording.json"), json).map_err(io_err(tmp.display()))?;
            Outcome::Extracted {
                streams: meta.streams.len(),
            }
        }
        Err(e) => {
            // Keep the error, drop any partial data.
            let _ = std::fs::remove_dir_all(&tmp);
            std::fs::create_dir_all(&tmp).map_err(io_err(tmp.display()))?;
            std::fs::write(tmp.join("error.txt"), e.message()).map_err(io_err(tmp.display()))?;
            Outcome::Failed(e.message().to_string())
        }
    };
    std::fs::rename(&tmp, &dest).map_err(io_err(dest.display()))?;
    // Older versions of the file are superseded.
    for e in std::fs::read_dir(&parent).into_iter().flatten().flatten() {
        if e.path() != dest {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    Ok(outcome)
}

/// Finds the recordings in every root and records them in the store. Returns them.
pub fn discover(cfg: &EffectiveConfig, max_time_per_root: Duration) -> Result<Vec<Entry>> {
    let set = roots::resolve(cfg);
    let mut entries = Vec::new();
    for root in &set.roots {
        for candidate in roots::candidates(root, 1_000_000, max_time_per_root).candidates {
            entries.push(Entry {
                candidate,
                hardware: root.hardware.clone(),
            });
        }
    }
    if let Err(e) = Store::for_config(cfg).write_candidates(&entries) {
        tracing::warn!(error = %e, "cannot record the recordings found");
    }
    Ok(entries)
}

/// Finds the root and recording for `root:path` (for the `ingest` command). The path
/// must stay inside the root.
pub fn locate(cfg: &EffectiveConfig, reference: &str) -> Result<(Root, Candidate)> {
    let rr = roots::RootRef::parse(reference)?;
    let set = roots::resolve(cfg);
    rr.resolve(&set)?;
    let root = set.get(&rr.root).cloned().ok_or_else(|| {
        err(
            ErrorKind::OutsideRoots,
            format!("no root named {:?}", rr.root),
        )
    })?;
    let candidate = roots::candidate(&root, &rr.rel).ok_or_else(|| {
        err(
            ErrorKind::RecordingUnreadable,
            format!("{reference} is not a recording Wise Bucket recognizes"),
        )
    })?;
    Ok((root, candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_fingerprints() {
        let c = Candidate {
            root: "bags".into(),
            path: "run1/a.mcap".into(),
            kind: RecordingKind::Mcap,
            size: 10,
            mtime_ns: 5,
        };
        let s = Store::new(Path::new("/state"));
        let d1 = s.current_dir(&c);
        assert!(d1.starts_with("/state/store"));
        let changed = Candidate {
            size: 11,
            ..c.clone()
        };
        let d2 = s.current_dir(&changed);
        assert_eq!(d1.parent(), d2.parent(), "same recording, same key");
        assert_ne!(d1, d2, "a changed file gets a new fingerprint");
        assert_eq!(recording_id(&c), "bags:run1/a.mcap");
    }
}
