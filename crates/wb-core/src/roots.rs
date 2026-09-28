//! Data roots: the folders Wise Bucket is allowed to read.
//!
//! Nothing outside a root can ever be attached. Roots are declared by the
//! engineer (CLI or config files); there is deliberately no MCP tool to add one.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

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
    pub robot: Option<String>,
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
    let raw = &spec.path;
    if decl.from_project {
        if raw.is_absolute() || raw.to_string_lossy().starts_with('~') {
            return Err(err(
                ErrorKind::RootOutsideProject,
                format!(
                    "project root {:?} must be a path relative to the repository, got {}",
                    spec.name,
                    raw.display()
                ),
            ));
        }
        if raw.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(err(
                ErrorKind::RootOutsideProject,
                format!(
                    "project root {:?} must not use '..' ({})",
                    spec.name,
                    raw.display()
                ),
            ));
        }
    }
    let joined = paths::absolutize(raw, &decl.base_dir);
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
    if decl.from_project {
        let base = dunce::canonicalize(&decl.base_dir).unwrap_or_else(|_| decl.base_dir.clone());
        if !paths::is_within(&canonical, &base) {
            return Err(err(
                ErrorKind::RootOutsideProject,
                format!(
                    "project root {:?} resolves to {}, outside the project {} (symlink?)",
                    spec.name,
                    canonical.display(),
                    base.display()
                ),
            ));
        }
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
        robot: spec.robot.clone(),
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

/// Recording kinds recognized when counting candidates (full readers arrive in later milestones).
#[derive(Debug, Clone, Default, Serialize)]
pub struct CandidateCounts {
    pub mcap: u64,
    pub rosbag2_dirs: u64,
    pub ros1_bag: u64,
    pub ulog: u64,
    pub other: u64,
    /// `true` if the scan stopped early (entry or time limit).
    pub truncated: bool,
}

impl CandidateCounts {
    pub fn total(&self) -> u64 {
        self.mcap + self.rosbag2_dirs + self.ros1_bag + self.ulog
    }
}

/// Counts candidate recordings under a root without following symlinks.
pub fn count_candidates(root: &Path, max_entries: u64, max_time: Duration) -> CandidateCounts {
    let start = Instant::now();
    let mut counts = CandidateCounts::default();
    let mut seen = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files = Vec::new();
        for entry in entries.flatten() {
            seen += 1;
            if seen > max_entries || start.elapsed() > max_time {
                counts.truncated = true;
                return counts;
            }
            let Ok(ft) = entry.file_type() else { continue };
            let name = entry.file_name().to_string_lossy().to_string();
            if ft.is_dir() {
                if !name.starts_with('.') {
                    stack.push(entry.path());
                }
            } else if ft.is_file() {
                files.push(name);
            }
        }
        // A rosbag2 folder (metadata.yaml + .mcap/.db3 splits) is one recording.
        if files.iter().any(|f| f == "metadata.yaml") {
            counts.rosbag2_dirs += 1;
            continue;
        }
        for name in files {
            match Path::new(&name)
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("mcap") => counts.mcap += 1,
                Some("bag") => counts.ros1_bag += 1,
                Some("ulg") => counts.ulog += 1,
                _ => counts.other += 1,
            }
        }
    }
    counts
}

/// Convenience: build a [`RootDecl`] for a user-config root.
pub fn user_decl(spec: RootSpec, cfg: &EffectiveConfig) -> RootDecl {
    RootDecl {
        spec,
        origin: Origin::UserConfig(cfg.user_config_path.value.clone()),
        from_project: false,
        base_dir: std::env::current_dir().unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(name: &str, path: &Path, from_project: bool, base: &Path) -> RootDecl {
        RootDecl {
            spec: RootSpec {
                name: name.into(),
                path: path.to_path_buf(),
                robot: None,
                include: vec![],
                exclude: vec![],
            },
            origin: Origin::Default,
            from_project,
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
            decl("outer", &d.path().join("a"), false, d.path()),
            decl("inner", &d.path().join("a/b"), false, d.path()),
            decl("other", &d.path().join("c"), false, d.path()),
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
            decl("x", d.path(), false, d.path()),
            decl("x", d.path(), false, d.path()),
            decl("gone", &d.path().join("nope"), false, d.path()),
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
    fn project_roots_must_stay_inside() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        std::fs::create_dir_all(repo.join("bags")).unwrap();
        std::fs::create_dir_all(d.path().join("elsewhere")).unwrap();
        resolve_decl(&decl("ok", Path::new("./bags"), true, &repo)).unwrap();
        for bad in [
            d.path().join("elsewhere"),
            PathBuf::from("../elsewhere"),
            PathBuf::from("~/x"),
        ] {
            assert_eq!(
                resolve_decl(&decl("b", &bad, true, &repo))
                    .unwrap_err()
                    .kind(),
                ErrorKind::RootOutsideProject
            );
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(d.path().join("elsewhere"), repo.join("link")).unwrap();
            assert_eq!(
                resolve_decl(&decl("l", Path::new("link"), true, &repo))
                    .unwrap_err()
                    .kind(),
                ErrorKind::RootOutsideProject
            );
        }
    }

    #[test]
    fn root_refs_cannot_escape() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("bags/run1")).unwrap();
        std::fs::create_dir_all(d.path().join("secret")).unwrap();
        let set = resolve_decls(&[decl("bags", &d.path().join("bags"), false, d.path())]);
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
    fn counts_recordings() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir_all(r.join("run1")).unwrap();
        std::fs::write(r.join("run1/metadata.yaml"), "").unwrap();
        std::fs::write(r.join("run1/run1_0.mcap"), "").unwrap();
        std::fs::write(r.join("loose.MCAP"), "").unwrap();
        std::fs::write(r.join("old.bag"), "").unwrap();
        std::fs::write(r.join("flight.ulg"), "").unwrap();
        let c = count_candidates(r, 10_000, Duration::from_secs(5));
        assert_eq!((c.mcap, c.rosbag2_dirs, c.ros1_bag, c.ulog), (1, 1, 1, 1));
        assert!(!c.truncated);
        let c = count_candidates(r, 2, Duration::from_secs(5));
        assert!(c.truncated);
    }
}
