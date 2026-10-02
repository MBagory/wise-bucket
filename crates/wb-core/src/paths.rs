//! Platform-specific default locations.

use std::path::{Path, PathBuf};

use directories::{BaseDirs, ProjectDirs};

use crate::error::{ErrorKind, Result, err};

/// Environment variables understood by Wise Bucket.
pub mod env {
    pub const CONFIG: &str = "WB_CONFIG";
    pub const STATE_DIR: &str = "WB_STATE_DIR";
    pub const RUNTIME_DIR: &str = "WB_RUNTIME_DIR";
    pub const KEEP_RUNNING: &str = "WB_KEEP_RUNNING";
}

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "wisebucket")
}

/// Default user configuration file.
///
/// * Linux: `~/.config/wisebucket/config.toml`
/// * macOS: `~/Library/Application Support/wisebucket/config.toml`
pub fn default_user_config_path() -> Result<PathBuf> {
    project_dirs()
        .map(|d| d.config_dir().join("config.toml"))
        .ok_or_else(|| {
            err(
                ErrorKind::StateDirUnavailable,
                "cannot determine the home directory",
            )
        })
}

/// Default state directory.
///
/// * Linux: `~/.local/share/wisebucket`
/// * macOS: `~/Library/Application Support/wisebucket`
pub fn default_state_dir() -> Result<PathBuf> {
    project_dirs()
        .map(|d| d.data_dir().to_path_buf())
        .ok_or_else(|| {
            err(
                ErrorKind::StateDirUnavailable,
                "cannot determine the home directory",
            )
        })
}

/// Expands a leading `~` or `~/` to the home directory.
pub fn expand_tilde(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if (s == "~" || s.starts_with("~/"))
        && let Some(home) = BaseDirs::new().map(|b| b.home_dir().to_path_buf())
    {
        return if s == "~" { home } else { home.join(&s[2..]) };
    }
    p.to_path_buf()
}

/// Makes `p` absolute relative to `base` (after `~` expansion).
pub fn absolutize(p: &Path, base: &Path) -> PathBuf {
    let p = expand_tilde(p);
    if p.is_absolute() { p } else { base.join(p) }
}

/// Whether paths on this platform should be compared case-insensitively.
///
/// macOS (APFS default) and Windows are case-insensitive; Linux is not.
pub fn case_insensitive_fs() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// A normalized key used to compare paths for containment/overlap.
pub fn path_key(p: &Path) -> PathBuf {
    if case_insensitive_fs() {
        PathBuf::from(p.to_string_lossy().to_lowercase())
    } else {
        p.to_path_buf()
    }
}

/// `true` if `inner` is `outer` or lies inside it (component-wise, case-aware).
pub fn is_within(inner: &Path, outer: &Path) -> bool {
    path_key(inner).starts_with(path_key(outer))
}
