//! Worker-only bounded resource preimages and durable recovery authority.
//!
//! This foundation preserves the exact ordered create/move/delete input and raw
//! binary contents, including omitted option values. It does NOT execute file
//! operations, manufacture postimages, or implement public Undo/Redo. A joined
//! coordinator must later add authorized ordered mutations and durable outcome
//! checkpoints before this journal can be used for filesystem rollback.
//!
//! Payload symlinks, special nodes, Windows reparse points and non-UTF-8 paths
//! or directory names are explicitly refused. Parent aliases are admitted only
//! with an unchanged original canonical mapping and retained parent identity.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
mod capture;
mod filesystem;
mod staging;

pub use staging::{RecoveryAuthority, StagedJournal};

pub const MAX_BYTES: usize = crate::persistence::MAX_BYTES;
const MAX_PATH_BYTES: usize = 4096;

/// All reads, traversal and retention charge these bounds before allocation.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_operations: usize,
    pub max_resources: usize,
    pub max_nodes: usize,
    pub max_bytes: usize,
    pub max_manifest_bytes: usize,
    pub max_archives: usize,
    pub max_metadata_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_operations: 4096,
            max_resources: 128,
            max_nodes: 4096,
            max_bytes: MAX_BYTES,
            max_manifest_bytes: 2 * 1024 * 1024,
            max_archives: 128,
            max_metadata_bytes: 8 * 1024 * 1024,
        }
    }
}
impl Limits {
    fn validate(self) -> Result<()> {
        ensure!(
            self.max_operations > 0
                && self.max_operations <= 4096
                && self.max_resources > 0
                && self.max_resources <= 128
                && self.max_nodes > 0
                && self.max_nodes <= 4096
                && self.max_bytes > 0
                && self.max_bytes <= MAX_BYTES
                && self.max_manifest_bytes > 0
                && self.max_manifest_bytes <= 2 * 1024 * 1024
                && self.max_archives <= 128
                && self.max_metadata_bytes <= 8 * 1024 * 1024,
            "Invalid resource journal bounds"
        );
        Ok(())
    }
}
/// The owning joined worker supplies its ORIGINAL cancellation/deadline values.
/// This module never spawns a thread or renews an admission clock.
pub struct Control<'a> {
    canceled: &'a AtomicBool,
    deadline: Instant,
}
impl<'a> Control<'a> {
    pub fn new(canceled: &'a AtomicBool, deadline: Instant) -> Self {
        Self { canceled, deadline }
    }
    pub fn check(&self) -> Result<()> {
        ensure!(
            !self.canceled.load(Ordering::Acquire) && Instant::now() < self.deadline,
            "Resource journal canceled or expired"
        );
        Ok(())
    }
}

/// Source option presence, including explicit null. Null is present for the
/// overwrite-undefined/ignoreIfExists decision, even though it is not truthy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Presence<T> {
    #[default]
    Absent,
    Null,
    Value(T),
}
impl<T> Presence<T> {
    pub fn is_present(&self) -> bool {
        !matches!(self, Self::Absent)
    }
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            _ => None,
        }
    }
}
/// Every option is preserved; staging never applies or silently ignores them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileOptions {
    pub overwrite: Presence<bool>,
    pub ignore_if_exists: Presence<bool>,
    pub ignore_if_not_exists: Presence<bool>,
    pub recursive: Presence<bool>,
    pub copy: Presence<bool>,
    pub folder: Presence<bool>,
    pub skip_trash_bin: Presence<bool>,
    pub max_size: Presence<f64>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    Create {
        path: PathBuf,
        options: FileOptions,
        contents: Option<Vec<u8>>,
    },
    Move {
        source: PathBuf,
        destination: PathBuf,
        options: FileOptions,
    },
    Delete {
        path: PathBuf,
        options: FileOptions,
    },
}
impl Operation {
    pub fn paths(&self) -> Vec<&Path> {
        match self {
            Self::Create { path, .. } | Self::Delete { path, .. } => vec![path],
            Self::Move {
                source,
                destination,
                ..
            } => vec![source, destination],
        }
    }
    pub fn options(&self) -> &FileOptions {
        match self {
            Self::Create { options, .. }
            | Self::Move { options, .. }
            | Self::Delete { options, .. } => options,
        }
    }
}
/// App supplies original retained model proof and archive serialization. The
/// journal never replaces native document identity with a synthetic wire ID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelProof {
    pub document: u64,
    pub revision: u64,
    pub text_epoch: u64,
    pub language_epoch: u64,
    pub saved_revision: u64,
    pub save_generation: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelArchive {
    pub proof: ModelProof,
    pub resource: PathBuf,
    pub text: Vec<u8>,
    pub metadata: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Identity {
    Unix { device: u64, inode: u64 },
    Windows { volume: u64, file_id: [u8; 16] },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    pub readonly: bool,
    pub unix_mode: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timestamp {
    pub seconds: i64,
    pub nanos: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum NodeKind {
    File,
    Directory,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeImage {
    pub path: PathBuf,
    pub kind: NodeKind,
    pub identity: Identity,
    pub permissions: Permissions,
    pub modified: Option<Timestamp>,
    pub bytes: usize,
    pub sha256: Option<[u8; 32]>,
    /// Exact direct child names, including ordinary dot/lock files. Symlinks,
    /// special nodes, invalid path encodings and reparse points are refused.
    pub children: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceImage {
    pub requested: PathBuf,
    pub canonical: PathBuf,
    pub ancestor_requested: PathBuf,
    pub ancestor: PathBuf,
    pub ancestor_identity: Identity,
    /// Original absent ancestors: future creation cannot renew this capture.
    pub missing_ancestors: Vec<PathBuf>,
    pub present: bool,
}
impl ResourceImage {
    fn metadata_bytes(&self) -> usize {
        self.requested.as_os_str().len()
            + self.canonical.as_os_str().len()
            + self.ancestor_requested.as_os_str().len()
            + self.ancestor.as_os_str().len()
            + 256
            + self
                .missing_ancestors
                .iter()
                .map(|value| value.as_os_str().len() + 24)
                .sum::<usize>()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionState {
    NotStarted,
}

/// Original bounded disk/parent proofs plus caller-owned model archives. Captures
/// overlapping trees once by canonical path, without rewriting operation order.
#[derive(Debug)]
pub struct Capture {
    operations: Vec<Operation>,
    resources: Vec<ResourceImage>,
    nodes: BTreeMap<PathBuf, NodeImage>,
    contents: BTreeMap<PathBuf, Vec<u8>>,
    archives: Vec<ModelArchive>,
    anchors: BTreeMap<PathBuf, std::fs::File>,
    handles: BTreeMap<PathBuf, std::fs::File>,
    limits: Limits,
}
impl Capture {
    pub fn prepare(
        operations: Vec<Operation>,
        archives: Vec<ModelArchive>,
        limits: Limits,
        control: &Control<'_>,
    ) -> Result<Self> {
        capture::prepare(operations, archives, limits, control)
    }
    pub fn operations(&self) -> &[Operation] {
        &self.operations
    }
    pub fn resources(&self) -> &[ResourceImage] {
        &self.resources
    }
    pub fn nodes(&self) -> impl Iterator<Item = &NodeImage> {
        self.nodes.values()
    }
    pub fn archives(&self) -> &[ModelArchive] {
        &self.archives
    }
    pub fn state(&self) -> ExecutionState {
        ExecutionState::NotStarted
    }
    pub fn file_contents(&self, canonical: &Path) -> Option<&[u8]> {
        self.contents.get(canonical).map(Vec::as_slice)
    }
    pub fn verify_originals(&self, control: &Control<'_>) -> Result<()> {
        capture::verify(self, control)
    }
}

fn path(path: &Path) -> Result<()> {
    crate::persistence::validate_path(path)?;
    ensure!(
        path.is_absolute() && path.to_str().is_some(),
        "Resource journal requires absolute UTF-8 paths"
    );
    ensure!(
        path.as_os_str().len() <= MAX_PATH_BYTES,
        "Resource journal path exceeds 4 KiB"
    );
    Ok(())
}
fn add(total: &mut usize, bytes: usize, limit: usize, description: &str) -> Result<()> {
    *total = total
        .checked_add(bytes)
        .context("Resource journal size overflow")?;
    ensure!(
        *total <= limit,
        "Resource journal {description} exceeds budget"
    );
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod worker_tests;
