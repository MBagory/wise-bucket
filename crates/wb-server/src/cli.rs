//! Command-line interface definition.

use std::path::PathBuf;

use clap::builder::styling::{AnsiColor, Styles};
use clap::{Args, Parser, Subcommand};

/// Help colors (cargo-like). clap drops them when stdout is not a terminal or `NO_COLOR` is set.
const STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default().bold())
    .usage(AnsiColor::Green.on_default().bold())
    .literal(AnsiColor::Cyan.on_default().bold())
    .placeholder(AnsiColor::Cyan.on_default());

/// Wise Bucket: an open-source MCP server that gives R&D teams long-term context and insights on their time-series data.
///
/// Without a subcommand, `serve` runs the MCP server on stdin/stdout.
#[derive(Debug, Parser)]
#[command(
    name = "wisebucket",
    version,
    propagate_version = true,
    styles = STYLES,
    after_help = "\
Examples:
  wisebucket setup                         Set up the database and recording folders
  wisebucket roots add bags ~/robot-logs   Let Wise Bucket read a folder
  wisebucket demo --all                    Try it on public sample recordings
  wisebucket sql \"SELECT * FROM streams\"  Query the store yourself
  wisebucket note flights: hardware=drone-2  Tell what the files don't say
  wisebucket rerun \"SELECT t, … FROM …\"  Send a query result to the Rerun viewer
  wisebucket doctor                        Check the installation"
)]
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

    /// State directory (the store of decoded recordings, logs) [env: WB_STATE_DIR]
    #[arg(long, global = true, value_name = "DIR")]
    pub state_dir: Option<PathBuf>,

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
    /// One-time machine setup: recording folders, and how to connect your agent
    #[command(after_help = "\
Examples:
  wisebucket setup                               Interactive
  wisebucket setup -y --root bags=~/robot-logs   Unattended, with one data root")]
    Setup(SetupArgs),
    /// Manage data roots (folders Wise Bucket may read)
    #[command(subcommand)]
    Roots(RootsCommand),
    /// Inspect the effective configuration
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Run a read-only SQL query over the store (what your agent's `sql` tool runs)
    #[command(after_help = "\
Examples:
  wisebucket sql \"SELECT recording, status FROM recordings\"
  wisebucket sql \"SELECT stream, rows FROM streams WHERE recording = 'bags:run3.mcap'\"
  wisebucket sql \"SELECT count(*) FROM stream('/imu')\"
  wisebucket sql --limit 20 \"SELECT t FROM stream('/imu') ORDER BY t\"")]
    Sql {
        /// A single SELECT query (tables: recordings, streams, stream('<name>'))
        query: String,
        /// Rows wanted at most (results are also capped at about 60 KB)
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Send the rows of a query to a running Rerun viewer (what your agent's `rerun` tool runs)
    #[command(after_help = "\
Examples:
  wisebucket rerun \"SELECT t, \\\"accelerometer_m_s2[2]\\\" AS az FROM stream('sensor_combined')\"
  wisebucket rerun --entity imu \"SELECT * FROM stream('/imu', 'bags:run3%')\"")]
    Rerun {
        /// A single SELECT query with a timestamp column `t`
        query: String,
        /// Entity path prefix in Rerun (series are named `<entity>/<column>`)
        #[arg(long)]
        entity: Option<String>,
        /// Address of the Rerun viewer
        #[arg(long, default_value = wb_data::rerun::DEFAULT_URL)]
        url: String,
    },
    /// Record what you know about recordings that their files don't say (what your agent's `note` tool runs)
    #[command(after_help = "\
Examples:
  wisebucket note flights: hardware=drone-2                      Every recording of a root
  wisebucket note flights:2026-10/ propellers='9x4.5 carbon'     Every recording under a folder
  wisebucket note flights:2026-10/flight3.ulg wind='gusts 8 m/s' One recording
  wisebucket note flights:2026-10/flight3.ulg wind=              Remove a key
  wisebucket note flights:2026-10/flight3.ulg                    Show its keys")]
    Note {
        /// A recording (`root:path`), a folder (`root:folder/`) or a root (`root:`)
        target: String,
        /// `key=value` pairs; an empty value removes the key
        #[arg(value_name = "KEY=VALUE")]
        values: Vec<String>,
    },
    /// Extract one recording into the store (run in the background by agent sessions)
    #[command(hide = true)]
    Ingest {
        /// The recording, as `root:path`
        recording: String,
    },
    /// Check the whole installation and explain how to fix problems
    Doctor,
    /// Download public sample recordings and declare them as the `demo` data root
    #[command(after_help = "\
Examples:
  wisebucket demo --list     Show the formats and their licenses
  wisebucket demo mcap ulog  Download two formats")]
    Demo(DemoArgs),
    /// Print a shell completion script to stdout
    #[command(after_help = "\
Examples:
  wisebucket completions zsh > ~/.zfunc/_wisebucket
  wisebucket completions bash > ~/.local/share/bash-completion/completions/wisebucket
  wisebucket completions fish > ~/.config/fish/completions/wisebucket.fish")]
    Completions {
        /// Target shell
        shell: clap_complete::Shell,
    },
    /// Documentation helpers (for contributors)
    #[command(subcommand, hide = true)]
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
    #[command(after_help = "\
Examples:
  wisebucket roots add bags ~/robot-logs
  wisebucket roots add field /data/field --exclude '**/tmp/**'")]
    Add {
        /// Short name, used in references like `name:relative/path`
        name: String,
        /// Folder path (absolute or ~/...)
        path: PathBuf,
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
pub enum DocsCommand {
    /// Generate reference pages (errors, CLI, configuration, tables) into docs/reference
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
        }
    }
}
