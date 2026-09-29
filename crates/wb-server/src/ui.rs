//! Terminal output helpers. Human messages go to stderr; data (JSON, snippets) to stdout.

use std::io::IsTerminal;

use wb_core::WbError;
use wb_core::error::ErrorKind;

/// Base URL of the documentation (Markdown files in `docs/` on GitHub). Error links are relative to it.
pub const DOCS_BASE: &str = "https://github.com/MBagory/wise-bucket/blob/main/docs/";

/// Converts a docs reference (`reference/errors.md#code`) to a URL.
pub fn docs_url(docs_ref: &str) -> String {
    format!("{DOCS_BASE}{docs_ref}")
}

pub fn ok(msg: impl AsRef<str>) {
    eprintln!("✔ {}", msg.as_ref());
}

pub fn warn(msg: impl AsRef<str>) {
    eprintln!("⚠ {}", msg.as_ref());
}

pub fn info(msg: impl AsRef<str>) {
    eprintln!("  {}", msg.as_ref());
}

pub fn step(msg: impl AsRef<str>) {
    eprintln!("… {}", msg.as_ref());
}

/// Prints an error with its code, fix and documentation link.
pub fn error(e: &WbError) {
    eprintln!("✖ {}", e.message());
    eprintln!("  code: {}", e.code());
    eprintln!("  fix:  {}", e.kind().fix());
    eprintln!("  docs: {}", docs_url(&e.docs_ref()));
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
