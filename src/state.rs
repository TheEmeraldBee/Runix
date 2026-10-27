use crate::context::Context;
use crate::files::FileKind;
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const VERSION: u32 = 1;

/// File placed by runix
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateEntry {
    pub kind: FileKind,
    pub target: PathBuf,
    pub source: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

/// Files placed by the last apply
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    pub files: Vec<StateEntry>,
}

impl State {
    /// Returns the runix state directory
    pub fn dir(ctx: &Context) -> PathBuf {
        ctx.state.clone()
    }

    /// Returns the state file path
    pub fn path(ctx: &Context) -> PathBuf {
        Self::dir(ctx).join("state.json")
    }

    /// Returns the state at path (empty if missing)
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("state file {} is invalid", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("failed to read {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let dir = path.parent().context("state path has no parent")?;
        fs::create_dir_all(dir)?;
        let tmp = path.with_extension("json.tmp");
        let state = Self {
            version: VERSION,
            files: self.files.clone(),
        };
        fs::write(&tmp, serde_json::to_string_pretty(&state)?)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn find(&self, target: &Path) -> Option<&StateEntry> {
        self.files.iter().find(|f| f.target == target)
    }

    /// Records a file, replacing any record for the same target
    pub fn upsert(&mut self, entry: StateEntry) {
        match self.files.iter_mut().find(|f| f.target == entry.target) {
            Some(f) => *f = entry,
            None => self.files.push(entry),
        }
    }
}
