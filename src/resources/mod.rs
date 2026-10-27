mod systemd;

pub use systemd::{Services, UserServices};

use crate::runner::Runner;
use anyhow::Result;
use std::collections::BTreeSet;

/// Managed set of named items on the system
pub trait Resource {
    /// Returns the unique name of the resource
    fn name(&self) -> &'static str;

    /// Returns the subset of items that are already in place
    fn present(&self, run: &Runner, items: &[String]) -> Result<BTreeSet<String>>;

    fn add(&self, run: &Runner, items: &[String]) -> Result<()>;

    fn remove(&self, run: &Runner, items: &[String]) -> Result<()>;

    /// Reports whether changes require root
    fn needs_root(&self) -> bool {
        false
    }
}
