//! `roots add|list|remove|check`.

use std::time::Duration;

use serde_json::json;
use wb_core::config::{self, Origin, RootDecl, RootSpec};
use wb_core::error::{ErrorKind, Result, err};
use wb_core::roots;

use crate::cli::{GlobalArgs, RootsCommand};
use crate::commands::setup::add_user_root;
use crate::ui;

pub async fn run(global: &GlobalArgs, cmd: &RootsCommand) -> Result<()> {
    let cfg = config::load(&global.overrides())?;
    match cmd {
        RootsCommand::Add {
            name,
            path,
            robot,
            include,
            exclude,
            in_project,
        } => {
            let spec = RootSpec {
                name: name.clone(),
                path: path.clone(),
                robot: robot.clone(),
                include: include.clone(),
                exclude: exclude.clone(),
            };
            if *in_project {
                let info = cfg.project.as_ref().ok_or_else(|| {
                    err(ErrorKind::RootOutsideProject, "no project found: run `wisebucket init` in the repository first, or pass --project")
                })?;
                let decl = RootDecl {
                    spec: spec.clone(),
                    origin: Origin::ProjectConfig(info.config_path.clone()),
                    from_project: true,
                    base_dir: info.dir.clone(),
                };
                let root = roots::check_new_root(&cfg.roots, decl)?;
                config::add_root_to_file(&info.config_path, &spec, true)?;
                ui::ok(format!(
                    "project root {:?} → {}",
                    root.name,
                    root.path.display()
                ));
            } else {
                add_user_root(&cfg, spec)?;
            }
            ui::info(
                "Restart your harness session (or reconnect the MCP server) to use the new root.",
            );
            Ok(())
        }
        RootsCommand::Remove { name, in_project } => {
            let file = if *in_project {
                cfg.project
                    .as_ref()
                    .map(|p| p.config_path.clone())
                    .ok_or_else(|| err(ErrorKind::RootInvalid, "no project found"))?
            } else {
                cfg.user_config_path.value.clone()
            };
            if config::remove_root_from_file(&file, name)? {
                ui::ok(format!(
                    "root {name:?} removed from {} (the folder itself was not touched)",
                    file.display()
                ));
                Ok(())
            } else {
                Err(err(
                    ErrorKind::RootInvalid,
                    format!("no root named {name:?} in {}", file.display()),
                ))
            }
        }
        RootsCommand::List => {
            let set = roots::resolve(&cfg);
            if global.json {
                ui::json(&json!({
                    "roots": set.roots,
                    "problems": problems_json(&set),
                }));
                return Ok(());
            }
            if cfg.roots.is_empty() {
                ui::warn("no roots declared. Add one with `wisebucket roots add <name> <path>`.");
            }
            for r in &set.roots {
                println!(
                    "{:<16} {}{}  [{}]",
                    r.name,
                    r.path.display(),
                    r.robot
                        .as_ref()
                        .map(|x| format!("  robot={x}"))
                        .unwrap_or_default(),
                    r.origin
                );
            }
            print_problems(&set);
            Ok(())
        }
        RootsCommand::Check => {
            let set = roots::resolve(&cfg);
            let mut out = Vec::new();
            for r in &set.roots {
                let c = roots::count_candidates(&r.path, 500_000, Duration::from_secs(30));
                if !global.json {
                    ui::ok(format!(
                        "{:<16} {} · {} candidate recordings ({} MCAP, {} rosbag2 folders, {} ROS 1, {} ULog){}",
                        r.name,
                        r.path.display(),
                        c.total(),
                        c.mcap,
                        c.rosbag2_dirs,
                        c.ros1_bag,
                        c.ulog,
                        if c.truncated { ", scan truncated" } else { "" }
                    ));
                }
                out.push(json!({ "root": r, "candidates": c }));
            }
            if global.json {
                ui::json(&json!({ "roots": out, "problems": problems_json(&set) }));
            } else {
                print_problems(&set);
                if cfg.roots.is_empty() {
                    ui::warn("no roots declared");
                }
            }
            if set.problems.is_empty() {
                Ok(())
            } else {
                Err(err(
                    ErrorKind::RootInvalid,
                    format!("{} root problem(s)", set.problems.len()),
                ))
            }
        }
    }
}

pub fn problems_json(set: &roots::RootSet) -> serde_json::Value {
    json!(
        set.problems
            .iter()
            .map(|(name, e)| {
                let mut j = e.to_json();
                j["root"] = json!(name);
                j
            })
            .collect::<Vec<_>>()
    )
}

fn print_problems(set: &roots::RootSet) {
    for (name, e) in &set.problems {
        ui::warn(format!(
            "{}: {} [{}] → {}",
            name.as_deref().unwrap_or("?"),
            e.message(),
            e.code(),
            ui::docs_url(&e.docs_ref())
        ));
    }
}
