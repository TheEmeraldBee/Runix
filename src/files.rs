use crate::context::Context;
use crate::runner::{Cmd, Runner};
use crate::state::{State, StateEntry};
use anyhow::{Context as _, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

/// How a file reaches its target
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Link,
    Copy,
}

#[derive(Debug, Clone)]
pub(crate) struct FileDecl {
    pub kind: FileKind,
    pub source: String,
    pub target: String,
}

/// Declared file with resolved paths
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub kind: FileKind,
    pub source: PathBuf,
    pub target: PathBuf,
    /// Source hash (copies only)
    pub hash: Option<String>,
}

impl Entry {
    /// Returns the state entry for this file
    pub fn record(&self) -> StateEntry {
        StateEntry {
            kind: self.kind,
            target: self.target.clone(),
            source: self.source.clone(),
            hash: self.hash.clone(),
        }
    }
}

/// Change to a managed file
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileAction {
    /// Place a file at a missing target
    Create(Entry),
    /// Refresh a file that runix placed
    Update(Entry),
    /// Replace a conflicting target, with an optional backup
    Replace {
        entry: Entry,
        reason: String,
        backup: bool,
    },
    /// Target that blocks the plan
    Conflict { entry: Entry, reason: String },
    /// Delete a file that left the config
    Remove(StateEntry),
    /// Unmanage a file that left the config but keep it on disk
    Release { entry: StateEntry, reason: String },
}

enum Check {
    Ok,
    Missing,
    Stale,
    Conflict { reason: String, hard: bool },
}

pub(crate) fn expand(ctx: &Context, decls: &[FileDecl]) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for decl in decls {
        let source = ctx
            .root
            .join(&decl.source)
            .canonicalize()
            .with_context(|| {
                format!("source {} not found in {}", decl.source, ctx.root.display())
            })?;
        if !source.starts_with(&ctx.root) {
            bail!("source {} is outside the project root", decl.source);
        }
        let target = expand_target(&decl.target)?;

        let mut pairs = Vec::new();
        if source.is_dir() {
            for item in WalkDir::new(&source).min_depth(1).sort_by_file_name() {
                let item = item?;
                if item.file_type().is_dir() {
                    continue;
                }
                let rel = item.path().strip_prefix(&source)?;
                pairs.push((item.path().to_path_buf(), target.join(rel)));
            }
        } else {
            pairs.push((source, target));
        }

        for (source, target) in pairs {
            if !seen.insert(target.clone()) {
                bail!("target {} is declared twice", target.display());
            }
            let hash = match decl.kind {
                FileKind::Copy => Some(hash(&source)?),
                FileKind::Link => None,
            };
            out.push(Entry {
                kind: decl.kind,
                source,
                target,
                hash,
            });
        }
    }
    Ok(out)
}

/// Returns target with ~ and environment variables expanded
pub(crate) fn expand_target(target: &str) -> Result<PathBuf> {
    let expanded = shellexpand::full(target)
        .map_err(|e| anyhow!("target {target}: variable {} is not set", e.var_name))?;
    let path = PathBuf::from(expanded.as_ref());
    if !path.is_absolute() {
        bail!("target {target} must be an absolute path");
    }
    Ok(path)
}

pub(crate) fn plan(ctx: &Context, entries: &[Entry], state: &State) -> Result<Vec<FileAction>> {
    let mut actions = Vec::new();
    for entry in entries {
        match check(entry, state)? {
            Check::Ok => {}
            Check::Missing => actions.push(FileAction::Create(entry.clone())),
            Check::Stale => actions.push(FileAction::Update(entry.clone())),
            Check::Conflict { reason, hard } if hard || !(ctx.cli.adopt || ctx.cli.force) => {
                actions.push(FileAction::Conflict {
                    entry: entry.clone(),
                    reason,
                })
            }
            Check::Conflict { reason, .. } => actions.push(FileAction::Replace {
                entry: entry.clone(),
                reason,
                backup: !ctx.cli.force,
            }),
        }
    }

    if !ctx.cli.no_prune {
        let declared: BTreeSet<&Path> = entries.iter().map(|e| e.target.as_path()).collect();
        for old in state
            .files
            .iter()
            .filter(|f| !declared.contains(f.target.as_path()))
        {
            if let Some(action) = prune(old) {
                actions.push(action);
            }
        }
    }
    Ok(actions)
}

fn check(entry: &Entry, state: &State) -> Result<Check> {
    let meta = match fs::symlink_metadata(&entry.target) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Check::Missing),
        Err(e) => {
            return Err(e).with_context(|| format!("failed to inspect {}", entry.target.display()));
        }
    };
    let owned = state.find(&entry.target).filter(|s| s.kind == entry.kind);
    let conflict = |reason: &str, hard: bool| {
        Ok(Check::Conflict {
            reason: reason.into(),
            hard,
        })
    };

    if meta.is_dir() {
        return conflict("is a directory", true);
    }
    match entry.kind {
        FileKind::Link => {
            if !meta.file_type().is_symlink() {
                return conflict("exists, not managed by runix", false);
            }
            let dest = fs::read_link(&entry.target)?;
            if dest == entry.source {
                Ok(Check::Ok)
            } else if owned.is_some_and(|s| s.source == dest) {
                Ok(Check::Stale)
            } else {
                conflict("links somewhere else", false)
            }
        }
        FileKind::Copy => {
            if !meta.is_file() {
                return conflict("is not a regular file", false);
            }
            let current = hash(&entry.target)?;
            if Some(&current) == entry.hash.as_ref() {
                Ok(Check::Ok)
            } else if owned.is_some_and(|s| s.hash.as_ref() == Some(&current)) {
                Ok(Check::Stale)
            } else if owned.is_some() {
                conflict("changed by hand", false)
            } else {
                conflict("exists, not managed by runix", false)
            }
        }
    }
}

fn prune(old: &StateEntry) -> Option<FileAction> {
    let release = |reason: &str| {
        Some(FileAction::Release {
            entry: old.clone(),
            reason: reason.into(),
        })
    };
    match old.kind {
        FileKind::Link => match fs::read_link(&old.target) {
            Ok(dest) if dest == old.source => Some(FileAction::Remove(old.clone())),
            Ok(_) => release("links somewhere else"),
            Err(_) if fs::symlink_metadata(&old.target).is_ok() => release("replaced by hand"),
            Err(_) => release("already gone"),
        },
        FileKind::Copy => match hash(&old.target) {
            Ok(h) if Some(&h) == old.hash.as_ref() => Some(FileAction::Remove(old.clone())),
            Ok(_) => release("changed by hand"),
            Err(_) => release("already gone"),
        },
    }
}

pub(crate) fn execute(ctx: &Context, run: &Runner, actions: &[FileAction]) -> Result<()> {
    let backups = State::dir(ctx).join("backups").join(timestamp());
    for action in actions {
        match action {
            FileAction::Create(entry) | FileAction::Update(entry) => place(ctx, run, entry)?,
            FileAction::Replace { entry, backup, .. } => {
                let root = needs_root(ctx, &entry.target);
                if *backup {
                    let dest =
                        backups.join(entry.target.strip_prefix("/").unwrap_or(&entry.target));
                    fs::create_dir_all(dest.parent().context("backup path has no parent")?)?;
                    if root {
                        run.run(
                            &Cmd::new("mv")
                                .arg(path_str(&entry.target))
                                .arg(path_str(&dest))
                                .sudo(true),
                        )?;
                    } else {
                        fs::rename(&entry.target, &dest)?;
                    }
                    println!("backed up {} to {}", entry.target.display(), dest.display());
                }
                place(ctx, run, entry)?;
            }
            FileAction::Remove(old) => {
                if needs_root(ctx, &old.target) {
                    run.run(
                        &Cmd::new("rm")
                            .arg("-f")
                            .arg(path_str(&old.target))
                            .sudo(true),
                    )?;
                } else {
                    fs::remove_file(&old.target)
                        .with_context(|| format!("failed to remove {}", old.target.display()))?;
                }
            }
            FileAction::Conflict { entry, reason } => {
                bail!("{} {reason}", entry.target.display())
            }
            FileAction::Release { .. } => {}
        }
    }
    Ok(())
}

fn place(ctx: &Context, run: &Runner, entry: &Entry) -> Result<()> {
    let parent = entry.target.parent().context("target has no parent")?;
    let (source, target) = (path_str(&entry.source), path_str(&entry.target));

    if needs_root(ctx, &entry.target) {
        run.run(&Cmd::new("mkdir").arg("-p").arg(path_str(parent)).sudo(true))?;
        let cmd = match entry.kind {
            FileKind::Link => Cmd::new("ln").args(["-sfn", &source, &target]),
            FileKind::Copy => Cmd::new("cp").args(["-f", &source, &target]),
        };
        return run.run(&cmd.sudo(true));
    }

    fs::create_dir_all(parent)?;
    if fs::symlink_metadata(&entry.target).is_ok() {
        fs::remove_file(&entry.target)?;
    }
    match entry.kind {
        FileKind::Link => symlink(&entry.source, &entry.target),
        FileKind::Copy => fs::copy(&entry.source, &entry.target).map(|_| ()),
    }
    .with_context(|| format!("failed to place {}", entry.target.display()))
}

pub(crate) fn needs_root(ctx: &Context, target: &Path) -> bool {
    !target.starts_with(&ctx.home)
}

/// Returns the hex SHA-256 of a file
pub(crate) fn hash(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use clap::Parser;

    fn setup() -> (tempfile::TempDir, Context) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let home = dir.path().join("home");
        fs::create_dir_all(root.join("dots/app")).unwrap();
        fs::create_dir_all(&home).unwrap();
        fs::write(root.join("dots/app/a.conf"), "a").unwrap();
        fs::write(root.join("dots/app/b.conf"), "b").unwrap();
        let cli = Cli::parse_from(["runix", "--root", root.to_str().unwrap()]);
        let mut ctx = Context::new(cli).unwrap();
        ctx.home = home.canonicalize().unwrap();
        (dir, ctx)
    }

    fn decl(ctx: &Context, kind: FileKind) -> Vec<FileDecl> {
        let target = ctx.home.join(".config/app").to_string_lossy().into_owned();
        vec![FileDecl {
            kind,
            source: "dots/app".into(),
            target,
        }]
    }

    fn apply(ctx: &Context, state: &State, kind: FileKind) -> (Vec<FileAction>, State) {
        let entries = expand(ctx, &decl(ctx, kind)).unwrap();
        let actions = plan(ctx, &entries, state).unwrap();
        execute(ctx, &Runner::default(), &actions).unwrap();
        (
            actions,
            State {
                version: 1,
                files: entries.iter().map(Entry::record).collect(),
            },
        )
    }

    #[test]
    fn links_each_file_then_settles() {
        let (_dir, ctx) = setup();
        let (actions, state) = apply(&ctx, &State::default(), FileKind::Link);
        assert_eq!(actions.len(), 2);
        let target = ctx.home.join(".config/app/a.conf");
        assert_eq!(
            fs::read_link(&target).unwrap(),
            ctx.root.join("dots/app/a.conf")
        );

        let entries = expand(&ctx, &decl(&ctx, FileKind::Link)).unwrap();
        assert!(plan(&ctx, &entries, &state).unwrap().is_empty());
    }

    #[test]
    fn unmanaged_target_conflicts() {
        let (_dir, ctx) = setup();
        fs::create_dir_all(ctx.home.join(".config/app")).unwrap();
        fs::write(ctx.home.join(".config/app/a.conf"), "mine").unwrap();
        let entries = expand(&ctx, &decl(&ctx, FileKind::Link)).unwrap();
        let actions = plan(&ctx, &entries, &State::default()).unwrap();
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, FileAction::Conflict { .. }))
        );
    }

    #[test]
    fn copy_updates_when_source_changes_and_detects_edits() {
        let (_dir, ctx) = setup();
        let (_, state) = apply(&ctx, &State::default(), FileKind::Copy);

        fs::write(ctx.root.join("dots/app/a.conf"), "a2").unwrap();
        let entries = expand(&ctx, &decl(&ctx, FileKind::Copy)).unwrap();
        let actions = plan(&ctx, &entries, &state).unwrap();
        assert!(matches!(actions.as_slice(), [FileAction::Update(_)]));

        fs::write(ctx.home.join(".config/app/b.conf"), "edited").unwrap();
        let actions = plan(&ctx, &entries, &state).unwrap();
        assert!(actions.iter().any(
            |a| matches!(a, FileAction::Conflict { reason, .. } if reason == "changed by hand")
        ));
    }

    #[test]
    fn prunes_removed_links() {
        let (_dir, ctx) = setup();
        let (_, state) = apply(&ctx, &State::default(), FileKind::Link);
        let actions = plan(&ctx, &[], &state).unwrap();
        assert_eq!(actions.len(), 2);
        execute(&ctx, &Runner::default(), &actions).unwrap();
        assert!(fs::symlink_metadata(ctx.home.join(".config/app/a.conf")).is_err());
    }
}
