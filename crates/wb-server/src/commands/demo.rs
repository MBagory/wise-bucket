//! `demo`: download public sample recordings and declare them as the `demo` root.

use std::time::Duration;

use dialoguer::MultiSelect;
use serde_json::json;
use wb_core::config::{self, RootSpec};
use wb_core::error::{IoContext, Result};
use wb_core::roots;
use wb_core::samples::{self, FORMATS, Format};

use crate::cli::{DemoArgs, GlobalArgs};
use crate::commands::setup::add_user_root;
use crate::ui;

/// Name of the data root holding the samples.
pub const ROOT_NAME: &str = "demo";

pub fn run(global: &GlobalArgs, a: &DemoArgs) -> Result<()> {
    if a.list {
        return list(global.json);
    }
    let cfg = config::load(&global.overrides())?;
    let chosen = choose(a)?;
    if chosen.is_empty() {
        ui::warn("no format selected");
        return Ok(());
    }

    let dir = cfg.state_dir.value.join("demo");
    for f in &chosen {
        ui::step(format!("{} ({})", f.key, ui::bytes(f.size())));
        samples::fetch(f, &dir, &|msg| ui::info(msg))?;
    }
    // Roots are stored canonical (e.g. /private/tmp on macOS); compare like with like.
    let dir = dunce::canonicalize(&dir).io_ctx(|| format!("resolve {}", dir.display()))?;
    ui::ok(format!("samples in {}", dir.display()));

    match cfg.roots.iter().find(|r| r.spec.name == ROOT_NAME) {
        None => add_user_root(
            &cfg,
            RootSpec {
                name: ROOT_NAME.into(),
                path: dir.clone(),
                include: vec![],
                exclude: vec![],
            },
        )?,
        Some(r) if r.spec.path == dir => {
            let summary = roots::resolve(&cfg)
                .get(ROOT_NAME)
                .map(|root| {
                    roots::count_candidates(root, 200_000, Duration::from_secs(10)).summary()
                })
                .unwrap_or_default();
            ui::ok(format!("root {ROOT_NAME:?} already declared · {summary}"));
        }
        Some(r) => ui::warn(format!(
            "a root named {ROOT_NAME:?} already points to {}; left unchanged. Declare the samples under another name with `wisebucket roots add <name> {}`",
            r.spec.path.display(),
            dir.display()
        )),
    }

    if global.json {
        ui::json(&json!({
            "dir": dir,
            "root": ROOT_NAME,
            "formats": chosen.iter().map(|f| f.key).collect::<Vec<_>>(),
        }));
    } else {
        ui::info(
            "An open agent session extracts the readable formats (ROS 2 bags and MCAP, ULog, MAVLink, ArduPilot, CAN, NMEA 0183 and 2000, Parquet) into the store as soon as each download finishes (on a first run, from its next `server_info` call); the others are listed as not readable yet.",
        );
        ui::info("Ask your agent: \"Which recordings does Wise Bucket have in the demo root?\"");
        ui::info(format!(
            "Remove later with `wisebucket roots remove {ROOT_NAME}` and by deleting {}",
            dir.display()
        ));
    }
    Ok(())
}

fn choose(a: &DemoArgs) -> Result<Vec<&'static Format>> {
    if !a.formats.is_empty() {
        // Values are validated by clap against the catalogue.
        return Ok(a.formats.iter().filter_map(|k| samples::find(k)).collect());
    }
    if a.all || !ui::interactive(a.yes) {
        return Ok(FORMATS.iter().collect());
    }
    let items: Vec<String> = FORMATS.iter().map(label).collect();
    let picked = MultiSelect::new()
        .with_prompt("Formats to download (space to select, enter to confirm)")
        .items(&items)
        .interact()
        .map_err(ui::prompt_err)?;
    Ok(picked.into_iter().map(|i| &FORMATS[i]).collect())
}

fn label(f: &Format) -> String {
    format!(
        "{:<10} {} ({}, {})",
        f.key,
        f.description,
        ui::bytes(f.size()),
        f.license
    )
}

fn list(as_json: bool) -> Result<()> {
    if as_json {
        ui::json(&json!(
            FORMATS
                .iter()
                .map(|f| json!({
                    "key": f.key,
                    "description": f.description,
                    "license": f.license,
                    "size": f.size(),
                    "files": f.files.iter().map(|s| json!({
                        "path": s.path,
                        "url": s.url,
                        "sha256": s.sha256,
                    })).collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>()
        ));
    } else {
        for f in FORMATS {
            println!("{}", label(f));
        }
    }
    Ok(())
}
