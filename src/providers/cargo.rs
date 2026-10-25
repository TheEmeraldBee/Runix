use super::{Package, Provider, SearchResult};
use crate::runner::{Cmd, Runner};
use anyhow::Result;
use std::collections::BTreeSet;

/// Provider for `cargo install` crates
#[derive(Debug, Clone, Copy, Default)]
pub struct Cargo;

impl Provider for Cargo {
    fn name(&self) -> &'static str {
        "cargo"
    }

    fn resolve(&self, _run: &Runner, specs: &[String]) -> Result<Vec<Package>> {
        Ok(specs
            .iter()
            .map(|s| Package::new(s.split('@').next().unwrap_or(s), s.as_str()))
            .collect())
    }

    fn explicit(&self, run: &Runner) -> Result<BTreeSet<String>> {
        Ok(parse_list(
            &run.read(&Cmd::new("cargo").args(["install", "--list"]))?,
        ))
    }

    fn install(&self, run: &Runner, pkgs: &[Package]) -> Result<()> {
        let specs = pkgs.iter().map(|p| p.spec.as_str());
        run.run(&Cmd::new("cargo").args(["install", "--locked"]).args(specs))
    }

    fn remove(&self, run: &Runner, names: &[String]) -> Result<()> {
        run.run(
            &Cmd::new("cargo")
                .arg("uninstall")
                .args(names.iter().cloned()),
        )
    }

    fn upgrade(&self, run: &Runner, pkgs: &[Package], _yes: bool) -> Result<()> {
        // cargo install replaces outdated crates with their newest version
        if pkgs.is_empty() {
            return Ok(());
        }
        self.install(run, pkgs)
    }

    fn search(&self, run: &Runner, query: &str) -> Result<Option<Vec<SearchResult>>> {
        let text = run.read(&Cmd::new("cargo").args(["search", query]))?;
        Ok(Some(parse_search(&text)))
    }
}

fn parse_list(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter(|l| !l.starts_with(char::is_whitespace))
        .filter_map(|l| l.split_whitespace().next())
        .map(String::from)
        .collect()
}

fn parse_search(text: &str) -> Vec<SearchResult> {
    text.lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once(" = \"")?;
            let (version, rest) = rest.split_once('"')?;
            let description = rest.trim_start().strip_prefix('#').unwrap_or_default();
            Some(SearchResult::new(name, version, description.trim()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_install_list() {
        let text = "ripgrep v14.1.0:\n    rg\nrunix v0.1.0 (/home/me/runix):\n    runix\n";
        let names: Vec<String> = parse_list(text).into_iter().collect();
        assert_eq!(names, ["ripgrep", "runix"]);
    }

    #[test]
    fn parses_search_output() {
        let text = "jj-cli = \"0.23.0\"    # Jujutsu VCS\njj-lib = \"0.23.0\"\n... and 98 crates more (use --limit N to see more)\n";
        assert_eq!(
            parse_search(text),
            [
                SearchResult::new("jj-cli", "0.23.0", "Jujutsu VCS"),
                SearchResult::new("jj-lib", "0.23.0", ""),
            ]
        );
    }
}
