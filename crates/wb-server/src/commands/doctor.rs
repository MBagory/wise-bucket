//! `doctor`: one command that checks the whole installation.

use std::process::ExitCode;

use serde::Serialize;
use serde_json::json;
use wb_core::config::{self, EffectiveConfig};
use wb_core::error::{ErrorKind, Result, WbError, docs_ref, err};
use wb_core::{fsutil, roots};
use wb_data::store::{Status as RecordingStatus, Store};

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
            None,
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

    store_checks(&cfg, &mut checks).await;

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

async fn store_checks(cfg: &EffectiveConfig, checks: &mut Vec<Check>) {
    let store = Store::for_config(cfg);
    let counts = store.counts();
    let n = |st: RecordingStatus| counts.get(&st).copied().unwrap_or(0);
    checks.push(Check::ok(
        "store",
        format!(
            "{} · {} · {} done, {} pending, {} failed",
            store.dir.display(),
            ui::bytes(store.size()),
            n(RecordingStatus::Done) + n(RecordingStatus::Missing),
            n(RecordingStatus::Pending) + n(RecordingStatus::Extracting),
            n(RecordingStatus::Failed)
        ),
    ));
    if n(RecordingStatus::Failed) > 0 {
        checks.push(Check::warn(
            "extraction",
            format!(
                "{} recording(s) failed: `wisebucket sql \"SELECT recording, error FROM recordings WHERE status = 'failed'\"`",
                n(RecordingStatus::Failed)
            ),
            Some(ErrorKind::RecordingUnreadable),
        ));
    }
    if store.paused() {
        checks.push(Check::warn(
            "extraction",
            "paused: under 10 GB free on the store's disk",
            Some(ErrorKind::Io),
        ));
    }
    match wb_data::sql::query(&store, "SELECT count(*) AS n FROM recordings", None).await {
        Ok(_) => checks.push(Check::ok("sql", "queries run")),
        Err(e) => checks.push(Check::fail("sql", &e)),
    }
}
