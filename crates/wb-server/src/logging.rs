//! Diagnostics logging. Never writes to stdout (reserved for MCP messages).

use std::io::IsTerminal;
use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

fn filter(verbose: u8) -> EnvFilter {
    EnvFilter::try_from_env("WB_LOG").unwrap_or_else(|_| {
        EnvFilter::new(match verbose {
            0 => "warn",
            1 => "info,sqlx=warn",
            _ => "debug,sqlx=info",
        })
    })
}

/// Stderr logging for CLI commands.
pub fn init_stderr(verbose: u8) {
    let _ = tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(std::io::stderr)
                .with_target(false)
                .with_ansi(std::io::stderr().is_terminal()),
        )
        .with(filter(verbose))
        .try_init();
}

/// Stderr + `<logs>/server.log` for the MCP server.
pub fn init_file(logs_dir: &Path) -> Option<WorkerGuard> {
    std::fs::create_dir_all(logs_dir).ok()?;
    let appender = tracing_appender::rolling::daily(logs_dir, "server.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let _ = tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(std::io::stderr)
                .with_target(false)
                .with_ansi(std::io::stderr().is_terminal()),
        )
        .with(fmt::layer().with_writer(writer).with_ansi(false))
        .with(
            EnvFilter::try_from_env("WB_LOG").unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn")),
        )
        .try_init();
    Some(guard)
}
