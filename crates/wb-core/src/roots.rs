//! Data roots: the folders Wise Bucket is allowed to read.
//!
//! Nothing outside a root can ever be attached. Roots are declared by the
//! engineer (CLI or config files); there is deliberately no MCP tool to add one.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};

use crate::config::{EffectiveConfig, Origin, RootDecl, RootSpec};
use crate::error::{ErrorKind, Result, WbError, err};
use crate::paths;

/// Maximum length of a root name.
pub const MAX_NAME_LEN: usize = 32;

/// A validated root.
#[derive(Debug, Clone, Serialize)]
pub struct Root {
    pub name: String,
    /// Canonical absolute path.
    pub path: PathBuf,
    pub hardware: Option<String>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub origin: Origin,
}

/// Validates a root name.
pub fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(err(
            ErrorKind::RootInvalid,
            format!(
                "invalid root name {name:?}: use lowercase letters, digits, '-' and '_', starting with a letter or digit, at most {MAX_NAME_LEN} characters"
            ),
        ))
    }
}

/// Resolves one declaration to a canonical, existing directory.
pub fn resolve_decl(decl: &RootDecl) -> Result<Root> {
    let spec = &decl.spec;
    validate_name(&spec.name)?;
    let joined = paths::absolutize(&spec.path, &decl.base_dir);
    let canonical = dunce::canonicalize(&joined).map_err(|e| {
        err(
            ErrorKind::RootInvalid,
            format!(
                "root {:?}: {} is not accessible: {e}",
                spec.name,
                joined.display()
            ),
        )
    })?;
    if !canonical.is_dir() {
        return Err(err(
            ErrorKind::RootInvalid,
            format!(
                "root {:?}: {} is not a directory",
                spec.name,
                canonical.display()
            ),
        ));
    }
    std::fs::read_dir(&canonical).map_err(|e| {
        err(
            ErrorKind::RootInvalid,
            format!(
                "root {:?}: {} is not readable: {e}",
                spec.name,
                canonical.display()
            ),
        )
    })?;
    Ok(Root {
        name: spec.name.clone(),
        path: canonical,
        hardware: spec.hardware.clone(),
        include: spec.include.clone(),
        exclude: spec.exclude.clone(),
        origin: decl.origin.clone(),
    })
}

/// Outcome of validating all declared roots.
#[derive(Debug, Default)]
pub struct RootSet {
    pub roots: Vec<Root>,
    /// Problems, each tied to a root name when possible.
    pub problems: Vec<(Option<String>, WbError)>,
}

impl RootSet {
    pub fn get(&self, name: &str) -> Option<&Root> {
        self.roots.iter().find(|r| r.name == name)
    }
}

/// Validates every declared root: names, paths, duplicates and overlaps.
///
/// Invalid roots are reported in [`RootSet::problems`] and left out, so one bad
/// entry never disables the others.
pub fn resolve(cfg: &EffectiveConfig) -> RootSet {
    resolve_decls(&cfg.roots)
}

pub fn resolve_decls(decls: &[RootDecl]) -> RootSet {
    let mut set = RootSet::default();
    let mut names: BTreeMap<String, usize> = BTreeMap::new();
    for decl in decls {
        *names.entry(decl.spec.name.clone()).or_default() += 1;
    }
    for decl in decls {
        let name = decl.spec.name.clone();
        if names[&name] > 1 {
            set.problems.push((
                Some(name.clone()),
                err(
                    ErrorKind::RootDuplicateName,
                    format!("root name {name:?} is declared {} times", names[&name]),
                ),
            ));
            continue;
        }
        match resolve_decl(decl) {
            Ok(root) => set.roots.push(root),
            Err(e) => set.problems.push((Some(name), e)),
        }
    }
    // Overlaps: drop both sides so the boundary stays unambiguous.
    let mut overlapping = vec![false; set.roots.len()];
    for i in 0..set.roots.len() {
        for j in (i + 1)..set.roots.len() {
            let (a, b) = (&set.roots[i], &set.roots[j]);
            if paths::is_within(&a.path, &b.path) || paths::is_within(&b.path, &a.path) {
                overlapping[i] = true;
                overlapping[j] = true;
                set.problems.push((
                    Some(a.name.clone()),
                    err(
                        ErrorKind::RootOverlap,
                        format!(
                            "roots {:?} ({}) and {:?} ({}) overlap",
                            a.name,
                            a.path.display(),
                            b.name,
                            b.path.display()
                        ),
                    ),
                ));
            }
        }
    }
    let mut i = 0;
    set.roots.retain(|_| {
        let keep = !overlapping[i];
        i += 1;
        keep
    });
    set
}

/// Checks that a new root would be valid next to the existing ones.
pub fn check_new_root(existing: &[RootDecl], new: RootDecl) -> Result<Root> {
    let mut all = existing.to_vec();
    let name = new.spec.name.clone();
    all.push(new);
    let set = resolve_decls(&all);
    if let Some((_, e)) = set.problems.into_iter().find(|(n, e)| {
        n.as_deref() == Some(&name)
            || e.kind() == ErrorKind::RootOverlap && e.message().contains(&format!("{name:?}"))
    }) {
        return Err(e);
    }
    set.roots
        .into_iter()
        .find(|r| r.name == name)
        .ok_or_else(|| err(ErrorKind::Internal, "new root missing after validation"))
}

/// A reference to a path inside a root: `name:relative/path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootRef {
    pub root: String,
    pub rel: PathBuf,
}

impl RootRef {
    pub fn parse(s: &str) -> Result<Self> {
        let (root, rel) = s.split_once(':').ok_or_else(|| {
            err(
                ErrorKind::OutsideRoots,
                format!("{s:?} is not a root reference (expected name:relative/path)"),
            )
        })?;
        validate_name(root)?;
        let rel = PathBuf::from(rel.trim_start_matches('/'));
        if rel
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
        {
            return Err(err(
                ErrorKind::OutsideRoots,
                format!("{s:?} must stay inside the root"),
            ));
        }
        Ok(Self {
            root: root.to_string(),
            rel,
        })
    }

    /// Resolves against the root set, refusing anything that escapes the root (symlinks included).
    pub fn resolve(&self, set: &RootSet) -> Result<PathBuf> {
        let root = set.get(&self.root).ok_or_else(|| {
            err(
                ErrorKind::OutsideRoots,
                format!("no root named {:?}", self.root),
            )
        })?;
        let p = dunce::canonicalize(root.path.join(&self.rel)).map_err(|e| {
            err(
                ErrorKind::OutsideRoots,
                format!("{}:{}: {e}", self.root, self.rel.display()),
            )
        })?;
        if !paths::is_within(&p, &root.path) {
            return Err(err(
                ErrorKind::OutsideRoots,
                format!(
                    "{}:{} resolves outside the root",
                    self.root,
                    self.rel.display()
                ),
            ));
        }
        Ok(p)
    }
}

/// Finds which root contains an absolute path and returns the root reference.
pub fn locate(set: &RootSet, path: &Path) -> Result<RootRef> {
    let p = dunce::canonicalize(path)
        .map_err(|e| err(ErrorKind::OutsideRoots, format!("{}: {e}", path.display())))?;
    for r in &set.roots {
        if paths::is_within(&p, &r.path) {
            let rel = p
                .strip_prefix(&r.path)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| {
                    // Case-insensitive match with different case: fall back to component count.
                    p.components().skip(r.path.components().count()).collect()
                });
            return Ok(RootRef {
                root: r.name.clone(),
                rel,
            });
        }
    }
    Err(err(
        ErrorKind::OutsideRoots,
        format!("{} is not inside any configured root", p.display()),
    ))
}

/// Kind of recording found in a data root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingKind {
    /// A rosbag2 folder: `metadata.yaml` plus `.mcap` or `.db3` splits.
    Rosbag2,
    Mcap,
    /// A rosbag2 SQLite file outside a rosbag2 folder.
    Db3,
    Ulog,
    Csv,
    JsonLines,
    Parquet,
    ArrowIpc,
    /// A ROS 1 bag: recognized, not readable yet.
    Ros1Bag,
}

impl RecordingKind {
    /// Kind of a loose file, from its extension (case-insensitive).
    pub fn from_extension(ext: &str) -> Option<Self> {
        Some(match ext.to_ascii_lowercase().as_str() {
            "mcap" => Self::Mcap,
            "db3" => Self::Db3,
            "ulg" => Self::Ulog,
            "csv" => Self::Csv,
            "jsonl" | "ndjson" => Self::JsonLines,
            "parquet" => Self::Parquet,
            "arrow" | "feather" | "ipc" => Self::ArrowIpc,
            "bag" => Self::Ros1Bag,
            _ => return None,
        })
    }

    /// Whether Wise Bucket can extract this kind into the store.
    pub fn supported(self) -> bool {
        self != Self::Ros1Bag
    }

    /// Plural label for summaries.
    pub fn label(self) -> &'static str {
        match self {
            Self::Rosbag2 => "rosbag2 folders",
            Self::Mcap => "MCAP",
            Self::Db3 => "rosbag2 SQLite",
            Self::Ulog => "ULog",
            Self::Csv => "CSV",
            Self::JsonLines => "JSON Lines",
            Self::Parquet => "Parquet",
            Self::ArrowIpc => "Arrow IPC",
            Self::Ros1Bag => "ROS 1 bags (not readable yet)",
        }
    }
}

/// A recording found in a data root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    /// Name of the data root.
    pub root: String,
    /// Path relative to the root (a folder for a rosbag2 recording).
    pub path: PathBuf,
    pub kind: RecordingKind,
    /// Size in bytes (all splits for a rosbag2 folder).
    pub size: u64,
    /// Latest modification time, in nanoseconds since the Unix epoch.
    pub mtime_ns: i64,
}

/// The recordings found in a root.
#[derive(Debug, Clone, Default)]
pub struct Walk {
    pub candidates: Vec<Candidate>,
    /// Files that are not recordings.
    pub other: u64,
    /// `true` if the walk stopped early (entry or time limit).
    pub truncated: bool,
}

/// How many recordings of each kind a root holds.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CandidateCounts {
    #[serde(flatten)]
    pub by_kind: BTreeMap<RecordingKind, u64>,
    pub other: u64,
    /// `true` if the scan stopped early (entry or time limit).
    pub truncated: bool,
}

impl CandidateCounts {
    /// Number of recordings, readable or not.
    pub fn total(&self) -> u64 {
        self.by_kind.values().sum()
    }

    pub fn get(&self, kind: RecordingKind) -> u64 {
        self.by_kind.get(&kind).copied().unwrap_or(0)
    }

    /// Human summary, e.g. `37 recordings (31 MCAP, 6 rosbag2 folders)`.
    pub fn summary(&self) -> String {
        let parts: Vec<String> = self
            .by_kind
            .iter()
            .map(|(k, n)| format!("{n} {}", k.label()))
            .collect();
        format!(
            "{} recording(s){}{}",
            self.total(),
            if parts.is_empty() {
                String::new()
            } else {
                format!(" ({})", parts.join(", "))
            },
            if self.truncated {
                ", scan stopped early"
            } else {
                ""
            }
        )
    }
}

impl From<&Walk> for CandidateCounts {
    fn from(w: &Walk) -> Self {
        let mut by_kind = BTreeMap::new();
        for c in &w.candidates {
            *by_kind.entry(c.kind).or_default() += 1;
        }
        Self {
            by_kind,
            other: w.other,
            truncated: w.truncated,
        }
    }
}

fn globset(patterns: &[String]) -> GlobSet {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        // Invalid patterns are ignored here; `doctor` would be the place to report them.
        if let Ok(g) = Glob::new(p) {
            b.add(g);
        }
    }
    b.build().unwrap_or_else(|_| GlobSet::empty())
}

fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// Lists the recordings in a root, without following symlinks, applying its
/// `include`/`exclude` globs (matched against file paths relative to the root; a
/// rosbag2 folder is matched through its `metadata.yaml`).
pub fn candidates(root: &Root, max_entries: u64, max_time: Duration) -> Walk {
    let include = globset(&root.include);
    let exclude = globset(&root.exclude);
    let wanted =
        |rel: &Path| (root.include.is_empty() || include.is_match(rel)) && !exclude.is_match(rel);
    let start = Instant::now();
    let mut walk = Walk::default();
    let mut seen = 0u64;
    let mut stack = vec![root.path.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files = Vec::new();
        for entry in entries.flatten() {
            seen += 1;
            if seen > max_entries || start.elapsed() > max_time {
                walk.truncated = true;
                return walk;
            }
            let Ok(ft) = entry.file_type() else { continue };
            let name = entry.file_name().to_string_lossy().to_string();
            if ft.is_dir() {
                if !name.starts_with('.') {
                    stack.push(entry.path());
                }
            } else if ft.is_file()
                && let Ok(meta) = entry.metadata()
            {
                files.push((name, meta));
            }
        }
        let rel_dir = dir.strip_prefix(&root.path).unwrap_or(&dir).to_path_buf();
        // A rosbag2 folder (metadata.yaml + .mcap/.db3 splits) is one recording.
        if files.iter().any(|(f, _)| f == "metadata.yaml") {
            // Globs describe files: a rosbag2 folder is matched through its metadata.yaml.
            if wanted(&rel_dir.join("metadata.yaml")) {
                walk.candidates.push(Candidate {
                    root: root.name.clone(),
                    path: rel_dir,
                    kind: RecordingKind::Rosbag2,
                    size: files.iter().map(|(_, m)| m.len()).sum(),
                    mtime_ns: files.iter().map(|(_, m)| mtime_ns(m)).max().unwrap_or(0),
                });
            }
            continue;
        }
        for (name, meta) in files {
            let rel = rel_dir.join(&name);
            let kind = Path::new(&name)
                .extension()
                .and_then(|e| e.to_str())
                .and_then(RecordingKind::from_extension);
            match kind {
                Some(kind) if wanted(&rel) => walk.candidates.push(Candidate {
                    root: root.name.clone(),
                    path: rel,
                    kind,
                    size: meta.len(),
                    mtime_ns: mtime_ns(&meta),
                }),
                Some(_) => {}
                None => walk.other += 1,
            }
        }
    }
    walk
}

/// The recording at `rel` inside a root (a file, or a rosbag2 folder), if it is one.
pub fn candidate(root: &Root, rel: &Path) -> Option<Candidate> {
    let abs = root.path.join(rel);
    let meta = std::fs::metadata(&abs).ok()?;
    let (kind, size, mtime) = if meta.is_dir() {
        let files: Vec<std::fs::Metadata> = std::fs::read_dir(&abs)
            .ok()?
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| e.metadata().ok())
            .collect();
        if !abs.join("metadata.yaml").is_file() {
            return None;
        }
        (
            RecordingKind::Rosbag2,
            files.iter().map(|m| m.len()).sum(),
            files.iter().map(mtime_ns).max().unwrap_or(0),
        )
    } else {
        let ext = abs.extension()?.to_str()?;
        (
            RecordingKind::from_extension(ext)?,
            meta.len(),
            mtime_ns(&meta),
        )
    };
    Some(Candidate {
        root: root.name.clone(),
        path: rel.to_path_buf(),
        kind,
        size,
        mtime_ns: mtime,
    })
}

/// Counts the recordings in a root (see [`candidates`]).
pub fn count_candidates(root: &Root, max_entries: u64, max_time: Duration) -> CandidateCounts {
    CandidateCounts::from(&candidates(root, max_entries, max_time))
}

/// Convenience: build a [`RootDecl`] for a user-config root.
pub fn user_decl(spec: RootSpec, cfg: &EffectiveConfig) -> RootDecl {
    RootDecl {
        spec,
        origin: Origin::UserConfig(cfg.user_config_path.value.clone()),
        base_dir: std::env::current_dir().unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(name: &str, path: &Path, base: &Path) -> RootDecl {
        RootDecl {
            spec: RootSpec {
                name: name.into(),
                path: path.to_path_buf(),
                hardware: None,
                include: vec![],
                exclude: vec![],
            },
            origin: Origin::Default,
            base_dir: base.to_path_buf(),
        }
    }

    #[test]
    fn names() {
        for ok in ["bags", "nas-flights", "a1", "x_y"] {
            validate_name(ok).unwrap();
        }
        for bad in ["", "Bags", "-x", "a b", "é", &"x".repeat(33)] {
            assert_eq!(
                validate_name(bad).unwrap_err().kind(),
                ErrorKind::RootInvalid,
                "{bad}"
            );
        }
    }

    #[test]
    fn overlap_is_rejected_and_both_dropped() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("a/b")).unwrap();
        std::fs::create_dir_all(d.path().join("c")).unwrap();
        let set = resolve_decls(&[
            decl("outer", &d.path().join("a"), d.path()),
            decl("inner", &d.path().join("a/b"), d.path()),
            decl("other", &d.path().join("c"), d.path()),
        ]);
        assert_eq!(
            set.roots
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>(),
            vec!["other"]
        );
        assert!(
            set.problems
                .iter()
                .any(|(_, e)| e.kind() == ErrorKind::RootOverlap)
        );
    }

    #[test]
    fn duplicates_and_missing_paths() {
        let d = tempfile::tempdir().unwrap();
        let set = resolve_decls(&[
            decl("x", d.path(), d.path()),
            decl("x", d.path(), d.path()),
            decl("gone", &d.path().join("nope"), d.path()),
        ]);
        assert!(set.roots.is_empty());
        assert_eq!(
            set.problems
                .iter()
                .filter(|(_, e)| e.kind() == ErrorKind::RootDuplicateName)
                .count(),
            2
        );
        assert!(
            set.problems
                .iter()
                .any(|(_, e)| e.kind() == ErrorKind::RootInvalid)
        );
    }

    #[test]
    fn root_refs_cannot_escape() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("bags/run1")).unwrap();
        std::fs::create_dir_all(d.path().join("secret")).unwrap();
        let set = resolve_decls(&[decl("bags", &d.path().join("bags"), d.path())]);
        let p = RootRef::parse("bags:run1").unwrap().resolve(&set).unwrap();
        assert!(p.ends_with("run1"));
        assert!(RootRef::parse("bags:../secret").is_err());
        assert_eq!(
            RootRef::parse("nope:x")
                .unwrap()
                .resolve(&set)
                .unwrap_err()
                .kind(),
            ErrorKind::OutsideRoots
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(d.path().join("secret"), d.path().join("bags/escape"))
                .unwrap();
            assert_eq!(
                RootRef::parse("bags:escape")
                    .unwrap()
                    .resolve(&set)
                    .unwrap_err()
                    .kind(),
                ErrorKind::OutsideRoots
            );
        }
        let r = locate(&set, &d.path().join("bags/run1")).unwrap();
        assert_eq!(
            r,
            RootRef {
                root: "bags".into(),
                rel: "run1".into()
            }
        );
        assert_eq!(
            locate(&set, &d.path().join("secret")).unwrap_err().kind(),
            ErrorKind::OutsideRoots
        );
    }

    #[test]
    fn finds_recordings_with_globs() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir_all(r.join("run1")).unwrap();
        std::fs::write(r.join("run1/metadata.yaml"), "").unwrap();
        std::fs::write(r.join("run1/run1_0.mcap"), "abc").unwrap();
        std::fs::write(r.join("loose.MCAP"), "").unwrap();
        std::fs::write(r.join("old.bag"), "").unwrap();
        std::fs::write(r.join("flight.ulg"), "").unwrap();
        std::fs::write(r.join("bench.csv"), "").unwrap();
        std::fs::write(r.join("notes.txt"), "").unwrap();
        std::fs::create_dir_all(r.join("tmp")).unwrap();
        std::fs::write(r.join("tmp/scratch.csv"), "").unwrap();
        let set = resolve_decls(&[decl("bags", r, r)]);
        let mut root = set.roots[0].clone();
        let c = count_candidates(&root, 10_000, Duration::from_secs(5));
        use RecordingKind::*;
        assert_eq!(
            [Rosbag2, Mcap, Ros1Bag, Ulog, Csv].map(|k| c.get(k)),
            [1, 1, 1, 1, 2]
        );
        assert_eq!((c.total(), c.other, c.truncated), (6, 1, false));
        let w = candidates(&root, 10_000, Duration::from_secs(5));
        let bag = w.candidates.iter().find(|c| c.kind == Rosbag2).unwrap();
        assert_eq!((bag.path.as_path(), bag.size), (Path::new("run1"), 3));
        assert!(bag.mtime_ns > 0);
        assert_eq!(candidate(&root, Path::new("run1")).as_ref(), Some(bag));
        assert_eq!(candidate(&root, Path::new("bench.csv")).unwrap().kind, Csv);
        assert!(candidate(&root, Path::new("notes.txt")).is_none());
        assert!(candidate(&root, Path::new("tmp")).is_none());

        root.exclude = vec!["tmp/**".into()];
        assert_eq!(
            count_candidates(&root, 10_000, Duration::from_secs(5)).get(Csv),
            1
        );
        root.exclude.clear();
        root.include = vec!["**/*.csv".into()];
        let c = count_candidates(&root, 10_000, Duration::from_secs(5));
        assert_eq!((c.total(), c.get(Csv)), (2, 2));
        root.include = vec!["**/metadata.yaml".into()];
        let c = count_candidates(&root, 10_000, Duration::from_secs(5));
        assert_eq!((c.total(), c.get(Rosbag2)), (1, 1));
        assert!(count_candidates(&root, 2, Duration::from_secs(5)).truncated);
    }
}
