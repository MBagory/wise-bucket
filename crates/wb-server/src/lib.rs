//! Wise Bucket's MCP server and command-line tool, shared by the `wisebucket` and `wbk` binaries.

mod cli;
mod commands;
mod logging;
mod mcp;
mod ui;

use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use cli::{Cli, Command};
use wb_core::Result;

#[tokio::main]
pub async fn main() -> ExitCode {
    let cli = Cli::parse();
    let serving = matches!(cli.command, None | Some(Command::Serve));
    if !serving {
        logging::init_stderr(cli.global.verbose);
    }
    let json = cli.global.json;
    match run(cli).await {
        Ok(code) => code,
        Err(e) => {
            if json {
                ui::error_json(&e);
            } else {
                ui::error(&e);
            }
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode> {
    let g = &cli.global;
    match &cli.command {
        Some(Command::Doctor) => return commands::doctor::run(g).await,
        None | Some(Command::Serve) => mcp::serve(&g.overrides()).await,
        Some(Command::Setup(a)) => commands::setup::run(g, a).await,
        Some(Command::Roots(c)) => commands::roots::run(g, c).await,
        Some(Command::Config(c)) => commands::config_cmd::run(g, c).await,
        Some(Command::Db(c)) => commands::db_cmd::run(g, c).await,
        Some(Command::Backup { file }) => commands::db_cmd::backup(g, file).await,
        Some(Command::Restore { file, yes }) => commands::db_cmd::restore(g, file, *yes).await,
        Some(Command::Demo(a)) => commands::demo::run(g, a),
        Some(Command::Docs(cli::DocsCommand::Gen { check, out })) => {
            commands::docs::run(out, *check)
        }
        Some(Command::Completions { shell }) => {
            // Complete the name this binary was invoked as (`wisebucket` or `wbk`).
            let name = std::env::args_os()
                .next()
                .and_then(|a| {
                    std::path::Path::new(&a)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "wisebucket".into());
            clap_complete::generate(*shell, &mut Cli::command(), name, &mut std::io::stdout());
            Ok(())
        }
    }
    .map(|()| ExitCode::SUCCESS)
}
