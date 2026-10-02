//! `init`: configure a robot repository for Wise Bucket and a harness.

use std::path::{Path, PathBuf};

use dialoguer::Input;
use serde_json::{Map, Value, json};
use wb_core::config::{self, ProjectConfigFile, RootDecl, RootSpec};
use wb_core::error::{ErrorKind, Result, err};
use wb_core::{fsutil, paths, roots};

use crate::cli::{GlobalArgs, Harness, InitArgs};
use crate::commands::setup::parse_root_arg;
use crate::ui;

/// Name of the server entry in MCP configuration files.
pub const SERVER_KEY: &str = "wise-bucket";

/// Path of this binary as harnesses should launch it: the `wisebucket` next to
/// it when run as the `wbk` alias, so both names write the same configuration.
pub fn server_exe() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let exe = dunce::canonicalize(&exe).unwrap_or(exe);
    let main = exe.with_file_name(format!("wisebucket{}", std::env::consts::EXE_SUFFIX));
    Ok(if main.is_file() { main } else { exe })
}

/// Builds the MCP server entry launching this binary for `project_dir`.
pub fn server_entry(global: &GlobalArgs, project_dir: &Path) -> Result<Value> {
    let exe = server_exe()?;
    let mut args = vec![
        "serve".to_string(),
        "--project".to_string(),
        project_dir.display().to_string(),
    ];
    // Persist explicit command-line choices so the harness uses the same state.
    if let Some(c) = &global.config {
        args.extend([
            "--config".into(),
            paths::absolutize(c, &std::env::current_dir()?)
                .display()
                .to_string(),
        ]);
    }
    if let Some(s) = &global.state_dir {
        args.extend([
            "--state-dir".into(),
            paths::absolutize(s, &std::env::current_dir()?)
                .display()
                .to_string(),
        ]);
    }
    Ok(json!({ "command": exe.display().to_string(), "args": args }))
}

/// Where each harness reads its project MCP configuration (None: global settings only).
pub fn harness_file(h: Harness, project_dir: &Path) -> Option<PathBuf> {
    match h {
        Harness::Claude => Some(project_dir.join(".mcp.json")),
        Harness::Kilo => Some(project_dir.join(".kilocode").join("mcp.json")),
        Harness::Cline => None,
    }
}

/// Result of writing the harness configuration.
#[derive(Debug, PartialEq, Eq)]
pub enum HarnessWrite {
    Created(PathBuf),
    Merged(PathBuf),
    AlreadyConfigured(PathBuf),
    /// Exists without our entry; the user must merge (snippet printed).
    NotOverwritten(PathBuf),
    PrintedOnly,
}

/// Writes (or merges into) the harness MCP configuration file.
pub fn write_harness_config(file: &Path, entry: &Value, merge: bool) -> Result<HarnessWrite> {
    let wrap = |entry: &Value| json!({ "mcpServers": { SERVER_KEY: entry } });
    match std::fs::read_to_string(file) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let text = serde_json::to_string_pretty(&wrap(entry)).unwrap_or_default() + "\n";
            fsutil::write_atomic(file, text.as_bytes())?;
            Ok(HarnessWrite::Created(file.to_path_buf()))
        }
        Err(e) => Err(e.into()),
        Ok(text) => {
            let mut doc: Value = serde_json::from_str(&text).map_err(|e| {
                err(
                    ErrorKind::ConfigInvalid,
                    format!("{} is not valid JSON: {e}", file.display()),
                )
            })?;
            let existing = doc.get("mcpServers").and_then(|s| s.get(SERVER_KEY));
            if existing == Some(entry) {
                return Ok(HarnessWrite::AlreadyConfigured(file.to_path_buf()));
            }
            if !merge {
                return Ok(HarnessWrite::NotOverwritten(file.to_path_buf()));
            }
            let obj = doc.as_object_mut().ok_or_else(|| {
                err(
                    ErrorKind::ConfigInvalid,
                    format!("{} must contain a JSON object", file.display()),
                )
            })?;
            let servers = obj
                .entry("mcpServers")
                .or_insert_with(|| Value::Object(Map::new()));
            let servers = servers
                .as_object_mut()
                .ok_or_else(|| err(ErrorKind::ConfigInvalid, "`mcpServers` must be an object"))?;
            servers.insert(SERVER_KEY.to_string(), entry.clone());
            let out = serde_json::to_string_pretty(&doc).unwrap_or_default() + "\n";
            fsutil::write_atomic(file, out.as_bytes())?;
            Ok(HarnessWrite::Merged(file.to_path_buf()))
        }
    }
}

fn default_project_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "robot".into())
}

pub async fn run(global: &GlobalArgs, args: &InitArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let dir = args
        .dir
        .as_deref()
        .or(global.project.as_deref())
        .map(|d| paths::absolutize(d, &cwd))
        .unwrap_or(cwd);
    let dir = dunce::canonicalize(&dir)
        .map_err(|e| err(ErrorKind::Io, format!("{}: {e}", dir.display())))?;
    let interactive = ui::interactive(args.yes) && !args.print;

    // Configuration as seen from this project (user config, roots, …).
    let mut overrides = global.overrides();
    overrides.project_dir = Some(dir.clone());
    let cfg = config::load(&overrides)?;

    // --- project configuration
    let project_cfg_path = dir.join(paths::PROJECT_CONFIG_REL);
    if project_cfg_path.is_file() {
        ui::ok(format!(
            "project config already present: {} (left unchanged)",
            project_cfg_path.display()
        ));
    } else {
        let mut name = args
            .name
            .clone()
            .unwrap_or_else(|| default_project_name(&dir));
        let mut robot = args.robot.clone();
        if interactive {
            name = Input::new()
                .with_prompt("Project name")
                .default(name)
                .interact_text()
                .map_err(ui::prompt_err)?;
            if robot.is_none() {
                let r: String = Input::new()
                    .with_prompt("Default robot (optional)")
                    .allow_empty(true)
                    .interact_text()
                    .map_err(ui::prompt_err)?;
                robot = Some(r.trim().to_string()).filter(|r| !r.is_empty());
            }
        }
        let mut specs = Vec::new();
        for r in &args.roots {
            let (name, path) = parse_root_arg(r)?;
            specs.push(RootSpec {
                name,
                path,
                robot: robot.clone(),
                include: vec![],
                exclude: vec![],
            });
        }
        // Validate project roots (inside the repository, no overlap with user roots).
        let mut decls = cfg.roots.clone();
        for spec in &specs {
            let decl = RootDecl {
                spec: spec.clone(),
                origin: config::Origin::ProjectConfig(project_cfg_path.clone()),
                from_project: true,
                base_dir: dir.clone(),
            };
            roots::check_new_root(&decls, decl.clone())?;
            decls.push(decl);
        }
        let file = ProjectConfigFile {
            project: Some(name),
            default_robot: robot,
            roots: specs,
        };
        let body =
            toml::to_string_pretty(&file).map_err(|e| err(ErrorKind::Internal, e.to_string()))?;
        let text = format!(
            "# Wise Bucket project configuration (commit this file).\n# Docs: {}\n\n{body}",
            ui::docs_url("reference/configuration.md")
        );
        if args.print {
            println!("# {}\n{text}", project_cfg_path.display());
        } else {
            fsutil::write_atomic(&project_cfg_path, text.as_bytes())?;
            ui::ok(format!("wrote {}", project_cfg_path.display()));
        }
    }

    // --- harness configuration
    let entry = server_entry(global, &dir)?;
    let snippet = serde_json::to_string_pretty(&json!({ "mcpServers": { SERVER_KEY: entry } }))
        .unwrap_or_default();
    let target = harness_file(args.harness, &dir);
    let outcome = match (&target, args.print) {
        (Some(file), false) => write_harness_config(file, &entry, args.merge)?,
        _ => HarnessWrite::PrintedOnly,
    };
    match &outcome {
        HarnessWrite::Created(p) => ui::ok(format!("wrote {}", p.display())),
        HarnessWrite::Merged(p) => ui::ok(format!(
            "added the `{SERVER_KEY}` server to {}",
            p.display()
        )),
        HarnessWrite::AlreadyConfigured(p) => ui::ok(format!("{} already configured", p.display())),
        HarnessWrite::NotOverwritten(p) => {
            let e = err(
                ErrorKind::McpConfigExists,
                format!(
                    "{} exists and was not modified. Add this entry to it, or re-run with --merge:",
                    p.display()
                ),
            );
            ui::warn(format!("{} [{}]", e.message(), e.code()));
            println!("{snippet}");
        }
        HarnessWrite::PrintedOnly => {
            if args.harness == Harness::Cline {
                ui::info(
                    "Cline reads MCP servers from its global settings (Cline › MCP Servers › Configure). Add:",
                );
            }
            println!("{snippet}");
        }
    }

    if !args.print {
        eprintln!();
        match args.harness {
            Harness::Claude => eprintln!(
                "Next: open Claude Code in {} (approve the project MCP server when asked), check that `/mcp` lists `{SERVER_KEY}`, then ask: \"call server_info\".",
                dir.display()
            ),
            Harness::Kilo => eprintln!(
                "Next: open the folder in VS Code with Kilo Code and enable the `{SERVER_KEY}` MCP server."
            ),
            Harness::Cline => eprintln!(
                "Next: paste the snippet in Cline's MCP settings and enable `{SERVER_KEY}`."
            ),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_config_create_then_idempotent_then_refuse_then_merge() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join(".mcp.json");
        let entry = json!({"command": "/bin/wb", "args": ["serve"]});
        assert_eq!(
            write_harness_config(&f, &entry, false).unwrap(),
            HarnessWrite::Created(f.clone())
        );
        assert_eq!(
            write_harness_config(&f, &entry, false).unwrap(),
            HarnessWrite::AlreadyConfigured(f.clone())
        );

        std::fs::write(&f, r#"{"mcpServers":{"other":{"command":"x"}}}"#).unwrap();
        assert_eq!(
            write_harness_config(&f, &entry, false).unwrap(),
            HarnessWrite::NotOverwritten(f.clone())
        );
        assert!(std::fs::read_to_string(&f).unwrap().contains("other"));
        assert_eq!(
            write_harness_config(&f, &entry, true).unwrap(),
            HarnessWrite::Merged(f.clone())
        );
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["mcpServers"][SERVER_KEY], entry);
    }

    #[test]
    fn invalid_json_is_reported_not_overwritten() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join(".mcp.json");
        std::fs::write(&f, "{oops").unwrap();
        let e = write_harness_config(&f, &json!({}), true).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::ConfigInvalid);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "{oops");
    }
}
