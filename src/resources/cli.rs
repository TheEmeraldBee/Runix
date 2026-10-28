use super::Resource;
use crate::runner::{Cmd, Runner};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;

/// CLI commands, each installed and removed with its own commands
pub struct CliCommands;

/// CLI command managed by `CliCommands`, identified by name
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliCommand {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<Cmd>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<Cmd>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove: Option<Cmd>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply: Option<Cmd>,
}

impl CliCommand {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            check: None,
            install: None,
            remove: None,
            apply: None,
        }
    }

    /// Sets the command that succeeds if the command is installed
    pub fn on_check(mut self, cmd: Cmd) -> Self {
        self.check = Some(cmd);
        self
    }

    /// Sets the command that installs the command
    pub fn on_install(mut self, cmd: Cmd) -> Self {
        self.install = Some(cmd);
        self
    }

    /// Sets the command that removes the command
    pub fn on_remove(mut self, cmd: Cmd) -> Self {
        self.remove = Some(cmd);
        self
    }

    /// Sets the command that runs on every apply
    pub fn on_apply(mut self, cmd: Cmd) -> Self {
        self.apply = Some(cmd);
        self
    }
}

impl PartialEq for CliCommand {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for CliCommand {}

impl PartialOrd for CliCommand {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CliCommand {
    fn cmp(&self, other: &Self) -> Ordering {
        self.name.cmp(&other.name)
    }
}

impl fmt::Display for CliCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

impl Resource for CliCommands {
    type Item = CliCommand;

    fn name(&self) -> &'static str {
        "cli"
    }

    fn present(&self, run: &Runner, item: &CliCommand) -> Result<Option<bool>> {
        item.check.as_ref().map(|cmd| run.succeeds(cmd)).transpose()
    }

    fn add(&self, run: &Runner, items: &[CliCommand]) -> Result<()> {
        for cmd in items.iter().filter_map(|i| i.install.as_ref()) {
            run.run(cmd)?;
        }
        Ok(())
    }

    fn remove(&self, run: &Runner, items: &[CliCommand]) -> Result<()> {
        for cmd in items.iter().filter_map(|i| i.remove.as_ref()) {
            run.run(cmd)?;
        }
        Ok(())
    }

    fn apply(&self, run: &Runner, items: &[CliCommand]) -> Result<()> {
        for cmd in items.iter().filter_map(|i| i.apply.as_ref()) {
            run.run(cmd)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_each_command() {
        let run = Runner::default();
        let unchecked = CliCommand::new("sh");
        assert_eq!(CliCommands.present(&run, &unchecked).unwrap(), None);

        let check = |name: &str| Cmd::new("sh").args(["-c", &format!("command -v {name}")]);
        let found = CliCommand::new("sh").on_check(check("sh"));
        let missing = CliCommand::new("missing").on_check(check("runix-missing-command"));
        assert_eq!(CliCommands.present(&run, &found).unwrap(), Some(true));
        assert_eq!(CliCommands.present(&run, &missing).unwrap(), Some(false));
    }

    #[test]
    fn commands_round_trip_through_state() {
        let item =
            CliCommand::new("rustup").on_remove(Cmd::new("rustup").arg("self").arg("uninstall"));
        let value = serde_json::to_value(&item).unwrap();
        let back: CliCommand = serde_json::from_value(value).unwrap();
        assert_eq!(back.remove, item.remove);
    }
}
