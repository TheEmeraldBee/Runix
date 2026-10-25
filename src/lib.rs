//! Runix is a framework for declarative system configuration

mod cli;
mod context;
mod runner;
mod style;
mod tree;

pub use cli::{Cli, Command};
pub use context::Context;
pub use runner::{Cmd, Runner};
pub use tree::Node;
