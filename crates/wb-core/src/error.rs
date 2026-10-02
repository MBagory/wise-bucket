//! Structured errors.
//!
//! Every error carries a stable [`ErrorKind::code`] and a documentation anchor
//! ([`ErrorKind::docs_ref`]). Tool results and CLI output always include both, so a
//! user (or the harness LLM) can jump straight to the fix in
//! `docs/reference/errors.md`, which is generated from [`CATALOG`].

use std::fmt;

/// Result alias used across Wise Bucket.
pub type Result<T, E = WbError> = std::result::Result<T, E>;

/// Documentation page that lists every error code.
pub const ERRORS_DOC: &str = "reference/errors.md";

macro_rules! error_kinds {
    ($( $variant:ident => $code:literal, $title:literal, $fix:literal; )*) => {
        /// Stable error categories. The string codes never change once released.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum ErrorKind { $( $variant, )* }

        impl ErrorKind {
            /// All kinds, in documentation order.
            pub const ALL: &'static [ErrorKind] = &[ $( ErrorKind::$variant, )* ];

            /// Stable, snake_case identifier (also the docs anchor).
            pub fn code(self) -> &'static str {
                match self { $( ErrorKind::$variant => $code, )* }
            }

            /// One-line human title.
            pub fn title(self) -> &'static str {
                match self { $( ErrorKind::$variant => $title, )* }
            }

            /// How to fix it (Markdown).
            pub fn fix(self) -> &'static str {
                match self { $( ErrorKind::$variant => $fix, )* }
            }
        }
    };
}

error_kinds! {
    ConfigInvalid => "config_invalid",
        "A configuration file could not be read or is invalid",
        "Run `wisebucket config show --origin` to see which file is involved, then fix the reported key. See the configuration reference for valid keys.";
    StateDirUnavailable => "state_dir_unavailable",
        "The state directory cannot be created or written",
        "Check permissions and free space, or choose another location with `--state-dir` or `WB_STATE_DIR`.";
    RootInvalid => "root_invalid",
        "A data root is invalid",
        "Root names use lowercase letters, digits, `-` and `_` (max 32 characters), and paths must point to an existing, readable directory. Fix it with `wisebucket roots add|remove`.";
    RootOverlap => "root_overlap",
        "Two data roots overlap",
        "A root cannot contain another root. Keep only the outer folder, or split them into sibling folders.";
    RootDuplicateName => "root_duplicate_name",
        "Two data roots share the same name",
        "Root names must be unique. Rename one of them with `wisebucket roots remove` then `roots add`.";
    OutsideRoots => "outside_roots",
        "A path is outside every configured data root",
        "Wise Bucket only reads files inside declared roots. Add the folder with `wisebucket roots add <name> <path>` (there is deliberately no MCP tool for this).";
    UnsupportedPlatform => "unsupported_platform",
        "Managed PostgreSQL is not available for this platform",
        "Wise Bucket supports Linux (x86_64, aarch64) and macOS (x86_64, arm64). On Windows, use WSL2.";
    DownloadFailed => "download_failed",
        "A download failed",
        "Check your network connection or proxy, then re-run the command (`setup` or `demo`): it resumes where it stopped.";
    ChecksumMismatch => "checksum_mismatch",
        "A downloaded file does not match its pinned checksum",
        "The file was corrupted, tampered with, or changed upstream. Re-run the command (`setup` or `demo`): files with a wrong checksum are downloaded again. If it persists, report it (see SECURITY.md).";
    DbNotInitialized => "db_not_initialized",
        "The managed database has not been set up yet",
        "Run `wisebucket setup` once on this machine.";
    DbStartFailed => "db_start_failed",
        "The managed PostgreSQL server could not be started",
        "Look at `<state-dir>/logs/postgres.log`. Common causes: a full disk, a stale `postmaster.pid` after a crash, or a socket directory that is not writable. `wisebucket doctor` checks all of them.";
    DbConnectFailed => "db_connect_failed",
        "Cannot connect to the database",
        "Run `wisebucket db status`, then `wisebucket doctor`.";
    VectorExtensionMissing => "vector_extension_missing",
        "The pgvector extension (0.8 or newer) is not available",
        "Re-run `wisebucket setup`: it reinstalls pgvector and recreates the extensions.";
    MigrationFailed => "migration_failed",
        "Applying database migrations failed",
        "Make a backup, then report the error with `wisebucket doctor` output attached. Do not edit the `wb` schema by hand.";
    SocketPathTooLong => "socket_path_too_long",
        "The PostgreSQL socket path is too long",
        "Unix sockets are limited to about 100 characters. Use a shorter `--state-dir`, or set `TMPDIR` to a short directory.";
    BackupFailed => "backup_failed",
        "Backup or restore failed",
        "Check the message for the `pg_dump`/`pg_restore` output.";
    Io => "io_error",
        "A file-system operation failed",
        "Check the path, permissions and free disk space mentioned in the message.";
    Internal => "internal_error",
        "Unexpected internal error",
        "This is a bug. Please open an issue with the message and the output of `wisebucket doctor`.";
}

/// A Wise Bucket error: a stable kind plus a contextual message.
#[derive(Debug)]
pub struct WbError {
    kind: ErrorKind,
    message: String,
}

impl WbError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn code(&self) -> &'static str {
        self.kind.code()
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// Relative link to the documentation entry, e.g. `reference/errors.md#root_overlap`.
    pub fn docs_ref(&self) -> String {
        docs_ref(self.kind)
    }

    /// JSON shape used in MCP tool results and `--json` CLI output.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "code": self.code(),
            "message": self.message,
            "docs_ref": self.docs_ref(),
            "fix": self.kind.fix(),
        })
    }
}

/// Documentation anchor for a kind.
pub fn docs_ref(kind: ErrorKind) -> String {
    format!("{ERRORS_DOC}#{}", kind.code())
}

impl fmt::Display for WbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code(), self.message)
    }
}

impl std::error::Error for WbError {}

impl From<std::io::Error> for WbError {
    fn from(e: std::io::Error) -> Self {
        WbError::new(ErrorKind::Io, e.to_string())
    }
}

/// Shorthand constructor: `err(ErrorKind::RootInvalid, "…")`.
pub fn err(kind: ErrorKind, message: impl Into<String>) -> WbError {
    WbError::new(kind, message)
}

/// Adds context to I/O errors.
pub trait IoContext<T> {
    fn io_ctx(self, what: impl FnOnce() -> String) -> Result<T>;
}

impl<T> IoContext<T> for std::io::Result<T> {
    fn io_ctx(self, what: impl FnOnce() -> String) -> Result<T> {
        self.map_err(|e| WbError::new(ErrorKind::Io, format!("{}: {e}", what())))
    }
}

/// Renders the errors reference page (Markdown) from the catalog.
pub fn render_errors_reference() -> String {
    let mut out = String::from(
        "<!-- GENERATED by `wisebucket docs gen`. Do not edit by hand. -->\n\n# Error reference\n\n\
         Every error reported by Wise Bucket, in the terminal or in an MCP tool result, carries a stable \
         `code` and a link to its entry below.\n\n| Code | Meaning |\n| --- | --- |\n",
    );
    for k in ErrorKind::ALL {
        out.push_str(&format!("| [`{0}`](#{0}) | {1} |\n", k.code(), k.title()));
    }
    for k in ErrorKind::ALL {
        out.push_str(&format!(
            "\n## {code}\n\n**{title}**\n\n{fix}\n",
            code = k.code(),
            title = k.title(),
            fix = k.fix()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn codes_are_unique_and_snake_case() {
        let mut seen = HashSet::new();
        for k in ErrorKind::ALL {
            let c = k.code();
            assert!(seen.insert(c), "duplicate code {c}");
            assert!(
                c.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'),
                "code {c} is not snake_case"
            );
            assert!(!k.fix().is_empty());
        }
    }

    #[test]
    fn every_code_is_documented_in_the_reference() {
        let page = render_errors_reference();
        for k in ErrorKind::ALL {
            assert!(page.contains(&format!("\n## {}\n", k.code())));
        }
    }

    #[test]
    fn json_shape_has_docs_ref() {
        let e = err(ErrorKind::RootOverlap, "a contains b");
        let j = e.to_json();
        assert_eq!(j["code"], "root_overlap");
        assert_eq!(j["docs_ref"], "reference/errors.md#root_overlap");
    }
}
