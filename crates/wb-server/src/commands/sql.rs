//! `sql`: read-only queries over the store. `rerun`: send a query result to the Rerun
//! viewer. `ingest` (hidden): extract one recording, run by agent sessions in a child
//! process.

use std::process::ExitCode;

use serde_json::{Value, json};
use wb_core::config;
use wb_core::error::{ErrorKind, Result, err};
use wb_data::store::{self, Outcome, Store};

use crate::cli::GlobalArgs;
use crate::ui;

const MAX_WIDTH: usize = 40;

fn cell(v: &Value) -> String {
    let s = match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if s.chars().count() > MAX_WIDTH {
        format!("{}…", s.chars().take(MAX_WIDTH - 1).collect::<String>())
    } else {
        s
    }
}

pub async fn run(global: &GlobalArgs, query: &str, limit: Option<usize>) -> Result<()> {
    let cfg = config::load(&global.overrides())?;
    let result = wb_data::sql::query(&Store::for_config(&cfg), query, limit).await?;
    if global.json {
        ui::json(&json!(result));
        return Ok(());
    }
    let cells: Vec<Vec<String>> = result
        .rows
        .iter()
        .map(|r| r.iter().map(cell).collect())
        .collect();
    let widths: Vec<usize> = result
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            cells
                .iter()
                .map(|r| r[i].chars().count())
                .chain([c.chars().count()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |values: Vec<&str>| {
        values
            .iter()
            .zip(&widths)
            .map(|(v, w)| format!("{v:<w$}"))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    println!(
        "{}",
        line(result.columns.iter().map(String::as_str).collect())
    );
    for r in &cells {
        println!("{}", line(r.iter().map(String::as_str).collect()));
    }
    ui::info(format!("{} row(s)", result.row_count));
    if let Some(hint) = &result.hint {
        ui::info(hint);
    }
    Ok(())
}

pub async fn rerun(
    global: &GlobalArgs,
    query: &str,
    entity: Option<&str>,
    url: &str,
) -> Result<()> {
    let cfg = config::load(&global.overrides())?;
    let sent = wb_data::rerun::send(&Store::for_config(&cfg), query, entity, url).await?;
    if global.json {
        ui::json(&json!(sent));
        return Ok(());
    }
    ui::ok(format!(
        "{} row(s) sent to {}: {} on timeline(s) {}",
        sent.rows,
        sent.url,
        sent.entities.join(", "),
        sent.timelines.join(", ")
    ));
    if !sent.skipped.is_empty() {
        ui::info(format!(
            "not sent (not numbers): {}",
            sent.skipped.join(", ")
        ));
    }
    Ok(())
}

pub async fn ingest(global: &GlobalArgs, recording: &str) -> Result<ExitCode> {
    let cfg = config::load(&global.overrides())?;
    let (root, candidate) = store::locate(&cfg, recording)?;
    match store::ingest(&Store::for_config(&cfg), &root, &candidate).await? {
        Outcome::Extracted { streams } => {
            ui::ok(format!("{recording}: {streams} stream(s) extracted"));
            tracing::info!(recording, streams, "extracted");
        }
        Outcome::UpToDate => ui::ok(format!("{recording}: already in the store")),
        Outcome::Busy => ui::ok(format!("{recording}: another session is extracting it")),
        Outcome::Failed(message) => {
            return Err(err(
                ErrorKind::RecordingUnreadable,
                format!("{recording}: {message}"),
            ));
        }
    }
    Ok(ExitCode::SUCCESS)
}
