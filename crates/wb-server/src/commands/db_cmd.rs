//! `db status|start|stop`, `backup`, `restore`.

use std::path::Path;

use serde_json::json;
use wb_core::config::{self, EffectiveConfig};
use wb_core::db::{self, Purpose, managed::Managed};
use wb_core::error::{ErrorKind, Result, err};
use wb_core::session;

use crate::cli::{DbCommand, GlobalArgs};
use crate::ui;

fn require_managed(cfg: &EffectiveConfig) -> Result<Managed> {
    db::managed_for(cfg).ok_or_else(|| {
        err(
            ErrorKind::ConfigInvalid,
            "this command controls the managed database; you are using an external PostgreSQL",
        )
    })
}

pub async fn run(global: &GlobalArgs, cmd: &DbCommand) -> Result<()> {
    let cfg = config::load(&global.overrides())?;
    match cmd {
        DbCommand::Status => status(global, &cfg).await,
        DbCommand::Start => {
            let m = require_managed(&cfg)?;
            m.start()?;
            ui::ok(format!(
                "managed PostgreSQL running (socket in {})",
                m.socket_dir()?.display()
            ));
            ui::info(
                "It stays up until `db stop` (sessions stop it when they end unless keep_running is set).",
            );
            Ok(())
        }
        DbCommand::Stop { force } => {
            let m = require_managed(&cfg)?;
            if !m.is_running()? {
                ui::ok("managed PostgreSQL is not running");
                return Ok(());
            }
            if !force {
                let handle = db::open(&cfg, Purpose::Cli).await?;
                let sessions = session::active(&handle.pool).await?;
                handle.pool.close().await;
                if !sessions.is_empty() {
                    return Err(err(
                        ErrorKind::DbStartFailed,
                        format!(
                            "{} MCP session(s) are connected; close them or use --force",
                            sessions.len()
                        ),
                    ));
                }
            }
            m.stop()?;
            ui::ok("managed PostgreSQL stopped");
            Ok(())
        }
    }
}

async fn status(global: &GlobalArgs, cfg: &EffectiveConfig) -> Result<()> {
    let mut report = json!({ "mode": cfg.database.value.mode_name(), "database": cfg.database.value.describe() });
    let managed = db::managed_for(cfg);
    let mut running = true;
    if let Some(m) = &managed {
        running = m.is_running()?;
        report["runtime_installed"] = json!(m.is_runtime_installed());
        report["pgvector_installed"] = json!(m.installed_pgvector_version());
        report["initialized"] = json!(m.is_initialized());
        report["running"] = json!(running);
        report["socket_dir"] = json!(m.socket_dir().ok());
        report["data_dir"] = json!(m.data_dir());
    }
    if running && managed.as_ref().is_none_or(|m| m.is_initialized()) {
        match db::open(cfg, Purpose::Cli).await {
            Ok(handle) => {
                let sessions = session::active(&handle.pool).await?;
                report["server"] = json!(handle.info);
                report["active_sessions"] = json!(sessions);
                handle.release(cfg).await?;
            }
            Err(e) => report["error"] = e.to_json(),
        }
    }
    if global.json {
        ui::json(&report);
        return Ok(());
    }
    println!("mode              {}", cfg.database.value.describe());
    if managed.is_some() {
        println!(
            "runtime           {}",
            yes_no(report["runtime_installed"].as_bool())
        );
        println!(
            "pgvector          {}",
            report["pgvector_installed"]
                .as_str()
                .unwrap_or("not installed")
        );
        println!(
            "initialized       {}",
            yes_no(report["initialized"].as_bool())
        );
        println!("running           {}", yes_no(report["running"].as_bool()));
        if let Some(s) = report["socket_dir"].as_str() {
            println!("socket dir        {s}");
        }
    }
    if let Some(s) = report.get("server") {
        println!(
            "server            PostgreSQL {}",
            s["server_version"].as_str().unwrap_or("?")
        );
        println!(
            "vector / pg_trgm  {} / {}",
            s["vector_version"].as_str().unwrap_or("-"),
            s["pg_trgm_version"].as_str().unwrap_or("-")
        );
        let sessions = report["active_sessions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        println!("active sessions   {}", sessions.len());
        for s in sessions {
            println!(
                "  - {} {} (pid {}) since {}",
                s["client_name"].as_str().unwrap_or("unknown client"),
                s["client_version"].as_str().unwrap_or(""),
                s["pid"],
                s["started_at"].as_str().unwrap_or("?")
            );
        }
    }
    if let Some(e) = report.get("error") {
        ui::warn(format!(
            "{} [{}]",
            e["message"].as_str().unwrap_or(""),
            e["code"].as_str().unwrap_or("")
        ));
    }
    Ok(())
}

fn yes_no(v: Option<bool>) -> &'static str {
    if v == Some(true) { "yes" } else { "no" }
}

pub async fn backup(global: &GlobalArgs, file: &Path) -> Result<()> {
    let cfg = config::load(&global.overrides())?;
    let handle = db::open(&cfg, Purpose::Cli).await?;
    let res = db::backup::backup(&cfg, file);
    handle.release(&cfg).await?;
    res?;
    ui::ok(format!("backup written to {}", file.display()));
    Ok(())
}

pub async fn restore(global: &GlobalArgs, file: &Path, yes: bool) -> Result<()> {
    if !yes {
        return Err(err(
            ErrorKind::BackupFailed,
            "restoring replaces all current Wise Bucket data; re-run with --yes to confirm",
        ));
    }
    let cfg = config::load(&global.overrides())?;
    let handle = db::open(&cfg, Purpose::Cli).await?;
    let res = db::backup::restore(&cfg, file);
    handle.release(&cfg).await?;
    res?;
    ui::ok(format!("restored from {}", file.display()));
    Ok(())
}
