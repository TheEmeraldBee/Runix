use crate::cli::{Cli, Command};
use crate::context::Context;
use crate::files::{self, FileAction, FileDecl, FileKind};
use crate::plan::{self, Plan};
use crate::providers::Provider;
use crate::runner::{Cmd, Runner};
use crate::state::State;
use crate::style;
use crate::tree::Node;
use anyhow::{Result, bail};
use clap::Parser;
use similar::TextDiff;
use std::collections::BTreeSet;
use std::io::{self, BufRead, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

const MASS_REMOVE: usize = 20;

struct Managed {
    provider: &'static dyn Provider,
    specs: Vec<String>,
    ignored: BTreeSet<String>,
    used: bool,
}

/// Declared state of the machine
pub struct System {
    ctx: Context,
    managed: Vec<Managed>,
    files: Vec<FileDecl>,
}

impl System {
    /// Returns an empty system for the process arguments
    pub fn init(providers: &[&'static dyn Provider]) -> Result<Self> {
        Self::from_cli(Cli::parse(), providers)
    }

    /// Returns an empty system for the given arguments
    pub fn from_cli(cli: Cli, providers: &[&'static dyn Provider]) -> Result<Self> {
        let mut managed: Vec<Managed> = Vec::new();
        for &provider in providers {
            if managed.iter().any(|m| m.provider.name() == provider.name()) {
                bail!("provider {} is registered twice", provider.name());
            }
            managed.push(Managed {
                provider,
                specs: Vec::new(),
                ignored: BTreeSet::new(),
                used: false,
            });
        }
        Ok(Self {
            ctx: Context::new(cli)?,
            managed,
            files: Vec::new(),
        })
    }

    /// Declares packages of a registered provider (panics if unregistered)
    pub fn pkgs<P: Provider>(&mut self, provider: P, pkgs: &[&str]) -> &mut Self {
        let managed = self.managed(&provider);
        managed.specs.extend(pkgs.iter().map(|p| p.to_string()));
        self
    }

    /// Excludes packages of a registered provider from removal (panics if unregistered)
    pub fn ignore<P: Provider>(&mut self, provider: P, pkgs: &[&str]) -> &mut Self {
        let managed = self.managed(&provider);
        managed.ignored.extend(pkgs.iter().map(|p| p.to_string()));
        self
    }

    /// Symlinks every file under source to the same place under target
    pub fn link(&mut self, source: &str, target: &str) -> &mut Self {
        self.file(FileKind::Link, source, target)
    }

    /// Copies every file under source to the same place under target
    pub fn clone(&mut self, source: &str, target: &str) -> &mut Self {
        self.file(FileKind::Copy, source, target)
    }

    /// Alias for `clone`
    pub fn copy(&mut self, source: &str, target: &str) -> &mut Self {
        self.file(FileKind::Copy, source, target)
    }

    pub fn context(&self) -> &Context {
        &self.ctx
    }

    pub fn hostname(&self) -> &str {
        &self.ctx.hostname
    }

    /// Reports whether a flag is set
    pub fn flag(&self, name: &str) -> bool {
        self.ctx.flags.contains(name)
    }

    pub fn env(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    /// Returns the invoking user
    pub fn user(&self) -> &str {
        &self.ctx.user
    }

    /// Returns the home directory of the invoking user
    pub fn home(&self) -> &Path {
        &self.ctx.home
    }

    /// Returns the project root
    pub fn root(&self) -> &Path {
        &self.ctx.root
    }

    /// Returns the changes `apply` makes
    pub fn plan(&self, run: &Runner) -> Result<Plan<'_>> {
        let prune = !self.ctx.cli.no_prune;
        let packages = self
            .used()
            .map(|m| plan::plan_packages(run, m.provider, &m.specs, &m.ignored, prune))
            .collect::<Result<Vec<_>>>()?;

        let state = State::load(&State::path(&self.ctx))?;
        let entries = files::expand(&self.ctx, &self.files)?;
        let files = files::plan(&self.ctx, &entries, &state)?;
        let next_state = plan::next_state(&self.ctx, &entries, &state);
        Ok(Plan {
            packages,
            files,
            next_state,
        })
    }

    /// Runs the subcommand from the command line
    pub fn apply(self) -> Result<()> {
        match self.ctx.command() {
            Command::Plan => self.cmd_plan(),
            Command::Apply => self.cmd_apply(false),
            Command::Update => self.cmd_apply(true),
            Command::Status => self.cmd_status(),
            Command::Diff => self.cmd_diff(),
            Command::Import => self.cmd_import(),
            Command::Search { query } => self.cmd_search(&query),
        }
    }

    fn managed(&mut self, provider: &dyn Provider) -> &mut Managed {
        let name = provider.name();
        let Some(managed) = self.managed.iter_mut().find(|m| m.provider.name() == name) else {
            panic!("provider {name} is not registered, add it to System::init");
        };
        managed.used = true;
        managed
    }

    fn used(&self) -> impl Iterator<Item = &Managed> {
        self.managed.iter().filter(|m| m.used)
    }

    fn file(&mut self, kind: FileKind, source: &str, target: &str) -> &mut Self {
        self.files.push(FileDecl {
            kind,
            source: source.into(),
            target: target.into(),
        });
        self
    }

    fn runner(&self) -> Runner {
        Runner {
            dry_run: self.ctx.cli.dry_run,
        }
    }

    fn cmd_plan(&self) -> Result<()> {
        let plan = self.plan(&self.runner())?;
        if plan.is_empty() {
            println!("Nothing to do, the system matches the config");
        } else {
            print!("{}", plan.tree(&self.ctx).render());
        }
        Ok(())
    }

    fn cmd_status(&self) -> Result<()> {
        let plan = self.plan(&self.runner())?;
        let lines = plan.summary();
        if lines.is_empty() {
            println!("In sync");
        }
        for line in lines {
            println!("{line}");
        }
        Ok(())
    }

    fn cmd_diff(&self) -> Result<()> {
        let plan = self.plan(&self.runner())?;
        for action in &plan.files {
            let entry = match action {
                FileAction::Update(e) => e,
                FileAction::Replace { entry, .. } | FileAction::Conflict { entry, .. } => entry,
                _ => continue,
            };
            if entry.kind != FileKind::Copy {
                continue;
            }
            let (Ok(old), Ok(new)) = (
                std::fs::read_to_string(&entry.target),
                std::fs::read_to_string(&entry.source),
            ) else {
                continue;
            };
            let diff = TextDiff::from_lines(&old, &new);
            let target = entry.target.display().to_string();
            let source = entry.source.display().to_string();
            print!("{}", diff.unified_diff().header(&target, &source));
        }
        Ok(())
    }

    fn cmd_apply(&self, upgrade: bool) -> Result<()> {
        self.ensure_not_root()?;
        let run = self.runner();
        let cli = &self.ctx.cli;

        if upgrade {
            if !cli.dry_run && self.used().any(|m| m.provider.needs_root()) {
                sudo_validate()?;
            }
            for m in self.used() {
                println!(
                    "{}",
                    style::bold(&format!("upgrading {}", m.provider.name()))
                );
                let pkgs = m.provider.resolve(&run, &m.specs)?;
                m.provider.upgrade(&run, &pkgs, cli.yes)?;
            }
        }

        let plan = self.plan(&run)?;
        let state_path = State::path(&self.ctx);
        if plan.is_empty() {
            println!("Nothing to do, the system matches the config");
            if !cli.dry_run {
                plan.next_state.save(&state_path)?;
            }
            return Ok(());
        }
        print!("{}", plan.tree(&self.ctx).render());

        let conflicts = plan.conflicts();
        if !conflicts.is_empty() {
            bail!(
                "{} conflicting files, use --adopt to back them up or --force to replace them",
                conflicts.len()
            );
        }
        if plan.removals() > MASS_REMOVE && !cli.allow_mass_remove {
            bail!(
                "the plan removes {} packages, use --allow-mass-remove to proceed",
                plan.removals()
            );
        }
        if cli.dry_run {
            println!("Dry run, nothing changed");
            return Ok(());
        }
        if !cli.yes && !confirm("Apply these changes?")? {
            println!("Aborted, nothing changed");
            return Ok(());
        }
        if plan.needs_root(&self.ctx) {
            sudo_validate()?;
        }

        files::execute(&self.ctx, &run, &plan.files)?;
        for p in &plan.packages {
            // Mark before removal, so recursive removal keeps declared dependencies
            if !p.mark_explicit.is_empty() {
                p.provider.mark_explicit(&run, &p.mark_explicit)?;
            }
            if !p.install.is_empty() {
                p.provider.install(&run, &p.install)?;
            }
            if !p.mark_dependency.is_empty() {
                p.provider.mark_dependency(&run, &p.mark_dependency)?;
            }
            if !p.remove.is_empty() {
                p.provider.remove(&run, &p.remove)?;
            }
        }
        plan.next_state.save(&state_path)?;
        println!("{}", style::green("Applied"));
        Ok(())
    }

    fn cmd_import(&self) -> Result<()> {
        let run = Runner::default();
        for m in &self.managed {
            match m.provider.explicit(&run) {
                Ok(names) if names.is_empty() => {}
                Ok(names) => {
                    println!("system.pkgs({}, &[", m.provider.type_name());
                    for name in names {
                        println!("    \"{name}\",");
                    }
                    println!("]);");
                }
                Err(e) => eprintln!("// {}: {e}", m.provider.name()),
            }
        }
        Ok(())
    }

    fn cmd_search(&self, query: &str) -> Result<()> {
        let run = Runner::default();
        let mut root = Node::new(style::bold(&format!("search results for {query}")));
        for m in &self.managed {
            let name = m.provider.name();
            match m.provider.search(&run, query) {
                Ok(Some(results)) => {
                    let mut node = Node::new(style::bold(name));
                    if results.is_empty() {
                        node.push(Node::new(style::dim("no matches")));
                    }
                    for r in results {
                        let mut item = Node::new(format!("{} {}", r.name, style::dim(&r.version)));
                        if !r.description.is_empty() {
                            item.push(Node::new(style::dim(&r.description)));
                        }
                        node.push(item);
                    }
                    root.push(node);
                }
                Ok(None) => eprintln!(
                    "{}",
                    style::yellow(&format!("warning: {name} cannot search, skipped"))
                ),
                Err(e) => eprintln!(
                    "{}",
                    style::yellow(&format!("warning: {name} search failed: {e}"))
                ),
            }
        }
        print!("{}", root.render());
        Ok(())
    }

    fn ensure_not_root(&self) -> Result<()> {
        if !self.ctx.cli.allow_root && std::fs::metadata("/proc/self")?.uid() == 0 {
            bail!(
                "runix runs as your user and asks sudo when needed, use --allow-root to override"
            );
        }
        Ok(())
    }
}

fn sudo_validate() -> Result<()> {
    Runner::default().run(&Cmd::new("sudo").arg("-v"))
}

fn confirm(question: &str) -> Result<bool> {
    print!("{question} [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{Cargo, Pacman};

    fn system(providers: &[&'static dyn Provider]) -> Result<System> {
        let cli = Cli::parse_from(["runix", "--root", env!("CARGO_MANIFEST_DIR")]);
        System::from_cli(cli, providers)
    }

    #[test]
    #[should_panic(expected = "provider cargo is not registered")]
    fn unregistered_provider_panics() {
        system(&[&Pacman]).unwrap().pkgs(Cargo, &["ripgrep"]);
    }

    #[test]
    fn duplicate_provider_fails() {
        assert!(system(&[&Pacman, &Pacman]).is_err());
    }
}
