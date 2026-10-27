mod brew;
mod cargo;
mod yay;

pub use brew::Brew;
pub use cargo::Cargo;
pub use yay::Yay;

use crate::runner::Runner;
use anyhow::Result;
use std::collections::BTreeSet;

/// Resolved package
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Package {
    pub name: String,
    /// Install argument
    pub spec: String,
}

impl Package {
    pub fn new(name: impl Into<String>, spec: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            spec: spec.into(),
        }
    }
}

/// Package search result
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    /// Argument for `pkgs`
    pub name: String,
    pub version: String,
    pub description: String,
}

impl SearchResult {
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            description: description.into(),
        }
    }
}

/// Package source
pub trait Provider {
    /// Returns the unique name of the provider
    fn name(&self) -> &'static str;

    /// Returns the Rust type name of the provider
    fn type_name(&self) -> &'static str {
        let full = std::any::type_name::<Self>();
        full.rsplit("::").next().unwrap_or(full)
    }

    /// Returns the packages for the declared specs
    fn resolve(&self, _run: &Runner, specs: &[String]) -> Result<Vec<Package>> {
        Ok(specs
            .iter()
            .map(|s| Package::new(s.as_str(), s.as_str()))
            .collect())
    }

    /// Returns explicitly installed packages
    fn explicit(&self, run: &Runner) -> Result<BTreeSet<String>>;

    fn installed(&self, run: &Runner) -> Result<BTreeSet<String>> {
        self.explicit(run)
    }

    /// Returns explicit packages no other package requires
    fn removable(&self, run: &Runner) -> Result<BTreeSet<String>> {
        self.explicit(run)
    }

    /// Returns the packages that pruning never removes
    fn protected(&self) -> &'static [&'static str] {
        &[]
    }

    /// Returns the packages that pruning always skips
    fn ignored(&self) -> &'static [&'static str] {
        &[]
    }

    /// Reports whether changes require root
    fn needs_root(&self) -> bool {
        false
    }

    fn install(&self, run: &Runner, pkgs: &[Package]) -> Result<()>;

    /// Marks installed dependencies as explicit
    fn mark_explicit(&self, _run: &Runner, _names: &[String]) -> Result<()> {
        Ok(())
    }

    /// Marks explicit packages as dependencies
    fn mark_dependency(&self, _run: &Runner, _names: &[String]) -> Result<()> {
        Ok(())
    }

    fn remove(&self, run: &Runner, names: &[String]) -> Result<()>;

    /// Upgrades every package of the provider
    fn upgrade(&self, run: &Runner, pkgs: &[Package], yes: bool) -> Result<()>;

    /// Returns the packages that match `query` (`None` if unsupported)
    fn search(&self, _run: &Runner, _query: &str) -> Result<Option<Vec<SearchResult>>> {
        Ok(None)
    }
}

fn lines(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}
