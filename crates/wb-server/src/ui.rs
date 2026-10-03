//! Terminal output helpers. Human messages go to stderr; data (JSON, snippets) to stdout.
//!
//! `anstream`'s print macros strip colors when the stream is not a terminal or `NO_COLOR` is set.

use std::io::IsTerminal;

use anstream::eprintln;
use clap::builder::styling::{AnsiColor, Style};
use serde::Serialize;
use wb_core::WbError;
use wb_core::error::ErrorKind;

pub use wb_core::error::DOCS_BASE;

/// Converts a docs reference (`reference/errors.md#code`) to a URL.
pub fn docs_url(docs_ref: &str) -> String {
    format!("{DOCS_BASE}{docs_ref}")
}

const DIM: Style = Style::new().dimmed();

/// Outcome of a step or check, shown as a colored glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

/// `✔`, `⚠` or `✖`, colored.
pub fn glyph(status: Status) -> String {
    let (style, g) = match status {
        Status::Ok => (AnsiColor::Green.on_default(), "✔"),
        Status::Warn => (AnsiColor::Yellow.on_default(), "⚠"),
        Status::Fail => (AnsiColor::Red.on_default().bold(), "✖"),
    };
    format!("{style}{g}{style:#}")
}

pub fn ok(msg: impl AsRef<str>) {
    eprintln!("{} {}", glyph(Status::Ok), msg.as_ref());
}

pub fn warn(msg: impl AsRef<str>) {
    eprintln!("{} {}", glyph(Status::Warn), msg.as_ref());
}

pub fn info(msg: impl AsRef<str>) {
    eprintln!("  {}", msg.as_ref());
}

pub fn step(msg: impl AsRef<str>) {
    eprintln!("{DIM}…{DIM:#} {}", msg.as_ref());
}

/// Prints an error with its code, fix and documentation link.
pub fn error(e: &WbError) {
    eprintln!("{} {}", glyph(Status::Fail), e.message());
    eprintln!("  {DIM}code:{DIM:#} {}", e.code());
    eprintln!("  {DIM}fix:{DIM:#}  {}", e.kind().fix());
    eprintln!("  {DIM}docs:{DIM:#} {}", docs_url(&e.docs_ref()));
}

/// Prints an error as JSON on stderr, for `--json` callers (stdout stays reserved for results).
pub fn error_json(e: &WbError) {
    eprintln!("{}", serde_json::json!({ "error": e.to_json() }));
}

/// Whether we may prompt the user.
pub fn interactive(no_prompt: bool) -> bool {
    !no_prompt && std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// Maps prompt I/O failures to a Wise Bucket error.
pub fn prompt_err(e: dialoguer::Error) -> WbError {
    WbError::new(ErrorKind::Io, format!("prompt failed: {e}"))
}

/// Prints JSON to stdout.
pub fn json(value: &serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_default()
    );
}

/// Human-readable byte size.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    format!("{v:.1} {}", UNITS[u])
}
