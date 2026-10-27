use super::{Package, Provider, SearchResult, lines};
use crate::runner::{Cmd, Runner};
use anyhow::Result;
use serde::Deserialize;
use std::collections::BTreeSet;

/// Provider for Homebrew formulae
#[derive(Debug, Clone, Copy, Default)]
pub struct Brew;

impl Provider for Brew {
    fn name(&self) -> &'static str {
        "brew"
    }

    fn explicit(&self, run: &Runner) -> Result<BTreeSet<String>> {
        Ok(lines(&run.read(
            &Cmd::new("brew").args(["leaves", "--installed-on-request"]),
        )?))
    }

    fn installed(&self, run: &Runner) -> Result<BTreeSet<String>> {
        Ok(lines(&run.read(&Cmd::new("brew").args([
            "list",
            "--formula",
            "--full-name",
        ]))?))
    }

    fn removable(&self, run: &Runner) -> Result<BTreeSet<String>> {
        let leaves = lines(&run.read(&Cmd::new("brew").arg("leaves"))?);
        let explicit = self.explicit(run)?;
        Ok(leaves.intersection(&explicit).cloned().collect())
    }

    fn install(&self, run: &Runner, pkgs: &[Package]) -> Result<()> {
        let specs = pkgs.iter().map(|p| p.spec.as_str());
        run.run(&Cmd::new("brew").args(["install", "--formula"]).args(specs))
    }

    fn mark_explicit(&self, run: &Runner, names: &[String]) -> Result<()> {
        run.run(
            &Cmd::new("brew")
                .args(["tab", "--installed-on-request"])
                .args(names.iter().cloned()),
        )
    }

    fn mark_dependency(&self, run: &Runner, names: &[String]) -> Result<()> {
        run.run(
            &Cmd::new("brew")
                .args(["tab", "--no-installed-on-request"])
                .args(names.iter().cloned()),
        )
    }

    fn remove(&self, run: &Runner, names: &[String]) -> Result<()> {
        run.run(
            &Cmd::new("brew")
                .args(["uninstall", "--formula"])
                .args(names.iter().cloned()),
        )?;
        run.run(&Cmd::new("brew").arg("autoremove"))
    }

    fn upgrade(&self, run: &Runner, _pkgs: &[Package], _yes: bool) -> Result<()> {
        run.run(&Cmd::new("brew").arg("update"))?;
        run.run(&Cmd::new("brew").args(["upgrade", "--formula"]))
    }

    fn search(&self, run: &Runner, query: &str) -> Result<Option<Vec<SearchResult>>> {
        let names = lines(&run.read_any(&Cmd::new("brew").args(["search", "--formula", query]))?);
        if names.is_empty() {
            return Ok(Some(Vec::new()));
        }
        let json = run.read(
            &Cmd::new("brew")
                .args(["info", "--json=v2", "--formula"])
                .args(names),
        )?;
        Ok(Some(parse_info(&json)?))
    }
}

#[derive(Deserialize)]
struct Info {
    formulae: Vec<Formula>,
}

#[derive(Deserialize)]
struct Formula {
    full_name: String,
    desc: Option<String>,
    versions: Versions,
}

#[derive(Deserialize)]
struct Versions {
    stable: Option<String>,
}

fn parse_info(json: &str) -> Result<Vec<SearchResult>> {
    let info: Info = serde_json::from_str(json)?;
    Ok(info
        .formulae
        .into_iter()
        .map(|f| {
            SearchResult::new(
                f.full_name,
                f.versions.stable.unwrap_or_default(),
                f.desc.unwrap_or_default(),
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_info_output() {
        let json = r#"{"formulae":[
            {"full_name":"jj","desc":"Git-compatible VCS","versions":{"stable":"0.23.0"}},
            {"full_name":"me/tap/tool","desc":null,"versions":{"stable":null}}
        ],"casks":[]}"#;
        assert_eq!(
            parse_info(json).unwrap(),
            [
                SearchResult::new("jj", "0.23.0", "Git-compatible VCS"),
                SearchResult::new("me/tap/tool", "", ""),
            ]
        );
    }
}
