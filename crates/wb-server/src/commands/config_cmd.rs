//! `config show|path`.

use serde_json::json;
use wb_core::config;
use wb_core::error::Result;
use wb_core::roots;

use crate::cli::{ConfigCommand, GlobalArgs};
use crate::ui;

pub async fn run(global: &GlobalArgs, cmd: &ConfigCommand) -> Result<()> {
    let cfg = config::load(&global.overrides())?;
    match cmd {
        ConfigCommand::Path => {
            if global.json {
                ui::json(&json!({
                    "user_config": cfg.user_config_path.value,
                    "project_config": cfg.project.as_ref().map(|p| &p.config_path),
                    "state_dir": cfg.state_dir.value,
                }));
            } else {
                println!(
                    "user config:    {}{}",
                    cfg.user_config_path.value.display(),
                    if cfg.user_config_exists {
                        ""
                    } else {
                        "  (not created yet)"
                    }
                );
                match &cfg.project {
                    Some(p) => println!(
                        "project config: {}{}",
                        p.config_path.display(),
                        if p.config_exists {
                            ""
                        } else {
                            "  (not created yet)"
                        }
                    ),
                    None => {
                        println!("project config: (no project found from the current directory)")
                    }
                }
                println!("state dir:      {}", cfg.state_dir.value.display());
            }
            Ok(())
        }
        ConfigCommand::Show { origin } => {
            let set = roots::resolve(&cfg);
            if global.json {
                ui::json(&json!({
                    "config": cfg,
                    "roots": set.roots,
                    "root_problems": crate::commands::roots::problems_json(&set),
                }));
                return Ok(());
            }
            let o = |orig: &config::Origin| {
                if *origin {
                    format!("   ← {orig}")
                } else {
                    String::new()
                }
            };
            println!(
                "user config file  {}{}",
                cfg.user_config_path.value.display(),
                o(&cfg.user_config_path.origin)
            );
            println!(
                "state_dir         {}{}",
                cfg.state_dir.value.display(),
                o(&cfg.state_dir.origin)
            );
            println!(
                "runtime_dir       {}{}",
                cfg.runtime_dir.value.display(),
                o(&cfg.runtime_dir.origin)
            );

            println!(
                "keep_running      {}{}",
                cfg.keep_running.value,
                o(&cfg.keep_running.origin)
            );
            match &cfg.project {
                Some(p) => {
                    println!("project dir       {}", p.dir.display());
                    if let Some(n) = &p.name {
                        println!("project           {}{}", n.value, o(&n.origin));
                    }
                    if let Some(r) = &p.default_robot {
                        println!("default_robot     {}{}", r.value, o(&r.origin));
                    }
                }
                None => println!("project           (none)"),
            }
            if set.roots.is_empty() {
                println!("roots             (none)");
            }
            for r in &set.roots {
                println!("root {:<12} {}{}", r.name, r.path.display(), o(&r.origin));
            }
            for (name, e) in &set.problems {
                ui::warn(format!(
                    "root {}: {} [{}]",
                    name.as_deref().unwrap_or("?"),
                    e.message(),
                    e.code()
                ));
            }
            Ok(())
        }
    }
}
