//! Runix is a framework for declarative system configuration

mod cli;
mod context;
mod files;
mod plan;
mod providers;
mod runner;
mod state;
mod style;
mod tree;

pub use cli::{Cli, Command};
pub use context::Context;
pub use files::{Entry, FileAction, FileKind};
pub use plan::{PackagePlan, Plan};
pub use providers::{Cargo, Package, Pacman, Provider, SearchResult};
pub use runner::{Cmd, Runner};
pub use state::{State, StateEntry};
pub use tree::Node;
