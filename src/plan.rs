use crate::context::Context;
use crate::files::{self, Entry, FileAction};
use crate::providers::{Package, Provider};
use crate::resources::Resource;
use crate::runner::Runner;
use crate::state::{State, StateEntry};
use crate::style;
use crate::tree::Node;
use anyhow::{Result, bail};
use std::collections::BTreeSet;
use std::path::Path;

/// Package changes for one provider
pub struct PackagePlan<'a> {
    pub provider: &'a dyn Provider,
    pub declared: Vec<Package>,
    pub install: Vec<Package>,
    pub mark_explicit: Vec<String>,
    pub mark_dependency: Vec<String>,
    pub remove: Vec<String>,
}

impl PackagePlan<'_> {
    /// Reports whether the provider needs no change
    pub fn is_empty(&self) -> bool {
        self.install.is_empty()
            && self.mark_explicit.is_empty()
            && self.mark_dependency.is_empty()
            && self.remove.is_empty()
    }
}

/// Item changes for one resource
pub struct ResourcePlan<'a, R: Resource> {
    pub resource: &'a R,
    pub declared: Vec<R::Item>,
    pub add: Vec<R::Item>,
    pub remove: Vec<R::Item>,
}

/// Item changes for a resource of any kind
pub trait ResourceChanges {
    /// Returns the name of the resource
    fn name(&self) -> &'static str;

    /// Returns the labels of the items to add
    fn added(&self) -> Vec<String>;

    /// Returns the labels of the items to remove
    fn removed(&self) -> Vec<String>;

    /// Reports whether the resource needs no change
    fn is_empty(&self) -> bool;

    /// Reports whether changes require root
    fn needs_root(&self) -> bool;

    /// Adds and removes the planned items
    fn execute(&self, run: &Runner) -> Result<()>;

    /// Runs the resource apply step for the declared items
    fn apply(&self, run: &Runner) -> Result<()>;
}

impl<R: Resource> ResourceChanges for ResourcePlan<'_, R> {
    fn name(&self) -> &'static str {
        self.resource.name()
    }

    fn added(&self) -> Vec<String> {
        self.add.iter().map(ToString::to_string).collect()
    }

    fn removed(&self) -> Vec<String> {
        self.remove.iter().map(ToString::to_string).collect()
    }

    fn is_empty(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty()
    }

    fn needs_root(&self) -> bool {
        self.resource.needs_root()
    }

    fn execute(&self, run: &Runner) -> Result<()> {
        if !self.add.is_empty() {
            self.resource.add(run, &self.add)?;
        }
        if !self.remove.is_empty() {
            self.resource.remove(run, &self.remove)?;
        }
        Ok(())
    }

    fn apply(&self, run: &Runner) -> Result<()> {
        self.resource.apply(run, &self.declared)
    }
}

/// Changes that `apply` makes
pub struct Plan<'a> {
    pub packages: Vec<PackagePlan<'a>>,
    pub files: Vec<FileAction>,
    pub resources: Vec<Box<dyn ResourceChanges + 'a>>,
    /// State saved after a successful `apply`
    pub next_state: State,
}

impl Plan<'_> {
    /// Reports whether the system already matches the config
    pub fn is_empty(&self) -> bool {
        self.packages.iter().all(PackagePlan::is_empty)
            && self.files.is_empty()
            && self.resources.iter().all(|r| r.is_empty())
    }

    /// Returns the file actions that block the plan
    pub fn conflicts(&self) -> Vec<&FileAction> {
        self.files
            .iter()
            .filter(|a| matches!(a, FileAction::Conflict { .. }))
            .collect()
    }

    /// Returns the number of packages that leave the explicit set
    pub fn removals(&self) -> usize {
        self.packages
            .iter()
            .map(|p| p.remove.len() + p.mark_dependency.len())
            .sum()
    }

    /// Reports whether any change requires root
    pub fn needs_root(&self, ctx: &Context) -> bool {
        let pkgs = self
            .packages
            .iter()
            .any(|p| !p.is_empty() && p.provider.needs_root());
        let files = self.files.iter().any(|a| match a {
            FileAction::Create(e) | FileAction::Update(e) => files::needs_root(ctx, &e.target),
            FileAction::Replace { entry, .. } | FileAction::Conflict { entry, .. } => {
                files::needs_root(ctx, &entry.target)
            }
            FileAction::Remove(s) => files::needs_root(ctx, &s.target),
            FileAction::Release { .. } => false,
        });
        let resources = self
            .resources
            .iter()
            .any(|r| !r.is_empty() && r.needs_root());
        pkgs || files || resources
    }

    /// Returns the plan as a display tree
    pub fn tree(&self, ctx: &Context) -> Node {
        let mut root = Node::new(style::bold(&format!("runix plan for {}", ctx.hostname)));
        for pkg in self.packages.iter().filter(|p| !p.is_empty()) {
            let mut node = Node::new(style::bold(pkg.provider.name()));
            for p in &pkg.install {
                node.push(Node::new(style::green(&format!("+ {}", p.spec))));
            }
            for name in &pkg.mark_explicit {
                node.push(Node::new(style::blue(&format!("* {name} (mark explicit)"))));
            }
            for name in &pkg.mark_dependency {
                node.push(Node::new(style::yellow(&format!(
                    "~ {name} (keep as dependency)"
                ))));
            }
            for name in &pkg.remove {
                node.push(Node::new(style::red(&format!("- {name}"))));
            }
            root.push(node);
        }
        if !self.files.is_empty() {
            let mut node = Node::new(style::bold("files"));
            for action in &self.files {
                node.push(Node::new(file_label(ctx, action)));
            }
            root.push(node);
        }
        for res in self.resources.iter().filter(|r| !r.is_empty()) {
            let mut node = Node::new(style::bold(res.name()));
            for item in res.added() {
                node.push(Node::new(style::green(&format!("+ {item}"))));
            }
            for item in res.removed() {
                node.push(Node::new(style::red(&format!("- {item}"))));
            }
            root.push(node);
        }
        root
    }

    /// Returns one line per area that differs from the config
    pub fn summary(&self) -> Vec<String> {
        let mut out = Vec::new();
        for p in self.packages.iter().filter(|p| !p.is_empty()) {
            out.push(format!(
                "{}: {} to install, {} to mark explicit, {} to remove",
                p.provider.name(),
                p.install.len(),
                p.mark_explicit.len(),
                p.remove.len() + p.mark_dependency.len(),
            ));
        }
        if !self.files.is_empty() {
            let count = |f: fn(&FileAction) -> bool| self.files.iter().filter(|a| f(a)).count();
            out.push(format!(
                "files: {} to place, {} to update, {} to remove, {} in conflict",
                count(|a| matches!(a, FileAction::Create(_) | FileAction::Replace { .. })),
                count(|a| matches!(a, FileAction::Update(_))),
                count(|a| matches!(a, FileAction::Remove(_))),
                count(|a| matches!(a, FileAction::Conflict { .. })),
            ));
        }
        for r in self.resources.iter().filter(|r| !r.is_empty()) {
            out.push(format!(
                "{}: {} to add, {} to remove",
                r.name(),
                r.added().len(),
                r.removed().len(),
            ));
        }
        out
    }
}

pub(crate) fn plan_packages<'a>(
    run: &Runner,
    provider: &'a dyn Provider,
    specs: &[String],
    ignored: &BTreeSet<String>,
    prune: bool,
) -> Result<PackagePlan<'a>> {
    let declared = provider.resolve(run, specs)?;
    let explicit = provider.explicit(run)?;
    let installed = provider.installed(run)?;

    let mut install = Vec::new();
    let mut mark_explicit = Vec::new();
    for pkg in &declared {
        if !installed.contains(&pkg.name) {
            install.push(pkg.clone());
        } else if !explicit.contains(&pkg.name) {
            mark_explicit.push(pkg.name.clone());
        }
    }

    let mut remove = Vec::new();
    let mut mark_dependency = Vec::new();
    if prune {
        let wanted: BTreeSet<&str> = declared.iter().map(|p| p.name.as_str()).collect();
        let removable = provider.removable(run)?;
        for name in explicit
            .iter()
            .filter(|n| !wanted.contains(n.as_str()) && !ignored.contains(*n))
        {
            if provider.protected().contains(&name.as_str()) {
                bail!(
                    "{}: refusing to remove protected package {name}, declare or ignore it",
                    provider.name()
                );
            }
            if removable.contains(name) {
                remove.push(name.clone());
            } else {
                mark_dependency.push(name.clone());
            }
        }
    }

    Ok(PackagePlan {
        provider,
        declared,
        install,
        mark_explicit,
        mark_dependency,
        remove,
    })
}

pub(crate) fn plan_resource<'a, R: Resource>(
    run: &Runner,
    resource: &'a R,
    declared: &BTreeSet<R::Item>,
    owned: &BTreeSet<R::Item>,
    prune: bool,
) -> Result<ResourcePlan<'a, R>> {
    let stale: Vec<R::Item> = if prune {
        owned.difference(declared).cloned().collect()
    } else {
        Vec::new()
    };
    let mut present = BTreeSet::new();
    for item in declared.iter().chain(&stale) {
        if resource
            .present(run, item)?
            .unwrap_or_else(|| owned.contains(item))
        {
            present.insert(item.clone());
        }
    }
    Ok(ResourcePlan {
        resource,
        declared: declared.iter().cloned().collect(),
        add: declared
            .iter()
            .filter(|i| !present.contains(*i))
            .cloned()
            .collect(),
        remove: stale.into_iter().filter(|i| present.contains(i)).collect(),
    })
}

pub(crate) fn next_state(ctx: &Context, entries: &[Entry], old: &State) -> State {
    let mut files: Vec<StateEntry> = entries.iter().map(Entry::record).collect();
    if ctx.cli.no_prune {
        let declared: BTreeSet<&Path> = entries.iter().map(|e| e.target.as_path()).collect();
        files.extend(
            old.files
                .iter()
                .filter(|f| !declared.contains(f.target.as_path()))
                .cloned(),
        );
    }
    State {
        version: old.version,
        files,
        resources: old.resources.clone(),
    }
}

fn file_label(ctx: &Context, action: &FileAction) -> String {
    let arrow = |e: &Entry| {
        let verb = if e.kind.is_link() { "->" } else { "<=" };
        let from = match &e.rendered {
            Some(r) => format!("{} (template)", source(ctx, &r.template)),
            None => source(ctx, &e.source),
        };
        format!("{} {verb} {from}", short(ctx, &e.target))
    };
    match action {
        FileAction::Create(e) => style::green(&format!("+ {}", arrow(e))),
        FileAction::Update(e) => style::blue(&format!("* {}", arrow(e))),
        FileAction::Replace {
            entry,
            reason,
            backup,
        } => {
            let how = if *backup {
                "back up and replace"
            } else {
                "replace"
            };
            style::yellow(&format!("! {} ({reason}, {how})", arrow(entry)))
        }
        FileAction::Conflict { entry, reason } => {
            style::magenta(&format!("x {} ({reason})", short(ctx, &entry.target)))
        }
        FileAction::Remove(s) => style::red(&format!("- {}", short(ctx, &s.target))),
        FileAction::Release { entry, reason } => style::dim(&format!(
            "? {} ({reason}, no longer managed)",
            short(ctx, &entry.target)
        )),
    }
}

fn short(ctx: &Context, path: &Path) -> String {
    match path.strip_prefix(&ctx.home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

fn source(ctx: &Context, path: &Path) -> String {
    path.strip_prefix(&ctx.root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    impl Provider for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn explicit(&self, _run: &Runner) -> Result<BTreeSet<String>> {
            Ok(set(&["keep", "old", "needed", "held"]))
        }
        fn installed(&self, _run: &Runner) -> Result<BTreeSet<String>> {
            Ok(set(&["keep", "old", "needed", "held", "dep"]))
        }
        fn removable(&self, _run: &Runner) -> Result<BTreeSet<String>> {
            Ok(set(&["keep", "old", "held"]))
        }
        fn install(&self, _run: &Runner, _pkgs: &[Package]) -> Result<()> {
            Ok(())
        }
        fn remove(&self, _run: &Runner, _names: &[String]) -> Result<()> {
            Ok(())
        }
        fn upgrade(&self, _run: &Runner, _pkgs: &[Package], _yes: bool) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn diffs_declared_against_installed() {
        let specs = ["keep", "dep", "new"].map(String::from);
        let plan = plan_packages(&Runner::default(), &Fake, &specs, &set(&["held"]), true).unwrap();
        assert_eq!(plan.install, [Package::new("new", "new")]);
        assert_eq!(plan.mark_explicit, ["dep"]);
        assert_eq!(plan.remove, ["old"]);
        assert_eq!(plan.mark_dependency, ["needed"]);
    }

    struct FakeResource;

    impl Resource for FakeResource {
        type Item = String;

        fn name(&self) -> &'static str {
            "fake"
        }
        fn present(&self, _run: &Runner, item: &String) -> Result<Option<bool>> {
            Ok(Some(set(&["kept", "old"]).contains(item)))
        }
        fn add(&self, _run: &Runner, _items: &[String]) -> Result<()> {
            Ok(())
        }
        fn remove(&self, _run: &Runner, _items: &[String]) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn adds_missing_and_removes_owned_leftovers() {
        let run = Runner::default();
        let declared = set(&["kept", "new"]);
        let owned = set(&["kept", "old", "gone"]);
        let plan = plan_resource(&run, &FakeResource, &declared, &owned, true).unwrap();
        assert_eq!(plan.add, ["new"]);
        assert_eq!(plan.remove, ["old"]);

        let plan = plan_resource(&run, &FakeResource, &declared, &owned, false).unwrap();
        assert!(plan.remove.is_empty());
    }

    struct StateResource;

    impl Resource for StateResource {
        type Item = String;

        fn name(&self) -> &'static str {
            "state"
        }
        fn add(&self, _run: &Runner, _items: &[String]) -> Result<()> {
            Ok(())
        }
        fn remove(&self, _run: &Runner, _items: &[String]) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn trusts_state_without_present() {
        let run = Runner::default();
        let declared = set(&["kept", "new"]);
        let owned = set(&["kept", "old"]);
        let plan = plan_resource(&run, &StateResource, &declared, &owned, true).unwrap();
        assert_eq!(plan.add, ["new"]);
        assert_eq!(plan.remove, ["old"]);
    }
}
