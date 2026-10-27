use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Config binary arguments
#[derive(Debug, Clone, Parser)]
#[command(about = "Declarative system configuration")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Skip the confirmation prompt
    #[arg(short, long, global = true)]
    pub yes: bool,

    /// Print the plan and change nothing
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// Override the hostname [env: RUNIX_HOST]
    #[arg(long, global = true)]
    pub host: Option<String>,

    /// Set a flag, repeatable [env: RUNIX_FLAGS, comma-separated]
    #[arg(long = "flag", global = true)]
    pub flags: Vec<String>,

    /// Override the project root [env: RUNIX_ROOT]
    #[arg(long, global = true)]
    pub root: Option<PathBuf>,

    /// Back up conflicting targets, then replace them
    #[arg(long, global = true)]
    pub adopt: bool,

    /// Replace conflicting targets without a backup
    #[arg(long, global = true)]
    pub force: bool,

    /// Install and place only, remove nothing
    #[arg(long, global = true)]
    pub no_prune: bool,

    /// Allow a plan that removes more than 20 packages
    #[arg(long, global = true)]
    pub allow_mass_remove: bool,

    /// Allow a run as root
    #[arg(long, global = true)]
    pub allow_root: bool,
}

/// Runix subcommand
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Print every change that apply will make
    Plan,
    /// Print the plan, confirm, then make the changes
    Apply,
    /// Upgrade every provider, then apply
    Update,
    /// Print a short summary of drift from the config
    Status,
    /// Print content diffs for copied files and templates
    Diff,
    /// Render linked templates again and change nothing else
    UpdateTemplates,
    /// Print the current explicit packages as config code
    Import,
    /// Search every registered provider for packages
    Search {
        /// Text to search for
        query: String,
    },
}
