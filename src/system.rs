use crate::cli::{Cli, Command};
use crate::context::Context;
use crate::files::{self, FileAction, FileDecl, FileKind};
use crate::plan::{self, Plan, ResourceChanges};
use crate::providers::Provider;
use crate::resources::Resource;
use crate::runner::{Cmd, Runner};
use crate::state::State;
use crate::style;
use crate::tree::Node;
use anyhow::{Context as _, Result, bail};
use clap::Parser;
use serde_json::Value;
use similar::TextDiff;
use std::any::Any;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::io::{self, BufRead, Write};
use std::path::Path;

const MASS_REMOVE: usize = 20;

struct Managed {
    provider: &'static dyn Provider,
    specs: Vec<String>,
    ignored: BTreeSet<String>,
    used: bool,
}

struct Ensured<R: Resource> {
    resource: R,
    items: BTreeSet<R::Item>,
}

trait Ensure {
    fn name(&self) -> &'static str;

    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Returns the item changes and the items to record in the state
    fn plan<'a>(
        &'a self,
        run: &Runner,
        owned: &[Value],
        prune: bool,
    ) -> Result<(Box<dyn ResourceChanges + 'a>, Vec<Value>)>;
}

impl<R: Resource> Ensure for Ensured<R> {
    fn name(&self) -> &'static str {
        self.resource.name()
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn plan<'a>(
        &'a self,
        run: &Runner,
        owned: &[Value],
        prune: bool,
    ) -> Result<(Box<dyn ResourceChanges + 'a>, Vec<Value>)> {
        let owned = owned
            .iter()
            .map(|v| serde_json::from_value(v.clone()))
            .collect::<Result<BTreeSet<R::Item>, _>>()
            .with_context(|| format!("state for resource {} is invalid", self.name()))?;
        let plan = plan::plan_resource(run, &self.resource, &self.items, &owned, prune)?;
        let mut next = self.items.clone();
        if !prune {
            next.extend(owned);
        }
        let next = next
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<_, _>>()?;
        Ok((Box::new(plan), next))
    }
}

/// Declared state of the machine
pub struct System {
    ctx: Context,
    managed: Vec<Managed>,
    resources: Vec<Box<dyn Ensure>>,
    files: RefCell<Vec<FileDecl>>,
}

/// Template declaration that takes text replacements
pub struct Template<'a> {
    files: &'a RefCell<Vec<FileDecl>>,
    index: usize,
}

impl Template<'_> {
    /// Replaces every occurrence of `from` with `to` in the rendered output (panics if `from` is empty)
    pub fn replace(self, from: &str, to: impl Into<String>) -> Self {
        assert!(!from.is_empty(), "template replacement text is empty");
        self.files.borrow_mut()[self.index]
            .replacements
            .push((from.into(), to.into()));
        self
    }
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
                ignored: provider.ignored().iter().map(|p| p.to_string()).collect(),
                used: false,
            });
        }
        Ok(Self {
            ctx: Context::new(cli)?,
            managed,
            resources: Vec::new(),
            files: RefCell::new(Vec::new()),
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

    /// Registers a resource (panics if it is registered twice)
    pub fn manage<R: Resource>(&mut self, resource: R) -> &mut Self {
        if self.has_resource(&resource) {
            panic!("resource {} is registered twice", resource.name());
        }
        self.resources.push(Box::new(Ensured {
            resource,
            items: BTreeSet::new(),
        }));
        self
    }

    /// Declares items of a registered resource (panics if unregistered)
    pub fn ensure<R, I>(&mut self, resource: R, items: &[I]) -> &mut Self
    where
        R: Resource,
        I: Clone + Into<R::Item>,
    {
        let name = resource.name();
        let Some(ensured) = self.resources.iter_mut().find(|r| r.name() == name) else {
            panic!("resource {name} is not registered, add it with System::manage");
        };
        let Some(ensured) = ensured.as_any_mut().downcast_mut::<Ensured<R>>() else {
            panic!("resource {name} is registered with a different type");
        };
        ensured.items.extend(items.iter().cloned().map(Into::into));
        self
    }

    /// Reports whether a resource is registered
    pub fn has_resource<R: Resource>(&self, resource: &R) -> bool {
        self.resources.iter().any(|r| r.name() == resource.name())
    }

    /// Reports whether a provider is registered
    pub fn has_provider<P: Provider>(&self, provider: P) -> bool {
        self.managed
            .iter()
            .any(|m| m.provider.name() == provider.name())
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

    /// Symlinks the rendered output of every file under source to the same place under target
    pub fn link_template(&self, source: &str, target: &str) -> Template<'_> {
        self.template(FileKind::LinkTemplate, source, target)
    }

    /// Copies the rendered output of every file under source to the same place under target
    pub fn clone_template(&self, source: &str, target: &str) -> Template<'_> {
        self.template(FileKind::CopyTemplate, source, target)
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
        let entries = files::expand(&self.ctx, &self.files.borrow())?;
        let files = files::plan(&self.ctx, &entries, &state)?;
        let mut next_state = plan::next_state(&self.ctx, &entries, &state);

        let mut resources = Vec::new();
        for r in &self.resources {
            let name = r.name();
            let owned = state.resources.get(name).map_or(&[][..], Vec::as_slice);
            let (changes, next) = r.plan(run, owned, prune)?;
            resources.push(changes);
            if next.is_empty() {
                next_state.resources.remove(name);
            } else {
                next_state.resources.insert(name.into(), next);
            }
        }
        for name in state.resources.keys() {
            if !self.resources.iter().any(|r| r.name() == name) {
                eprintln!(
                    "{}",
                    style::yellow(&format!(
                        "warning: resource {name} has items in the state but is not registered"
                    ))
                );
            }
        }

        Ok(Plan {
            packages,
            files,
            resources,
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
            Command::UpdateTemplates => self.cmd_update_templates(),
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
        self.files.get_mut().push(FileDecl {
            kind,
            source: source.into(),
            target: target.into(),
            replacements: Vec::new(),
        });
        self
    }

    fn template(&self, kind: FileKind, source: &str, target: &str) -> Template<'_> {
        let mut files = self.files.borrow_mut();
        files.push(FileDecl {
            kind,
            source: source.into(),
            target: target.into(),
            replacements: Vec::new(),
        });
        Template {
            files: &self.files,
            index: files.len() - 1,
        }
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
            let current = match entry.kind {
                FileKind::Link => continue,
                FileKind::LinkTemplate => &entry.source,
                FileKind::Copy | FileKind::CopyTemplate => &entry.target,
            };
            let new = match &entry.rendered {
                Some(r) => Ok(String::from_utf8_lossy(&r.content).into_owned()),
                None => std::fs::read_to_string(&entry.source),
            };
            let (Ok(old), Ok(new)) = (std::fs::read_to_string(current), new) else {
                continue;
            };
            let diff = TextDiff::from_lines(&old, &new);
            let target = entry.target.display().to_string();
            let source = entry
                .rendered
                .as_ref()
                .map_or(&entry.source, |r| &r.template)
                .display()
                .to_string();
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
                for r in &plan.resources {
                    r.apply(&run)?;
                }
                plan.next_state.save(&state_path)?;
            }
            return Ok(());
        }
        print!("{}", plan.tree(&self.ctx).render());

        ensure_no_conflicts(&plan)?;
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
        for r in &plan.resources {
            r.execute(&run)?;
        }
        for r in &plan.resources {
            r.apply(&run)?;
        }
        plan.next_state.save(&state_path)?;
        println!("{}", style::green("Applied"));
        Ok(())
    }

    fn cmd_update_templates(&self) -> Result<()> {
        self.ensure_not_root()?;
        let state_path = State::path(&self.ctx);
        let state = State::load(&state_path)?;
        let entries = files::expand(&self.ctx, &self.files.borrow())?;

        let mut actions = Vec::new();
        let mut unplaced = 0;
        for entry in entries.iter().filter(|e| e.kind == FileKind::LinkTemplate) {
            match files::plan_entry(&self.ctx, entry, &state)? {
                Some(FileAction::Create(_)) => unplaced += 1,
                Some(action) => actions.push(action),
                None => {}
            }
        }
        if unplaced > 0 {
            println!(
                "{}",
                style::yellow(&format!(
                    "{unplaced} templates are not placed yet, run apply first"
                ))
            );
        }

        let mut next_state = state;
        for action in &actions {
            if let FileAction::Update(entry) | FileAction::Replace { entry, .. } = action {
                next_state.upsert(entry.record());
            }
        }
        let plan = Plan {
            packages: Vec::new(),
            files: actions,
            resources: Vec::new(),
            next_state,
        };
        if plan.is_empty() {
            println!("Templates are up to date");
            return Ok(());
        }
        print!("{}", plan.tree(&self.ctx).render());
        ensure_no_conflicts(&plan)?;
        if self.ctx.cli.dry_run {
            println!("Dry run, nothing changed");
            return Ok(());
        }

        files::execute(&self.ctx, &self.runner(), &plan.files)?;
        plan.next_state.save(&state_path)?;
        println!("{}", style::green("Updated templates"));
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
        if !self.ctx.cli.allow_root && !nix::unistd::Uid::current().is_root() {
            bail!(
                "runix runs as your user and asks sudo when needed, use --allow-root to override"
            );
        }
        Ok(())
    }
}

fn ensure_no_conflicts(plan: &Plan) -> Result<()> {
    let conflicts = plan.conflicts();
    if !conflicts.is_empty() {
        bail!(
            "{} conflicting files, use --adopt to back them up or --force to replace them",
            conflicts.len()
        );
    }
    Ok(())
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
    use crate::providers::{Cargo, Yay};
    use crate::resources::{Services, UserServices};

    fn system(providers: &[&'static dyn Provider]) -> Result<System> {
        let cli = Cli::parse_from(["runix", "--root", env!("CARGO_MANIFEST_DIR")]);
        System::from_cli(cli, providers)
    }

    #[test]
    #[should_panic(expected = "provider cargo is not registered")]
    fn unregistered_provider_panics() {
        system(&[&Yay]).unwrap().pkgs(Cargo, &["ripgrep"]);
    }

    #[test]
    #[should_panic(expected = "resource services is not registered")]
    fn unregistered_resource_panics() {
        system(&[]).unwrap().ensure(Services, &["sshd"]);
    }

    #[test]
    #[should_panic(expected = "resource user-services is registered twice")]
    fn duplicate_resource_panics() {
        system(&[])
            .unwrap()
            .manage(UserServices)
            .manage(UserServices);
    }

    #[test]
    fn duplicate_provider_fails() {
        assert!(system(&[&Yay, &Yay]).is_err());
    }
}
