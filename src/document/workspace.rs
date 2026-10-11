//! Allocation-prepared text/history leases. No filesystem or active-editor authority.
//! The App controller publishes the complete cohort only after every lease succeeds.
use super::*;
use anyhow::ensure;
use std::{
    mem::size_of,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) const MAX_WORKSPACE_EDITS: usize = 4096;
pub(crate) const MAX_WORKSPACE_METADATA: usize = 8 * 1024 * 1024;
const MAX_VIEWS: usize = 5;
const MAX_SELECTIONS: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceHistoryDirection {
    Undo,
    Redo,
}
#[derive(Clone, Debug)]
pub(crate) struct WorkspaceHistoryMember {
    pub document: u64,
    pub before: u64,
    pub after: u64,
}
#[derive(Debug)]
pub(crate) struct WorkspaceHistoryGroup {
    pub id: u64,
    pub members: Vec<WorkspaceHistoryMember>,
}
impl WorkspaceHistoryGroup {
    pub(crate) fn new(id: u64, prepared: &[WorkspaceTextPrepared]) -> Result<Arc<Self>> {
        ensure!(
            id != 0 && prepared.len() <= 128,
            "Invalid workspace history cohort"
        );
        let mut members = Vec::new();
        members
            .try_reserve_exact(prepared.len())
            .context("Workspace history allocation refused")?;
        for text in prepared.iter().filter(|text| !text.changes.is_empty()) {
            ensure!(
                !members
                    .iter()
                    .any(|member: &WorkspaceHistoryMember| member.document == text.source.document),
                "Duplicate workspace history member"
            );
            members.push(WorkspaceHistoryMember {
                document: text.source.document,
                before: text.source.revision,
                after: text.source.next_revision,
            });
        }
        ensure!(!members.is_empty(), "Empty workspace history group");
        Ok(Arc::new(Self { id, members }))
    }
}

/// Source-only snapshot; view navigation and accepted display publications do not
/// invalidate it. Baseline publication, path/source edits and history ABA do.
#[derive(Clone)]
pub(crate) struct WorkspaceSource {
    document: u64,
    text: Rope,
    path: Option<PathBuf>,
    eol: String,
    revision: u64,
    epoch: u64,
    saved_revision: u64,
    save_generation: u64,
    next_revision: u64,
    undo: Option<Arc<()>>,
    redo: Option<Arc<()>>,
    undo_len: usize,
    redo_len: usize,
}
impl WorkspaceSource {
    pub(crate) fn document(&self) -> u64 {
        self.document
    }
    pub(crate) fn text(&self) -> &Rope {
        &self.text
    }
    pub(crate) fn eol(&self) -> &str {
        &self.eol
    }
    pub(crate) fn current(&self, doc: &Document) -> bool {
        self.document == doc.id
            && self.path == doc.path
            && self.eol == doc.eol
            && self.revision == doc.revision
            && self.epoch == doc.text_epoch
            && self.saved_revision == doc.saved_revision
            && self.save_generation == doc.save_generation
            && self.next_revision == doc.next_revision
            && self.undo_len == doc.undo.len()
            && self.redo_len == doc.redo.len()
            && same_head(
                self.undo.as_ref(),
                doc.undo.last().map(|s| &s.history_identity),
            )
            && same_head(
                self.redo.as_ref(),
                doc.redo.last().map(|s| &s.history_identity),
            )
    }
    /// Worker-only Rope preparation. All ranges are scalar/disjoint/coalesced.
    pub(crate) fn prepare(
        self,
        changes: Vec<(Range<usize>, String)>,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<WorkspaceTextPrepared> {
        guard(cancel, deadline)?;
        ensure!(
            changes.len() <= MAX_WORKSPACE_EDITS,
            "Workspace text exceeds 4096 edits"
        );
        ensure!(
            changes
                .iter()
                .all(|(r, _)| r.start <= r.end && r.end <= self.text.len_chars())
                && changes
                    .windows(2)
                    .all(|pair| pair[0].0.end <= pair[1].0.start
                        && pair[0].0.start != pair[1].0.start),
            "Unvalidated workspace scalar edits"
        );
        let mut changed = Vec::new();
        changed
            .try_reserve_exact(changes.len())
            .context("Workspace edit allocation refused")?;
        for (range, text) in changes {
            guard(cancel, deadline)?;
            let old = self.text.slice(range.clone());
            if old.len_bytes() != text.len() || !old.chunks().flat_map(str::bytes).eq(text.bytes())
            {
                changed.push((range, text));
            }
        }
        if changed.is_empty() {
            return Ok(WorkspaceTextPrepared {
                text: self.text.clone(),
                source: self,
                changes: Vec::new(),
            });
        }
        self.next_revision
            .checked_add(1)
            .context("Workspace revision identity exhausted")?;
        self.epoch
            .checked_add(changed.len() as u64)
            .context("Workspace text epoch exhausted")?;
        let removed = changed.iter().try_fold(0usize, |n, (r, _)| {
            n.checked_add(self.text.slice(r.clone()).len_bytes())
                .context("Workspace removed bytes overflow")
        })?;
        let added = changed.iter().try_fold(0usize, |n, (_, s)| {
            n.checked_add(s.len())
                .context("Workspace added bytes overflow")
        })?;
        let size = self
            .text
            .len_bytes()
            .checked_sub(removed)
            .and_then(|n| n.checked_add(added))
            .context("Workspace result overflow")?;
        ensure!(
            size as u64 <= MAX_FILE_BYTES,
            "Workspace result exceeds 32 MiB"
        );
        let mut text = self.text.clone();
        let mut positions = Vec::new();
        positions
            .try_reserve_exact(changed.len())
            .context("Workspace position allocation refused")?;
        for (range, replacement) in changed.into_iter().rev() {
            guard(cancel, deadline)?;
            let change = PositionChange {
                bytes: ByteChange {
                    range: text.char_to_byte(range.start)..text.char_to_byte(range.end),
                    added: replacement.len(),
                },
                added: replacement.chars().count(),
                range: range.clone(),
            };
            text.remove(range.clone());
            text.insert(range.start, &replacement);
            positions.push(change);
        }
        guard(cancel, deadline)?;
        Ok(WorkspaceTextPrepared {
            source: self,
            text,
            changes: positions,
        })
    }
}
fn same_head(a: Option<&Arc<()>>, b: Option<&Arc<()>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}
pub(crate) fn guard(cancel: &AtomicBool, deadline: Instant) -> Result<()> {
    ensure!(
        !cancel.load(Ordering::Acquire) && Instant::now() < deadline,
        "Workspace preparation cancelled or expired"
    );
    Ok(())
}
pub(crate) struct WorkspaceTextPrepared {
    source: WorkspaceSource,
    text: Rope,
    // Actual application order, from right to left; each byte range refers to
    // that intermediate source, exactly as ordinary native edit history does.
    changes: Vec<PositionChange>,
}
impl WorkspaceTextPrepared {
    pub(crate) fn document(&self) -> u64 {
        self.source.document
    }
    pub(crate) fn changed(&self) -> bool {
        !self.changes.is_empty()
    }
    pub(crate) fn bytes(&self) -> usize {
        self.text.len_bytes()
    }
    pub(crate) fn current(&self, doc: &Document) -> bool {
        self.source.current(doc)
    }
}

struct Body {
    text: Rope,
    view: ViewState,
    other_views: std::collections::HashMap<u64, ViewState>,
    revision: u64,
    next_revision: u64,
    epoch: u64,
    bytes: std::collections::VecDeque<(u64, ByteChange)>,
    folding: std::collections::VecDeque<folding::Change>,
}
enum HistoryAction {
    Edit(Snapshot),
    Move {
        direction: WorkspaceHistoryDirection,
        reverse: Snapshot,
    },
}
/// Exclusive lifetime prevents another mutation between preparation and publish.
pub(crate) struct WorkspaceCommit<'a> {
    document: &'a mut Document,
    body: Body,
    history: HistoryAction,
    metadata: usize,
}
impl WorkspaceCommit<'_> {
    pub(crate) fn metadata_bytes(&self) -> usize {
        self.metadata
    }
    /// All fallible validation/reservation happens before this infallible move.
    pub(crate) fn publish(self) {
        let doc = self.document;
        match self.history {
            HistoryAction::Edit(snapshot) => {
                doc.undo.push(snapshot);
                if doc.undo.len() > HISTORY_LIMIT {
                    doc.undo.remove(0);
                }
                doc.redo.clear();
            }
            HistoryAction::Move {
                direction: WorkspaceHistoryDirection::Undo,
                reverse,
            } => {
                doc.undo.pop();
                doc.redo.push(reverse);
            }
            HistoryAction::Move {
                direction: WorkspaceHistoryDirection::Redo,
                reverse,
            } => {
                doc.redo.pop();
                doc.undo.push(reverse);
                if doc.undo.len() > HISTORY_LIMIT {
                    doc.undo.remove(0);
                }
            }
        }
        doc.text = self.body.text;
        doc.view = self.body.view;
        doc.other_views = self.body.other_views;
        doc.revision = self.body.revision;
        doc.next_revision = self.body.next_revision;
        doc.text_epoch = self.body.epoch;
        doc.byte_changes = self.body.bytes;
        doc.folding_changes = self.body.folding;
        doc.typing = None;
        // SaveSnapshot/baseline/path/encoding authority is deliberately retained.
        // Syntax and typing caches qualify their entries by the changed epoch.
    }
}
impl Document {
    pub(crate) fn capture_workspace_source(&self) -> Result<WorkspaceSource> {
        ensure!(
            self.text.len_bytes() as u64 <= MAX_FILE_BYTES
                && matches!(self.eol.as_str(), "\n" | "\r\n"),
            "Unsupported workspace source"
        );
        if let Some(path) = &self.path {
            ensure!(
                path.as_os_str().len() <= 4096,
                "Workspace path exceeds 4 KiB"
            );
        }
        Ok(WorkspaceSource {
            document: self.id,
            text: self.text.clone(),
            path: self.path.clone(),
            eol: self.eol.clone(),
            revision: self.revision,
            epoch: self.text_epoch,
            saved_revision: self.saved_revision,
            save_generation: self.save_generation,
            next_revision: self.next_revision,
            undo: self.undo.last().map(|s| s.history_identity.clone()),
            redo: self.redo.last().map(|s| s.history_identity.clone()),
            undo_len: self.undo.len(),
            redo_len: self.redo.len(),
        })
    }
    pub(crate) fn workspace_history(
        &self,
        direction: WorkspaceHistoryDirection,
    ) -> Option<Arc<WorkspaceHistoryGroup>> {
        let head = match direction {
            WorkspaceHistoryDirection::Undo => self.undo.last(),
            WorkspaceHistoryDirection::Redo => self.redo.last(),
        }?;
        head.workspace_group.clone()
    }
    pub(crate) fn workspace_history_current(
        &self,
        group: &Arc<WorkspaceHistoryGroup>,
        direction: WorkspaceHistoryDirection,
    ) -> bool {
        let Some(member) = group
            .members
            .iter()
            .find(|member| member.document == self.id)
        else {
            return false;
        };
        self.revision
            == match direction {
                WorkspaceHistoryDirection::Undo => member.after,
                WorkspaceHistoryDirection::Redo => member.before,
            }
            && self
                .workspace_history(direction)
                .is_some_and(|current| Arc::ptr_eq(&current, group))
    }
    pub(crate) fn workspace_history_bytes(
        &self,
        direction: WorkspaceHistoryDirection,
    ) -> Result<usize> {
        let snapshot = match direction {
            WorkspaceHistoryDirection::Undo => self.undo.last(),
            WorkspaceHistoryDirection::Redo => self.redo.last(),
        }
        .context("Workspace history disappeared")?;
        Ok(snapshot.text.len_bytes())
    }
    pub(crate) fn prepare_workspace_commit<'a>(
        &'a mut self,
        prepared: WorkspaceTextPrepared,
        group: Arc<WorkspaceHistoryGroup>,
        available: usize,
    ) -> Result<WorkspaceCommit<'a>> {
        ensure!(
            prepared.changed() && prepared.current(self),
            "Workspace source changed before publication"
        );
        ensure!(
            group.members.iter().any(|m| m.document == self.id
                && m.before == self.revision
                && m.after == self.next_revision),
            "Workspace history group is foreign"
        );
        self.next_revision
            .checked_add(1)
            .context("Workspace revision identity exhausted")?;
        self.text_epoch
            .checked_add(prepared.changes.len() as u64)
            .context("Workspace text epoch exhausted")?;
        let metadata = self.workspace_metadata(prepared.changes.len(), available)?;
        self.undo
            .try_reserve(1)
            .context("Workspace Undo reservation refused")?;
        let mut before = self.snapshot();
        before.workspace_group = Some(group);
        before.changes = prepared.changes.clone();
        let mut fork = self.workspace_fork();
        // A transaction changes no selection ordinal or direction. Each live
        // view is mapped from its current coordinates, even after cursor motion.
        for change in &prepared.changes {
            fork.record_byte_change(change.bytes.clone());
            fork.record_folding_change(change.range.clone(), change.added);
            change.map(&mut fork.view);
            for view in fork.other_views.values_mut() {
                change.map(view);
            }
        }
        fork.text = prepared.text;
        fork.revision = self.next_revision;
        fork.next_revision = self.next_revision + 1;
        fork.observe_folding_selection();
        Ok(WorkspaceCommit {
            body: fork.workspace_body(),
            document: self,
            history: HistoryAction::Edit(before),
            metadata,
        })
    }
    pub(crate) fn prepare_workspace_history<'a>(
        &'a mut self,
        group: &Arc<WorkspaceHistoryGroup>,
        direction: WorkspaceHistoryDirection,
        available: usize,
    ) -> Result<WorkspaceCommit<'a>> {
        ensure!(
            self.workspace_history_current(group, direction),
            "Workspace history peer changed; undo its independent edit first"
        );
        let target = match direction {
            WorkspaceHistoryDirection::Undo => self.undo.last(),
            WorkspaceHistoryDirection::Redo => self.redo.last(),
        }
        .context("Workspace history disappeared")?;
        ensure!(
            target.changes.len() <= MAX_WORKSPACE_EDITS,
            "Workspace history edit count exceeds 4096"
        );
        self.text_epoch
            .checked_add(target.changes.len() as u64)
            .context("Workspace history epoch exhausted")?;
        let mut metadata = self.workspace_metadata(target.changes.len(), available)?;
        metadata = metadata
            .checked_add(snapshot_bytes(target)?)
            .context("Workspace history metadata overflow")?;
        ensure!(
            metadata <= available.min(MAX_WORKSPACE_METADATA),
            "Workspace history metadata exceeds budget"
        );
        let snapshot = target.clone();
        let reverse = self.reverse_snapshot(&snapshot);
        match direction {
            WorkspaceHistoryDirection::Undo => self.redo.try_reserve(1),
            WorkspaceHistoryDirection::Redo => self.undo.try_reserve(1),
        }
        .context("Workspace reverse-history reservation refused")?;
        let mut fork = self.workspace_fork();
        fork.restore(snapshot);
        Ok(WorkspaceCommit {
            body: fork.workspace_body(),
            document: self,
            history: HistoryAction::Move { direction, reverse },
            metadata,
        })
    }
    fn workspace_metadata(&self, count: usize, available: usize) -> Result<usize> {
        ensure!(
            count <= MAX_WORKSPACE_EDITS && self.other_views.len() < MAX_VIEWS,
            "Workspace view/edit count exceeds bounds"
        );
        let mut bytes = 4096usize
            + self.byte_changes.len() * std::mem::size_of::<(u64, ByteChange)>()
            + self.folding_changes.len() * std::mem::size_of::<folding::Change>();
        let mut mapping_units = 0usize;
        for view in std::iter::once(&self.view).chain(self.other_views.values()) {
            ensure!(
                view.secondary.len() < MAX_SELECTIONS && view.cursor_history.len() <= 128,
                "Workspace view selection metadata exceeds bounds"
            );
            let mut selections = view.secondary.len() + 1;
            for cohort in &view.cursor_history {
                ensure!(
                    cohort.len() <= MAX_SELECTIONS,
                    "Workspace cursor history cohort exceeds bounds"
                );
                selections = selections
                    .checked_add(cohort.len())
                    .context("Workspace selection count overflow")?;
            }
            // Three copies cover mapped live views, checkpoint, reverse state.
            mapping_units = mapping_units
                .checked_add(selections)
                .context("Workspace mapping count overflow")?;
            let payload = selections
                .checked_mul(size_of::<Selection>())
                .and_then(|n| n.checked_mul(3))
                .context("Workspace view payload overflow")?;
            bytes = bytes
                .checked_add(payload + view.cursor_history.len() * size_of::<Vec<Selection>>() * 3)
                .context("Workspace metadata overflow")?;
            if let Some(snippet) = &view.snippet {
                mapping_units = mapping_units
                    .checked_add(snippet.workspace_metadata()? / 8)
                    .context("Workspace mapping overflow")?;
                bytes = bytes
                    .checked_add(
                        snippet
                            .workspace_metadata()?
                            .checked_mul(3)
                            .context("Workspace snippet payload overflow")?,
                    )
                    .context("Workspace metadata overflow")?;
            }
            mapping_units = mapping_units
                .checked_add(view.pairs.workspace_metadata()? / 8)
                .context("Workspace mapping overflow")?;
            bytes = bytes
                .checked_add(
                    view.pairs
                        .workspace_metadata()?
                        .checked_mul(3)
                        .context("Workspace pairs payload overflow")?,
                )
                .context("Workspace metadata overflow")?;
            view.snippet_generation
                .checked_add(count as u64)
                .context("Workspace snippet generation exhausted")?;
        }
        bytes = bytes
            .checked_add(
                count
                    .checked_mul(size_of::<PositionChange>() * 4)
                    .context("Workspace position payload overflow")?,
            )
            .context("Workspace metadata overflow")?;
        ensure!(
            mapping_units
                .checked_mul(count)
                .is_some_and(|work| work <= 65_536),
            "Workspace live-view mapping exceeds bounded publication work"
        );
        // Journal rings and bounded HashMap/header slack are included above.
        ensure!(
            bytes <= available.min(MAX_WORKSPACE_METADATA),
            "Workspace metadata exceeds 8 MiB"
        );
        Ok(bytes)
    }
    fn workspace_fork(&self) -> Document {
        Document {
            id: self.id,
            text: self.text.clone(),
            view: self.view.clone(),
            active_view: self.active_view,
            other_views: self.other_views.clone(),
            path: self.path.clone(),
            revision: self.revision,
            saved_revision: self.saved_revision,
            disk_content: self.disk_content.clone(),
            eol: self.eol.clone(),
            tab_size: self.tab_size,
            insert_spaces: self.insert_spaces,
            line_numbers: self.line_numbers,
            undo: Vec::new(),
            redo: Vec::new(),
            next_revision: self.next_revision,
            text_epoch: self.text_epoch,
            save_generation: self.save_generation,
            byte_changes: self.byte_changes.clone(),
            folding_policy: self.folding_policy.clone(),
            folding_changes: self.folding_changes.clone(),
            typing: None,
            typing_context: typing::ContextCache::default(),
            language_configuration: self.language_configuration.clone(),
        }
    }
    fn workspace_body(self) -> Body {
        Body {
            text: self.text,
            view: self.view,
            other_views: self.other_views,
            revision: self.revision,
            next_revision: self.next_revision,
            epoch: self.text_epoch,
            bytes: self.byte_changes,
            folding: self.folding_changes,
        }
    }
}
fn snapshot_bytes(snapshot: &Snapshot) -> Result<usize> {
    ensure!(
        snapshot.secondary.len() < MAX_SELECTIONS && snapshot.changes.len() <= MAX_WORKSPACE_EDITS,
        "Workspace snapshot metadata exceeds bounds"
    );
    let mut bytes = snapshot.secondary.len() * size_of::<Selection>()
        + snapshot.changes.len() * size_of::<PositionChange>()
        + size_of::<Snapshot>();
    if let Some(after) = &snapshot.after_selections {
        ensure!(
            after.len() <= MAX_SELECTIONS,
            "Workspace endpoint selections exceed bounds"
        );
        bytes = bytes
            .checked_add(after.len() * size_of::<Selection>())
            .context("Workspace snapshot overflow")?;
    }
    if let Some(snippet) = &snapshot.snippet {
        bytes = bytes
            .checked_add(snippet.workspace_metadata()?)
            .context("Workspace snapshot overflow")?;
    }
    bytes = bytes
        .checked_add(snapshot.pairs.workspace_metadata()?)
        .context("Workspace snapshot overflow")?;
    Ok(bytes)
}

#[cfg(test)]
mod tests;
