use super::{Package, Provider, SearchResult, lines};
use crate::runner::{Cmd, Runner};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};

/// Provider for native repo packages
#[derive(Debug, Clone, Copy, Default)]
pub struct Pacman;

impl Provider for Pacman {
    fn name(&self) -> &'static str {
        "pacman"
    }

    fn resolve(&self, run: &Runner, specs: &[String]) -> Result<Vec<Package>> {
        let groups = groups(run, specs)?;
        let mut out = Vec::new();
        for spec in specs {
            if let Some(members) = groups.get(spec) {
                out.extend(members.iter().map(|m| Package::new(m.as_str(), m.as_str())));
            } else {
                let name = spec.rsplit('/').next().unwrap_or(spec);
                out.push(Package::new(name, spec.as_str()));
            }
        }
        Ok(out)
    }

    fn explicit(&self, run: &Runner) -> Result<BTreeSet<String>> {
        Ok(lines(&run.read_any(&Cmd::new("pacman").arg("-Qqen"))?))
    }

    fn installed(&self, run: &Runner) -> Result<BTreeSet<String>> {
        Ok(lines(&run.read_any(&Cmd::new("pacman").arg("-Qq"))?))
    }

    fn removable(&self, run: &Runner) -> Result<BTreeSet<String>> {
        Ok(lines(&run.read_any(&Cmd::new("pacman").arg("-Qqentt"))?))
    }

    fn protected(&self) -> &'static [&'static str] {
        &["base", "glibc", "pacman", "sudo", "systemd"]
    }

    fn needs_root(&self) -> bool {
        true
    }

    fn install(&self, run: &Runner, pkgs: &[Package]) -> Result<()> {
        let specs = pkgs.iter().map(|p| p.spec.as_str());
        run.run(&pacman().args(["-S", "--needed", "--noconfirm"]).args(specs))
    }

    fn mark_explicit(&self, run: &Runner, names: &[String]) -> Result<()> {
        run.run(
            &pacman()
                .args(["-D", "--asexplicit"])
                .args(names.iter().cloned()),
        )
    }

    fn mark_dependency(&self, run: &Runner, names: &[String]) -> Result<()> {
        run.run(
            &pacman()
                .args(["-D", "--asdeps"])
                .args(names.iter().cloned()),
        )
    }

    fn remove(&self, run: &Runner, names: &[String]) -> Result<()> {
        run.run(
            &pacman()
                .args(["-Rns", "--noconfirm"])
                .args(names.iter().cloned()),
        )
    }

    fn upgrade(&self, run: &Runner, _pkgs: &[Package], yes: bool) -> Result<()> {
        let cmd = pacman().arg("-Syu");
        run.run(&if yes { cmd.arg("--noconfirm") } else { cmd })
    }

    fn search(&self, run: &Runner, query: &str) -> Result<Option<Vec<SearchResult>>> {
        let text = run.read_any(&Cmd::new("pacman").args(["-Ss", query]))?;
        Ok(Some(parse_search(&text)))
    }
}

fn pacman() -> Cmd {
    Cmd::new("pacman").sudo(true)
}

fn parse_search(text: &str) -> Vec<SearchResult> {
    let mut out: Vec<SearchResult> = Vec::new();
    for line in text.lines() {
        if line.starts_with(char::is_whitespace) {
            if let Some(last) = out.last_mut() {
                last.description = line.trim().to_string();
            }
        } else if let Some((name, rest)) = line.split_once(' ') {
            let version = rest.split_whitespace().next().unwrap_or_default();
            out.push(SearchResult::new(name, version, ""));
        }
    }
    out
}

fn groups(run: &Runner, specs: &[String]) -> Result<BTreeMap<String, Vec<String>>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if specs.iter().all(|s| s.contains('/')) {
        return Ok(out);
    }
    let known = lines(&run.read_any(&Cmd::new("pacman").arg("-Sg"))?);
    let wanted: Vec<&String> = specs.iter().filter(|s| known.contains(*s)).collect();
    if wanted.is_empty() {
        return Ok(out);
    }
    let listing = run.read(
        &Cmd::new("pacman")
            .arg("-Sg")
            .args(wanted.into_iter().cloned()),
    )?;
    for line in listing.lines() {
        if let Some((group, pkg)) = line.split_once(' ') {
            out.entry(group.to_string())
                .or_default()
                .push(pkg.trim().to_string());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_output() {
        let text = "extra/jujutsu 0.23.0-1 [installed]\n    Git-compatible VCS\ncachyos/jj-fzf 0.25-1\n    Fuzzy finder for jj\n";
        assert_eq!(
            parse_search(text),
            [
                SearchResult::new("extra/jujutsu", "0.23.0-1", "Git-compatible VCS"),
                SearchResult::new("cachyos/jj-fzf", "0.25-1", "Fuzzy finder for jj"),
            ]
        );
    }
}
