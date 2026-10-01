//! Backup and restore with `pg_dump -Fc` / `pg_restore`.
//!
//! Passwords are passed through `PGPASSWORD`, never on the command line.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::EffectiveConfig;
use crate::error::{ErrorKind, Result, err};

use super::managed::{APP_ROLE, DB_NAME, SOCKET_PORT};
use super::managed_for;

struct Target {
    bin_dir: PathBuf,
    args: Vec<String>,
    password: String,
}

fn target(cfg: &EffectiveConfig) -> Result<Target> {
    let m = managed_for(cfg);
    Ok(Target {
        bin_dir: m.pg_home()?.join("bin"),
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
        password: m.secrets()?.app_password,
    })
}

fn run(tool: &str, t: &Target, extra: &[&str]) -> Result<()> {
    let exe = t.bin_dir.join(tool);
    let mut cmd = Command::new(&exe);
    cmd.args(&t.args).args(extra).env("PGPASSWORD", &t.password);
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
