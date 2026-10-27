//! Runix is a framework for declarative system configuration

mod cli;
mod context;
mod files;
mod plan;
mod providers;
mod resources;
mod runner;
mod state;
mod style;
mod system;
mod tree;

pub use cli::{Cli, Command};
pub use context::Context;
pub use files::{Entry, FileAction, FileKind, Rendered};
pub use plan::{PackagePlan, Plan, ResourcePlan};
pub use providers::{Brew, Cargo, Package, Provider, SearchResult, Yay};
pub use resources::{Resource, Services, UserServices};
pub use runner::{Cmd, Runner};
pub use state::{State, StateEntry};
pub use system::{System, Template};
pub use tree::Node;

/// Common imports for configs
pub mod prelude {
    pub use crate::{
        Brew, Cargo, Cmd, Provider, Resource, Runner, Services, System, UserServices, Yay,
    };
    pub use anyhow::{Result, bail};
}
