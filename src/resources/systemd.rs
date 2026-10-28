use super::Resource;
use crate::runner::{Cmd, Runner};
use anyhow::Result;

/// Systemd system units, enabled and started
pub struct Services;

/// Systemd user units, enabled and started
pub struct UserServices;

impl Resource for Services {
    type Item = String;

    fn name(&self) -> &'static str {
        "services"
    }

    fn present(&self, run: &Runner, item: &String) -> Result<Option<bool>> {
        enabled(run, false, item).map(Some)
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
    type Item = String;

    fn name(&self) -> &'static str {
        "user-services"
    }

    fn present(&self, run: &Runner, item: &String) -> Result<Option<bool>> {
        enabled(run, true, item).map(Some)
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

fn enabled(run: &Runner, user: bool, unit: &str) -> Result<bool> {
    let state = run.read_any(&systemctl(user).args(["is-enabled", unit]))?;
    Ok(state.trim() == "enabled")
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
