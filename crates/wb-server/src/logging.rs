//! Diagnostics logging. Never writes to stdout (reserved for MCP messages).

use std::io::IsTerminal;
use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

fn filter(verbose: u8) -> EnvFilter {
    EnvFilter::try_from_env("WB_LOG").unwrap_or_else(|_| {
        EnvFilter::new(match verbose {
            // The Rerun SDK repeats on exit the error `rerun` already reports.
            0 => "warn,re_sdk=off,re_grpc_client=off",
            1 => "info,datafusion=warn",
            _ => "debug",
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

/// `<logs>/server.log` for the MCP server, plus warnings and errors on stderr (the agent
/// keeps every stderr line in its own MCP log, as an error).
pub fn init_file(logs_dir: &Path) -> Option<WorkerGuard> {
    std::fs::create_dir_all(logs_dir).ok()?;
    let appender = tracing_appender::rolling::daily(logs_dir, "server.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let _ = tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(std::io::stderr.with_max_level(tracing::Level::WARN))
                .with_target(false)
                .with_ansi(std::io::stderr().is_terminal()),
        )
        .with(fmt::layer().with_writer(writer).with_ansi(false))
        .with(
            // rmcp logs each session's handshake at INFO and the errors it answers the
            // client with at WARN: the client already reports those.
            EnvFilter::try_from_env("WB_LOG")
                .unwrap_or_else(|_| EnvFilter::new("info,datafusion=warn,rmcp=error")),
        )
        .try_init();
    Some(guard)
}
