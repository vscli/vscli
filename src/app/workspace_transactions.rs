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
    native: bool,
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
            native: false,
        })
    }
    pub(crate) fn native(request: &str, retired: Arc<AtomicBool>) -> Result<Self> {
        ensure!(
            !request.is_empty()
                && request.len() <= 128
                && !request.contains('\0')
                && !retired.load(Ordering::Acquire),
            "Invalid native workspace origin"
        );
        Ok(Self {
            session: 0,
            request: request.into(),
            owner: String::new(),
            retired,
            native: true,
        })
    }
    pub(crate) fn is_native(&self) -> bool {
        self.native
    }
    pub(super) fn lifetime(&self) -> Arc<AtomicBool> {
        self.retired.clone()
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
enum Target {
    Retained(WorkspaceSource),
    Unopened(PathBuf),
}
struct Input {
    uri: String,
    target: Target,
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
            .is_none_or(|value| value == &Value::Bool(true));
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
    inventory: Vec<(MirrorSnapshot, Option<WorkspaceSource>)>,
    retained_bytes: usize,
    target_receipts: bool,
    refactoring_save: bool,
    context: CaptureContext,
    deadline: Instant,
}
struct Prepared {
    origin: Origin,
    items: Vec<WorkspaceTextPrepared>,
    new_documents: Vec<Document>,
    targets: Vec<TargetReceipt>,
    target_receipts: bool,
    refactoring_save: bool,
    inventory: Vec<MirrorSnapshot>,
    context: CaptureContext,
    deadline: Instant,
}
/// Worker-issued identity mapping for an original request URI. IDs come only
/// from the captured retained source or an actually loaded Document. Versions
/// remain the optional host's mirror authority after successful publication.
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct TargetReceipt {
    pub uri: String,
    pub document: u64,
    #[serde(rename = "savedResource", skip_serializing_if = "Option::is_none")]
    pub saved_resource: Option<String>,
    #[serde(rename = "savedGeneration", skip_serializing_if = "Option::is_none")]
    pub saved_generation: Option<u64>,
}
pub(crate) struct Completion {
    pub token: u64,
    pub origin: Origin,
    pub applied: bool,
    pub targets: Vec<TargetReceipt>,
    pub target_receipts: bool,
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
    pub(super) completed: Option<Completion>,
    pub(super) refactoring: Option<super::refactoring_saves::Cohort>,
}
impl State {
    pub(crate) fn occupied(&self) -> bool {
        self.pending.is_some()
            || self.completed.is_some()
            || self.refactoring.is_some()
            || self.worker.active.is_some()
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
    let args = args
        .as_object()
        .context("Unsupported workspace transaction fields")?;
    ensure!(
        args.contains_key("entries")
            && args.contains_key("metadata")
            && args.keys().all(|key| matches!(
                key.as_str(),
                "entries" | "metadata" | "targetReceipts" | "refactoringSave"
            )),
        "Unsupported workspace transaction fields"
    );
    let target_receipts = args
        .get("targetReceipts")
        .map(|value| {
            value
                .as_bool()
                .context("Invalid workspace target receipts flag")
        })
        .transpose()?
        .unwrap_or(false);
    let metadata = fields(
        &args["metadata"],
        &["isRefactoring"],
        "Unsupported workspace metadata",
    )?;
    let refactoring = metadata["isRefactoring"]
        .as_bool()
        .context("Invalid workspace refactoring flag")?;
    let entries = args["entries"]
        .as_array()
        .context("Workspace entries must be an array")?;
    ensure!(
        !entries.is_empty() && entries.len() <= MAX_TARGETS,
        "Workspace entries require 1–128 targets"
    );
    // Plain text is one performed resource group. Future resource backends
    // must retain the performed ARRAY count before set deduplication.
    let refactoring_save = refactoring && context.refactoring_auto_save && entries.len() > 1;
    if let Some(requested) = args.get("refactoringSave") {
        ensure!(
            requested.as_bool() == Some(refactoring_save),
            "Refactoring save policy differs from original native settings"
        );
    }
    // Retain the complete original identity inventory before any worker IO.
    // Canonical aliases can then resolve to dirty hidden/shared models rather
    // than reopening disk content and silently replacing their source.
    let mut inventory = Vec::new();
    inventory
        .try_reserve_exact(targets.len())
        .context("Workspace inventory reservation refused")?;
    ensure!(
        targets.len() == documents.len() + hidden.len(),
        "Incomplete original workspace inventory"
    );
    for mirror in targets {
        ensure!(
            mirror.document != 0
                && mirror.document < (1u64 << 53)
                && mirror.version < (1u64 << 53)
                && (mirror.version != 0 || origin.is_native()),
            "Invalid original workspace mirror version"
        );
        ensure!(
            !inventory
                .iter()
                .any(
                    |(old, _): &(MirrorSnapshot, Option<WorkspaceSource>)| old.document
                        == mirror.document
                        || old.uri == mirror.uri
                ),
            "Ambiguous original workspace inventory"
        );
        let doc = documents
            .iter()
            .chain(hidden)
            .find(|doc| doc.id == mirror.document)
            .context("Original workspace mirror was closed")?;
        ensure!(
            doc.path == mirror.path
                && doc.revision == mirror.revision
                && doc.text_epoch() == mirror.text_epoch
                && doc.resource_epoch() == mirror.resource_epoch
                && doc.language_epoch() == mirror.language_epoch,
            "Workspace source changed before queue admission"
        );
        let source = if doc.is_read_only() {
            None
        } else {
            Some(doc.capture_workspace_source()?)
        };
        inventory.push((mirror.clone(), source));
    }
    let mut inputs = Vec::new();
    inputs
        .try_reserve_exact(entries.len())
        .context("Workspace target reservation refused")?;
    let (mut count, mut raw) = (0usize, 0usize);
    for entry in entries {
        let entry = entry.as_object().context("Unsupported workspace entry")?;
        ensure!(
            entry.contains_key("uri")
                && entry.contains_key("edits")
                && entry
                    .keys()
                    .all(|key| matches!(key.as_str(), "uri" | "edits" | "document" | "version")),
            "Unsupported workspace entry"
        );
        let positive = |name: &str| -> Result<Option<u64>> {
            entry
                .get(name)
                .map(|value| {
                    let number = value
                        .as_u64()
                        .context("Invalid workspace identity/version")?;
                    ensure!(
                        number != 0 && number < (1u64 << 53),
                        "Workspace identity/version must be positive safe integers"
                    );
                    Ok(number)
                })
                .transpose()
        };
        let id = positive("document")?;
        let version = positive("version")?;
        let uri = entry["uri"].as_str().context("Invalid workspace URI")?;
        ensure!(
            !uri.is_empty() && uri.len() <= 4096 && !uri.contains('\0'),
            "Workspace URI exceeds bounds"
        );
        let target = if let Some((mirror, source)) =
            inventory.iter().find(|(mirror, _)| mirror.uri == uri)
        {
            ensure!(
                id.is_none_or(|id| id == mirror.document)
                    && version.is_none_or(|version| version == mirror.version),
                "Workspace target is outside its captured mirrors"
            );
            let source = source
                .as_ref()
                .context("Virtual documents are read-only")?
                .clone();
            ensure!(
                !inputs.iter().any(|input: &Input| matches!(&input.target,
                Target::Retained(old) if old.document() == source.document())),
                "Duplicate workspace target group"
            );
            Target::Retained(source)
        } else {
            ensure!(
                id.is_none() && version.is_none(),
                "Unopened workspace URI must omit document/version"
            );
            // URL parsing is source-only; canonicalization and filesystem loading
            // belong exclusively to the bounded actual worker.
            let parsed = url::Url::parse(uri).context("Invalid unopened workspace URI")?;
            ensure!(
                parsed.scheme() == "file"
                    && parsed.query().is_none()
                    && parsed.fragment().is_none(),
                "Unsupported unopened workspace URI"
            );
            let path = parsed
                .to_file_path()
                .map_err(|_| anyhow::anyhow!("Unsupported local file URI"))?;
            ensure!(path.is_absolute(), "Workspace file URI must be absolute");
            Target::Unopened(path)
        };
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
            uri: uri.to_owned(),
            target,
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
        inventory,
        retained_bytes: bytes,
        target_receipts,
        refactoring_save,
        context,
        deadline: now
            .checked_add(DEADLINE)
            .context("Workspace deadline overflow")?,
    }))
}
impl App {
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
            let Target::Retained(source) = &input.target else {
                continue;
            };
            let mut models = self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .filter(|doc| doc.id == source.document());
            let doc = models
                .next()
                .context("Workspace target closed before dispatch")?;
            ensure!(
                models.next().is_none() && source.current(doc),
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
        self.retire_refactoring_saves();
        while self.workspace_transactions.refactoring.is_some() {
            self.poll_native_saves();
            self.poll_refactoring_saves();
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
        self.guard_file_participation(self.extension_host.as_ref());
        self.poll_workspace_transactions_guarded()
    }
    /// Service dispatch temporarily owns the Client outside App. Validate the
    /// actual original host rather than treating that scoped move as retirement.
    pub(super) fn poll_workspace_transactions_with_host(
        &mut self,
        host: &crate::extensions::Client,
    ) -> bool {
        self.guard_file_participation(Some(host));
        self.poll_workspace_transactions_guarded()
    }
    fn poll_workspace_transactions_guarded(&mut self) -> bool {
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
        let mut refactoring_save = false;
        let result = result.map_err(anyhow::Error::msg).and_then(|prepared| {
            ensure!(
                pending.origin.alive() && Instant::now() < pending.deadline,
                "Workspace owner retired or transaction expired"
            );
            refactoring_save = prepared.refactoring_save;
            self.publish_workspace_transaction(token, prepared)
        });
        match result {
            Ok((targets, target_receipts)) => {
                if refactoring_save {
                    match super::refactoring_saves::Cohort::capture(
                        self,
                        token,
                        pending.origin.clone(),
                        &targets,
                        target_receipts,
                    ) {
                        Ok(cohort) => {
                            self.workspace_transactions.refactoring = Some(cohort);
                            return true;
                        }
                        Err(error) => {
                            self.workspace_transactions.completed = Some(Completion {
                                token,
                                origin: pending.origin,
                                applied: true,
                                targets,
                                target_receipts,
                                reason: Some(bounded_reason(&format!(
                                    "Refactoring saves could not start; edits remain applied: {error:#}"
                                ))),
                            });
                        }
                    }
                } else {
                    self.workspace_transactions.completed = Some(Completion {
                        token,
                        origin: pending.origin,
                        applied: true,
                        targets,
                        target_receipts,
                        reason: None,
                    });
                }
            }
            Err(error) => {
                self.workspace_transactions.completed = Some(Completion {
                    token,
                    origin: pending.origin,
                    applied: false,
                    targets: Vec::new(),
                    target_receipts: false,
                    reason: Some(bounded_reason(&format!("{error:#}"))),
                });
            }
        }
        true
    }
    fn publish_workspace_transaction(
        &mut self,
        token: u64,
        mut prepared: Prepared,
    ) -> Result<(Vec<TargetReceipt>, bool)> {
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
        ensure!(
            self.documents.len() + self.hidden_documents.len() + prepared.new_documents.len()
                <= MAX_TARGETS,
            "Workspace exceeds 128 retained models"
        );
        // Reject admission/removal/path changes while the worker was resolving
        // aliases. An unobserved new retained model must never be duplicated.
        ensure!(
            self.documents.len() + self.hidden_documents.len() == prepared.inventory.len()
                && prepared.inventory.iter().all(|mirror| self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .any(|doc| doc.id == mirror.document
                        && doc.path == mirror.path
                        && doc.resource_epoch() == mirror.resource_epoch)),
            "Original workspace inventory changed before publication"
        );
        for doc in &mut prepared.new_documents {
            self.settings.apply(doc);
            self.configure_document_language(doc)?;
        }
        for item in &prepared.items {
            let mut models = self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .chain(&prepared.new_documents)
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
            .chain(&prepared.new_documents)
            .try_fold(0usize, |n, doc| {
                n.checked_add(doc.text.len_bytes())
                    .context("Workspace model bytes overflow")
            })?;
        for item in &prepared.items {
            let old = self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .chain(&prepared.new_documents)
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
        self.hidden_documents
            .try_reserve_exact(prepared.new_documents.len())
            .context("Workspace hidden model reservation refused")?;
        if !prepared.items.iter().any(WorkspaceTextPrepared::changed) {
            ensure!(
                prepared.origin.alive() && Instant::now() < prepared.deadline,
                "Workspace owner retired or transaction expired during commit preparation"
            );
            self.hidden_documents.extend(prepared.new_documents);
            return Ok((prepared.targets, prepared.target_receipts));
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
        for doc in self
            .documents
            .iter_mut()
            .chain(&mut self.hidden_documents)
            .chain(&mut prepared.new_documents)
        {
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
        self.hidden_documents.extend(prepared.new_documents);
        if change.changed {
            self.editor_groups = groups;
            self.apply_group_change(change);
        }
        self.invalidate_editor_presentation();
        self.observe_outline();
        self.observe_breadcrumbs();
        Ok((prepared.targets, prepared.target_receipts))
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
    let mut targets = Vec::new();
    targets
        .try_reserve_exact(capture.inputs.len())
        .context("Workspace target receipts reservation refused")?;
    let mut new_documents = Vec::new();
    new_documents
        .try_reserve_exact(capture.inputs.len())
        .context("Workspace loaded model reservation refused")?;
    let mut loaded_bytes = capture.retained_bytes;
    let mut canonical_inventory = Vec::new();
    if capture
        .inputs
        .iter()
        .any(|input| matches!(input.target, Target::Unopened(_)))
    {
        canonical_inventory
            .try_reserve_exact(capture.inventory.len())
            .context("Workspace alias reservation refused")?;
        for (mirror, source) in &capture.inventory {
            crate::document::workspace::guard(cancel, capture.deadline)?;
            if let Some(path) = &mirror.path {
                let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
                canonical_inventory.push((canonical, source.as_ref()));
            }
        }
    }
    for input in &capture.inputs {
        crate::document::workspace::guard(cancel, capture.deadline)?;
        ensure!(capture.origin.alive(), "Workspace owner retired");
        let source = match &input.target {
            Target::Retained(source) => source.clone(),
            Target::Unopened(path) => {
                let canonical = std::fs::canonicalize(path)
                    .context("Cannot resolve unopened workspace file")?;
                crate::document::workspace::guard(cancel, capture.deadline)?;
                let mut matching = canonical_inventory
                    .iter()
                    .filter(|(path, _)| *path == canonical);
                if let Some((_, source)) = matching.next() {
                    ensure!(
                        matching.next().is_none(),
                        "Ambiguous retained workspace alias"
                    );
                    source.context("Virtual documents are read-only")?.clone()
                } else {
                    ensure!(
                        capture.inventory.len() + new_documents.len() < MAX_TARGETS,
                        "Workspace exceeds 128 retained models"
                    );
                    ensure!(
                        !new_documents
                            .iter()
                            .any(|doc: &Document| doc.path.as_ref() == Some(&canonical)),
                        "Duplicate workspace target group"
                    );
                    let doc = Document::open_workspace_existing(
                        &canonical,
                        MAX_DOCUMENTS - loaded_bytes,
                        cancel,
                        capture.deadline,
                    )?;
                    loaded_bytes += doc.text.len_bytes();
                    let source = doc.capture_workspace_source()?;
                    new_documents.push(doc);
                    source
                }
            }
        };
        ensure!(
            !items
                .iter()
                .any(|item: &WorkspaceTextPrepared| item.document() == source.document()),
            "Duplicate workspace target group"
        );
        ensure!(
            source.document() != 0 && source.document() < (1u64 << 53),
            "Workspace loaded document identity exceeds safe integer bounds"
        );
        targets.push(TargetReceipt {
            uri: input.uri.clone(),
            document: source.document(),
            saved_resource: None,
            saved_generation: None,
        });
        let mut edits = Vec::new();
        edits
            .try_reserve_exact(input.edits.len())
            .context("Workspace scalar edit reservation refused")?;
        for edit in &input.edits {
            let start = offset(&source, edit.start)?;
            let end = offset(&source, edit.end)?;
            ensure!(start <= end, "Workspace range is reversed");
            let replacement = normalize(
                &edit.text,
                source.eol(),
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
        items.push(source.prepare(coalesced, cancel, capture.deadline)?);
    }
    crate::document::workspace::guard(cancel, capture.deadline)?;
    Ok(Prepared {
        origin: capture.origin.clone(),
        items,
        new_documents,
        targets,
        target_receipts: capture.target_receipts,
        refactoring_save: capture.refactoring_save,
        inventory: capture
            .inventory
            .iter()
            .map(|(mirror, _)| mirror.clone())
            .collect(),
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

#[cfg(test)]
mod unopened_tests;
