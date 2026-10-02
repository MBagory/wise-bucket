//! `db status|start|stop`, `backup`, `restore`.

use std::path::Path;

use dialoguer::Confirm;
use serde_json::json;
use wb_core::config::{self, EffectiveConfig};
use wb_core::db::{self, Purpose};
use wb_core::error::{ErrorKind, Result, err};
use wb_core::session;

use crate::cli::{DbCommand, GlobalArgs};
use crate::ui;

pub async fn run(global: &GlobalArgs, cmd: &DbCommand) -> Result<()> {
    let cfg = config::load(&global.overrides())?;
    match cmd {
        DbCommand::Status => status(global, &cfg).await,
        DbCommand::Start => {
            let m = db::managed_for(&cfg);
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
            let m = db::managed_for(&cfg);
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
    let m = db::managed_for(cfg);
    let running = m.is_running()?;
    let mut report = json!({
        "runtime_installed": m.is_runtime_installed(),
        "pgvector_installed": m.installed_pgvector_version(),
        "initialized": m.is_initialized(),
        "running": running,
        "socket_dir": m.socket_dir().ok(),
        "data_dir": m.data_dir(),
    });
    if running && m.is_initialized() {
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
        if !ui::interactive(false) {
            return Err(err(
                ErrorKind::BackupFailed,
                "restoring replaces all current Wise Bucket data; re-run with --yes to confirm",
            ));
        }
        let confirmed = Confirm::new()
            .with_prompt(format!(
                "Replace all current Wise Bucket data with {}?",
                file.display()
            ))
            .default(false)
            .interact()
            .map_err(ui::prompt_err)?;
        if !confirmed {
            ui::warn("restore cancelled; nothing changed");
            return Ok(());
        }
    }
    let cfg = config::load(&global.overrides())?;
    let handle = db::open(&cfg, Purpose::Cli).await?;
    let res = db::backup::restore(&cfg, file);
    handle.release(&cfg).await?;
    res?;
    ui::ok(format!("restored from {}", file.display()));
    Ok(())
}
