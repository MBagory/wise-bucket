//! Backup and restore with `pg_dump -Fc` / `pg_restore`.
//!
//! Passwords are passed through `PGPASSWORD`, never on the command line.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{Database, EffectiveConfig};
use crate::error::{ErrorKind, Result, err};

use super::managed::{APP_ROLE, DB_NAME, SOCKET_PORT};
use super::managed_for;

struct Target {
    bin_dir: Option<PathBuf>,
    args: Vec<String>,
    password: Option<String>,
}

/// Splits `postgres://user:pass@host/db` into (`postgres://user@host/db`, `pass`).
fn split_password(url: &str) -> (String, Option<String>) {
    if let Some(scheme_end) = url.find("://") {
        let rest = &url[scheme_end + 3..];
        if let Some(at) = rest.rfind('@') {
            let creds = &rest[..at];
            if let Some(colon) = creds.find(':') {
                let without = format!(
                    "{}{}{}",
                    &url[..scheme_end + 3],
                    &creds[..colon],
                    &rest[at..]
                );
                return (without, Some(creds[colon + 1..].to_string()));
            }
        }
    }
    (url.to_string(), None)
}

fn target(cfg: &EffectiveConfig) -> Result<Target> {
    match &cfg.database.value {
        Database::Managed => {
            let m = managed_for(cfg).ok_or_else(|| err(ErrorKind::Internal, "managed expected"))?;
            Ok(Target {
                bin_dir: Some(m.pg_home()?.join("bin")),
                args: vec![
                    "-h".into(),
                    m.socket_dir()?.to_string_lossy().into(),
                    "-p".into(),
                    SOCKET_PORT.to_string(),
                    "-U".into(),
                    APP_ROLE.into(),
                    "-d".into(),
                    DB_NAME.into(),
                ],
                password: Some(m.secrets()?.app_password),
            })
        }
        Database::External { url } => {
            let (url, password) = split_password(url);
            Ok(Target {
                bin_dir: None,
                args: vec!["-d".into(), url],
                password,
            })
        }
    }
}

fn run(tool: &str, t: &Target, extra: &[&str]) -> Result<()> {
    let exe = t
        .bin_dir
        .as_ref()
        .map(|d| d.join(tool))
        .unwrap_or_else(|| PathBuf::from(tool));
    let mut cmd = Command::new(&exe);
    cmd.args(&t.args).args(extra);
    if let Some(p) = &t.password {
        cmd.env("PGPASSWORD", p);
    }
    let out = cmd.output().map_err(|e| {
        err(
            ErrorKind::BackupFailed,
            format!("cannot run {}: {e}", exe.display()),
        )
    })?;
    if !out.status.success() {
        return Err(err(
            ErrorKind::BackupFailed,
            format!(
                "{tool} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        ));
    }
    Ok(())
}

/// Writes a custom-format dump of the Wise Bucket database. The server must be running.
pub fn backup(cfg: &EffectiveConfig, file: &Path) -> Result<()> {
    let t = target(cfg)?;
    let f = file.to_string_lossy();
    // Only Wise Bucket's own objects: extensions belong to the superuser and are
    // recreated by `setup`/migrations, so they must not be part of the dump.
    run(
        "pg_dump",
        &t,
        &[
            "-Fc",
            "--no-owner",
            "-n",
            "wb",
            "-n",
            "wb_views",
            "-t",
            "public._sqlx_migrations",
            "-f",
            &f,
        ],
    )
}

/// Restores a dump produced by [`backup`], replacing existing objects. The server must be running.
pub fn restore(cfg: &EffectiveConfig, file: &Path) -> Result<()> {
    if !file.is_file() {
        return Err(err(
            ErrorKind::BackupFailed,
            format!("{} not found", file.display()),
        ));
    }
    let t = target(cfg)?;
    let f = file.to_string_lossy();
    run(
        "pg_restore",
        &t,
        &[
            "--clean",
            "--if-exists",
            "--no-owner",
            "--single-transaction",
            &f,
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_is_split_out() {
        assert_eq!(
            split_password("postgres://u:p%40ss@h:5432/db"),
            ("postgres://u@h:5432/db".into(), Some("p%40ss".into()))
        );
        assert_eq!(
            split_password("postgres://h/db"),
            ("postgres://h/db".into(), None)
        );
    }
}
