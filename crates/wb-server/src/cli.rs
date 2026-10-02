//! Command-line interface definition.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Wise Bucket: evidence-backed, iterative robot investigations for your AI coding agent.
///
/// Without a subcommand, `serve` runs the MCP server on stdin/stdout.
#[derive(Debug, Parser)]
#[command(name = "wisebucket", version, propagate_version = true)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Args)]
pub struct GlobalArgs {
    /// User configuration file [env: WB_CONFIG]
    #[arg(long, global = true, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// State directory (database, caches, logs) [env: WB_STATE_DIR]
    #[arg(long, global = true, value_name = "DIR")]
    pub state_dir: Option<PathBuf>,

    /// Keep the managed PostgreSQL running after the last session [env: WB_KEEP_RUNNING]
    #[arg(long, global = true)]
    pub keep_running: bool,

    /// Machine-readable JSON output (where supported)
    #[arg(long, global = true)]
    pub json: bool,

    /// More diagnostic logging on stderr (-v, -vv)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the MCP server on stdin/stdout (what your harness launches)
    Serve,
    /// One-time machine setup: database, recording folders, and how to connect your agent
    Setup(SetupArgs),
    /// Manage data roots (folders Wise Bucket may read)
    #[command(subcommand)]
    Roots(RootsCommand),
    /// Inspect the effective configuration
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Control the managed PostgreSQL server
    #[command(subcommand)]
    Db(DbCommand),
    /// Write a backup of the Wise Bucket database (custom pg_dump format)
    Backup {
        /// Output file
        file: PathBuf,
    },
    /// Restore a backup made with `backup` (replaces current data)
    Restore {
        /// Backup file
        file: PathBuf,
        /// Confirm replacing the current data
        #[arg(long)]
        yes: bool,
    },
    /// Check the whole installation and explain how to fix problems
    Doctor,
    /// Download public sample recordings and declare them as the `demo` data root
    Demo(DemoArgs),
    /// Documentation helpers
    #[command(subcommand)]
    Docs(DocsCommand),
}

#[derive(Debug, Args)]
pub struct SetupArgs {
    /// Never prompt; use flags and defaults
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Add a data root: `name=path` (repeatable)
    #[arg(long = "root", value_name = "NAME=PATH")]
    pub roots: Vec<String>,

    /// Do not ask about data roots
    #[arg(long)]
    pub no_roots: bool,
}

#[derive(Debug, Args)]
pub struct DemoArgs {
    /// Formats to download (default: choose interactively, or all without a terminal)
    #[arg(value_name = "FORMAT", value_parser = clap::builder::PossibleValuesParser::new(wb_core::samples::keys()))]
    pub formats: Vec<String>,

    /// Download every format
    #[arg(long, conflicts_with = "formats")]
    pub all: bool,

    /// List the formats, their sources and licenses, then exit
    #[arg(long)]
    pub list: bool,

    /// Never prompt (downloads every format unless some are named)
    #[arg(long, short = 'y')]
    pub yes: bool,
}

#[derive(Debug, Subcommand)]
pub enum RootsCommand {
    /// Declare a folder Wise Bucket may read
    Add {
        /// Short name, used in references like `name:relative/path`
        name: String,
        /// Folder path (absolute or ~/...)
        path: PathBuf,
        /// Default robot for recordings in this folder
        #[arg(long)]
        robot: Option<String>,
        /// Glob pattern of files to consider (repeatable)
        #[arg(long)]
        include: Vec<String>,
        /// Glob pattern of files to ignore (repeatable)
        #[arg(long)]
        exclude: Vec<String>,
    },
    /// List declared roots
    List,
    /// Remove a root (the folder itself is never touched)
    Remove { name: String },
    /// Validate roots and count candidate recordings
    Check,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Show the effective configuration
    Show {
        /// Show where each value comes from
        #[arg(long)]
        origin: bool,
    },
    /// Print the configuration file locations
    Path,
}

#[derive(Debug, Subcommand)]
pub enum DbCommand {
    /// Show database mode, state and active sessions
    Status,
    /// Start the managed PostgreSQL server
    Start,
    /// Stop the managed PostgreSQL server
    Stop {
        /// Stop even if MCP sessions are connected
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum DocsCommand {
    /// Generate reference pages (errors, CLI, configuration) into docs/reference
    Gen {
        /// Fail if the committed pages are out of date instead of writing them
        #[arg(long)]
        check: bool,
        /// Output directory
        #[arg(long, default_value = "docs/reference")]
        out: PathBuf,
    },
}

impl GlobalArgs {
    pub fn overrides(&self) -> wb_core::config::Overrides {
        wb_core::config::Overrides {
            config_path: self.config.clone(),
            state_dir: self.state_dir.clone(),
            keep_running: self.keep_running.then_some(true),
        }
    }
}
