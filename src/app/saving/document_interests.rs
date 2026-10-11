//! Native save-receipt ownership for the internal document service.
//! The public JavaScript TextDocument.save hook is a separate adoption step.
use super::*;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

const INTERESTS: usize = 8;
const PREAUTH: Duration = Duration::from_secs(30);
const PATH_WAIT: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct Caller {
    pub(in crate::app) session: u64,
    pub(in crate::app) request: String,
    pub(in crate::app) owner: String,
    native_host: bool,
}
impl Caller {
    /// Native caller validation MUST precede this constructor. It validates
    /// retention bounds, not whether an extension is actually selected/live.
    pub(in crate::app) fn new(session: u64, request: &str, owner: &str) -> Result<Self> {
        ensure!(session != 0, "Save caller session is missing");
        ensure!(
            !request.is_empty() && request.len() <= 128 && !request.contains('\0'),
            "Save caller request exceeds 128 bytes or contains NUL"
        );
        ensure!(
            !owner.is_empty() && owner.len() <= 256 && !owner.contains('\0'),
            "Save caller owner exceeds 256 bytes or contains NUL"
        );
        Ok(Self {
            session,
            request: request.into(),
            owner: owner.into(),
            native_host: true,
        })
    }
    #[cfg(test)]
    fn fixture(session: u64, request: &str, owner: &str) -> Self {
        let mut caller = Self::new(session, request, owner).unwrap();
        caller.native_host = false;
        caller
    }
    fn current(&self, app: &App) -> Result<()> {
        if self.native_host {
            app.extension_host
                .as_ref()
                .context("Save caller host was stopped/replaced")?
                .validate_native_origin(self.session, &self.owner, None, None)?;
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct Interest(u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum Admission {
    Enqueued(Interest),
    NeedsPath(Interest),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum Receipt {
    /// Finished is actually joined and publish_save succeeded. Gateway MUST
    /// still synchronize mirrors/didSave before answering its exact caller.
    Committed {
        document: u64,
        save_generation: u64,
    },
    NotSaved,
    /// An untyped worker error does not prove disk was never committed.
    Failed(String),
    /// Disk committed but model publication failed: never report false.
    CommittedButUnpublished(String),
}
#[derive(Debug)]
pub(in crate::app) struct Completion {
    pub(in crate::app) caller: Caller,
    pub(in crate::app) interest: Interest,
    pub(in crate::app) retired: bool,
    pub(in crate::app) receipt: Receipt,
}
struct Proof {
    document: u64,
    path: Option<PathBuf>,
    epoch: u64,
    revision: u64,
    saved_revision: u64,
    save_generation: u64,
    eol: String,
    profile: u64,
    settings: Arc<Vec<serde_json::Map<String, Value>>>,
    workspace: PathBuf,
}
impl Proof {
    fn capture(app: &App, document: u64) -> Result<Self> {
        let doc = app
            .documents
            .iter()
            .chain(&app.hidden_documents)
            .find(|doc| doc.id == document)
            .context("Save document is no longer retained")?;
        let bounded = |path: &Path| -> Result<()> {
            let bytes = path.as_os_str().as_encoded_bytes();
            ensure!(
                !bytes.is_empty() && bytes.len() <= 4096 && !bytes.contains(&0),
                "Save interest path requires 1–4096 bytes without NUL"
            );
            Ok(())
        };
        if let Some(path) = &doc.path {
            bounded(path)?;
        }
        bounded(&app.workspace.root)?;
        ensure!(
            doc.eol == "\n" || doc.eol == "\r\n",
            "Save interest has unsupported EOL"
        );
        ensure!(
            app.settings_error.is_none(),
            "Save interest cannot authorize failed settings"
        );
        Ok(Self {
            document,
            path: doc.path.clone(),
            epoch: doc.text_epoch(),
            revision: doc.revision,
            saved_revision: doc.saved_revision,
            save_generation: doc.save_generation(),
            eol: doc.eol.clone(),
            profile: app.settings_profile_generation(),
            settings: app.settings.extension_layers().clone(),
            workspace: app.workspace.root.clone(),
        })
    }
    fn model<'a>(&self, app: &'a App) -> Result<&'a Document> {
        ensure!(
            app.settings_error.is_none()
                && self.profile == app.settings_profile_generation()
                && Arc::ptr_eq(&self.settings, app.settings.extension_layers())
                && self.workspace == app.workspace.root,
            "Save caller settings/workspace ownership changed"
        );
        let doc = app
            .documents
            .iter()
            .chain(&app.hidden_documents)
            .find(|doc| doc.id == self.document)
            .context("Save caller model was closed")?;
        ensure!(
            doc.path == self.path
                && doc.saved_revision == self.saved_revision
                && doc.save_generation() == self.save_generation
                && doc.eol == self.eol,
            "Save caller path/baseline ownership changed"
        );
        Ok(doc)
    }
    fn current(&self, app: &App) -> Result<()> {
        let doc = self.model(app)?;
        ensure!(
            doc.text_epoch() == self.epoch && doc.revision == self.revision,
            "Save caller text changed before authorization"
        );
        Ok(())
    }
}
enum Location {
    Path,
    Intent(u64),
    Actual(u64),
    Complete(Receipt),
}
struct Entry {
    id: Interest,
    caller: Caller,
    proof: Proof,
    location: Location,
    started: Instant,
    retired: bool,
}
#[derive(Default)]
pub(in crate::app) struct Interests {
    next: u64,
    entries: Vec<Entry>,
}

impl App {
    /// Specific-model native admission; does not focus, promote, clone models,
    /// display a prompt or attach Close/Quit. Caller/mirror authority is a
    /// mandatory prerequisite supplied by the internal native service gateway.
    pub(in crate::app) fn request_document_save_interest(
        &mut self,
        caller: Caller,
        document: u64,
    ) -> Result<Admission> {
        ensure!(
            !self.saving.shutting_down && self.file_job.is_none(),
            "Persistence is settling or another file operation is active"
        );
        ensure!(
            self.saving.document_interests.entries.len() < INTERESTS,
            "Document save caller limit reached (8)"
        );
        ensure!(
            !self
                .saving
                .document_interests
                .entries
                .iter()
                .any(|entry| entry.caller.session == caller.session
                    && entry.caller.request == caller.request),
            "Duplicate document save request"
        );
        // Validate constructor bounds even for an internally constructed DTO.
        Caller::new(caller.session, &caller.request, &caller.owner)?;
        let proof = Proof::capture(self, document)?;
        let needs_path = proof.path.is_none();
        let next = self
            .saving
            .document_interests
            .next
            .checked_add(1)
            .context("Document save interest identity exhausted")?;
        self.saving
            .document_interests
            .entries
            .try_reserve(1)
            .context("Document save interest allocation refused")?;
        let id = Interest(next);
        self.saving.document_interests.entries.push(Entry {
            id,
            caller,
            proof,
            location: Location::Path,
            started: Instant::now(),
            retired: false,
        });
        self.saving.document_interests.next = next;
        if needs_path {
            return Ok(Admission::NeedsPath(id));
        }
        if let Err(error) = self.enqueue_document_save_interest(id, None) {
            self.saving
                .document_interests
                .entries
                .retain(|entry| entry.id != id);
            return Err(error);
        }
        Ok(Admission::Enqueued(id))
    }
    fn enqueue_document_save_interest(
        &mut self,
        id: Interest,
        destination: Option<PathBuf>,
    ) -> Result<()> {
        let entry = self
            .saving
            .document_interests
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .context("Document save interest retired")?;
        ensure!(
            !entry.retired && matches!(entry.location, Location::Path),
            "Document save path interest retired"
        );
        entry.proof.current(self)?;
        ensure!(
            entry.started.elapsed() < PATH_WAIT,
            "Document Save As wait expired"
        );
        let document = entry.proof.document;
        let path_started = entry.started;
        let next_intent = self
            .saving
            .next_intent
            .checked_add(1)
            .context("Save intent identity exhausted")?;
        let entry = self
            .saving
            .document_interests
            .entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .expect("interest present");
        entry.location = Location::Intent(next_intent);
        // Reset BEFORE enqueue, which may synchronously dispatch an idle worker.
        // Only accepting the path begins a new phase; polls never renew it.
        entry.started = Instant::now();
        let enqueued = self.enqueue_native_save(Intent {
            id: 0,
            document,
            destination,
            continuation: None,
            automatic: None,
            stage: Stage::Actions,
            notice: None,
            service: Some(id),
        });
        if let Err(error) = enqueued {
            // A failed synchronous dispatch may already have published latest.
            // Roll back only this exact admission before returning an error.
            if self
                .saving
                .latest
                .as_ref()
                .is_some_and(|intent| intent.id == next_intent && intent.service == Some(id))
            {
                self.saving.latest = None;
                self.cancel_save_code_actions();
                self.cancel_save_formatting();
            }
            let entry = self
                .saving
                .document_interests
                .entries
                .iter_mut()
                .find(|entry| entry.id == id)
                .expect("interest retained");
            entry.location = Location::Path;
            entry.started = path_started;
            return Err(error);
        }
        let intent = self.saving.next_intent;
        let location = self
            .saving
            .active
            .as_ref()
            .filter(|active| active.intent == intent)
            .map_or(Location::Intent(intent), |active| {
                Location::Actual(active.id)
            });
        let entry = self
            .saving
            .document_interests
            .entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .expect("non-awaiting interest admission");
        entry.location = location;
        Ok(())
    }
    pub(in crate::app) fn document_save_path_pending(&self, session: u64, request: &str) -> bool {
        self.saving.document_interests.entries.iter().any(|entry| {
            entry.caller.session == session
                && entry.caller.request == request
                && !entry.retired
                && matches!(entry.location, Location::Path)
                && entry.started.elapsed() < PATH_WAIT
        })
    }
    pub(in crate::app) fn document_save_caller_pending(&self, session: u64, request: &str) -> bool {
        self.saving
            .document_interests
            .entries
            .iter()
            .any(|entry| entry.caller.session == session && entry.caller.request == request)
    }
    pub(in crate::app) fn begin_document_save_path_prompt(
        &mut self,
        interest: Interest,
        document: u64,
    ) -> Result<()> {
        ensure!(
            self.prompt.is_none()
                && self.modal.is_none()
                && self.pending.is_none()
                && self.saving.closing.is_none(),
            "Document Save As cannot replace another prompt/close"
        );
        let entry = self
            .saving
            .document_interests
            .entries
            .iter()
            .find(|entry| entry.id == interest)
            .context("Save As interest retired")?;
        ensure!(
            entry.proof.document == document && matches!(entry.location, Location::Path),
            "Save As interest no longer owns its model"
        );
        entry.proof.current(self)?;
        self.saving.save_as_origin = Some(SaveAsOrigin {
            document,
            service: Some(interest),
            pane: None,
            membership: None,
            eligibility: self.captured_close_eligibility(None),
        });
        let path = self.workspace.root.join("untitled.txt");
        self.start_prompt(PromptKind::SaveAs, path.to_string_lossy().into_owned());
        Ok(())
    }
    pub(in crate::app) fn retire_document_save_prompt(&mut self, interest: Interest) {
        if self
            .saving
            .save_as_origin
            .as_ref()
            .is_some_and(|origin| origin.service == Some(interest))
        {
            self.saving.save_as_origin = None;
            if self
                .prompt
                .as_ref()
                .is_some_and(|prompt| matches!(prompt.kind, PromptKind::SaveAs))
            {
                self.prompt = None;
                self.focus = Focus::Editor;
            }
        }
    }
    pub(in crate::app) fn retire_all_document_save_callers(&mut self) {
        let callers: Vec<_> = self
            .saving
            .document_interests
            .entries
            .iter()
            .map(|entry| (entry.caller.session, entry.caller.owner.clone()))
            .collect();
        for (session, owner) in callers {
            self.retire_document_save_callers(session, &owner);
        }
    }
    pub(in crate::app) fn accept_document_save_path(
        &mut self,
        id: Interest,
        destination: PathBuf,
    ) -> Result<()> {
        self.enqueue_document_save_interest(id, Some(destination))
    }
    pub(in crate::app) fn retire_document_save_interest(&mut self, id: Interest) -> bool {
        let Some(index) = self
            .saving
            .document_interests
            .entries
            .iter()
            .position(|entry| entry.id == id)
        else {
            return false;
        };
        let entry = &mut self.saving.document_interests.entries[index];
        if entry.retired || matches!(entry.location, Location::Complete(_)) {
            return false;
        }
        entry.retired = true;
        match entry.location {
            Location::Path => entry.location = Location::Complete(Receipt::NotSaved),
            Location::Intent(intent) => {
                entry.location = Location::Complete(Receipt::NotSaved);
                if self
                    .saving
                    .latest
                    .as_ref()
                    .is_some_and(|latest| latest.id == intent)
                {
                    self.saving.latest = None;
                    self.cancel_save_code_actions();
                    self.cancel_save_formatting();
                }
            }
            Location::Actual(worker)
                if self
                    .saving
                    .active
                    .as_ref()
                    .is_some_and(|active| active.id == worker && !active.authorized) =>
            {
                self.saving.worker.reject(worker);
            }
            // An actual worker owns its outcome until Finished+join; no
            // speculative false/true, and no actual capacity release here.
            Location::Actual(_) | Location::Complete(_) => {}
        }
        true
    }
    pub(in crate::app) fn retire_document_save_callers(&mut self, session: u64, owner: &str) {
        // Eight metadata IDs max; no source or model copies.
        let ids: Vec<_> = self
            .saving
            .document_interests
            .entries
            .iter()
            .filter(|entry| entry.caller.session == session && entry.caller.owner == owner)
            .map(|entry| entry.id)
            .collect();
        for id in ids {
            self.retire_document_save_interest(id);
        }
    }
    // Keep authoritative receipts in the bounded table until a reply actually
    // enters the transport FIFO. Send backpressure must never readmit a save.
    pub(in crate::app) fn document_save_completion(&self) -> Option<Completion> {
        let entry = self
            .saving
            .document_interests
            .entries
            .iter()
            .find(|entry| matches!(entry.location, Location::Complete(_)))?;
        let Location::Complete(receipt) = &entry.location else {
            unreachable!()
        };
        Some(Completion {
            caller: entry.caller.clone(),
            interest: entry.id,
            retired: entry.retired,
            receipt: receipt.clone(),
        })
    }
    pub(in crate::app) fn take_document_save_completion(&mut self) -> Option<Completion> {
        let index = self
            .saving
            .document_interests
            .entries
            .iter()
            .position(|entry| matches!(entry.location, Location::Complete(_)))?;
        let entry = self.saving.document_interests.entries.remove(index);
        let Location::Complete(receipt) = entry.location else {
            unreachable!()
        };
        Some(Completion {
            caller: entry.caller,
            interest: entry.id,
            retired: entry.retired,
            receipt,
        })
    }
    pub(in crate::app) fn document_save_waits_for_host(&self, intent: u64) -> bool {
        self.extension_host.is_none()
            && self.saving.document_interests.entries.iter().any(|entry| {
                entry.caller.native_host
                    && matches!(entry.location, Location::Intent(current) if current == intent)
            })
    }
    pub(in crate::app) fn check_document_save_intent(&self, intent: u64) -> Result<()> {
        if let Some(interest) = self
            .saving
            .latest
            .as_ref()
            .filter(|latest| latest.id == intent)
            .and_then(|latest| latest.service)
        {
            ensure!(self.saving.document_interests.entries.iter().any(|entry| entry.id == interest && matches!(entry.location, Location::Intent(current) if current == intent)), "Document save intent lost its exact caller interest");
        }
        for entry in &self.saving.document_interests.entries {
            if matches!(entry.location, Location::Intent(current) if current == intent) {
                ensure!(
                    !entry.retired && entry.started.elapsed() < PREAUTH,
                    "Document save caller retired/expired before authorization"
                );
                entry.caller.current(self)?;
                entry.proof.current(self)?;
            }
        }
        Ok(())
    }
    pub(in crate::app) fn advance_document_save_owned_edit(
        &mut self,
        intent: u64,
        old_epoch: u64,
        old_revision: u64,
    ) -> Result<()> {
        let Some(index) = self.saving.document_interests.entries.iter().position(
            |entry| matches!(entry.location, Location::Intent(current) if current == intent),
        ) else {
            return Ok(());
        };
        let entry = &self.saving.document_interests.entries[index];
        ensure!(
            !entry.retired
                && entry.started.elapsed() < PREAUTH
                && entry.proof.epoch == old_epoch
                && entry.proof.revision == old_revision,
            "Save caller owned-edit proof changed"
        );
        let doc = entry.proof.model(self)?;
        let epoch = doc.text_epoch();
        let revision = doc.revision;
        let proof = &mut self.saving.document_interests.entries[index].proof;
        proof.epoch = epoch;
        proof.revision = revision;
        Ok(())
    }
    pub(in crate::app) fn capture_document_save_worker(&mut self, intent: u64, worker: u64) {
        for entry in &mut self.saving.document_interests.entries {
            if matches!(entry.location, Location::Intent(current) if current == intent) {
                entry.location = Location::Actual(worker);
            }
        }
    }
    pub(in crate::app) fn authorize_document_save_interest(&self, worker: u64) -> Result<()> {
        if let Some(interest) = self
            .saving
            .active
            .as_ref()
            .filter(|active| active.id == worker)
            .and_then(|active| active.service)
        {
            ensure!(self.saving.document_interests.entries.iter().any(|entry| entry.id == interest && matches!(entry.location, Location::Actual(current) if current == worker)), "Document save worker lost its exact caller interest");
        }
        for entry in &self.saving.document_interests.entries {
            if matches!(entry.location, Location::Actual(current) if current == worker) {
                ensure!(
                    !entry.retired && entry.started.elapsed() < PREAUTH,
                    "Save caller retired/expired before commit"
                );
                entry.caller.current(self)?;
                entry.proof.current(self)?;
            }
        }
        Ok(())
    }
    pub(in crate::app) fn finish_document_save_interest(&mut self, worker: u64, receipt: Receipt) {
        // One native intent carries at most one admitted caller. A duplicate
        // worker result cannot complete a replacement/previous session caller.
        if let Some(entry) =
            self.saving.document_interests.entries.iter_mut().find(
                |entry| matches!(entry.location, Location::Actual(current) if current == worker),
            )
        {
            entry.location = Location::Complete(receipt);
        }
    }
    pub(in crate::app) fn poll_document_save_interests(&mut self) -> bool {
        let now = Instant::now();
        let mut retire = Vec::new();
        for entry in &self.saving.document_interests.entries {
            if entry.retired || matches!(entry.location, Location::Complete(_)) {
                continue;
            }
            let expired = now.saturating_duration_since(entry.started)
                >= if matches!(entry.location, Location::Path) {
                    PATH_WAIT
                } else {
                    PREAUTH
                };
            let invalid = match entry.location {
                Location::Path => {
                    expired
                        || (entry.caller.current(self).is_err()
                            || entry.proof.current(self).is_err())
                        || self.saving.shutting_down
                }
                Location::Intent(intent) => {
                    expired
                        || self.saving.shutting_down
                        || !self
                            .saving
                            .latest
                            .as_ref()
                            .is_some_and(|latest| latest.id == intent)
                        || (entry.caller.current(self).is_err()
                            || entry.proof.current(self).is_err())
                }
                Location::Actual(worker) => self.saving.active.as_ref().is_some_and(|active| {
                    active.id == worker
                        && !active.authorized
                        && (expired
                            || self.saving.shutting_down
                            || (entry.caller.current(self).is_err()
                                || entry.proof.current(self).is_err()))
                }),
                Location::Complete(_) => false,
            };
            if invalid {
                retire.push(entry.id);
            }
        }
        let changed = !retire.is_empty();
        for id in retire {
            self.retire_document_save_interest(id);
        }
        changed
    }
}

#[cfg(test)]
mod tests;
