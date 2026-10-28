use crate::style;
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;
use std::process::{Command, Output, Stdio};

/// External command
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cmd {
    program: String,
    args: Vec<String>,
    sudo: bool,
}

impl Cmd {
    pub fn new(program: &str) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            sudo: false,
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Sets whether the command runs as root
    pub fn sudo(mut self, on: bool) -> Self {
        self.sudo = on;
        self
    }

    fn command(&self) -> Command {
        let mut cmd = if self.sudo {
            let mut cmd = Command::new("sudo");
            cmd.arg(&self.program);
            cmd
        } else {
            Command::new(&self.program)
        };
        cmd.args(&self.args);
        cmd
    }

    fn spawn_error(&self, err: io::Error) -> anyhow::Error {
        if err.kind() == io::ErrorKind::NotFound {
            anyhow!("{} is not installed", self.program)
        } else {
            anyhow!("failed to start {}: {err}", self.program)
        }
    }
}

impl fmt::Display for Cmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.sudo {
            write!(f, "sudo ")?;
        }
        write!(f, "{}", self.program)?;
        for arg in &self.args {
            write!(f, " {arg}")?;
        }
        Ok(())
    }
}

/// External command runner
#[derive(Debug, Clone, Default)]
pub struct Runner {
    pub dry_run: bool,
}

impl Runner {
    /// Returns the stdout of a read-only command
    pub fn read(&self, cmd: &Cmd) -> Result<String> {
        let out = self.output(cmd)?;
        if !out.status.success() {
            bail!(
                "{cmd} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Returns the stdout of a command whatever its exit status
    pub fn read_any(&self, cmd: &Cmd) -> Result<String> {
        Ok(String::from_utf8_lossy(&self.output(cmd)?.stdout).into_owned())
    }

    /// Reports whether a read-only command exits successfully
    pub fn succeeds(&self, cmd: &Cmd) -> Result<bool> {
        Ok(self.output(cmd)?.status.success())
    }

    /// Runs a command that changes the system
    pub fn run(&self, cmd: &Cmd) -> Result<()> {
        if self.dry_run {
            println!("{}", style::dim(&format!("$ {cmd} (dry run)")));
            return Ok(());
        }
        println!("{}", style::dim(&format!("$ {cmd}")));
        let status = cmd.command().status().map_err(|e| cmd.spawn_error(e))?;
        if !status.success() {
            bail!("{cmd} failed with {status}");
        }
        Ok(())
    }

    fn output(&self, cmd: &Cmd) -> Result<Output> {
        cmd.command()
            .stdin(Stdio::null())
            .output()
            .map_err(|e| cmd.spawn_error(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_sudo_command() {
        let cmd = Cmd::new("pacman").args(["-S", "fish"]).sudo(true);
        assert_eq!(cmd.to_string(), "sudo pacman -S fish");
    }
}
