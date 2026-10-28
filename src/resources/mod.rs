mod cli;
mod systemd;

pub use cli::{CliCommand, CliCommands};
pub use systemd::{Services, UserServices};

use crate::runner::Runner;
use anyhow::Result;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fmt::Display;

/// Managed set of items on the system
pub trait Resource: 'static {
    /// Item that the resource manages
    type Item: Clone + Ord + Display + Serialize + DeserializeOwned;

    /// Returns the unique name of the resource
    fn name(&self) -> &'static str;

    /// Reports whether an item is already in place (`None` to trust the state)
    fn present(&self, _run: &Runner, _item: &Self::Item) -> Result<Option<bool>> {
        Ok(None)
    }

    fn add(&self, run: &Runner, items: &[Self::Item]) -> Result<()>;

    fn remove(&self, run: &Runner, items: &[Self::Item]) -> Result<()>;

    /// Brings the declared items up to date on every apply
    fn apply(&self, _run: &Runner, _items: &[Self::Item]) -> Result<()> {
        Ok(())
    }

    /// Reports whether changes require root
    fn needs_root(&self) -> bool {
        false
    }
}
