//! `setup`: one-time machine setup (database + recording folders).

use std::path::PathBuf;
use std::time::Duration;

use dialoguer::{Confirm, Input, Select};
use wb_core::config::{self, Database, EffectiveConfig, Origin, RootSpec};
use wb_core::db::{self, Purpose, managed::Managed};
use wb_core::error::{ErrorKind, Result, err};
use wb_core::{fsutil, paths, roots};

use crate::cli::{GlobalArgs, SetupArgs};
use crate::ui;

/// Parses `name=path` (used by `setup --root` and `init --root`).
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
    let mut cfg = config::load(&global.overrides())?;
    let user_cfg = cfg.user_config_path.value.clone();

    eprintln!("Wise Bucket setup");
    ui::info(format!("user configuration: {}", user_cfg.display()));

    // --- state directory
    if interactive && cfg.state_dir.origin == Origin::Default {
        let answer: String = Input::new()
            .with_prompt("State directory (database, caches, logs)")
            .default(cfg.state_dir.value.display().to_string())
            .interact_text()
            .map_err(ui::prompt_err)?;
        let chosen = paths::absolutize(
            PathBuf::from(answer.trim()).as_path(),
            &std::env::current_dir()?,
        );
        if chosen != cfg.state_dir.value {
            config::set_key_in_file(
                &user_cfg,
                "state_dir",
                chosen.display().to_string().into(),
                false,
            )?;
            cfg = config::load(&global.overrides())?;
        }
    }
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
            "only {} free on this disk; managed PostgreSQL needs about 200 MB now and more as data grows",
            ui::bytes(free)
        ));
    }

    // --- database
    let mut use_external = matches!(cfg.database.value, Database::External { .. });
    if interactive && !use_external {
        let choice = Select::new()
            .with_prompt("Database")
            .items([
                "Managed local PostgreSQL + pgvector (recommended)",
                "My own PostgreSQL server (>= 15 with pgvector >= 0.8)",
            ])
            .default(0)
            .interact()
            .map_err(ui::prompt_err)?;
        if choice == 1 {
            let url: String = Input::new()
                .with_prompt("PostgreSQL URL (postgres://user:password@host:5432/db)")
                .interact_text()
                .map_err(ui::prompt_err)?;
            cfg.database = config::Sourced::new(
                Database::External {
                    url: url.trim().to_string(),
                },
                Origin::Cli("setup".into()),
            );
            use_external = true;
        }
    }

    if use_external {
        setup_external(&cfg).await?;
        config::set_key_in_file(&user_cfg, "database.mode", "external".into(), false)?;
    } else {
        setup_managed(&cfg).await?;
        if !cfg.user_config_exists || cfg.database.origin == Origin::Default {
            config::set_key_in_file(&user_cfg, "database.mode", "managed".into(), false)?;
        }
    }

    // --- roots
    let cfg = config::load(&global.overrides()).or_else(|e| {
        // External mode written to the file needs the URL from the environment; reuse the in-memory config.
        if use_external {
            Ok(cfg.clone())
        } else {
            Err(e)
        }
    })?;
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
    eprintln!(
        "Next: in your robot repository, run `wise-bucket-server init`, then open your harness (e.g. Claude Code)."
    );
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
    config::add_root_to_file(&cfg.user_config_path.value, &stored, false)?;
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

async fn setup_external(cfg: &EffectiveConfig) -> Result<()> {
    ui::step(format!("checking {}", cfg.database.value.describe()));
    let handle = db::open(cfg, Purpose::Cli).await?;
    ui::ok(format!(
        "database ready: PostgreSQL {}, vector {}, pg_trgm {}, user {}",
        handle.info.server_version,
        handle.info.vector_version.as_deref().unwrap_or("?"),
        handle.info.pg_trgm_version.as_deref().unwrap_or("?"),
        handle.info.user
    ));
    handle.release(cfg).await?;
    ui::info(format!(
        "The URL is not stored. Export it for your harness, e.g. `export {}=…`, or add `--database-url` to the MCP config args.",
        wb_core::paths::env::DATABASE_URL
    ));
    Ok(())
}
