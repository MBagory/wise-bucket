//! `doctor`: one command that checks the whole installation.

use std::process::ExitCode;

use serde::Serialize;
use serde_json::json;
use wb_core::config::{self, EffectiveConfig};
use wb_core::db::{self, Purpose};
use wb_core::error::{ErrorKind, Result, WbError, docs_ref, err};
use wb_core::{fsutil, roots};

use crate::cli::GlobalArgs;
use crate::ui::{self, Status};

#[derive(Debug, Serialize)]
pub struct Check {
    pub name: String,
    pub status: Status,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs_ref: Option<String>,
}

impl Check {
    fn ok(name: &str, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: Status::Ok,
            message: message.into(),
            code: None,
            docs_ref: None,
        }
    }
    fn warn(name: &str, message: impl Into<String>, kind: Option<ErrorKind>) -> Self {
        Self {
            name: name.into(),
            status: Status::Warn,
            message: message.into(),
            code: kind.map(|k| k.code()),
            docs_ref: kind.map(docs_ref),
        }
    }
    fn fail(name: &str, e: &WbError) -> Self {
        Self {
            name: name.into(),
            status: Status::Fail,
            message: e.message().to_string(),
            code: Some(e.code()),
            docs_ref: Some(e.docs_ref()),
        }
    }
}

/// Fails through the exit code only: the failures and their fixes are already listed.
pub async fn run(global: &GlobalArgs) -> Result<ExitCode> {
    let checks = collect(global).await;
    let failed = checks.iter().filter(|c| c.status == Status::Fail).count();
    let warned = checks.iter().filter(|c| c.status == Status::Warn).count();
    if global.json {
        ui::json(&json!({ "checks": checks, "failed": failed, "warnings": warned }));
    } else {
        for c in &checks {
            anstream::println!("{} {:<18} {}", ui::glyph(c.status), c.name, c.message);
            if let (Some(code), Some(r)) = (c.code, &c.docs_ref) {
                println!("  {:<18} ↳ {code} · {}", "", ui::docs_url(r));
            }
        }
        println!();
        println!(
            "{} check(s), {failed} failed, {warned} warning(s)",
            checks.len()
        );
    }
    Ok(if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

async fn collect(global: &GlobalArgs) -> Vec<Check> {
    let mut checks = Vec::new();
    let cfg = match config::load(&global.overrides()) {
        Ok(c) => {
            checks.push(Check::ok(
                "configuration",
                format!("loaded ({})", c.user_config_path.value.display()),
            ));
            c
        }
        Err(e) => {
            checks.push(Check::fail("configuration", &e));
            return checks;
        }
    };
    if !cfg.user_config_exists {
        checks.push(Check::warn(
            "user config",
            "not created yet: run `wisebucket setup`",
            Some(ErrorKind::DbNotInitialized),
        ));
    }

    // State directory and disk space.
    match fsutil::ensure_private_dir(&cfg.state_dir.value) {
        Ok(()) => {
            let free = fsutil::available_space(&cfg.state_dir.value);
            match free {
                Some(f) if f < 1024 * 1024 * 1024 => checks.push(Check::warn(
                    "state directory",
                    format!(
                        "{} · only {} free",
                        cfg.state_dir.value.display(),
                        ui::bytes(f)
                    ),
                    Some(ErrorKind::Io),
                )),
                Some(f) => checks.push(Check::ok(
                    "state directory",
                    format!("{} · {} free", cfg.state_dir.value.display(), ui::bytes(f)),
                )),
                None => checks.push(Check::ok(
                    "state directory",
                    cfg.state_dir.value.display().to_string(),
                )),
            }
        }
        Err(e) => checks.push(Check::fail(
            "state directory",
            &err(
                ErrorKind::StateDirUnavailable,
                format!("{}: {}", cfg.state_dir.value.display(), e.message()),
            ),
        )),
    }

    database_checks(&cfg, &mut checks).await;

    // Roots.
    let set = roots::resolve(&cfg);
    if cfg.roots.is_empty() {
        checks.push(Check::warn(
            "data roots",
            "none declared: `wisebucket roots add <name> <path>`",
            Some(ErrorKind::OutsideRoots),
        ));
    }
    for r in &set.roots {
        checks.push(Check::ok(
            "data root",
            format!("{} → {} ({})", r.name, r.path.display(), r.origin),
        ));
    }
    for (_, e) in &set.problems {
        checks.push(Check::fail("data root", e));
    }

    checks
}

async fn database_checks(cfg: &EffectiveConfig, checks: &mut Vec<Check>) {
    let m = db::managed_for(cfg);
    if !m.is_runtime_installed() {
        checks.push(Check::fail(
            "postgresql",
            &err(
                ErrorKind::DbNotInitialized,
                "managed PostgreSQL is not installed",
            ),
        ));
        return;
    }
    checks.push(Check::ok(
        "postgresql",
        format!("{} installed", db::managed::PG_VERSION),
    ));
    match m.installed_pgvector_version() {
        Some(v) if v == db::managed::PGVECTOR_VERSION => {
            checks.push(Check::ok("pgvector", format!("{v} installed")))
        }
        Some(v) => checks.push(Check::warn(
            "pgvector",
            format!(
                "{v} installed, {} expected: re-run setup",
                db::managed::PGVECTOR_VERSION
            ),
            Some(ErrorKind::VectorExtensionMissing),
        )),
        None => {
            let e = db::managed::check_toolchain().err().unwrap_or_else(|| {
                err(
                    ErrorKind::VectorExtensionMissing,
                    "pgvector is not installed: re-run setup",
                )
            });
            checks.push(Check::fail("pgvector", &e));
            return;
        }
    }
    if !m.is_initialized() {
        checks.push(Check::fail(
            "database",
            &err(ErrorKind::DbNotInitialized, "cluster not initialized"),
        ));
        return;
    }
    match m.socket_dir() {
        Ok(d) => checks.push(Check::ok("socket", d.display().to_string())),
        Err(e) => {
            checks.push(Check::fail("socket", &e));
            return;
        }
    }
    match db::open(cfg, Purpose::Cli).await {
        Ok(handle) => {
            let i = &handle.info;
            let sessions = wb_core::session::active(&handle.pool)
                .await
                .map(|s| s.len())
                .unwrap_or(0);
            checks.push(Check::ok(
                "database",
                format!(
                    "PostgreSQL {} · vector {} · pg_trgm {} · {} active session(s){}",
                    i.server_version,
                    i.vector_version.as_deref().unwrap_or("?"),
                    i.pg_trgm_version.as_deref().unwrap_or("?"),
                    sessions,
                    if handle.started_here {
                        " (started for this check)"
                    } else {
                        ""
                    }
                ),
            ));
            if let Err(e) = handle.release(cfg).await {
                checks.push(Check::fail("database stop", &e));
            }
        }
        Err(e) => checks.push(Check::fail("database", &e)),
    }
}
