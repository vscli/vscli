use super::*;
use crate::extensions::services::{NativeService, Operation, SERVICE_LIFETIME};
use anyhow::{Context as _, bail};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, TryRecvError},
};
const MAX_HIDDEN: usize = 16;
const MAX_MODELS: usize = 128;
const MAX_CONTEXT_SELECTIONS: usize = 16_384;
const MAX_BYTES: usize = 4 * 1024 * 1024;
#[derive(Clone, PartialEq)]
struct DocumentContext {
    id: u64,
    epoch: u64,
    language_epoch: u64,
    resource_epoch: u64,
    revision: u64,
    saved: u64,
    path: Option<PathBuf>,
    selections: Vec<crate::document::Selection>,
}
#[derive(Clone, PartialEq)]
pub(super) struct Context {
    snapshot: Option<Snapshot>,
}
#[derive(Clone, PartialEq)]
struct Snapshot {
    epoch: u64,
    profile: u64,
    settings: Arc<Vec<serde_json::Map<String, Value>>>,
    groups: crate::editor_groups::UiProof,
    workspace: PathBuf,
    focus: Focus,
    active: Option<u64>,
    panes: Vec<(u64, u64)>,
    pane_selections: Vec<(usize, Option<usize>, Vec<crate::document::Selection>)>,
    active_pane: usize,
    visible: usize,
    documents: Vec<DocumentContext>,
}
impl Context {
    pub(super) fn check_budget(app: &App) -> Result<()> {
        if app
            .documents
            .len()
            .saturating_add(app.hidden_documents.len())
            > MAX_MODELS
        {
            bail!("Native extension contexts support at most 128 retained models");
        }
        // Count the actual retained snapshots, including an active view that is
        // captured both as a document and as a pane. Inspect lengths only.
        let mut selections = 0usize;
        let mut add = |secondary: usize| -> Result<()> {
            selections = selections.saturating_add(secondary).saturating_add(1);
            if selections > MAX_CONTEXT_SELECTIONS {
                bail!(
                    "Native extension context exceeds 16,384 document/pane selections; reduce selections and retry"
                );
            }
            Ok(())
        };
        for doc in app.documents.iter().chain(&app.hidden_documents) {
            add(doc.secondary.len())?;
        }
        for pane in &app.panes {
            if let Some(doc) = app.documents.iter().find(|doc| doc.id == pane.document) {
                add(doc.view_state(Some(pane.id)).secondary.len())?;
            }
        }
        Ok(())
    }
    pub(super) fn capture(app: &App) -> Self {
        if Self::check_budget(app).is_err() {
            // Preserve the infallible caller API, but never compare two invalid
            // captures as an authorized context or clone over-budget selections.
            return Self { snapshot: None };
        }
        Self {
            snapshot: Some(Snapshot {
                epoch: app.extension_services.epoch,
                profile: app.settings_profile_generation(),
                settings: app.settings.extension_layers().clone(),
                groups: app.editor_groups.proof(),
                workspace: app.workspace.root.clone(),
                focus: app.focus.clone(),
                active: app.active_document().map(|doc| doc.id),
                panes: app.panes.iter().map(|p| (p.id, p.document)).collect(),
                pane_selections: app
                    .panes
                    .iter()
                    .filter_map(|p| {
                        app.documents.iter().find(|d| d.id == p.document).map(|d| {
                            let view = d.view_state(Some(p.id));
                            (view.cursor, view.anchor, view.secondary.clone())
                        })
                    })
                    .collect(),
                active_pane: app.active_pane,
                visible: app.documents.len(),
                documents: app
                    .documents
                    .iter()
                    .chain(&app.hidden_documents)
                    .map(|d| DocumentContext {
                        id: d.id,
                        epoch: d.text_epoch(),
                        language_epoch: d.language_epoch(),
                        resource_epoch: d.resource_epoch(),
                        revision: d.revision,
                        saved: d.saved_revision,
                        path: d.path.clone(),
                        selections: d.selections(),
                    })
                    .collect(),
            }),
        }
    }
    pub(super) fn same_editor(&self, app: &App) -> bool {
        self.snapshot.as_ref().is_some_and(|original| {
            Arc::ptr_eq(&original.settings, app.settings.extension_layers())
                && self == &Self::capture(app)
        })
    }
    pub(super) fn accept_next_interaction(&mut self) {
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.epoch = snapshot.epoch.wrapping_add(1);
        }
    }
    fn valid(&self, app: &App) -> bool {
        self.same_editor(app) && app.prompt.is_none() && app.modal.is_none()
    }
    fn customer_open_current(&self, app: &App) -> bool {
        self.snapshot.as_ref().is_some_and(|original| {
            original.workspace == app.workspace.root
                && original.profile == app.settings_profile_generation()
                && Arc::ptr_eq(&original.settings, app.settings.extension_layers())
        })
    }
    fn customer_existing_current(&self, app: &App, path: &Path) -> bool {
        self.snapshot
            .as_ref()
            .and_then(|snapshot| {
                snapshot
                    .documents
                    .iter()
                    .find(|doc| doc.path.as_deref() == Some(path))
            })
            .is_some_and(|original| {
                app.documents
                    .iter()
                    .chain(&app.hidden_documents)
                    .any(|doc| {
                        doc.id == original.id
                            && doc.resource_epoch() == original.resource_epoch
                            && doc.path.as_deref() == Some(path)
                    })
            })
    }
    fn service_current(&self, app: &App, operation: &Operation) -> bool {
        // Opening a text document retains it without creating or focusing an
        // editor. Indexers can do that while a native picker/prompt is active;
        // source, workspace, settings, ownership and version proofs still hold.
        if matches!(operation, Operation::Open(_)) {
            self.same_editor(app)
        } else {
            self.valid(app)
        }
    }
}
enum Output {
    Existing(PathBuf),
    Loaded(Box<Document>),
    State(Value),
}
struct Job {
    service: NativeService,
    context: Option<Context>,
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Result<Output, String>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[derive(Default)]
pub(super) struct State {
    pub config_root: Option<PathBuf>,
    pub epoch: u64,
    job: Option<Job>,
    contexts: std::collections::HashMap<(u64, String), Context>,
    pub(super) editor_state_completed: Option<super::extension_editor_state::Completed>,
    pub(super) tabs_completed: Option<super::extension_editor_tabs::Completed>,
    shown_completed: Option<ShownCompleted>,
}
struct ShownCompleted {
    service: NativeService,
    result: Result<Value, String>,
}
pub(super) fn resolved(path: &Path, workspace: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        workspace.join(path)
    };
    if let Ok(canonical) = std::fs::canonicalize(&path) {
        return Ok(canonical);
    }
    Ok(
        std::fs::canonicalize(path.parent().context("Document path has no parent")?)?
            .join(path.file_name().context("Document path has no filename")?),
    )
}
impl App {
    pub(super) fn extension_services_pending(&self) -> bool {
        self.extension_services.job.is_some()
            || self.extension_services.shown_completed.is_some()
            || self.extension_services.editor_state_completed.is_some()
            || self.extension_services.tabs_completed.is_some()
    }
    pub(super) fn cancel_extension_services(&mut self) {
        self.retire_extension_tab_close();
        self.cancel_extension_file_search();
        self.cancel_virtual_documents();
        if let Some((token, origin)) = self.pending_workspace_transaction()
            && !origin.is_native()
        {
            self.cancel_workspace_transaction(token);
        }
        if let Some(job) = &self.extension_services.job
            && !job.service.customer_owned
        {
            job.cancel.store(true, Ordering::Release);
        }
    }
    pub fn configure_extension_storage(&mut self, root: Option<&Path>) {
        self.extension_services.config_root = root.map(Path::to_owned);
    }
    pub(super) fn poll_extension_services(&mut self) -> bool {
        let Some(mut host) = self.extension_host.take() else {
            self.retire_rename_before_publication(None);
            self.retire_extension_tab_close();
            self.extension_services.shown_completed = None;
            self.extension_services.contexts.clear();
            if let Some((token, origin)) = self.pending_workspace_transaction()
                && !origin.is_native()
            {
                self.cancel_workspace_transaction(token);
            }
            let changed = self.poll_extension_file_search(None)
                | self.poll_virtual_documents(None)
                | self.poll_workspace_transactions();
            if let Some(token) = self
                .workspace_transaction_completion()
                .filter(|done| !done.origin.is_native())
                .map(|done| done.token)
            {
                self.take_workspace_transaction_completion(token);
            }
            self.retire_all_document_save_callers();
            while let Some(completion) = self.take_document_save_completion() {
                self.retire_document_save_prompt(completion.interest);
            }
            if let Some(job) = &self.extension_services.job {
                job.cancel.store(true, Ordering::Release);
            }
            // Retain an occupied worker until it actually finishes even after host retirement.
            if self.extension_services.job.as_ref().is_some_and(|job| {
                job.thread
                    .as_ref()
                    .is_some_and(|thread| thread.is_finished())
            }) {
                self.extension_services.job = None;
            }
            return changed;
        };
        let changed = self.poll_services_with_host(&mut host);
        self.extension_host = Some(host);
        changed
    }
    fn poll_services_with_host(&mut self, host: &mut crate::extensions::Client) -> bool {
        if let Err(error) = self.bind_api_document_customer(host.session) {
            self.message = format!("Native document customer refused: {error:#}");
            return false;
        }
        // Capture every queued document operation when first observed by App,
        // including requests waiting behind a filesystem write. A later focus,
        // pane or input change must not become their new authorization context.
        let pending: std::collections::HashSet<_> = host
            .queued_services()
            .map(|service| (service.request.session, service.id.clone()))
            .collect();
        self.extension_services
            .contexts
            .retain(|key, _| pending.contains(key));
        for service in host.queued_services().filter(|service| {
            !matches!(
                service.operation,
                Operation::LoadBegin(_)
                    | Operation::LoadCancel
                    | Operation::State(_)
                    | Operation::Save(_)
                    | Operation::WorkspaceApply(_)
                    | Operation::FindFiles(..)
                    | Operation::UntitledOpen(_)
                    | Operation::VirtualOpen(_)
                    | Operation::VirtualChange(_)
                    | Operation::EditorState(_)
                    | Operation::TabsClose { .. }
            )
        }) {
            let key = (service.request.session, service.id.clone());
            if !self.extension_services.contexts.contains_key(&key) {
                // An over-budget receipt is permanently invalid too. Never
                // rebind a queued operation after the user reduces selections.
                let context = Context::capture(self);
                self.extension_services.contexts.insert(key, context);
            }
        }
        let editor_changed = self.poll_editor_state_completion(host);
        if self.extension_services.editor_state_completed.is_some() || editor_changed {
            return editor_changed;
        }
        let tabs_changed = self.poll_extension_tab_close(host);
        if self.extension_services.tabs_completed.is_some() || tabs_changed {
            return tabs_changed;
        }
        let shown_changed = self.poll_shown_editor_completion(host);
        if self.extension_services.shown_completed.is_some() || shown_changed {
            return shown_changed;
        }
        let virtual_changed = self.poll_virtual_documents(Some(host));
        if self.virtual_documents_pending() || virtual_changed {
            return virtual_changed;
        }
        let file_search_changed = self.poll_extension_file_search(Some(host));
        if self.file_search_pending() || file_search_changed {
            return file_search_changed;
        }
        self.retire_rename_before_publication(Some(host));
        if let Some((token, origin)) = self.pending_workspace_transaction()
            && !origin.is_native()
        {
            let valid = host.service().is_some_and(|service| {
                service.id == origin.request
                    && service.request.session == origin.session
                    && service.request.owner == origin.owner
                    && matches!(service.operation, Operation::WorkspaceApply(_))
                    && host.service_valid(service).is_ok()
            });
            if !valid {
                self.cancel_workspace_transaction(token);
            }
        }
        let workspace_changed = self.poll_workspace_transactions_with_host(host);
        if let Some(done) = self.workspace_transaction_completion()
            && !done.origin.is_native()
        {
            let token = done.token;
            let origin = done.origin.clone();
            let mut result = serde_json::json!({"applied":done.applied,"reason":done.reason.clone().unwrap_or_default()});
            if done.applied && done.target_receipts {
                result["targets"] = serde_json::json!(done.targets);
            }
            if let Some(service) = host.service().cloned().filter(|service| {
                service.id == origin.request
                    && service.request.session == origin.session
                    && service.request.owner == origin.owner
                    && matches!(service.operation, Operation::WorkspaceApply(_))
            }) {
                let result = host.service_valid(&service).map(|()| result);
                if self.answer_native_service(host, &service, result) {
                    self.take_workspace_transaction_completion(token);
                }
            } else if origin.session != host.session || !origin.alive() {
                self.take_workspace_transaction_completion(token);
            }
            return true;
        }
        if self.workspace_transactions_pending() {
            return workspace_changed;
        }

        if let Some(completion) = self.document_save_completion() {
            self.retire_document_save_prompt(completion.interest);
            if let Some(service) = host.service().cloned()
                && service.request.session == completion.caller.session
                && service.id == completion.caller.request
                && service.request.owner == completion.caller.owner
                && matches!(service.operation, Operation::Save(_))
            {
                use super::saving::document_interests::Receipt;
                let result =
                    if host.service_valid(&service).is_err() {
                        let state = if completion.retired {
                            "retired"
                        } else {
                            "unavailable"
                        };
                        Err(anyhow::anyhow!(
                            "Document save caller {state}; native receipt retained"
                        ))
                    } else {
                        match completion.receipt {
                            Receipt::Committed {
                                document,
                                save_generation,
                            } => {
                                if document
                                    == match &service.operation {
                                        Operation::Save(save) => save.document,
                                        _ => unreachable!(),
                                    }
                                    && self.documents.iter().chain(&self.hidden_documents).any(
                                        |doc| {
                                            doc.id == document
                                                && doc.save_generation() >= save_generation
                                        },
                                    )
                                {
                                    Ok(Value::Bool(true))
                                } else {
                                    Err(anyhow::anyhow!(
                                        "Disk committed but save publication is no longer retained"
                                    ))
                                }
                            }
                            Receipt::NotSaved => Ok(Value::Bool(false)),
                            Receipt::Failed(error) => Err(anyhow::anyhow!(
                                "Native save failed without a successful receipt: {error}"
                            )),
                            Receipt::CommittedButUnpublished(error) => Err(anyhow::anyhow!(
                                "Disk committed but native model acknowledgement failed: {error}"
                            )),
                        }
                    };
                if self.answer_native_service(host, &service, result) {
                    self.take_document_save_completion();
                }
            } else {
                // The old session/FIFO no longer owns an answer; discard only
                // its receipt interest, never an authorized native operation.
                self.take_document_save_completion();
            }
            return true;
        }
        if let Some(job) = &self.extension_services.job {
            let valid = host.service_valid(&job.service).is_ok()
                && job.context.as_ref().is_none_or(|context| {
                    if job.service.customer_owned {
                        context.customer_open_current(self)
                    } else {
                        context.service_current(self, &job.service.operation)
                    }
                });
            if !valid {
                job.cancel.store(true, Ordering::Release);
            }
            if job
                .thread
                .as_ref()
                .is_some_and(|thread| !thread.is_finished())
            {
                return false;
            }
            let result = match job.receiver.try_recv() {
                Ok(result) => result.map_err(anyhow::Error::msg),
                Err(TryRecvError::Empty) => return false,
                Err(TryRecvError::Disconnected) => {
                    Err(anyhow::anyhow!("Native service worker stopped"))
                }
            };
            let mut job = self.extension_services.job.take().unwrap();
            if let Some(thread) = job.thread.take() {
                let _ = thread.join();
            }
            let result = if valid && !job.cancel.load(Ordering::Acquire) {
                result
                    .and_then(|output| {
                        if job.service.customer_owned
                            && let Output::Existing(path) = &output
                        {
                            anyhow::ensure!(
                                job.context.as_ref().is_some_and(
                                    |context| context.customer_existing_current(self, path)
                                ),
                                "Original document load resource binding retired"
                            );
                        }
                        Ok(output)
                    })
                    .and_then(|output| self.install_service_output(output))
                    .and_then(|value| {
                        if matches!(job.service.operation, Operation::Open(_)) {
                            let id = value["document"]
                                .as_u64()
                                .context("Missing opened model identity")?;
                            self.acquire_api_document(
                                job.service.request.session,
                                id,
                                std::time::Instant::now(),
                            )?;
                        }
                        Ok(value)
                    })
            } else {
                Err(anyhow::anyhow!(
                    "Native service cancelled by newer editor or host context; retry"
                ))
            };
            self.answer_native_service(host, &job.service, result);
            return true;
        }
        let Some(service) = host.service().cloned() else {
            return false;
        };
        if matches!(service.operation, Operation::Save(_))
            && self.document_save_caller_pending(service.request.session, &service.id)
        {
            let waiting = self.document_save_path_pending(service.request.session, &service.id);
            if host.set_document_save_path_wait(&service, waiting).is_err()
                || host.service_valid(&service).is_err()
            {
                self.retire_document_save_callers(service.request.session, &service.request.owner);
            }
            return false;
        }
        let validation = host.service_valid(&service).and_then(|()| {
            if !matches!(
                service.operation,
                Operation::LoadBegin(_)
                    | Operation::LoadCancel
                    | Operation::State(_)
                    | Operation::Save(_)
                    | Operation::WorkspaceApply(_)
                    | Operation::FindFiles(..)
                    | Operation::UntitledOpen(_)
                    | Operation::VirtualOpen(_)
                    | Operation::VirtualChange(_)
                    | Operation::EditorState(_)
                    | Operation::TabsClose { .. }
            ) {
                Context::check_budget(self)?;
            }
            if !matches!(
                service.operation,
                Operation::LoadBegin(_)
                    | Operation::LoadCancel
                    | Operation::State(_)
                    | Operation::Save(_)
                    | Operation::WorkspaceApply(_)
                    | Operation::FindFiles(..)
                    | Operation::UntitledOpen(_)
                    | Operation::VirtualOpen(_)
                    | Operation::VirtualChange(_)
                    | Operation::EditorState(_)
                    | Operation::TabsClose { .. }
            ) && !self
                .extension_services
                .contexts
                .get(&(service.request.session, service.id.clone()))
                .is_some_and(|context| {
                    if service.customer_owned {
                        context.customer_open_current(self)
                    } else {
                        context.service_current(self, &service.operation)
                    }
                })
            {
                bail!("Native document service cancelled by newer editor context; retry");
            }
            if !matches!(
                service.operation,
                Operation::Open(_)
                    | Operation::LoadBegin(_)
                    | Operation::LoadCancel
                    | Operation::State(_)
                    | Operation::Save(_)
                    | Operation::WorkspaceApply(_)
                    | Operation::FindFiles(..)
                    | Operation::UntitledOpen(_)
                    | Operation::VirtualOpen(_)
                    | Operation::VirtualChange(_)
                    | Operation::EditorState(_)
                    | Operation::TabsClose { .. }
            ) && (self.prompt.is_some() || self.modal.is_some())
            {
                bail!("Native document service cannot replace an active prompt or modal");
            }
            Ok(())
        });
        if let Err(error) = validation {
            self.answer_native_service(host, &service, Err(error));
            return true;
        }
        match &service.operation {
            Operation::LoadBegin(ticket) => {
                let result = (|| -> Result<Value> {
                    anyhow::ensure!(
                        ticket.context.current(
                            &self.workspace.root,
                            self.settings.extension_layers(),
                            self.settings_profile_generation()
                        ),
                        "Original document admission context changed"
                    );
                    ticket.issue(host.session)?;
                    Ok(json!({"load":ticket.id}))
                })();
                if result.is_err() {
                    ticket.retire();
                }
                self.answer_native_service(host, &service, result);
            }
            Operation::LoadCancel => {
                self.answer_native_service(host, &service, Ok(json!({"canceled":true})));
            }
            Operation::TabsClose { .. } => self.start_extension_tab_close(host, service.clone()),
            Operation::EditorState(_) => self.start_editor_state_service(host, service.clone()),
            Operation::UntitledOpen(_)
            | Operation::VirtualOpen(_)
            | Operation::VirtualChange(_) => {
                if let Err(error) = self.start_virtual_document(host, service.clone()) {
                    self.answer_native_service(host, &service, Err(error));
                }
            }
            Operation::FindFiles(..) => {
                if let Err(error) = self.start_extension_file_search(service.clone())
                    && let Err(answer_error) = host.answer_service(&service, Err(error))
                {
                    self.message =
                        format!("Native file discovery refusal reply retained: {answer_error:#}");
                }
            }
            Operation::WorkspaceApply(capture) => {
                if let Err(error) = self.start_workspace_transaction(capture.clone()) {
                    let mut reason = format!("{error:#}");
                    let mut end = reason.len().min(4096);
                    while !reason.is_char_boundary(end) {
                        end -= 1;
                    }
                    reason.truncate(end);
                    self.answer_native_service(
                        host,
                        &service,
                        Ok(serde_json::json!({"applied":false,"reason":reason})),
                    );
                }
            }
            Operation::Save(save) => {
                if self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .any(|doc| doc.id == save.document && doc.is_read_only())
                {
                    self.answer_native_service(host, &service, Ok(Value::Bool(false)));
                    return true;
                }
                let result = (|| -> Result<()> {
                    let doc = self
                        .documents
                        .iter()
                        .chain(&self.hidden_documents)
                        .find(|doc| doc.id == save.document)
                        .context("Save document is no longer retained")?;
                    if !host.service_save_document_current(&service, doc) {
                        bail!("Document changed before save admission; retry");
                    }
                    let needs_path = doc.path.is_none() && doc.associated_path().is_none();
                    if needs_path
                        && (self.prompt.is_some() || self.modal.is_some() || self.pending.is_some())
                    {
                        bail!("Document Save As cannot replace another prompt/modal");
                    }
                    use super::saving::document_interests::{Admission, Caller};
                    let caller =
                        Caller::new(service.request.session, &service.id, &service.request.owner)?;
                    match self.request_document_save_interest(caller, save.document)? {
                        Admission::Enqueued(interest) => {
                            self.retire_document_save_prompt(interest);
                        }
                        Admission::NeedsPath(interest) => {
                            if let Err(error) =
                                self.begin_document_save_path_prompt(interest, save.document)
                            {
                                self.retire_document_save_interest(interest);
                                return Err(error);
                            }
                            if let Err(error) = host.set_document_save_path_wait(&service, true) {
                                self.retire_document_save_interest(interest);
                                self.retire_document_save_prompt(interest);
                                return Err(error);
                            }
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    self.answer_native_service(host, &service, Err(error));
                }
            }
            Operation::Open(open) => {
                let path = PathBuf::from(&open.path);
                let path = if path.is_absolute() {
                    path
                } else {
                    self.workspace.root.join(path)
                };
                if let Some(id) = self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .find(|d| d.path.as_ref() == Some(&path))
                    .map(|doc| doc.id)
                {
                    let result = self
                        .acquire_api_document(
                            service.request.session,
                            id,
                            std::time::Instant::now(),
                        )
                        .map(|()| serde_json::json!({"document":id}));
                    self.answer_native_service(host, &service, result);
                    return true;
                }
                let remaining = MAX_BYTES.saturating_sub(
                    self.documents
                        .iter()
                        .chain(&self.hidden_documents)
                        .map(|d| d.text.len_bytes())
                        .sum::<usize>(),
                );
                let existing: Vec<_> = self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .filter_map(|d| d.path.clone())
                    .collect();
                let workspace = self.workspace.root.clone();
                let capacity = self.hidden_documents.len() < MAX_HIDDEN;
                let (sender, receiver) = mpsc::sync_channel(1);
                let cancel = Arc::new(AtomicBool::new(false));
                let flag = cancel.clone();
                let started = service.started;
                let thread = std::thread::spawn(move || {
                    let result = (|| -> Result<Output> {
                        if flag.load(Ordering::Acquire) || started.elapsed() >= SERVICE_LIFETIME {
                            bail!("Document open cancelled");
                        }
                        let path = resolved(&path, &workspace)?;
                        if existing.contains(&path) {
                            return Ok(Output::Existing(path));
                        }
                        if !capacity {
                            bail!("Hidden document limit reached (16)");
                        }
                        let doc = Document::open_existing_bounded(&path, remaining as u64)?;
                        if flag.load(Ordering::Acquire) || started.elapsed() >= SERVICE_LIFETIME {
                            bail!("Document open cancelled");
                        }
                        Ok(Output::Loaded(Box::new(doc)))
                    })()
                    .map_err(|error| format!("{error:#}"));
                    let _ = sender.send(result);
                });
                let context = self
                    .extension_services
                    .contexts
                    .get(&(service.request.session, service.id.clone()))
                    .cloned();
                self.extension_services.job = Some(Job {
                    service,
                    context,
                    cancel,
                    receiver,
                    thread: Some(thread),
                });
            }
            Operation::State(patch) => {
                let Some(store) = host.state_store() else {
                    self.answer_native_service(
                        host,
                        &service,
                        Err(anyhow::anyhow!(
                            "Extension state storage was not configured"
                        )),
                    );
                    return true;
                };
                let patch = patch.clone();
                let owner = service.request.owner.clone();
                let started = service.started;
                let (sender, receiver) = mpsc::sync_channel(1);
                let cancel = Arc::new(AtomicBool::new(false));
                let flag = cancel.clone();
                let thread = std::thread::spawn(move || {
                    let result = store
                        .update_with(
                            &owner,
                            patch.scope,
                            &patch.key,
                            (!patch.remove).then_some(patch.value),
                            || {
                                if flag.load(Ordering::Acquire)
                                    || started.elapsed() >= SERVICE_LIFETIME
                                {
                                    bail!("Extension state update cancelled before commit");
                                }
                                Ok(())
                            },
                        )
                        .map(|values| Output::State(Value::Object(values)))
                        .map_err(|error| format!("{error:#}"));
                    let _ = sender.send(result);
                });
                self.extension_services.job = Some(Job {
                    service,
                    context: None,
                    cancel,
                    receiver,
                    thread: Some(thread),
                });
            }
            Operation::Show(show) => {
                let result = self.show_extension_document(host, show);
                self.refresh_extension_editor_inventory();
                host.set_editor_inventory(self.extension_editor_inventory.clone());
                host.set_editor_tabs(self.extension_editor_tabs.clone());
                let result = result.and_then(|mut value| {
                    if let Some(view) = value["view"].as_u64() {
                        value
                            .as_object_mut()
                            .context("Invalid shown editor receipt")?
                            .remove("view");
                        if host.editor_inventory_explicit() {
                            let editor = self
                                .extension_editor_inventory
                                .id_for_view(show.document, view)
                                .context("Shown native editor inventory is absent")?;
                            value["editor"] = editor.into();
                        }
                    }
                    Ok(value)
                });
                self.extension_services.shown_completed = Some(ShownCompleted {
                    service: service.clone(),
                    result: result
                        .map_err(|error| crate::extensions::services::bounded_error(&error)),
                });
                self.poll_shown_editor_completion(host);
            }
            Operation::Command(command) => {
                let result = if command.id == "files.participants.resetChoice" {
                    self.reset_file_participant_choice().map(|()| Value::Null)
                } else if self.active_document().is_none() || self.focus != Focus::Editor {
                    Err(anyhow::anyhow!(
                        "Native delegated command requires the focused editor"
                    ))
                } else {
                    self.execute(&command.id, Value::Null);
                    Ok(Value::Null)
                };
                self.answer_native_service(host, &service, result);
            }
        }
        true
    }
    fn install_service_output(&mut self, output: Output) -> Result<Value> {
        match output {
            Output::State(values) => Ok(serde_json::json!({"values":values})),
            Output::Existing(path) => {
                let doc = self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .find(|d| d.path.as_ref() == Some(&path))
                    .context("Document closed while resolving its path")?;
                Ok(serde_json::json!({"document":doc.id}))
            }
            Output::Loaded(mut doc) => {
                if let Some(existing) = self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .find(|d| d.path == doc.path)
                {
                    return Ok(serde_json::json!({"document":existing.id}));
                }
                if self.hidden_documents.len() >= MAX_HIDDEN
                    || self
                        .documents
                        .iter()
                        .chain(&self.hidden_documents)
                        .map(|d| d.text.len_bytes())
                        .sum::<usize>()
                        + doc.text.len_bytes()
                        > MAX_BYTES
                {
                    bail!("Hidden document or mirror budget changed; retry");
                }
                self.settings.apply(&mut doc);
                self.configure_document_language(&mut doc)?;
                let id = doc.id;
                self.hidden_documents.push(*doc);
                Ok(serde_json::json!({"document":id}))
            }
        }
    }
    /// Show is staged jointly with bounded native membership/layout changes.
    /// `{view}` is private receipt metadata; the gateway replaces it with the
    /// published opaque editor identity after mirroring, never before.
    fn show_extension_document(
        &mut self,
        host: &crate::extensions::Client,
        show: &crate::extensions::services::Show,
    ) -> Result<Value> {
        use crate::editor_groups::Change;
        use crate::editor_layout::Direction;
        if show.preview == Some(true) {
            bail!("Preview showTextDocument is not supported by this native backend yet");
        }
        if self.group_fallback {
            bail!(
                "Native editor views are unavailable while recovered buffers exceed the group limit"
            );
        }
        if show
            .view_column
            .is_some_and(|column| !matches!(column, -2 | -1 | 1..=4))
        {
            bail!("Native viewColumn requires Active, Beside or columns 1–4");
        }
        let doc = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .find(|doc| doc.id == show.document)
            .context("Document is no longer open")?;
        if !host.service_document_current(doc, show.version) {
            bail!("Document changed before showTextDocument; retry");
        }
        let selection = show
            .selection
            .as_ref()
            .map(|range| -> Result<_> {
                let start = crate::lsp::offset(doc, range.start)?;
                let end = crate::lsp::offset(doc, range.end)?;
                if start > end {
                    bail!("Selection range is reversed");
                }
                Ok(crate::document::Selection {
                    anchor: Some(start),
                    cursor: end,
                    desired_column: None,
                })
            })
            .transpose()?;
        let previous_group = self.editor_groups.active_group();
        let previous_focus = self.focus.clone();
        let mut groups = self.editor_groups.clone();
        let mut change = Change::default();
        let mut topology = None;
        let target = if groups.groups().is_empty() {
            if show.view_column.is_some_and(|column| column > 1) {
                bail!("Sparse native viewColumn requires unsupported empty intermediate groups");
            }
            let opened = groups.open(show.document)?;
            let target = opened
                .active
                .context("Native show did not admit an editor")?
                .group;
            topology = Some(opened.clone());
            change = opened;
            target
        } else {
            let active = groups
                .active_group()
                .context("Native active group is unavailable")?;
            let (existing, source) = match show.view_column.unwrap_or(-1) {
                -1 => (Some(active), active),
                -2 => (self.native_show_right_group(active)?, active),
                column => {
                    let ordinal = column as usize - 1;
                    if ordinal > groups.groups().len() {
                        bail!(
                            "Sparse native viewColumn requires unsupported empty intermediate groups"
                        );
                    }
                    (
                        groups.groups().get(ordinal).map(|group| group.id()),
                        groups.groups().last().expect("nonempty groups").id(),
                    )
                }
            };
            let target = if let Some(existing) = existing {
                existing
            } else {
                groups.focus_group(source)?;
                let split = groups.split_active()?;
                let target = split
                    .active
                    .context("Native side group was not created")?
                    .group;
                topology = Some(split.clone());
                change.created_groups = split.created_groups.clone();
                change.changed = true;
                let temporary = split.active.expect("validated split");
                let opened = groups.open_in_group(target, show.document)?;
                change.inserted.extend(opened.inserted);
                change.promoted.extend(opened.promoted);
                if temporary.document == show.document {
                    change.inserted.push(temporary);
                } else {
                    // The copied editor exists only in staged Groups. Removing
                    // it here never closes a live source view or dirty model.
                    groups.close(temporary)?;
                }
                target
            };
            if topology.is_none() {
                let opened = groups.open_in_group(target, show.document)?;
                change = opened;
            }
            target
        };
        if show.preserve_focus
            && let Some(previous) = previous_group
        {
            let focused = groups.focus_group(previous)?;
            change.changed |= focused.changed;
        }
        change.active = groups.active_membership();
        // Showing in a new group is an ordinary open, not the explicit Split
        // command's source-view copy operation. Shared new views start at origin.
        if topology.is_some() {
            change.previous = None;
        }
        let layout = self.prepare_group_layout(
            &groups,
            topology.as_ref().unwrap_or(&change),
            topology
                .as_ref()
                .and_then(|change| change.previous.map(|_| Direction::Right)),
        )?;
        let hidden = self
            .hidden_documents
            .iter()
            .position(|doc| doc.id == show.document);
        if hidden.is_some() {
            self.documents
                .try_reserve(1)
                .context("Cannot reserve shown model storage")?;
        }
        let doc = self
            .documents
            .iter_mut()
            .chain(&mut self.hidden_documents)
            .find(|doc| doc.id == show.document)
            .expect("validated retained model");
        doc.reserve_editor_view(target.value())?;
        // All Result-returning admission is complete before ownership moves.
        let old_view = doc.editor_view_stamp().0;
        doc.display_view(target.value());
        doc.display_view(old_view);
        self.publish_group_layout(groups, layout);
        if let Some(hidden) = hidden {
            self.documents.push(self.hidden_documents.remove(hidden));
        }
        self.apply_group_change(change);
        if let Some(selection) = selection {
            let doc = self
                .documents
                .iter_mut()
                .find(|doc| doc.id == show.document)
                .expect("published shown model");
            let view = doc
                .editor_view_identity_for(target.value())
                .expect("prepared target view");
            doc.set_editor_selections(&view, vec![selection])
                .expect("prevalidated view and selection");
        }
        self.focus = if show.preserve_focus {
            previous_focus
        } else {
            Focus::Editor
        };
        self.remember_active_file();
        Ok(serde_json::json!({"document":show.document,"view":target.value()}))
    }
    fn native_show_right_group(
        &self,
        active: crate::editor_groups::GroupId,
    ) -> Result<Option<crate::editor_groups::GroupId>> {
        // Bounded logical geometry chooses the nearest right-hand group. This
        // is not a rendered hit map and does not access source/editor content.
        let geometry = self
            .editor_layout
            .project(Rect::new(0, 0, 4096, 4096), Some(active))?;
        let origin = geometry
            .placement(active)
            .context("Native active group geometry unavailable")?
            .outer;
        Ok(geometry
            .placements()
            .iter()
            .flatten()
            .filter(|candidate| {
                candidate.group != active
                    && candidate.outer.x >= origin.right()
                    && candidate.outer.y < origin.bottom()
                    && candidate.outer.bottom() > origin.y
            })
            .min_by_key(|candidate| {
                (
                    candidate.outer.x - origin.right(),
                    (u32::from(candidate.outer.y) * 2 + u32::from(candidate.outer.height))
                        .abs_diff(u32::from(origin.y) * 2 + u32::from(origin.height)),
                )
            })
            .map(|candidate| candidate.group))
    }
    fn answer_native_service(
        &mut self,
        host: &mut crate::extensions::Client,
        service: &NativeService,
        result: Result<Value>,
    ) -> bool {
        self.extension_services
            .contexts
            .remove(&(service.request.session, service.id.clone()));
        if service.request.session != host.session {
            return false;
        }
        self.refresh_extension_editor_inventory();
        host.set_editor_inventory(self.extension_editor_inventory.clone());
        host.set_editor_tabs(self.extension_editor_tabs.clone());
        // The full authoritative mirror is queued before the promise settles.
        let result = result.and_then(|value| {
            host.sync_with_hidden(&self.documents, &self.hidden_documents, self.active)?;
            Ok(value)
        });
        if let Err(error) = host.answer_service(service, result) {
            self.message = format!("Native extension service reply failed: {error:#}");
            false
        } else {
            true
        }
    }
    fn poll_shown_editor_completion(&mut self, host: &mut crate::extensions::Client) -> bool {
        let Some(done) = self.extension_services.shown_completed.take() else {
            return false;
        };
        if done.service.request.session != host.session
            || !host
                .service()
                .is_some_and(|pending| pending.id == done.service.id)
        {
            return true;
        }
        self.refresh_extension_editor_inventory();
        host.set_editor_inventory(self.extension_editor_inventory.clone());
        host.set_editor_tabs(self.extension_editor_tabs.clone());
        if let Err(error) =
            host.sync_with_hidden(&self.documents, &self.hidden_documents, self.active)
        {
            self.message = format!("Shown editor mirror receipt retained: {error:#}");
            self.extension_services.shown_completed = Some(done);
            return false;
        }
        let result = host
            .completed_show_owner_valid(&done.service)
            .and_then(|()| {
                let value = done.result.clone().map_err(anyhow::Error::msg)?;
                if let Some(editor) = value["editor"].as_str() {
                    let document = value["document"]
                        .as_u64()
                        .context("Invalid shown document receipt")?;
                    let doc = self
                        .documents
                        .iter()
                        .chain(&self.hidden_documents)
                        .find(|doc| doc.id == document)
                        .context("Original shown model retired")?;
                    let proof = host.editor_view_source(editor, doc)?;
                    anyhow::ensure!(
                        host.editor_id_for_document(document, proof.view_id())
                            .as_deref()
                            == Some(editor),
                        "Original shown editor retired before acknowledgement"
                    );
                }
                Ok(value)
            });
        if let Err(error) = host.answer_service(&done.service, result) {
            self.message = format!("Shown editor acknowledgement retained: {error:#}");
            self.extension_services.shown_completed = Some(done);
            return false;
        }
        self.extension_services
            .contexts
            .remove(&(done.service.request.session, done.service.id));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{Duration, Instant};
    #[test]
    fn customer_read_context_ignores_other_models_but_retires_original_resource_aba_and_profile() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        let path = root.path().join("original.txt");
        std::fs::write(&path, "original 猫🙂\r\n").unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        app.open(&path).unwrap();
        let context = Context::capture(&app);
        app.hidden_documents.push(Document::from_text("unrelated"));
        assert!(context.customer_open_current(&app));
        assert!(context.customer_existing_current(&app, &path));
        app.doc_mut()
            .set_resource_path(Some(root.path().join("other.txt")))
            .unwrap();
        app.doc_mut().set_resource_path(Some(path.clone())).unwrap();
        assert!(!context.customer_existing_current(&app, &path));
        app.configure_settings(None).unwrap();
        assert!(!context.customer_open_current(&app));
    }
    fn until(app: &mut App, check: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            app.poll();
            if check(app) {
                return;
            }
            assert!(Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn fixture(root: &Path) -> PathBuf {
        let folder = root.join("extension");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("package.json"),r#"{"publisher":"test","name":"documents","version":"1.0.0","main":"index.cjs","contributes":{"commands":[{"command":"documents.show","title":"Show hidden model"},{"command":"documents.edit","title":"Edit native model"},{"command":"documents.state","title":"Update state"}]}}"#).unwrap();
        std::fs::write(folder.join("index.cjs"),r#"
const vscode = require('vscode'); const assert = require('node:assert/strict');
exports.activate = async context => {
  let opened; vscode.workspace.onDidOpenTextDocument(doc=>{opened=doc; assert.equal(vscode.window.activeTextEditor,undefined);});
  const doc = await vscode.workspace.openTextDocument(vscode.Uri.file(require('node:path').join(vscode.workspace.rootPath,'input.txt')));
  assert.equal(opened,doc); assert.equal(vscode.window.activeTextEditor,undefined); assert.deepEqual(vscode.window.visibleTextEditors,[]);
  assert.equal(doc.getText(),'α😀\r\nsecond');
  await context.globalState.update('launches',context.globalState.get('launches',0)+1);
  vscode.commands.registerCommand('documents.show',async()=>{
    const editor=await vscode.window.showTextDocument(doc,{preview:false,selection:new vscode.Range(0,1,0,3)});
    assert.equal(editor.document,doc); assert.equal(vscode.window.activeTextEditor,editor); assert.equal(editor.selection.end.character,3);
    await vscode.window.showInformationMessage('shown same identity');
  });
  vscode.commands.registerCommand('documents.edit',async()=>{
    const editor=vscode.window.activeTextEditor;
    assert(await editor.edit(edit=>edit.insert(new vscode.Position(1,0),'changed ')));
    await vscode.commands.executeCommand('undo'); assert.equal(doc.getText(),'α😀\r\nsecond');
    await vscode.commands.executeCommand('redo'); assert.equal(doc.getText(),'α😀\r\nchanged second');
    await vscode.window.showInformationMessage('native undo redo mirrored');
  });
  vscode.commands.registerCommand('documents.unawaited',()=>{
    void context.globalState.update('unawaited','persisted');
    void vscode.window.showInformationMessage('returned unawaited update');
  });
  vscode.commands.registerCommand('documents.state',async()=>{
    await Promise.all([context.globalState.update('__proto__',null),context.workspaceState.update('workspace','local')]);
    assert(context.globalState.keys().includes('__proto__')); assert.equal(context.globalState.get('__proto__'),null);
    await vscode.window.showInformationMessage('state '+context.globalState.get('launches'));
  });
};
"#).unwrap();
        folder
    }
    #[test]
    fn activation_opens_hidden_then_shows_same_model_native_undo_and_persistent_state() {
        let root = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("input.txt"), "α😀\r\nsecond").unwrap();
        let package = crate::extensions::Package::read(&fixture(root.path())).unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.configure_extension_storage(Some(config.path()));
        app.start_extension_packages(vec![package]).unwrap();
        until(&mut app, |app| {
            app.extension_host.as_ref().is_some_and(|host| host.ready)
        });
        assert!(app.documents.is_empty());
        assert!(app.active_document().is_none());
        assert!(app.panes.is_empty());
        assert_eq!(app.hidden_documents.len(), 1);
        let id = app.hidden_documents[0].id;
        app.execute("documents.show", Value::Null);
        until(&mut app, |app| app.message.contains("shown same identity"));
        assert_eq!(app.doc().id, id);
        assert!(app.hidden_documents.is_empty());
        assert_eq!(app.doc().selections().len(), 1);
        assert_eq!(app.doc().selected_text().as_deref(), Some("😀"));
        app.execute("documents.edit", Value::Null);
        until(&mut app, |app| {
            app.message.contains("native undo redo mirrored")
        });
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), "α😀\r\nchanged second");
        assert_eq!(
            std::fs::read_to_string(root.path().join("input.txt")).unwrap(),
            "α😀\r\nsecond"
        );
        app.execute("documents.state", Value::Null);
        until(&mut app, |app| app.message.contains("state 1"));
        app.execute("documents.unawaited", Value::Null);
        until(&mut app, |app| {
            app.message.contains("returned unawaited update")
                && app.extension_host.as_ref().is_some_and(|host| !host.busy())
                && app.extension_services.job.is_none()
        });
        let store = crate::extension_state::Store::new(config.path(), root.path()).unwrap();
        assert_eq!(
            store
                .read("test.documents", crate::extension_state::Scope::Global)
                .unwrap()["unawaited"],
            "persisted"
        );
        assert_eq!(
            store
                .read("test.documents", crate::extension_state::Scope::Global)
                .unwrap()["launches"],
            1
        );
        assert_eq!(
            store
                .read("test.documents", crate::extension_state::Scope::Global)
                .unwrap()["__proto__"],
            Value::Null
        );
        assert_eq!(
            store
                .read("test.documents", crate::extension_state::Scope::Workspace)
                .unwrap()["workspace"],
            "local"
        );
    }
    #[test]
    fn show_validates_utf16_before_visibility_and_preserves_other_shared_view_selections() {
        let root = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.hidden_documents
            .push(Document::from_text("α😀\r\nsecond"));
        let id = app.hidden_documents[0].id;
        let prepared = crate::extensions::Client::prepare_with_hidden(
            &[],
            &app.hidden_documents,
            0,
            &app.settings,
        )
        .unwrap();
        let mut host =
            crate::extensions::Client::start_prepared("node", &extension, root.path(), prepared)
                .unwrap();
        let owner = host.packages[0].id.clone();
        host.queue_service("nativeDocumentShow",json!("bad-range"),json!({"session":host.session,"owner":owner,"generation":1,"args":{"document":id,"version":1,"selection":{"start":{"line":0,"character":2},"end":{"line":0,"character":3}}}})).unwrap();
        app.extension_host = Some(host);
        app.poll_extension_services();
        assert!(app.documents.is_empty());
        assert_eq!(app.hidden_documents[0].id, id);
        let doc = app.hidden_documents.remove(0);
        app.install_open_document(doc).unwrap();
        app.doc_mut().move_to(1, false);
        app.split_editor(false);
        app.doc_mut().move_to(5, false);
        app.doc_mut().secondary.push(crate::document::Selection {
            cursor: 7,
            anchor: None,
            desired_column: None,
        });
        let first_pane = app.panes[0].id;
        let second_pane = app.panes[1].id;
        let first_cursor = app.doc().view_state(Some(first_pane)).cursor;
        let mut host = app.extension_host.take().unwrap();
        host.sync_with_hidden(&app.documents, &[], app.active)
            .unwrap();
        // Call the validated application stage directly: transport queue versions
        // and generation rejection are covered separately in service tests.
        app.show_extension_document(
            &host,
            &crate::extensions::services::Show {
                document: id,
                version: 1,
                view_column: None,
                preserve_focus: false,
                preview: None,
                selection: Some(crate::lsp::Range {
                    start: crate::lsp::Position {
                        line: 0,
                        character: 1,
                    },
                    end: crate::lsp::Position {
                        line: 0,
                        character: 3,
                    },
                }),
            },
        )
        .unwrap();
        assert_eq!(app.panes[app.active_pane].id, second_pane);
        assert_eq!(app.doc().selected_text().as_deref(), Some("😀"));
        assert!(app.doc().secondary.is_empty());
        assert_eq!(app.doc().view_state(Some(first_pane)).cursor, first_cursor);
        assert_eq!(app.doc().text.to_string(), "α😀\r\nsecond");
        app.extension_host = Some(host);
    }
    #[test]
    fn native_show_beside_preserves_focus_shared_dirty_identity_and_redo() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("a.txt");
        std::fs::write(&file, "α😀\r\nsecond").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.install_open_document(Document::open_existing(&file).unwrap())
            .unwrap();
        let id = app.doc().id;
        let end = app.doc().len();
        app.doc_mut().move_to(end, false);
        app.doc_mut().insert("!", false);
        app.doc_mut().move_to(0, false);
        app.doc_mut().insert("x", false);
        app.doc_mut().undo();
        app.doc_mut().set_selections(vec![
            crate::document::Selection::caret(3),
            crate::document::Selection::caret(6),
        ]);
        let source_group = app.panes[0].id;
        let source_selections = app.doc().selections();
        let source = (
            app.doc().text.to_string(),
            app.doc().revision,
            app.doc().text_epoch(),
            app.doc().saved_revision,
            app.doc().save_generation(),
            app.doc().dirty(),
        );
        app.focus = Focus::Explorer;
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let prepared = crate::extensions::Client::prepare_with_hidden(
            &app.documents,
            &app.hidden_documents,
            app.active,
            &app.settings,
        )
        .unwrap();
        let host =
            crate::extensions::Client::start_prepared("node", &extension, root.path(), prepared)
                .unwrap();
        let show = crate::extensions::services::Show {
            document: id,
            version: 1,
            selection: Some(crate::lsp::Range {
                start: crate::lsp::Position {
                    line: 0,
                    character: 1,
                },
                end: crate::lsp::Position {
                    line: 0,
                    character: 3,
                },
            }),
            view_column: Some(-2),
            preserve_focus: true,
            preview: Some(false),
        };
        let result = app.show_extension_document(&host, &show).unwrap();
        let target = result["view"].as_u64().unwrap();
        assert_ne!(target, source_group);
        assert_eq!(app.panes.len(), 2);
        assert_eq!(app.documents.len(), 1);
        assert!(app.hidden_documents.is_empty());
        assert_eq!(app.panes[app.active_pane].id, source_group);
        assert!(matches!(app.focus, Focus::Explorer));
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().selections(), source_selections);
        let target_view = app.doc().view_state(Some(target));
        assert_eq!((target_view.anchor, target_view.cursor), (Some(1), 2));
        assert!(target_view.secondary.is_empty());
        assert_eq!(
            (
                app.doc().text.to_string(),
                app.doc().revision,
                app.doc().text_epoch(),
                app.doc().saved_revision,
                app.doc().save_generation(),
                app.doc().dirty()
            ),
            source
        );
        // Beside reuses the existing right-hand group, rather than adding a third.
        let repeat = app.show_extension_document(&host, &show).unwrap();
        assert_eq!(repeat["view"], target);
        assert_eq!(app.panes.len(), 2);
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), format!("x{}", source.0));
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), source.0);
        assert_eq!(std::fs::read_to_string(file).unwrap(), "α😀\r\nsecond");
    }
    #[test]
    fn show_hidden_model_column_and_failed_admissions_are_atomic() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.install_open_document(Document::from_text("first"))
            .unwrap();
        let first = app.doc().id;
        app.doc_mut().move_to(3, false);
        app.hidden_documents
            .push(Document::from_text("α😀\r\nsecond"));
        let hidden = app.hidden_documents[0].id;
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let prepared = crate::extensions::Client::prepare_with_hidden(
            &app.documents,
            &app.hidden_documents,
            app.active,
            &app.settings,
        )
        .unwrap();
        let host =
            crate::extensions::Client::start_prepared("node", &extension, root.path(), prepared)
                .unwrap();
        let original = app.editor_groups.proof();
        let first_selections = app.doc().selections();
        let mut show = crate::extensions::services::Show {
            document: hidden,
            version: 1,
            selection: None,
            view_column: Some(4),
            preserve_focus: false,
            preview: Some(false),
        };
        for (column, preview) in [
            (Some(4), Some(false)),
            (Some(0), Some(false)),
            (Some(2), Some(true)),
        ] {
            show.view_column = column;
            show.preview = preview;
            assert!(app.show_extension_document(&host, &show).is_err());
            assert!(app.editor_groups.proof_current(&original));
            assert_eq!(app.documents.len(), 1);
            assert_eq!(app.hidden_documents.len(), 1);
            assert_eq!(app.doc().id, first);
            assert_eq!(app.doc().selections(), first_selections);
        }
        show.view_column = Some(2);
        show.preview = Some(false);
        show.preserve_focus = true;
        let result = app.show_extension_document(&host, &show).unwrap();
        assert_eq!(app.panes.len(), 2);
        assert_eq!(app.documents.len(), 2);
        assert!(app.hidden_documents.is_empty());
        assert_eq!(app.doc().id, first);
        assert_eq!(app.doc().selections(), first_selections);
        let target = app.documents.iter().find(|doc| doc.id == hidden).unwrap();
        assert_eq!(
            target
                .editor_view_identity_for(result["view"].as_u64().unwrap())
                .unwrap()
                .view_id(),
            result["view"].as_u64().unwrap()
        );
        assert_eq!(target.text.to_string(), "α😀\r\nsecond");
        assert!(!target.dirty());
        let original = app.editor_groups.proof();
        show.view_column = Some(1);
        show.selection = Some(crate::lsp::Range {
            start: crate::lsp::Position {
                line: 0,
                character: 2,
            },
            end: crate::lsp::Position {
                line: 0,
                character: 3,
            },
        });
        assert!(app.show_extension_document(&host, &show).is_err());
        assert!(app.editor_groups.proof_current(&original));
        assert_eq!(app.doc().id, first);
        assert_eq!(app.doc().selections(), first_selections);
    }
    fn paused_shown_editor_receipt() -> (tempfile::TempDir, App) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("receipt.txt");
        std::fs::write(&path, "α😀\r\nsecond\r\n").unwrap();
        let extension = root.path().join("receipt-extension");
        std::fs::create_dir(&extension).unwrap();
        std::fs::write(extension.join("package.json"), r#"{"publisher":"test","name":"show-receipt","version":"1.0.0","main":"extension.cjs","contributes":{"commands":[{"command":"receipt.show","title":"Show exact receipt"}]}}"#).unwrap();
        std::fs::write(
            extension.join("extension.cjs"),
            r#"
const vscode = require('vscode');
// This fixture pauses a committed receipt behind the mirror budget. Its
// positive identity checks must not depend on loaded CI reaching the next
// poll within the separately tested production acknowledgement timeout.
const realSetTimeout = globalThis.setTimeout;
globalThis.setTimeout = (callback, delay, ...args) => realSetTimeout(callback, delay === 5000 ? 60000 : delay, ...args);
exports.activate = context => {
  context.subscriptions.push(vscode.commands.registerCommand('receipt.show', async () => {
    const original = vscode.window.activeTextEditor.document;
    try {
      const editor = await vscode.window.showTextDocument(original, {
        viewColumn: vscode.ViewColumn.Beside, preserveFocus: true, preview: false,
      });
      if (editor.document !== original) throw new Error('Shared model identity changed');
      await vscode.window.showInformationMessage('receipt-ack column=' + editor.viewColumn);
    } catch (error) {
      await vscode.window.showInformationMessage('receipt-rejected ' + error.message);
    }
  }));
};
"#,
        )
        .unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.install_open_document(Document::open_existing(&path).unwrap())
            .unwrap();
        let end = app.doc().len();
        app.doc_mut().move_to(end, false);
        app.doc_mut().insert("!", false);
        app.doc_mut().move_to(0, false);
        app.doc_mut().insert("x", false);
        app.doc_mut().undo();
        // The initial model and its one visible pane each have 8,192
        // selections. Admission is exactly valid; a new origin pane adds one
        // selection and pauses only the post-Show mirror, after publication.
        app.doc_mut().secondary = vec![crate::document::Selection::caret(0); 8191];
        Context::check_budget(&app).unwrap();
        app.start_extension_packages(vec![crate::extensions::Package::read(&extension).unwrap()])
            .unwrap();
        until(&mut app, |app| {
            app.extension_host.as_ref().is_some_and(|host| host.ready)
        });
        app.execute("receipt.show", Value::Null);
        until(&mut app, |app| {
            app.extension_services.shown_completed.is_some()
        });
        assert_eq!(app.panes.len(), 2);
        assert_eq!(app.documents.len(), 1);
        assert!(app.hidden_documents.is_empty());
        assert!(Context::check_budget(&app).is_err());
        assert!(app.message.contains("Shown editor mirror receipt retained"));
        let receipt = app.extension_services.shown_completed.as_ref().unwrap();
        assert!(receipt.result.as_ref().unwrap()["editor"].is_string());
        assert!(
            app.extension_host
                .as_ref()
                .unwrap()
                .editor_inventory_explicit()
        );
        assert_eq!(
            app.extension_host.as_ref().unwrap().service().unwrap().id,
            receipt.service.id
        );
        (root, app)
    }
    fn shown_receipt_source(app: &App) -> (u64, String, u64, u64, u64, u64, bool) {
        let doc = &app.documents[0];
        (
            doc.id,
            doc.text.to_string(),
            doc.revision,
            doc.text_epoch(),
            doc.saved_revision,
            doc.save_generation(),
            doc.dirty(),
        )
    }
    #[test]
    fn retained_show_receipt_resumes_mirror_without_replaying_native_show() {
        let (root, mut app) = paused_shown_editor_receipt();
        let source = shown_receipt_source(&app);
        let group_proof = app.editor_groups.proof();
        let receipt = app.extension_services.shown_completed.as_ref().unwrap();
        let original_editor = receipt.result.as_ref().unwrap()["editor"]
            .as_str()
            .unwrap()
            .to_owned();
        let request = receipt.service.id.clone();
        for _ in 0..3 {
            app.poll_extension_services();
            assert!(app.editor_groups.proof_current(&group_proof));
            assert_eq!(app.panes.len(), 2);
            let receipt = app.extension_services.shown_completed.as_ref().unwrap();
            assert_eq!(receipt.service.id, request);
            assert_eq!(receipt.result.as_ref().unwrap()["editor"], original_editor);
            assert_eq!(shown_receipt_source(&app), source);
        }
        // This is an intentional user selection reduction, not a source edit.
        app.doc_mut().clear_secondary();
        until(&mut app, |app| app.message == "receipt-ack column=2");
        assert!(app.extension_services.shown_completed.is_none());
        assert!(app.extension_host.as_ref().unwrap().service().is_none());
        assert!(app.editor_groups.proof_current(&group_proof));
        assert_eq!(app.panes.len(), 2);
        let target = app.panes[1].id;
        assert_eq!(
            app.extension_host
                .as_ref()
                .unwrap()
                .editor_id_for_document(source.0, target),
            Some(original_editor)
        );
        assert_eq!(shown_receipt_source(&app), source);
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), format!("x{}", source.1));
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), source.1);
        assert_eq!(
            std::fs::read_to_string(root.path().join("receipt.txt")).unwrap(),
            "α😀\r\nsecond\r\n"
        );
    }
    #[test]
    fn retained_show_receipt_visibility_aba_rejects_replacement_editor_acknowledgement() {
        let (root, mut app) = paused_shown_editor_receipt();
        let source = shown_receipt_source(&app);
        let original_editor = app
            .extension_services
            .shown_completed
            .as_ref()
            .unwrap()
            .result
            .as_ref()
            .unwrap()["editor"]
            .as_str()
            .unwrap()
            .to_owned();
        let original_target = app
            .editor_groups
            .memberships(source.0)
            .find(|member| member.group.value() == app.panes[1].id)
            .unwrap();
        // These real membership transitions are deliberately unpolled: the
        // App-owned inventory must retire A's handle even when mirrors coalesce.
        app.focus_pane(1);
        app.install_open_document(Document::from_text("other猫\r\n"))
            .unwrap();
        let other = app.doc().id;
        app.focus_tab(original_target).unwrap();
        app.focus_pane(0);
        let replacement = app
            .extension_editor_inventory
            .id_for_view(source.0, original_target.group.value())
            .unwrap();
        assert_ne!(replacement, original_editor);
        app.doc_mut().clear_secondary();
        let group_proof = app.editor_groups.proof();
        until(&mut app, |app| app.message.contains("receipt-rejected"));
        assert!(
            app.message
                .contains("Original editor is no longer published"),
            "{}",
            app.message
        );
        assert!(app.extension_services.shown_completed.is_none());
        assert!(app.extension_host.as_ref().unwrap().service().is_none());
        assert!(app.editor_groups.proof_current(&group_proof));
        assert_eq!(app.panes.len(), 2);
        assert_eq!(app.documents.len(), 2);
        assert_eq!(
            app.extension_host
                .as_ref()
                .unwrap()
                .editor_id_for_document(source.0, original_target.group.value()),
            Some(replacement)
        );
        assert_eq!(shown_receipt_source(&app), source);
        assert_eq!(
            app.documents
                .iter()
                .find(|doc| doc.id == other)
                .unwrap()
                .text
                .to_string(),
            "other猫\r\n"
        );
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), format!("x{}", source.1));
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), source.1);
        assert_eq!(
            std::fs::read_to_string(root.path().join("receipt.txt")).unwrap(),
            "α😀\r\nsecond\r\n"
        );
    }
    #[test]
    fn hidden_dirty_recovery_preserves_native_undo_baseline_without_clean_hidden_models() {
        let root = tempfile::tempdir().unwrap();
        let recovery = tempfile::tempdir().unwrap();
        let file = root.path().join("hidden.txt");
        std::fs::write(&file, "original").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        let mut dirty = Document::open_existing(&file).unwrap();
        dirty.insert("dirty ", false);
        app.hidden_documents.push(dirty);
        app.hidden_documents
            .push(Document::from_text("clean hidden"));
        assert!(app.documents.is_empty());
        assert_eq!(app.recovery_documents().len(), 1);
        let worker = crate::recovery::Worker::start(
            crate::recovery::Recovery::new(recovery.path().into()).unwrap(),
        )
        .unwrap();
        worker.preserve_refs(&app.recovery_documents()).unwrap();
        let store = crate::recovery::Recovery::new(recovery.path().into()).unwrap();
        let (mut docs, _) = store.restore().unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].text.to_string(), "dirty original");
        assert!(docs[0].dirty());
        docs[0].undo();
        assert_eq!(docs[0].text.to_string(), "original");
        assert_eq!(std::fs::read_to_string(file).unwrap(), "original");
    }
    #[test]
    fn stopping_a_host_cancels_accepted_state_before_commit_and_preserves_prior_payload() {
        let root = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let mut app = App::new(root.path().into(), Profile::Linux);
        let prepared = crate::extensions::Client::prepare(&[], 0, &app.settings)
            .unwrap()
            .with_storage_root(Some(config.path().into()));
        let mut host =
            crate::extensions::Client::start_prepared("node", &extension, root.path(), prepared)
                .unwrap();
        let owner = host.packages[0].id.clone();
        let store = host.state_store().unwrap();
        store
            .update(
                &owner,
                crate::extension_state::Scope::Global,
                "retained",
                Some(json!(1)),
            )
            .unwrap();
        let owner_path = std::fs::read_dir(config.path().join("state/extension-storage"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let payload = owner_path.join("global.json");
        let before = std::fs::read(&payload).unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(owner_path.join("global.lock"))
            .unwrap();
        lock.lock().unwrap();
        host.queue_service("nativeStateWrite",json!("pending-write"),json!({"session":host.session,"owner":owner,"command":1,"commandOwner":owner,"args":{"scope":"global","key":"retained","remove":false,"value":2}})).unwrap();
        app.extension_host = Some(host);
        app.poll_extension_services();
        assert!(app.extension_services.job.is_some());
        app.stop_extension_host();
        assert!(app.extension_services.job.is_some());
        drop(lock);
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.extension_services.job.is_some() {
            app.poll_extension_services();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(std::fs::read(payload).unwrap(), before);
        assert_eq!(
            store
                .read(&owner, crate::extension_state::Scope::Global)
                .unwrap()["retained"],
            1
        );
    }
    #[test]
    fn queued_show_keeps_receipt_context_while_waiting_behind_state_commit() {
        let root = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.hidden_documents.push(Document::from_text("retained"));
        let id = app.hidden_documents[0].id;
        let prepared = crate::extensions::Client::prepare_with_hidden(
            &[],
            &app.hidden_documents,
            0,
            &app.settings,
        )
        .unwrap()
        .with_storage_root(Some(config.path().into()));
        let mut host =
            crate::extensions::Client::start_prepared("node", &extension, root.path(), prepared)
                .unwrap();
        let owner = host.packages[0].id.clone();
        let owner_path = std::fs::read_dir(config.path().join("state/extension-storage"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(owner_path.join("global.lock"))
            .unwrap();
        lock.lock().unwrap();
        host.queue_service("nativeStateWrite",json!("first-state"),json!({"session":host.session,"owner":owner,"args":{"scope":"global","key":"first","remove":false,"value":true}})).unwrap();
        host.queue_service("nativeDocumentShow",json!("queued-show"),json!({"session":host.session,"owner":owner,"generation":1,"args":{"document":id,"version":1,"selection":null}})).unwrap();
        app.extension_host = Some(host);
        app.poll_extension_services();
        assert!(app.extension_services.job.is_some());
        app.focus = Focus::Explorer;
        drop(lock);
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.extension_services.job.is_some() {
            app.poll_extension_services();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        app.poll_extension_services();
        assert!(app.documents.is_empty());
        assert_eq!(app.hidden_documents[0].id, id);
        assert!(app.focus == Focus::Explorer);
        assert!(app.extension_host.as_ref().unwrap().service().is_none());
        let store = crate::extension_state::Store::new(config.path(), root.path()).unwrap();
        assert_eq!(
            store
                .read(&owner, crate::extension_state::Scope::Global)
                .unwrap()["first"],
            true
        );
    }
    #[test]
    fn over_budget_queued_services_cannot_rebind_after_held_worker_and_selection_reduction() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("dirty.txt");
        let unopened = root.path().join("unopened.txt");
        std::fs::write(&path, "original猫\r\n").unwrap();
        std::fs::write(&unopened, "must stay unopened").unwrap();
        let mut dirty = Document::open_existing(&path).unwrap();
        dirty.insert("dirty ", false);
        dirty.secondary = vec![crate::document::Selection::caret(0); 8192];
        let id = dirty.id;
        let revision = dirty.revision;
        let text = dirty.text.to_string();
        let mut other = Document::default();
        other.secondary = vec![crate::document::Selection::caret(0); 8190];
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.hidden_documents = vec![dirty, other];
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let prepared = crate::extensions::Client::prepare_with_hidden(
            &[],
            &app.hidden_documents,
            0,
            &app.settings,
        )
        .unwrap();
        let mut host =
            crate::extensions::Client::start_prepared("node", &extension, root.path(), prepared)
                .unwrap();
        app.hidden_documents[1]
            .secondary
            .push(crate::document::Selection::caret(0));
        let owner = host.packages[0].id.clone();
        let session = host.session;
        host.queue_service("nativeStateWrite",json!("held-state"),json!({"session":session,"owner":owner,"args":{"scope":"global","key":"first","remove":false,"value":true}})).unwrap();
        host.queue_service(
            "nativeDocumentOpen",
            json!("queued-open"),
            json!({"session":session,"owner":owner,"generation":1,"args":{"path":unopened}}),
        )
        .unwrap();
        host.queue_service("nativeDocumentShow",json!("queued-show"),json!({"session":session,"owner":owner,"generation":1,"args":{"document":id,"version":1,"selection":null}})).unwrap();
        // Hold the existing worker's completion deterministically: the queue
        // must record both document contexts even before its reply can arrive.
        let (sender, receiver) = mpsc::sync_channel(1);
        app.extension_services.job = Some(Job {
            service: host.service().unwrap().clone(),
            context: None,
            cancel: Arc::new(AtomicBool::new(false)),
            receiver,
            thread: None,
        });
        app.extension_host = Some(host);
        assert!(Context::check_budget(&app).is_err());
        app.poll_extension_services();
        assert_eq!(app.extension_services.contexts.len(), 2);
        assert!(
            app.extension_services
                .contexts
                .values()
                .all(|context| context.snapshot.is_none())
        );
        for doc in &mut app.hidden_documents {
            doc.secondary.clear();
        }
        Context::check_budget(&app).unwrap();
        app.poll_extension_services();
        assert!(
            app.extension_services
                .contexts
                .values()
                .all(|context| context.snapshot.is_none())
        );
        assert!(app.extension_services.job.is_some());
        sender
            .send(Ok(Output::State(json!({"first":true}))))
            .unwrap();
        app.poll_extension_services();
        // Hidden selection changes now also publish new mirror generations.
        // The original over-budget context remains refused independently of
        // that additional wire guard, even after selections fit again.
        for _ in 0..2 {
            let host = app.extension_host.as_ref().unwrap();
            let service = host.service().unwrap();
            assert!(
                app.extension_services.contexts[&(service.request.session, service.id.clone())]
                    .snapshot
                    .is_none()
            );
            app.poll_extension_services();
            assert!(app.extension_services.job.is_none());
            assert!(app.documents.is_empty());
            assert_eq!(app.hidden_documents.len(), 2);
        }
        assert!(app.extension_host.as_ref().unwrap().service().is_none());
        assert!(app.extension_services.contexts.is_empty());
        assert_eq!(app.hidden_documents[0].id, id);
        assert_eq!(app.hidden_documents[0].revision, revision);
        assert_eq!(app.hidden_documents[0].text.to_string(), text);
        assert!(app.hidden_documents[0].dirty());
        app.hidden_documents[0].undo();
        assert_eq!(app.hidden_documents[0].id, id);
        assert_eq!(app.hidden_documents[0].text.to_string(), "original猫\r\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original猫\r\n");
        assert_eq!(
            std::fs::read_to_string(&unopened).unwrap(),
            "must stay unopened"
        );
    }
    #[test]
    fn context_selection_budget_counts_document_and_inactive_pane_snapshots_before_cloning() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("kept.txt");
        std::fs::write(&path, "original猫\r\n").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.open(&path).unwrap();
        app.doc_mut().insert("dirty ", false);
        app.split_editor(false);
        let current = app.panes[app.active_pane].id;
        let other = app.panes.iter().find(|pane| pane.id != current).unwrap().id;
        app.doc_mut().secondary = vec![crate::document::Selection::caret(0); 8190];
        app.hidden_documents.push(Document::default());
        // Document + active pane each retain 8191; hidden + inactive pane each 1.
        Context::check_budget(&app).unwrap();
        let context = Context::capture(&app);
        let snapshot = context.snapshot.as_ref().unwrap();
        assert_eq!(
            snapshot
                .documents
                .iter()
                .map(|doc| doc.selections.len())
                .sum::<usize>()
                + snapshot
                    .pane_selections
                    .iter()
                    .map(|(_, _, secondary)| secondary.len() + 1)
                    .sum::<usize>(),
            MAX_CONTEXT_SELECTIONS
        );
        assert!(context.same_editor(&app));
        let id = app.doc().id;
        let revision = app.doc().revision;
        let text = app.doc().text.to_string();
        app.doc_mut().activate_view(other);
        app.doc_mut()
            .secondary
            .push(crate::document::Selection::caret(0));
        app.doc_mut().activate_view(current);
        assert!(
            Context::check_budget(&app)
                .unwrap_err()
                .to_string()
                .contains("16,384")
        );
        let invalid = Context::capture(&app);
        assert!(
            invalid.snapshot.is_none(),
            "over-budget capture must allocate no snapshot"
        );
        assert!(
            !invalid.same_editor(&app),
            "invalid sentinels cannot authorize one another"
        );
        assert!(!context.same_editor(&app));
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().revision, revision);
        assert_eq!(app.doc().text.to_string(), text);
        assert!(app.doc().dirty());
        app.doc_mut().activate_view(other);
        app.doc_mut().secondary.pop();
        app.doc_mut().activate_view(current);
        assert!(context.same_editor(&app));
        app.doc_mut().undo();
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), "original猫\r\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original猫\r\n");
    }
    #[test]
    fn service_selection_limit_rejects_without_snapshot_worker_or_native_edits() {
        let root = tempfile::tempdir().unwrap();
        let mut app = queued_app(root.path());
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.doc_mut().insert("dirty猫\r\n", false);
        app.doc_mut().secondary = vec![crate::document::Selection::caret(0); 8192];
        let id = app.doc().id;
        let revision = app.doc().revision;
        assert!(
            Context::check_budget(&app)
                .unwrap_err()
                .to_string()
                .contains("16,384")
        );
        app.poll_extension_services();
        assert!(app.extension_services.contexts.is_empty());
        assert!(app.extension_services.job.is_none());
        assert!(app.hidden_documents.is_empty());
        assert!(app.extension_host.as_ref().unwrap().service().is_none());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().revision, revision);
        assert_eq!(app.doc().text.to_string(), "dirty猫\r\n");
        assert!(app.doc().dirty());
        app.doc_mut().undo();
        assert!(app.doc().is_empty());
        assert_eq!(app.doc().id, id);
    }
    #[test]
    fn model_count_limit_rejects_before_capturing_context_or_starting_file_work() {
        let root = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.documents = (0..129).map(|_| Document::default()).collect();
        assert!(Context::capture(&app).snapshot.is_none());
        let mut host = crate::extensions::Client::start(
            "node",
            &extension,
            root.path(),
            &app.documents[..128],
            0,
            &app.settings,
        )
        .unwrap();
        host.queue_service("nativeDocumentOpen",json!("too-many-models"),json!({"session":host.session,"owner":host.packages[0].id,"generation":1,"args":{"path":"input.txt"}})).unwrap();
        app.extension_host = Some(host);
        app.poll_extension_services();
        assert!(app.extension_services.contexts.is_empty());
        assert!(app.extension_services.job.is_none());
        assert!(app.hidden_documents.is_empty());
        assert_eq!(app.documents.len(), 129);
        assert!(app.extension_host.as_ref().unwrap().service().is_none());
    }
    fn queued_app(root: &Path) -> App {
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let mut app = App::new(root.into(), Profile::Linux);
        let mut host =
            crate::extensions::Client::start("node", &extension, root, &[], 0, &app.settings)
                .unwrap();
        let owner = host.packages[0].id.clone();
        host.queue_service("nativeDocumentOpen",json!("synthetic"),json!({"session":host.session,"owner":owner,"generation":1,"args":{"path":root.join("input.txt")}})).unwrap();
        app.extension_host = Some(host);
        app
    }
    #[test]
    fn stale_worker_reply_never_adds_hidden_models_and_occupied_slot_is_retained() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("input.txt"), "text").unwrap();
        let mut app = queued_app(root.path());
        assert!(app.poll_extension_services());
        assert!(app.extension_services.job.is_some());
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.doc_mut().insert("newer typing", false);
        let id = app.doc().id;
        app.cancel_extension_services();
        assert!(app.extension_services.job.is_some());
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.extension_services.job.is_some() {
            app.poll_extension_services();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(app.hidden_documents.is_empty());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), "newer typing");
    }
    #[test]
    fn hidden_open_worker_cannot_publish_after_equal_settings_reload_and_restore() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.txt");
        let input = root.path().join("input.txt");
        std::fs::write(&source, "original猫\r\n").unwrap();
        std::fs::write(&input, "unopened🙂\r\n").unwrap();
        let mut app = queued_app(root.path());
        let host = app.extension_host.take();
        app.open(&source).unwrap();
        until(&mut app, |app| app.active_document().is_some());
        app.doc_mut().insert("dirty ", false);
        app.split_editor(false);
        app.extension_host = host;
        let id = app.doc().id;
        let revision = app.doc().revision;
        let selections = app.doc().selections();
        let proof = app.doc().capture_save(source.clone()).unwrap();
        let members: Vec<_> = app.editor_groups().memberships(id).collect();
        assert_eq!(members.len(), 2);
        assert!(app.poll_extension_services());
        assert!(app.extension_services.job.is_some());
        let original = app.settings.extension_layers().clone();
        // Equal setting values must not revive an asynchronous authorization:
        // each load creates a new settings identity, even after restoration.
        app.settings = crate::settings::Settings::load_editor(&[]).unwrap();
        app.settings = crate::settings::Settings::load_editor(&[]).unwrap();
        assert_eq!(original.as_ref(), app.settings.extension_layers().as_ref());
        assert!(!Arc::ptr_eq(&original, app.settings.extension_layers()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.extension_services.job.is_some() {
            app.poll_extension_services();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(app.hidden_documents.is_empty());
        assert!(app.extension_host.as_ref().unwrap().service().is_none());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().revision, revision);
        assert_eq!(app.doc().selections(), selections);
        assert_eq!(app.doc().text.to_string(), "dirty original猫\r\n");
        let current = app.doc().capture_save(source.clone()).unwrap();
        assert_eq!(current.text_epoch(), proof.text_epoch());
        assert_eq!(current.save_generation(), proof.save_generation());
        for member in members {
            assert!(app.editor_groups().membership_current(member));
        }
        app.execute("undo", Value::Null);
        assert_eq!(app.doc().text.to_string(), "original猫\r\n");
        app.execute("redo", Value::Null);
        assert_eq!(app.doc().text.to_string(), "dirty original猫\r\n");
        assert_eq!(app.doc().id, id);
        assert_eq!(std::fs::read_to_string(source).unwrap(), "original猫\r\n");
        assert_eq!(std::fs::read_to_string(input).unwrap(), "unopened🙂\r\n");
    }
    #[cfg(unix)]
    #[test]
    fn native_alias_open_of_deleted_hidden_file_moves_identity_without_reading_disk() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("real");
        std::fs::create_dir(&folder).unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&folder, &alias).unwrap();
        let path = folder.join("input.txt");
        std::fs::write(&path, "original").unwrap();
        let mut doc = Document::open_existing(&path).unwrap();
        let id = doc.id;
        doc.insert("dirty ", false);
        std::fs::remove_file(&path).unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.hidden_documents.push(doc);
        app.open(&alias.join("input.txt")).unwrap();
        // A second request rejected by the occupied slot must leave the first
        // accepted loader valid rather than canceling both native opens.
        app.open(&alias.join("input.txt")).unwrap();
        assert!(app.documents.is_empty());
        until(&mut app, |app| app.active_document().is_some());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), "dirty original");
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "original");
        assert!(!path.exists());
        assert!(app.hidden_documents.is_empty());
    }
}

#[cfg(test)]
mod save_receipt_tests;
