//! `setup`: one-time machine setup (database, recording folders, agent connection).

use std::path::PathBuf;
use std::time::Duration;

use dialoguer::{Confirm, Input};
use serde_json::{Value, json};
use wb_core::config::{self, EffectiveConfig, RootSpec};
use wb_core::db::{self, Purpose, managed::Managed};
use wb_core::error::{ErrorKind, Result, err};
use wb_core::{fsutil, paths, roots};

use crate::cli::{GlobalArgs, SetupArgs};
use crate::ui;

/// Parses `name=path` (used by `setup --root`).
pub fn parse_root_arg(s: &str) -> Result<(String, PathBuf)> {
    let (name, path) = s.split_once('=').ok_or_else(|| {
        err(
            ErrorKind::RootInvalid,
            format!("{s:?}: expected NAME=PATH, e.g. bags=~/robot-logs/bags"),
        )
    })?;
    roots::validate_name(name)?;
    Ok((name.to_string(), PathBuf::from(path)))
}

pub async fn run(global: &GlobalArgs, args: &SetupArgs) -> Result<()> {
    let interactive = ui::interactive(args.yes);
    let cfg = config::load(&global.overrides())?;
    let user_cfg = cfg.user_config_path.value.clone();

    eprintln!("Wise Bucket setup");
    ui::info(format!("user configuration: {}", user_cfg.display()));

    // --- state directory
    let state_dir = cfg.state_dir.value.clone();
    fsutil::ensure_private_dir(&state_dir).map_err(|e| {
        err(
            ErrorKind::StateDirUnavailable,
            format!("{}: {}", state_dir.display(), e.message()),
        )
    })?;
    ui::ok(format!("state directory: {}", state_dir.display()));
    if let Some(free) = fsutil::available_space(&state_dir)
        && free < 2 * 1024 * 1024 * 1024
    {
        ui::warn(format!(
            "only {} free on this disk; managed PostgreSQL needs about 200 MB now and more as data grows. To use another disk, re-run with `--state-dir <dir>` or set `state_dir` in {}",
            ui::bytes(free),
            user_cfg.display()
        ));
    }

    // --- database
    setup_managed(&cfg).await?;
    if !cfg.user_config_exists {
        // Creates the user config so later commands know setup has run.
        config::set_key_in_file(&user_cfg, "database.keep_running", false.into())?;
    }

    // --- roots
    let cfg = config::load(&global.overrides())?;
    let mut added = 0usize;
    for spec in &args.roots {
        let (name, path) = parse_root_arg(spec)?;
        add_user_root(
            &cfg,
            RootSpec {
                name,
                path,
                robot: None,
                include: vec![],
                exclude: vec![],
            },
        )?;
        added += 1;
    }
    if interactive && !args.no_roots {
        let existing = roots::resolve(&cfg);
        if !existing.roots.is_empty() {
            ui::info(format!(
                "already declared roots: {}",
                existing
                    .roots
                    .iter()
                    .map(|r| r.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let mut first = added == 0 && existing.roots.is_empty();
        loop {
            let q = if first {
                "Add a folder containing recordings (rosbags, MCAP, …)?"
            } else {
                "Add another folder?"
            };
            if !Confirm::new()
                .with_prompt(q)
                .default(first)
                .interact()
                .map_err(ui::prompt_err)?
            {
                break;
            }
            first = false;
            let path: String = Input::new()
                .with_prompt("  Folder path")
                .interact_text()
                .map_err(ui::prompt_err)?;
            let path = paths::expand_tilde(PathBuf::from(path.trim()).as_path());
            let default_name = path
                .file_name()
                .map(|n| {
                    n.to_string_lossy().to_lowercase().replace(
                        |c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_',
                        "-",
                    )
                })
                .unwrap_or_else(|| "bags".into());
            let name: String = Input::new()
                .with_prompt("  Short name")
                .default(default_name)
                .interact_text()
                .map_err(ui::prompt_err)?;
            let robot: String = Input::new()
                .with_prompt("  Default robot for these recordings (optional)")
                .allow_empty(true)
                .interact_text()
                .map_err(ui::prompt_err)?;
            let spec = RootSpec {
                name: name.trim().to_string(),
                path,
                robot: Some(robot.trim().to_string()).filter(|r| !r.is_empty()),
                include: vec![],
                exclude: vec![],
            };
            match add_user_root(
                &config::load(&global.overrides()).unwrap_or_else(|_| cfg.clone()),
                spec,
            ) {
                Ok(()) => {}
                Err(e) => ui::error(&e),
            }
        }
    }

    eprintln!();
    ui::ok(format!("configuration written to {}", user_cfg.display()));
    print_connect(global)
}

/// Name of the server entry in MCP configuration files.
const SERVER_KEY: &str = "wise-bucket";

/// Path of this binary as harnesses should launch it: the `wisebucket` next to
/// it when run as the `wbk` alias, so both names give the same command.
fn server_exe() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let exe = dunce::canonicalize(&exe).unwrap_or(exe);
    let main = exe.with_file_name(format!("wisebucket{}", std::env::consts::EXE_SUFFIX));
    Ok(if main.is_file() { main } else { exe })
}

/// The MCP server entry (`command` + `args`) launching this binary.
fn server_entry(global: &GlobalArgs) -> Result<Value> {
    let cwd = std::env::current_dir()?;
    let mut args = vec!["serve".to_string()];
    // Persist explicit command-line choices so the harness uses the same state.
    if let Some(c) = &global.config {
        args.extend([
            "--config".into(),
            paths::absolutize(c, &cwd).display().to_string(),
        ]);
    }
    if let Some(s) = &global.state_dir {
        args.extend([
            "--state-dir".into(),
            paths::absolutize(s, &cwd).display().to_string(),
        ]);
    }
    Ok(json!({ "command": server_exe()?.display().to_string(), "args": args }))
}

/// Single-quotes `s` for a POSIX shell when it is not a plain word.
fn sh_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:@".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// Prints how to register Wise Bucket once, user-wide, in each agent. Never edits harness files.
fn print_connect(global: &GlobalArgs) -> Result<()> {
    let entry = server_entry(global)?;
    let words: Vec<String> = std::iter::once(entry["command"].as_str().unwrap_or_default())
        .chain(
            entry["args"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        )
        .map(sh_quote)
        .collect();
    eprintln!();
    eprintln!("Connect your agent (once, for all your folders). Claude Code:");
    println!(
        "claude mcp add --scope user {SERVER_KEY} -- {}",
        words.join(" ")
    );
    eprintln!("Other agents (Kilo Code, Cline, Cursor, …): add this to their global MCP settings:");
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({ "mcpServers": { SERVER_KEY: entry } }))
            .unwrap_or_default()
    );
    eprintln!("Then start a new agent session and ask: \"Is Wise Bucket ready?\"");
    Ok(())
}

/// Validates and appends a root to the user configuration, reporting what it contains.
pub fn add_user_root(cfg: &EffectiveConfig, spec: RootSpec) -> Result<()> {
    let root = roots::check_new_root(&cfg.roots, roots::user_decl(spec.clone(), cfg))?;
    let counts = roots::count_candidates(&root.path, 200_000, Duration::from_secs(10));
    // Store the canonical absolute path so the root does not depend on the current directory.
    let stored = RootSpec {
        path: root.path.clone(),
        ..spec
    };
    config::add_root_to_file(&cfg.user_config_path.value, &stored)?;
    ui::ok(format!(
        "root {:?} → {} · {} candidate recordings ({} MCAP, {} rosbag2 folders, {} ROS 1, {} ULog){}",
        root.name,
        root.path.display(),
        counts.total(),
        counts.mcap,
        counts.rosbag2_dirs,
        counts.ros1_bag,
        counts.ulog,
        if counts.truncated {
            ", scan truncated"
        } else {
            ""
        }
    ));
    Ok(())
}

async fn setup_managed(cfg: &EffectiveConfig) -> Result<()> {
    let m = Managed::new(&cfg.state_dir.value, &cfg.runtime_dir.value);
    let progress = |msg: &str| ui::step(msg);
    let m2 = m.clone();
    // Downloads and the pgvector build are blocking; keep them off the async runtime.
    tokio::task::spawn_blocking(move || -> Result<()> {
        let p = |msg: &str| ui::step(msg);
        m2.install_runtime(&p)?;
        m2.install_pgvector(&p)?;
        m2.init_cluster(&p)?;
        Ok(())
    })
    .await
    .map_err(|e| err(ErrorKind::Internal, e.to_string()))??;
    ui::ok(format!(
        "PostgreSQL {} + pgvector {} ready",
        db::managed::PG_VERSION,
        db::managed::PGVECTOR_VERSION
    ));

    let was_running = m.is_running()?;
    m.start()?;
    m.bootstrap(&progress).await?;
    let handle = db::open(cfg, Purpose::Cli).await?;
    ui::ok(format!(
        "database ready: PostgreSQL {}, vector {}, pg_trgm {}",
        handle.info.server_version,
        handle.info.vector_version.as_deref().unwrap_or("?"),
        handle.info.pg_trgm_version.as_deref().unwrap_or("?")
    ));
    handle.release(cfg).await?;
    if !was_running && !cfg.keep_running.value {
        db::stop_if_idle(&m).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quoting() {
        assert_eq!(sh_quote("/usr/bin/wisebucket"), "/usr/bin/wisebucket");
        assert_eq!(
            sh_quote("/Users/me/Library/Application Support/wb"),
            "'/Users/me/Library/Application Support/wb'"
        );
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
    }
}
