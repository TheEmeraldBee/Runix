use crate::cli::{Cli, Command};
use anyhow::{Context as _, Result};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::PathBuf;

/// Facts about the current run
#[derive(Debug, Clone)]
pub struct Context {
    pub cli: Cli,
    pub hostname: String,
    pub flags: BTreeSet<String>,
    pub root: PathBuf,
    pub home: PathBuf,
    pub state: PathBuf,
    pub user: String,
}

impl Context {
    /// Returns the context for the given command line
    pub fn new(cli: Cli) -> Result<Self> {
        let hostname = cli
            .host
            .clone()
            .or_else(|| env::var("RUNIX_HOST").ok())
            .or_else(|| read_trimmed("/etc/hostname"))
            .or_else(|| read_trimmed("/proc/sys/kernel/hostname"))
            .unwrap_or_else(|| "localhost".into());

        let mut flags: BTreeSet<String> = cli.flags.iter().cloned().collect();
        if let Ok(list) = env::var("RUNIX_FLAGS") {
            flags.extend(
                list.split(',')
                    .map(str::trim)
                    .filter(|f| !f.is_empty())
                    .map(String::from),
            );
        }

        let root = cli
            .root
            .clone()
            .or_else(|| env::var_os("RUNIX_ROOT").map(PathBuf::from))
            .or_else(|| env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from))
            .map_or_else(env::current_dir, Ok)?;
        let root = root
            .canonicalize()
            .with_context(|| format!("project root {} not found", root.display()))?;

        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME is not set")?;
        let state = env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/state"))
            .join("runix");
        let user = env::var("SUDO_USER")
            .or_else(|_| env::var("USER"))
            .unwrap_or_default();

        Ok(Self {
            cli,
            hostname,
            flags,
            root,
            home,
            state,
            user,
        })
    }

    /// Returns the subcommand to run (plan by default)
    pub fn command(&self) -> Command {
        self.cli.command.clone().unwrap_or(Command::Plan)
    }
}

fn read_trimmed(path: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}
