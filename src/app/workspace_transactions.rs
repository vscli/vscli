//! Retained native text cohorts: one actual preparation, atomic publication,
//! and peer-validated grouped history. Optional-host framing remains separate.
use super::*;
use crate::{
    document::{
        WorkspaceHistoryDirection, WorkspaceHistoryGroup, WorkspaceSource, WorkspaceTextPrepared,
    },
    extensions::providers::MirrorSnapshot,
};
use anyhow::{Context, ensure};
use serde_json::Map;
use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const MAX_TARGETS: usize = 128;
const MAX_EDITS: usize = 4096;
const MAX_RAW: usize = 1024 * 1024;
const MAX_EXPANDED: usize = 2 * 1024 * 1024;
const MAX_DOCUMENTS: usize = 4 * 1024 * 1024;
const MAX_METADATA: usize = 8 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(5);

/// Issued by the native service AFTER trusted receipt admission. This flag must
/// be a fresh owner/session lifetime and may only transition false -> true.
/// The public wire has no lifetime flag and cannot manufacture this authority.
#[derive(Clone)]
pub(crate) struct Origin {
    pub session: u64,
    pub request: String,
    pub owner: String,
    retired: Arc<AtomicBool>,
}
impl Origin {
    pub(crate) fn new(
        session: u64,
        request: &str,
        owner: &str,
        retired: Arc<AtomicBool>,
    ) -> Result<Self> {
        ensure!(
            session != 0
                && !request.is_empty()
                && request.len() <= 128
                && !request.contains('\0')
                && !owner.is_empty()
                && owner.len() <= 256
                && !owner.contains('\0'),
            "Invalid workspace origin"
        );
        ensure!(
            !retired.load(Ordering::Acquire),
            "Workspace owner lifetime retired"
        );
        Ok(Self {
            session,
            request: request.into(),
            owner: owner.into(),
            retired,
        })
    }
    pub(crate) fn alive(&self) -> bool {
        !self.retired.load(Ordering::Acquire)
    }
}
#[derive(Clone, Copy)]
struct Position {
    line: usize,
    character: usize,
}
struct WireEdit {
    start: Position,
    end: Position,
    text: String,
}
struct Input {
    source: WorkspaceSource,
    edits: Vec<WireEdit>,
}
#[derive(Clone)]
pub(crate) struct CaptureContext {
    settings: Arc<Vec<Map<String, Value>>>,
    profile: u64,
    workspace: PathBuf,
    refactoring_auto_save: bool,
}
impl CaptureContext {
    pub(crate) fn new(
        settings: Arc<Vec<Map<String, Value>>>,
        profile: u64,
        workspace: PathBuf,
    ) -> Result<Self> {
        ensure!(
            workspace.is_absolute() && workspace.as_os_str().len() <= 4096,
            "Invalid workspace capture root"
        );
        let refactoring_auto_save = settings
            .iter()
            .rev()
            .find_map(|layer| layer.get("files.refactoring.autoSave"))
            .and_then(Value::as_bool)
            != Some(false);
        Ok(Self {
            settings,
            profile,
            workspace,
            refactoring_auto_save,
        })
    }
}
pub(crate) struct Capture {
    origin: Origin,
    inputs: Vec<Input>,
    context: CaptureContext,
    deadline: Instant,
}
struct Prepared {
    origin: Origin,
    items: Vec<WorkspaceTextPrepared>,
    context: CaptureContext,
    deadline: Instant,
}
pub(crate) struct Completion {
    pub token: u64,
    pub origin: Origin,
    pub applied: bool,
    pub reason: Option<String>,
}
struct Pending {
    token: u64,
    origin: Origin,
    deadline: Instant,
}
#[derive(Default)]
pub(crate) struct State {
    worker: Worker,
    pending: Option<Pending>,
    completed: Option<Completion>,
}
impl State {
    pub(crate) fn occupied(&self) -> bool {
        self.pending.is_some() || self.completed.is_some() || self.worker.active.is_some()
    }
}
struct Actual {
    token: u64,
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<std::result::Result<Prepared, String>>,
    thread: JoinHandle<()>,
}
#[derive(Default)]
struct Worker {
    next: u64,
    active: Option<Actual>,
    #[cfg(test)]
    gates: Option<Gates>,
}
impl Worker {
    fn start(&mut self, capture: Arc<Capture>) -> Result<u64> {
        ensure!(
            self.active.is_none(),
            "Workspace preparation is actually occupied"
        );
        let next = self
            .next
            .checked_add(1)
            .context("Workspace transaction identity exhausted")?;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, receiver) = mpsc::sync_channel(1);
        #[cfg(test)]
        let gates = self.gates.take();
        let thread = thread::Builder::new()
            .name("vscli-workspace-edit".into())
            .spawn(move || {
                #[cfg(test)]
                if let Some(gates) = &gates {
                    gates.wait(GatePoint::BeforePrepare);
                }
                let result =
                    prepare(capture, &flag).map_err(|error| bounded_reason(&format!("{error:#}")));
                let _ = tx.send(result);
                #[cfg(test)]
                if let Some(gates) = &gates {
                    gates.wait(GatePoint::AfterReply);
                }
            })
            .context("Cannot start workspace preparation")?;
        self.next = next;
        self.active = Some(Actual {
            token: next,
            cancel,
            receiver,
            thread,
        });
        Ok(next)
    }
    fn cancel(&mut self, token: u64) {
        if let Some(actual) = &self.active
            && actual.token == token
        {
            actual.cancel.store(true, Ordering::Release);
        }
    }
    fn poll(&mut self) -> Option<(u64, std::result::Result<Prepared, String>)> {
        if !self.active.as_ref()?.thread.is_finished() {
            return None;
        }
        let actual = self.active.take()?;
        let result = if actual.thread.join().is_err() {
            Err("Workspace worker panicked".into())
        } else if actual.cancel.load(Ordering::Acquire) {
            Err("Workspace transaction cancelled".into())
        } else {
            actual
                .receiver
                .try_recv()
                .unwrap_or_else(|_| Err("Workspace worker exited without result".into()))
        };
        Some((actual.token, result))
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(actual) = &self.active {
            actual.cancel.store(true, Ordering::Release);
        }
    }
}
fn bounded_reason(text: &str) -> String {
    let mut end = text.len().min(4096);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}
fn fields<'a>(value: &'a Value, names: &[&str], message: &str) -> Result<&'a Map<String, Value>> {
    let map = value.as_object().context(message.to_owned())?;
    ensure!(
        map.len() == names.len() && names.iter().all(|name| map.contains_key(*name)),
        "{message}"
    );
    Ok(map)
}
fn coordinate(value: &Value) -> Result<Position> {
    let value = fields(value, &["line", "character"], "Invalid workspace position")?;
    let read = |name: &str| -> Result<usize> {
        let value = value[name]
            .as_u64()
            .context("Invalid workspace coordinate")?;
        ensure!(
            value <= i32::MAX as u64,
            "Workspace coordinate exceeds protocol bounds"
        );
        Ok(value as usize)
    };
    Ok(Position {
        line: read("line")?,
        character: read("character")?,
    })
}
/// Caller supplies only target snapshots captured from the trusted native
/// mirror at receipt. No URI canonicalization/IO or whole-source scan here.
pub(crate) fn capture_workspace_transaction(
    documents: &[Document],
    hidden: &[Document],
    targets: &[MirrorSnapshot],
    origin: Origin,
    args: &Value,
    context: CaptureContext,
    now: Instant,
) -> Result<Arc<Capture>> {
    ensure!(
        origin.alive()
            && targets.len() <= MAX_TARGETS
            && documents
                .len()
                .checked_add(hidden.len())
                .is_some_and(|count| count <= MAX_TARGETS),
        "Workspace admission retired/over budget"
    );
    let args = fields(
        args,
        &["entries", "metadata"],
        "Unsupported workspace transaction fields",
    )?;
    let metadata = fields(
        &args["metadata"],
        &["isRefactoring"],
        "Unsupported workspace metadata",
    )?;
    let refactoring = metadata["isRefactoring"]
        .as_bool()
        .context("Invalid workspace refactoring flag")?;
    ensure!(
        !refactoring || !context.refactoring_auto_save,
        "Refactoring autosave requires a qualified save backend"
    );
    let entries = args["entries"]
        .as_array()
        .context("Workspace entries must be an array")?;
    ensure!(
        !entries.is_empty() && entries.len() <= MAX_TARGETS,
        "Workspace entries require 1–128 targets"
    );
    let mut inputs = Vec::new();
    inputs
        .try_reserve_exact(entries.len())
        .context("Workspace target reservation refused")?;
    let (mut count, mut raw) = (0usize, 0usize);
    for entry in entries {
        let entry = fields(
            entry,
            &["document", "version", "uri", "edits"],
            "Unsupported workspace entry",
        )?;
        let id = entry["document"]
            .as_u64()
            .context("Invalid workspace target identity")?;
        let version = entry["version"]
            .as_u64()
            .context("Invalid workspace version")?;
        ensure!(
            id != 0 && id < (1u64 << 53) && version != 0 && version < (1u64 << 53),
            "Workspace identity/version must be positive safe integers"
        );
        let uri = entry["uri"].as_str().context("Invalid workspace URI")?;
        ensure!(
            !uri.is_empty() && uri.len() <= 4096 && !uri.contains('\0'),
            "Workspace URI exceeds bounds"
        );
        ensure!(
            !inputs
                .iter()
                .any(|input: &Input| input.source.document() == id),
            "Duplicate workspace target group"
        );
        let mut matching = targets.iter().filter(|target| {
            target.document == id && target.uri == uri && target.version == version
        });
        let target = matching
            .next()
            .context("Workspace target is outside its captured mirrors")?;
        ensure!(
            matching.next().is_none(),
            "Ambiguous workspace target mirror"
        );
        let mut models = documents.iter().chain(hidden).filter(|doc| doc.id == id);
        let doc = models.next().context("Workspace target was closed")?;
        ensure!(!doc.is_read_only(), "Virtual documents are read-only");
        ensure!(
            models.next().is_none(),
            "Ambiguous retained workspace target"
        );
        ensure!(
            doc.path == target.path
                && doc.revision == target.revision
                && doc.text_epoch() == target.text_epoch,
            "Workspace source changed before queue admission"
        );
        let edits = entry["edits"]
            .as_array()
            .context("Workspace edits must be an array")?;
        count = count
            .checked_add(edits.len())
            .context("Workspace edit count overflow")?;
        ensure!(count <= MAX_EDITS, "Workspace exceeds 4096 edits");
        let mut staged = Vec::new();
        staged
            .try_reserve_exact(edits.len())
            .context("Workspace edit reservation refused")?;
        for edit in edits {
            let edit = fields(
                edit,
                &["range", "newText"],
                "Unsupported workspace TextEdit",
            )?;
            let range = fields(&edit["range"], &["start", "end"], "Invalid workspace range")?;
            let text = edit["newText"]
                .as_str()
                .context("Invalid workspace replacement")?;
            raw = raw
                .checked_add(text.len())
                .context("Workspace raw text overflow")?;
            ensure!(raw <= MAX_RAW, "Workspace exceeds 1 MiB raw text");
            staged.push(WireEdit {
                start: coordinate(&range["start"])?,
                end: coordinate(&range["end"])?,
                text: text.to_owned(),
            });
        }
        inputs.push(Input {
            source: doc.capture_workspace_source()?,
            edits: staged,
        });
    }
    ensure!(count != 0, "Empty workspace transaction");
    let bytes = documents.iter().chain(hidden).try_fold(0usize, |n, doc| {
        n.checked_add(doc.text.len_bytes())
            .context("Workspace model bytes overflow")
    })?;
    ensure!(bytes <= MAX_DOCUMENTS, "Workspace mirrors exceed 4 MiB");
    Ok(Arc::new(Capture {
        origin,
        inputs,
        context,
        deadline: now
            .checked_add(DEADLINE)
            .context("Workspace deadline overflow")?,
    }))
}
impl App {
    #[cfg(test)]
    pub(crate) fn capture_workspace_transaction(
        &self,
        origin: Origin,
        args: &Value,
        targets: &[MirrorSnapshot],
        now: Instant,
    ) -> Result<Arc<Capture>> {
        ensure!(
            self.settings_error.is_none(),
            "Workspace settings are unavailable"
        );
        let context = CaptureContext::new(
            self.settings.extension_layers().clone(),
            self.settings_profile_generation(),
            self.workspace.root.clone(),
        )?;
        capture_workspace_transaction(
            &self.documents,
            &self.hidden_documents,
            targets,
            origin,
            args,
            context,
            now,
        )
    }
    /// Dispatch retains the original capture/deadline, never resnapshots while
    /// waiting behind another service. A terminal receipt also occupies the lane.
    pub(crate) fn start_workspace_transaction(&mut self, capture: Arc<Capture>) -> Result<u64> {
        ensure!(
            !self.workspace_transactions.occupied(),
            "Workspace transaction lane occupied"
        );
        ensure!(
            capture.origin.alive() && Instant::now() < capture.deadline,
            "Workspace transaction expired before dispatch"
        );
        ensure!(
            self.settings_error.is_none()
                && capture.context.profile == self.settings_profile_generation()
                && Arc::ptr_eq(&capture.context.settings, self.settings.extension_layers())
                && capture.context.workspace == self.workspace.root,
            "Workspace configuration changed before dispatch"
        );
        self.check_workspace_inventory()?;
        for input in &capture.inputs {
            let mut models = self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .filter(|doc| doc.id == input.source.document());
            let doc = models
                .next()
                .context("Workspace target closed before dispatch")?;
            ensure!(
                models.next().is_none() && input.source.current(doc),
                "Workspace source/history/baseline changed before dispatch"
            );
        }
        let origin = capture.origin.clone();
        let deadline = capture.deadline;
        let token = self.workspace_transactions.worker.start(capture)?;
        self.workspace_transactions.pending = Some(Pending {
            token,
            origin,
            deadline,
        });
        Ok(token)
    }
    pub(crate) fn workspace_transactions_pending(&self) -> bool {
        self.workspace_transactions.occupied()
    }
    pub(crate) fn pending_workspace_transaction(&self) -> Option<(u64, Origin)> {
        self.workspace_transactions
            .pending
            .as_ref()
            .map(|p| (p.token, p.origin.clone()))
    }
    fn check_workspace_inventory(&self) -> Result<()> {
        ensure!(
            self.documents
                .len()
                .checked_add(self.hidden_documents.len())
                .is_some_and(|n| n <= MAX_TARGETS),
            "Workspace exceeds 128 retained models"
        );
        Ok(())
    }
    pub(crate) fn settle_workspace_transactions(&mut self) {
        if let Some(actual) = self.workspace_transactions.worker.active.as_ref() {
            actual.cancel.store(true, Ordering::Release);
        }
        while self.workspace_transactions.worker.active.is_some() {
            self.poll_workspace_transactions();
            std::thread::sleep(Duration::from_millis(5));
        }
        self.workspace_transactions.pending = None;
        self.workspace_transactions.completed = None;
    }

    pub(crate) fn cancel_workspace_transaction(&mut self, token: u64) {
        self.workspace_transactions.worker.cancel(token);
    }
    pub(crate) fn workspace_transaction_completion(&self) -> Option<&Completion> {
        self.workspace_transactions.completed.as_ref()
    }
    /// Call ONLY after exact terminal response enters the transport FIFO, or its
    /// exact caller/session has physically retired. It cannot release actual work.
    pub(crate) fn take_workspace_transaction_completion(
        &mut self,
        token: u64,
    ) -> Option<Completion> {
        if self
            .workspace_transactions
            .completed
            .as_ref()
            .is_some_and(|done| done.token == token)
        {
            self.workspace_transactions.completed.take()
        } else {
            None
        }
    }
    pub(crate) fn poll_workspace_transactions(&mut self) -> bool {
        if let Some(pending) = &self.workspace_transactions.pending
            && (!pending.origin.alive() || Instant::now() >= pending.deadline)
        {
            self.workspace_transactions.worker.cancel(pending.token);
        }
        let Some((token, result)) = self.workspace_transactions.worker.poll() else {
            return false;
        };
        let Some(pending) = self
            .workspace_transactions
            .pending
            .take()
            .filter(|pending| pending.token == token)
        else {
            return true;
        };
        let result = result.map_err(anyhow::Error::msg).and_then(|prepared| {
            ensure!(
                pending.origin.alive() && Instant::now() < pending.deadline,
                "Workspace owner retired or transaction expired"
            );
            self.publish_workspace_transaction(token, prepared)
        });
        let reason = result
            .as_ref()
            .err()
            .map(|error| bounded_reason(&format!("{error:#}")));
        self.workspace_transactions.completed = Some(Completion {
            token,
            origin: pending.origin,
            applied: result.is_ok(),
            reason,
        });
        true
    }
    fn publish_workspace_transaction(&mut self, token: u64, prepared: Prepared) -> Result<()> {
        ensure!(
            prepared.origin.alive()
                && Instant::now() < prepared.deadline
                && self.settings_error.is_none()
                && prepared.context.profile == self.settings_profile_generation()
                && Arc::ptr_eq(&prepared.context.settings, self.settings.extension_layers())
                && prepared.context.workspace == self.workspace.root,
            "Workspace configuration/lifetime changed"
        );
        self.check_workspace_inventory()?;
        for item in &prepared.items {
            let mut models = self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .filter(|doc| doc.id == item.document());
            let doc = models.next().context("Workspace target was closed")?;
            ensure!(!doc.is_read_only(), "Virtual documents are read-only");
            ensure!(
                models.next().is_none() && item.current(doc),
                "Workspace source/history/baseline changed before publication"
            );
        }
        let mut total = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .try_fold(0usize, |n, doc| {
                n.checked_add(doc.text.len_bytes())
                    .context("Workspace model bytes overflow")
            })?;
        for item in &prepared.items {
            let old = self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .find(|doc| doc.id == item.document())
                .expect("validated target");
            total = total
                .checked_sub(old.text.len_bytes())
                .and_then(|n| n.checked_add(item.bytes()))
                .context("Workspace result bytes overflow")?;
        }
        ensure!(
            total <= MAX_DOCUMENTS,
            "Workspace result exceeds 4 MiB native mirrors"
        );
        if !prepared.items.iter().any(WorkspaceTextPrepared::changed) {
            return Ok(());
        }
        let group = WorkspaceHistoryGroup::new(token, &prepared.items)?;
        // Promotion reserves/checks all group counters BEFORE any text changes.
        let mut groups = self.editor_groups.clone();
        let mut change = crate::editor_groups::Change {
            previous: groups.active_membership(),
            ..Default::default()
        };
        if !self.group_fallback {
            for item in prepared.items.iter().filter(|item| item.changed()) {
                let next = groups.promote_document(item.document())?;
                change.changed |= next.changed;
                change.promoted.extend(next.promoted);
            }
            change.active = groups.active_membership();
        }
        let mut items: Vec<_> = prepared
            .items
            .into_iter()
            .filter(WorkspaceTextPrepared::changed)
            .collect();
        let mut commits = Vec::new();
        commits
            .try_reserve_exact(items.len())
            .context("Workspace commit reservation refused")?;
        let mut remaining = MAX_METADATA;
        for doc in self.documents.iter_mut().chain(&mut self.hidden_documents) {
            if let Some(index) = items.iter().position(|item| item.document() == doc.id) {
                let item = items.swap_remove(index);
                let commit = doc.prepare_workspace_commit(item, group.clone(), remaining)?;
                remaining -= commit.metadata_bytes();
                commits.push(commit);
            }
        }
        ensure!(
            items.is_empty(),
            "Workspace target disappeared during commit staging"
        );
        ensure!(
            prepared.origin.alive() && Instant::now() < prepared.deadline,
            "Workspace owner retired or transaction expired during commit preparation"
        );
        for commit in commits {
            commit.publish();
        }
        if change.changed {
            self.editor_groups = groups;
            self.apply_group_change(change);
        }
        self.invalidate_editor_presentation();
        self.observe_outline();
        self.observe_breadcrumbs();
        Ok(())
    }
    pub(crate) fn undo_editor(&mut self) {
        self.history_editor(WorkspaceHistoryDirection::Undo);
    }
    pub(crate) fn redo_editor(&mut self) {
        self.history_editor(WorkspaceHistoryDirection::Redo);
    }
    fn history_editor(&mut self, direction: WorkspaceHistoryDirection) {
        let Some(group) = self.doc().workspace_history(direction) else {
            match direction {
                WorkspaceHistoryDirection::Undo => self.doc_mut().undo(),
                WorkspaceHistoryDirection::Redo => self.doc_mut().redo(),
            }
            return;
        };
        let result = self.publish_workspace_history(group, direction);
        if let Err(error) = result {
            self.message = format!("Grouped workspace history refused; work retained: {error:#}");
        }
    }
    fn publish_workspace_history(
        &mut self,
        group: Arc<WorkspaceHistoryGroup>,
        direction: WorkspaceHistoryDirection,
    ) -> Result<()> {
        self.check_workspace_inventory()?;
        ensure!(
            group.id != 0 && group.members.len() <= MAX_TARGETS,
            "Workspace history cohort exceeds bounds"
        );
        for member in &group.members {
            let mut models = self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .filter(|doc| doc.id == member.document);
            let doc = models
                .next()
                .context("Workspace history peer closed; retain other work")?;
            ensure!(
                models.next().is_none() && doc.workspace_history_current(&group, direction),
                "Workspace peer has independent history; undo it first"
            );
        }
        let mut total = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .try_fold(0usize, |n, doc| {
                n.checked_add(doc.text.len_bytes())
                    .context("Workspace history model bytes overflow")
            })?;
        for member in &group.members {
            let doc = self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .find(|doc| doc.id == member.document)
                .expect("validated history member");
            let restored = doc.workspace_history_bytes(direction)?;
            total = total
                .checked_sub(doc.text.len_bytes())
                .and_then(|n| n.checked_add(restored))
                .context("Workspace history restored bytes overflow")?;
        }
        ensure!(
            total <= MAX_DOCUMENTS,
            "Workspace history exceeds native mirror byte budget"
        );
        let mut groups = self.editor_groups.clone();
        let mut change = crate::editor_groups::Change {
            previous: groups.active_membership(),
            ..Default::default()
        };
        if !self.group_fallback {
            for member in &group.members {
                let next = groups.promote_document(member.document)?;
                change.changed |= next.changed;
                change.promoted.extend(next.promoted);
            }
            change.active = groups.active_membership();
        }
        let mut commits = Vec::new();
        commits
            .try_reserve_exact(group.members.len())
            .context("Workspace history commit reservation refused")?;
        let mut remaining = MAX_METADATA;
        for doc in self.documents.iter_mut().chain(&mut self.hidden_documents) {
            if group.members.iter().any(|member| member.document == doc.id) {
                let commit = doc.prepare_workspace_history(&group, direction, remaining)?;
                remaining -= commit.metadata_bytes();
                commits.push(commit);
            }
        }
        for commit in commits {
            commit.publish();
        }
        if change.changed {
            self.editor_groups = groups;
            self.apply_group_change(change);
        }
        self.invalidate_editor_presentation();
        self.observe_outline();
        self.observe_breadcrumbs();
        Ok(())
    }
}
fn prepare(capture: Arc<Capture>, cancel: &AtomicBool) -> Result<Prepared> {
    let mut expanded = 0usize;
    let mut items = Vec::new();
    items
        .try_reserve_exact(capture.inputs.len())
        .context("Workspace prepared reservation refused")?;
    for input in &capture.inputs {
        crate::document::workspace::guard(cancel, capture.deadline)?;
        ensure!(capture.origin.alive(), "Workspace owner retired");
        let mut edits = Vec::new();
        edits
            .try_reserve_exact(input.edits.len())
            .context("Workspace scalar edit reservation refused")?;
        for edit in &input.edits {
            let start = offset(&input.source, edit.start)?;
            let end = offset(&input.source, edit.end)?;
            ensure!(start <= end, "Workspace range is reversed");
            let replacement = normalize(
                &edit.text,
                input.source.eol(),
                MAX_EXPANDED - expanded,
                cancel,
                capture.deadline,
            )?;
            expanded += replacement.len();
            edits.push((start..end, replacement));
        }
        // Stable sort keeps original insertion order. Coalesce zero-width peers;
        // a replacement sharing that start remains an explicit ambiguity refusal.
        edits.sort_by_key(|(range, _)| (range.start, range.end));
        let mut coalesced: Vec<(Range<usize>, String)> = Vec::new();
        coalesced
            .try_reserve_exact(edits.len())
            .context("Workspace coalescing reservation refused")?;
        for (range, text) in edits {
            if let Some((previous, combined)) = coalesced.last_mut() {
                if previous.start == range.start
                    && previous.start == previous.end
                    && range.start == range.end
                {
                    combined
                        .try_reserve(text.len())
                        .context("Workspace insertion reservation refused")?;
                    combined.push_str(&text);
                    continue;
                }
                ensure!(
                    previous.end <= range.start && previous.start != range.start,
                    "Workspace edits overlap or share ambiguous starts"
                );
            }
            coalesced.push((range, text));
        }
        items.push(
            input
                .source
                .clone()
                .prepare(coalesced, cancel, capture.deadline)?,
        );
    }
    crate::document::workspace::guard(cancel, capture.deadline)?;
    Ok(Prepared {
        origin: capture.origin.clone(),
        items,
        context: capture.context.clone(),
        deadline: capture.deadline,
    })
}
fn offset(source: &WorkspaceSource, position: Position) -> Result<usize> {
    let rope = source.text();
    ensure!(
        position.line < rope.len_lines(),
        "Workspace line is invalid"
    );
    let line = rope.line(position.line);
    let mut length = line.len_chars();
    if length > 0 && line.char(length - 1) == '\n' {
        length -= 1;
    }
    if length > 0 && line.char(length - 1) == '\r' {
        length -= 1;
    }
    let logical = line.slice(..length);
    ensure!(
        position.character <= logical.len_utf16_cu(),
        "Workspace column crosses logical EOL"
    );
    let offset = logical.utf16_cu_to_char(position.character);
    ensure!(
        logical.char_to_utf16_cu(offset) == position.character,
        "Workspace range splits a surrogate"
    );
    Ok(rope.line_to_char(position.line) + offset)
}
fn normalize(
    text: &str,
    eol: &str,
    available: usize,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<String> {
    let mut bytes = 0usize;
    let mut chars = text.chars().peekable();
    let mut count = 0;
    while let Some(ch) = chars.next() {
        if count % 4096 == 0 {
            crate::document::workspace::guard(cancel, deadline)?;
        }
        count += 1;
        let add = if ch == '\r' || ch == '\n' {
            if ch == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            eol.len()
        } else {
            ch.len_utf8()
        };
        bytes = bytes
            .checked_add(add)
            .context("Workspace normalization overflow")?;
        ensure!(
            bytes <= available,
            "Workspace normalized text exceeds 2 MiB"
        );
    }
    let mut result = String::new();
    result
        .try_reserve_exact(bytes)
        .context("Workspace replacement reservation refused")?;
    let mut chars = text.chars().peekable();
    let mut count = 0;
    while let Some(ch) = chars.next() {
        if count % 4096 == 0 {
            crate::document::workspace::guard(cancel, deadline)?;
        }
        count += 1;
        if ch == '\r' || ch == '\n' {
            if ch == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            result.push_str(eol);
        } else {
            result.push(ch);
        }
    }
    Ok(result)
}
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GatePoint {
    BeforePrepare,
    AfterReply,
}
#[cfg(test)]
struct Gates {
    points: Vec<GatePoint>,
    entered: mpsc::SyncSender<GatePoint>,
    release: mpsc::Receiver<()>,
}
#[cfg(test)]
impl Gates {
    fn wait(&self, point: GatePoint) {
        if self.points.contains(&point) {
            let _ = self.entered.send(point);
            let _ = self.release.recv_timeout(Duration::from_secs(6));
        }
    }
}
#[cfg(test)]
fn fixture_worker(
    points: Vec<GatePoint>,
) -> (Worker, mpsc::Receiver<GatePoint>, mpsc::SyncSender<()>) {
    let (entered, rx) = mpsc::sync_channel(1);
    let (tx, release) = mpsc::sync_channel(1);
    (
        Worker {
            gates: Some(Gates {
                points,
                entered,
                release,
            }),
            next: 0,
            active: None,
        },
        rx,
        tx,
    )
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod public_gateway_tests;
