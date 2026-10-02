//! Wise Bucket's MCP server and command-line tool, shared by the `wisebucket` and `wbk` binaries.

mod cli;
mod commands;
mod logging;
mod mcp;
mod ui;

use std::process::ExitCode;

use clap::Parser;
use cli::{Cli, Command};
use wb_core::Result;

#[tokio::main]
pub async fn main() -> ExitCode {
    let cli = Cli::parse();
    let serving = matches!(cli.command, None | Some(Command::Serve));
    if !serving {
        logging::init_stderr(cli.global.verbose);
    }
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            ui::error(&e);
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    let g = &cli.global;
    match &cli.command {
        None | Some(Command::Serve) => mcp::serve(&g.overrides()).await,
        Some(Command::Setup(a)) => commands::setup::run(g, a).await,
        Some(Command::Roots(c)) => commands::roots::run(g, c).await,
        Some(Command::Config(c)) => commands::config_cmd::run(g, c).await,
        Some(Command::Db(c)) => commands::db_cmd::run(g, c).await,
        Some(Command::Backup { file }) => commands::db_cmd::backup(g, file).await,
        Some(Command::Restore { file, yes }) => commands::db_cmd::restore(g, file, *yes).await,
        Some(Command::Doctor) => commands::doctor::run(g).await,
        Some(Command::Demo(a)) => commands::demo::run(g, a),
        Some(Command::Docs(cli::DocsCommand::Gen { check, out })) => {
            commands::docs::run(out, *check)
        }
    }
}
