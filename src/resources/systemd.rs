use super::Resource;
use crate::runner::{Cmd, Runner};
use anyhow::Result;
use std::collections::BTreeSet;

/// Systemd system units, enabled and started
pub struct Services;

/// Systemd user units, enabled and started
pub struct UserServices;

impl Resource for Services {
    fn name(&self) -> &'static str {
        "services"
    }

    fn present(&self, run: &Runner, items: &[String]) -> Result<BTreeSet<String>> {
        enabled(run, false, items)
    }

    fn add(&self, run: &Runner, items: &[String]) -> Result<()> {
        enable(run, false, items)
    }

    fn remove(&self, run: &Runner, items: &[String]) -> Result<()> {
        disable(run, false, items)
    }

    fn needs_root(&self) -> bool {
        true
    }
}

impl Resource for UserServices {
    fn name(&self) -> &'static str {
        "user-services"
    }

    fn present(&self, run: &Runner, items: &[String]) -> Result<BTreeSet<String>> {
        enabled(run, true, items)
    }

    fn add(&self, run: &Runner, items: &[String]) -> Result<()> {
        enable(run, true, items)
    }

    fn remove(&self, run: &Runner, items: &[String]) -> Result<()> {
        disable(run, true, items)
    }
}

fn systemctl(user: bool) -> Cmd {
    if user {
        Cmd::new("systemctl").arg("--user")
    } else {
        Cmd::new("systemctl")
    }
}

fn enabled(run: &Runner, user: bool, items: &[String]) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    for unit in items {
        let state = run.read_any(&systemctl(user).args(["is-enabled", unit.as_str()]))?;
        if state.trim() == "enabled" {
            out.insert(unit.clone());
        }
    }
    Ok(out)
}

fn enable(run: &Runner, user: bool, items: &[String]) -> Result<()> {
    run.run(&systemctl(user).arg("daemon-reload").sudo(!user))?;
    run.run(
        &systemctl(user)
            .args(["enable", "--now"])
            .args(items)
            .sudo(!user),
    )
}

fn disable(run: &Runner, user: bool, items: &[String]) -> Result<()> {
    run.run(
        &systemctl(user)
            .args(["disable", "--now"])
            .args(items)
            .sudo(!user),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_changes_use_sudo() {
        let cmd = systemctl(false).args(["enable", "--now"]).sudo(true);
        assert_eq!(cmd.to_string(), "sudo systemctl enable --now");
        assert_eq!(systemctl(true).to_string(), "systemctl --user");
    }
}
