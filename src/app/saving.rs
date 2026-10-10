//! Native save ownership. Filesystem work belongs to the bounded save worker.
use super::*;
use crate::{
    document::SaveSnapshot,
    save_worker::{Event as SaveEvent, Outcome, PreparedInfo, Worker},
};
use anyhow::{Context, ensure};

const MODELS: usize = 128;
const PATH_BYTES: usize = 512 * 1024;

#[derive(Clone)]
struct Continuation {
    action: AfterSave,
    pane: Option<u64>,
    generation: u64,
}

#[cfg(test)]
mod autosave_tests;
#[cfg(test)]
mod close_pending_tests;
#[cfg(test)]
mod settings_lane_tests;
#[cfg(test)]
mod shutdown_tests;
#[cfg(test)]
mod tests;
#[derive(Clone)]
pub(super) struct Intent {
    pub(super) document: u64,
    pub(super) destination: Option<PathBuf>,
    continuation: Option<Continuation>,
    pub(super) automatic: Option<Automatic>,
    pub(super) formatting_done: bool,
    pub(super) notice: Option<String>,
}
#[derive(Clone)]
pub(super) struct Automatic {
    generation: u64,
    workspace: PathBuf,
    proof: crate::autosave::ModelProof,
}
struct ModelProof {
    id: u64,
    path: Option<PathBuf>,
    text_epoch: u64,
    save_generation: u64,
}
struct Active {
    id: u64,
    snapshot: SaveSnapshot,
    models: Vec<ModelProof>,
    continuation: Option<Continuation>,
    authorized: bool,
    notice: Option<String>,
    automatic: Option<Automatic>,
}
#[derive(Default)]
pub(super) struct State {
    worker: Worker,
    active: Option<Active>,
    pub(super) latest: Option<Intent>,
    pub(super) formatting: super::save_formatting::State,
    formatting_dispatch: bool,
    next_id: u64,
    close_generation: u64,
    save_as_origin: Option<(u64, Option<u64>)>,
    closing: Option<AfterSave>,
    shutting_down: bool,
    autosave: crate::autosave::Scheduler,
    autosave_models: Vec<crate::autosave::ModelProof>,
    autosave_settings: Option<std::sync::Arc<Vec<serde_json::Map<String, Value>>>>,
    autosave_workspace: Option<PathBuf>,
    autosave_generation: u64,
    autosave_disabled: bool,
    autosave_error: Option<String>,
    #[cfg(test)]
    autosave_now: Option<std::time::Instant>,
}
impl App {
    pub(super) fn autosave_now(&self) -> std::time::Instant {
        #[cfg(test)]
        if let Some(now) = self.saving.autosave_now {
            return now;
        }
        std::time::Instant::now()
    }
    pub(super) fn retire_autosave_interest(&mut self) {
        match self.saving.autosave_generation.checked_add(1) {
            Some(next) => self.saving.autosave_generation = next,
            None => {
                self.saving.autosave_disabled = true;
                self.message =
                    "Autosave ownership exhausted; manual editing retained, restart the editor"
                        .into();
            }
        }
        self.saving.autosave.clear();
        self.saving.autosave_models.clear();
        self.saving.autosave_settings = None;
        if self
            .saving
            .latest
            .as_ref()
            .is_some_and(|intent| intent.automatic.is_some())
        {
            self.saving.latest = None;
        }
        if let Some(active) = &self.saving.active
            && active.automatic.is_some()
            && !active.authorized
        {
            self.saving.worker.reject(active.id);
        }
    }
    pub(super) fn poll_autosave(&mut self, now: std::time::Instant) -> bool {
        if !self.running
            || self.saving.shutting_down
            || self.saving.autosave_disabled
            || self.settings_error.is_some()
        {
            return false;
        }
        let settings = self.settings.extension_layers().clone();
        let refresh = self
            .saving
            .autosave_settings
            .as_ref()
            .is_none_or(|old| !std::sync::Arc::ptr_eq(old, &settings))
            || self.saving.autosave_workspace.as_ref() != Some(&self.workspace.root);
        if refresh {
            if self.saving.autosave_settings.is_some() {
                self.retire_autosave_interest();
            }
            self.saving.autosave_settings = Some(settings);
            self.saving.autosave_workspace = Some(self.workspace.root.clone());
        }
        let result = self.observe_autosave_models(now, refresh);
        if let Err(error) = result {
            let error = format!("Autosave paused; unsaved work retained: {error:#}");
            if self.saving.autosave_error.as_ref() != Some(&error) {
                self.retire_autosave_interest();
                self.message = error.clone();
                self.saving.autosave_error = Some(error);
                return true;
            }
            return false;
        }
        self.saving.autosave_error = None;
        if self.saves_pending()
            || self.file_job.is_some()
            || self.saving.closing.is_some()
            || matches!(self.modal, Some(Modal::Confirm(_)))
            || self
                .prompt
                .as_ref()
                .is_some_and(|prompt| matches!(prompt.kind, PromptKind::SaveAs))
        {
            return false;
        }
        let Some(proof) = self.saving.autosave.take_due(now) else {
            return false;
        };
        let automatic = Automatic {
            generation: self.saving.autosave_generation,
            workspace: self.workspace.root.clone(),
            proof: proof.clone(),
        };
        if let Err(error) = self.enqueue_native_save(Intent {
            document: proof.id,
            destination: None,
            continuation: None,
            automatic: Some(automatic),
            formatting_done: false,
            notice: None,
        }) {
            self.saving.autosave.failed(&proof);
            self.message = format!("Save failed; autosave retained unsaved work: {error:#}");
        }
        true
    }
    fn observe_autosave_models(&mut self, now: std::time::Instant, refresh: bool) -> Result<()> {
        let count = self
            .documents
            .len()
            .checked_add(self.hidden_documents.len())
            .context("Autosave inventory exhausted")?;
        ensure!(count <= MODELS, "Autosave exceeds 128 retained models");
        let mut bytes = 0usize;
        for doc in self.documents.iter().chain(&self.hidden_documents) {
            if let Some(path) = &doc.path {
                let name = path.as_os_str().as_encoded_bytes();
                ensure!(
                    !name.is_empty()
                        && name.len() <= 4096
                        && !name.contains(&0)
                        && path.file_name().is_some(),
                    "Autosave model path requires 1–4096 bytes and a filename without NUL"
                );
                bytes = bytes
                    .checked_add(name.len())
                    .context("Autosave path budget exhausted")?;
                ensure!(bytes <= PATH_BYTES, "Autosave model paths exceed 512 KiB");
            }
        }
        for (index, doc) in self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .enumerate()
        {
            let previous = self.saving.autosave_models.get_mut(index);
            if let Some(proof) =
                previous.filter(|proof| proof.id == doc.id && proof.path == doc.path)
            {
                proof.text_epoch = doc.text_epoch();
                proof.save_generation = doc.save_generation();
                proof.dirty = doc.dirty();
                if refresh {
                    let language = doc
                        .path
                        .as_deref()
                        .map_or("plaintext", crate::languages::language);
                    proof.policy = self.settings.auto_save(language);
                }
            } else {
                let language = doc
                    .path
                    .as_deref()
                    .map_or("plaintext", crate::languages::language);
                let proof = crate::autosave::ModelProof {
                    id: doc.id,
                    path: doc.path.clone(),
                    text_epoch: doc.text_epoch(),
                    save_generation: doc.save_generation(),
                    dirty: doc.dirty(),
                    policy: self.settings.auto_save(language),
                };
                if index < self.saving.autosave_models.len() {
                    self.saving.autosave_models[index] = proof;
                } else {
                    self.saving.autosave_models.push(proof);
                }
            }
        }
        self.saving.autosave_models.truncate(count);
        self.saving
            .autosave
            .observe(now, &self.saving.autosave_models)
    }
    fn suppress_failed_save_snapshot(&mut self, snapshot: &SaveSnapshot) {
        let language = snapshot
            .source_path()
            .map_or("plaintext", crate::languages::language);
        self.saving.autosave.failed(&crate::autosave::ModelProof {
            id: snapshot.document_id(),
            path: snapshot.source_path().map(Path::to_owned),
            text_epoch: snapshot.text_epoch(),
            save_generation: snapshot.save_generation(),
            dirty: true,
            policy: self.settings.auto_save(language),
        });
    }
    pub fn saves_pending(&self) -> bool {
        self.saving.worker.busy() || self.saving.latest.is_some()
    }
    pub(super) fn document_save_pending(&self, id: u64) -> bool {
        self.saving.formatting.document_pending(id)
            || self
                .saving
                .active
                .as_ref()
                .is_some_and(|active| active.snapshot.document_id() == id)
    }
    fn save_continuation(
        &self,
        action: Option<AfterSave>,
        pane: Option<u64>,
    ) -> Option<Continuation> {
        action.map(|action| Continuation {
            action,
            pane,
            generation: self.saving.close_generation,
        })
    }
    pub(super) fn capture_save_as_origin(&mut self) {
        self.saving.save_as_origin = self
            .active_document()
            .map(|doc| (doc.id, self.panes.get(self.active_pane).map(|pane| pane.id)));
    }
    pub(super) fn request_native_save(&mut self, after: Option<AfterSave>) -> Result<()> {
        ensure!(
            self.file_job.is_none(),
            "Wait for the file operation to finish before saving"
        );
        let doc = self
            .active_document()
            .context("Open a document before saving")?;
        if doc.path.is_none() {
            self.pending = after;
            self.save_as();
            return Ok(());
        }
        let intent = Intent {
            document: doc.id,
            destination: None,
            continuation: self
                .save_continuation(after, self.panes.get(self.active_pane).map(|pane| pane.id)),
            automatic: None,
            formatting_done: false,
            notice: None,
        };
        self.enqueue_native_save(intent)
    }
    pub(super) fn request_native_save_as(&mut self, destination: PathBuf) -> Result<()> {
        let (document, pane) = self
            .saving
            .save_as_origin
            .take()
            .context("Save As origin retired; invoke Save As again")?;
        let after = self.pending.take();
        self.enqueue_native_save(Intent {
            document,
            destination: Some(destination),
            continuation: self.save_continuation(after, pane),
            automatic: None,
            formatting_done: false,
            notice: None,
        })
    }
    fn enqueue_native_save(&mut self, intent: Intent) -> Result<()> {
        ensure!(
            !self.saving.shutting_down,
            "Editor shutdown is settling saves"
        );
        ensure!(
            self.file_job.is_none(),
            "Wait for the file operation to finish before saving"
        );
        let doc = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .find(|doc| doc.id == intent.document)
            .context("Save document was closed; no other editor was saved")?;
        let target = intent
            .destination
            .clone()
            .or_else(|| doc.path.clone())
            .context("Choose a path with Save As first")?;
        // Admission precedes superseding valid work. This cheap temporary
        // capture is discarded; queued work never retains its stale Rope.
        doc.capture_save(target)?;
        self.saving
            .next_id
            .checked_add(1)
            .context("Save request identity exhausted; restart the editor")?;
        let mut count = 0;
        let mut bytes = 0usize;
        let mut ids = std::collections::BTreeSet::new();
        for doc in self.documents.iter().chain(&self.hidden_documents) {
            count += 1;
            ensure!(
                count <= MODELS && ids.insert(doc.id),
                "Save requires at most 128 distinct retained document models"
            );
            if let Some(path) = &doc.path {
                let encoded = path.as_os_str().as_encoded_bytes();
                ensure!(
                    !encoded.is_empty() && encoded.len() <= 4096 && !encoded.contains(&0),
                    "Save model paths require 1–4096 bytes without NUL"
                );
                bytes = bytes
                    .checked_add(encoded.len())
                    .context("Save model path budget exhausted")?;
                ensure!(bytes <= PATH_BYTES, "Save model paths exceed 512 KiB");
            }
        }
        // One latest desired action carries no Rope. It will be recaptured only
        // after any earlier actual worker and its receipt have settled.
        if let Some(active) = &self.saving.active
            && !active.authorized
        {
            self.saving.worker.reject(active.id);
        }
        self.cancel_save_formatting();
        self.saving.latest = Some(intent);
        self.message = "Saving…".into();
        self.dispatch_native_save()
    }
    fn dispatch_native_save(&mut self) -> Result<()> {
        if self.saving.worker.busy() || self.saving.shutting_down || self.file_job.is_some() {
            return Ok(());
        }
        if self.saving.formatting.pending() {
            return Ok(());
        }
        let Some(intent) = self.saving.latest.as_ref().cloned() else {
            return Ok(());
        };
        if !intent.formatting_done && intent.automatic.is_none() && !self.saving.formatting_dispatch
        {
            let language = self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .find(|doc| doc.id == intent.document)
                .and_then(|doc| doc.path.as_deref())
                .map_or("plaintext", crate::languages::language);
            if self.settings.save_formatting(language) != crate::settings::SaveFormatting::Off {
                return Ok(());
            }
        }
        if !intent.formatting_done && self.begin_save_formatting(&intent)? {
            return Ok(());
        }
        let intent = self.saving.latest.take().context("Save intent retired")?;
        let doc = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .find(|doc| doc.id == intent.document)
            .context("Save document was closed; buffer ownership retired")?;
        let target = intent
            .destination
            .clone()
            .or_else(|| doc.path.clone())
            .context("Choose a path with Save As first")?;
        let snapshot = doc.capture_save(target)?;
        let mut models = Vec::new();
        let mut inventory = Vec::new();
        let mut bytes = 0usize;
        for doc in self.documents.iter().chain(&self.hidden_documents) {
            ensure!(
                models.len() < MODELS,
                "Save proof exceeds 128 retained document models"
            );
            if let Some(path) = &doc.path {
                bytes = bytes
                    .checked_add(path.as_os_str().len())
                    .context("Save model path budget exhausted")?;
                ensure!(bytes <= PATH_BYTES, "Save model paths exceed 512 KiB");
                if doc.id != snapshot.document_id() {
                    inventory.push((doc.id, path.clone()));
                }
            }
            models.push(ModelProof {
                id: doc.id,
                path: doc.path.clone(),
                text_epoch: doc.text_epoch(),
                save_generation: doc.save_generation(),
            });
        }
        let id = self
            .saving
            .next_id
            .checked_add(1)
            .context("Save request identity exhausted; restart the editor")?;
        self.saving
            .worker
            .try_start(id, snapshot.clone(), inventory)?;
        self.saving.next_id = id;
        self.saving.active = Some(Active {
            id,
            snapshot,
            models,
            continuation: intent.continuation,
            authorized: false,
            notice: intent.notice,
            automatic: intent.automatic,
        });
        Ok(())
    }
    fn authorize_native_save(&self, info: &PreparedInfo) -> Result<()> {
        let active = self
            .saving
            .active
            .as_ref()
            .context("Save request retired")?;
        ensure!(
            active.id == info.id && active.snapshot.target() == info.requested_path,
            "Save request identity changed"
        );
        ensure!(
            !self.saving.shutting_down && self.file_job.is_none(),
            "Save retired while filesystem ownership changed"
        );
        let doc = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .find(|doc| doc.id == active.snapshot.document_id())
            .context("Save document was closed")?;
        doc.check_save_snapshot(&active.snapshot)?;
        if let Some(automatic) = &active.automatic {
            ensure!(
                automatic.generation == self.saving.autosave_generation
                    && automatic.workspace == self.workspace.root
                    && self.settings_error.is_none()
                    && !self.saving.autosave_disabled
                    && !matches!(self.modal, Some(Modal::Confirm(_))),
                "Autosave policy or close ownership changed; automatic save retired"
            );
            let language = doc
                .path
                .as_deref()
                .map_or("plaintext", crate::languages::language);
            ensure!(
                doc.dirty() && self.settings.auto_save(language) == automatic.proof.policy,
                "Autosave policy changed; automatic save retired"
            );
        }
        let mut count = 0;
        for doc in self.documents.iter().chain(&self.hidden_documents) {
            count += 1;
            ensure!(
                count <= MODELS,
                "Save proof exceeds 128 retained document models"
            );
            let Some(path) = &doc.path else {
                continue;
            };
            let proof = active
                .models
                .iter()
                .find(|proof| proof.id == doc.id)
                .context("A file model opened during save preparation; retry Save")?;
            ensure!(
                proof.path.as_ref() == Some(path),
                "A retained file path changed during save preparation; retry Save"
            );
            if doc.id != active.snapshot.document_id()
                && (info.matching_models.contains(&doc.id)
                    || path == &info.canonical_path
                    || path == &info.requested_path)
            {
                ensure!(
                    !doc.dirty(),
                    "Another retained model of this file has unsaved changes; Save As elsewhere"
                );
                ensure!(
                    proof.text_epoch == doc.text_epoch()
                        && proof.save_generation == doc.save_generation(),
                    "A retained alias changed during save preparation; retry Save"
                );
            }
        }
        Ok(())
    }
    pub(super) fn poll_native_saves(&mut self) -> bool {
        let mut changed = false;
        if let Some(event) = self.saving.worker.poll() {
            changed = true;
            match event {
                SaveEvent::Prepared(info) => {
                    let result = self.authorize_native_save(&info).and_then(|()| {
                        self.invalidate_disk_watch_publications()?;
                        self.saving.worker.authorize(info.id)
                    });
                    if let Err(error) = result {
                        self.saving.worker.reject(info.id);
                        self.message = format!("Save retired; unsaved work retained: {error:#}");
                    } else if let Some(active) = self.saving.active.as_mut() {
                        active.authorized = true;
                    }
                }
                SaveEvent::Finished {
                    id,
                    snapshot,
                    result,
                } => {
                    // Even superseded authorized operations really persisted.
                    // Their receipts must publish before recapturing the next intent.
                    let active = self.saving.active.take().filter(|active| active.id == id);
                    match result {
                        Ok(Outcome::Committed(commit)) => {
                            let fence = self.invalidate_disk_watch_publications();
                            let published = self
                                .documents
                                .iter_mut()
                                .chain(&mut self.hidden_documents)
                                .find(|doc| doc.id == snapshot.document_id())
                                .context("Saved model is no longer retained")
                                .and_then(|doc| doc.publish_save(&snapshot, commit.path.clone()));
                            if let Err(error) = published {
                                self.message = format!(
                                    "Saved {}; newer buffer metadata retained: {error:#}",
                                    commit.path.display()
                                );
                            } else {
                                self.message = format!("Saved {}", commit.path.display());
                                self.recent_files.touch(commit.path.clone());
                                for doc in
                                    self.documents.iter_mut().chain(&mut self.hidden_documents)
                                {
                                    if doc.id == snapshot.document_id() {
                                        self.settings.apply(doc);
                                    }
                                }
                                self.refresh_document_language_configurations();
                                self.language_saved_snapshot(&commit.path, snapshot.text());
                                if let Some(continuation) = active
                                    .as_ref()
                                    .and_then(|active| active.continuation.clone())
                                {
                                    self.finish_save_continuation(
                                        snapshot.document_id(),
                                        continuation,
                                    );
                                }
                            }
                            if let Some(loader) = self.settings_loader.as_mut()
                                && let Err(error) = loader.force_reload()
                            {
                                self.message =
                                    format!("File saved; settings reload failed: {error:#}");
                            }
                            if let Err(error) = fence {
                                self.message =
                                    format!("File saved; disk refresh disabled: {error:#}");
                            }
                            if let Some(warning) = commit.durability_warning {
                                self.message = format!("File saved; durability warning: {warning}");
                            }
                            if let Some(notice) =
                                active.as_ref().and_then(|active| active.notice.as_ref())
                            {
                                self.message.push_str(" · ");
                                self.message.push_str(notice);
                            }
                        }
                        Ok(Outcome::Rejected | Outcome::Expired) => {
                            self.suppress_failed_save_snapshot(&snapshot);
                            if self.saving.latest.is_none() {
                                self.message =
                                    "Save retired before commit; unsaved work retained".into();
                            }
                        }
                        Err(error) => {
                            self.suppress_failed_save_snapshot(&snapshot);
                            self.message = format!("Save failed; unsaved work retained: {error}")
                        }
                    }
                }
            }
        }
        self.saving.formatting_dispatch = true;
        let dispatched = self.dispatch_native_save();
        self.saving.formatting_dispatch = false;
        if let Err(error) = dispatched {
            self.message = format!("Save failed; unsaved work retained: {error:#}");
            changed = true;
        }
        if !self.persistence_pending()
            && let Some(action) = self.saving.closing.take()
        {
            self.request_close(action);
            changed = true;
        }
        changed
    }
    fn finish_save_continuation(&mut self, document: u64, continuation: Continuation) {
        if continuation.generation != self.saving.close_generation {
            return;
        }
        if self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .find(|doc| doc.id == document)
            .is_none_or(Document::dirty)
        {
            self.message =
                "Snapshot saved; newer unsaved edits retained. Close again to review them.".into();
            return;
        }
        match continuation.action {
            AfterSave::Close => {
                let Some(index) = self.panes.iter().position(|pane| {
                    Some(pane.id) == continuation.pane && pane.document == document
                }) else {
                    return;
                };
                let focused = self.panes.get(self.active_pane).map(|pane| pane.id);
                let focus = self.focus.clone();
                self.focus_pane(index);
                self.close_pane();
                if let Some(index) = self.panes.iter().position(|pane| Some(pane.id) == focused) {
                    self.focus_pane(index);
                    self.focus = focus;
                }
            }
            action @ (AfterSave::Quit | AfterSave::CloseAll) => self.request_close(action),
        }
    }
    pub(super) fn persistence_pending(&self) -> bool {
        self.saves_pending() || self.settings_writes_busy()
    }
    pub(super) fn defer_close_for_persistence(&mut self, action: AfterSave) -> bool {
        if matches!(action, AfterSave::Close)
            && let Some(document) = self.active_document().map(|doc| doc.id)
        {
            let pane = self.panes.get(self.active_pane).map(|pane| pane.id);
            let generation = self.saving.close_generation;
            let target = if let Some(intent) = self
                .saving
                .latest
                .as_mut()
                .filter(|intent| intent.document == document)
            {
                Some(&mut intent.continuation)
            } else if let Some(active) = self
                .saving
                .active
                .as_mut()
                .filter(|active| active.snapshot.document_id() == document)
            {
                Some(&mut active.continuation)
            } else {
                None
            };
            if let Some(target) = target {
                // A broader current close request must not become a single-pane
                // close merely because Ctrl+W arrived before its save receipt.
                if self.saving.closing.is_none()
                    && !target.as_ref().is_some_and(|continuation| {
                        continuation.generation == generation
                            && !matches!(continuation.action, AfterSave::Close)
                    })
                {
                    *target = Some(Continuation {
                        action: AfterSave::Close,
                        pane,
                        generation,
                    });
                }
                self.message =
                    "Waiting for pending save before closing… Escape cancels closing".into();
                return true;
            }
        }
        if matches!(action, AfterSave::Quit | AfterSave::CloseAll) && self.persistence_pending() {
            self.saving.closing = Some(action);
            self.message =
                "Waiting for pending persistence before closing… Escape cancels closing".into();
            true
        } else {
            false
        }
    }
    pub(super) fn cancel_deferred_save_close(&mut self) -> bool {
        let generation = self.saving.close_generation;
        let deferred_close = |continuation: &Option<Continuation>| {
            continuation.as_ref().is_some_and(|continuation| {
                continuation.generation == generation
                    && matches!(continuation.action, AfterSave::Close)
            })
        };
        if self.saving.closing.is_some()
            || self
                .saving
                .active
                .as_ref()
                .is_some_and(|active| deferred_close(&active.continuation))
            || self
                .saving
                .latest
                .as_ref()
                .is_some_and(|intent| deferred_close(&intent.continuation))
        {
            self.cancel_save_continuations();
            self.message = "Closing canceled; pending saves continue".into();
            true
        } else {
            false
        }
    }
    pub(super) fn cancel_save_continuations(&mut self) {
        self.pending = None;
        self.saving.save_as_origin = None;
        self.saving.closing = None;
        if let Some(active) = self.saving.active.as_mut() {
            active.continuation = None;
        }
        if let Some(intent) = self.saving.latest.as_mut() {
            intent.continuation = None;
        }
        match self.saving.close_generation.checked_add(1) {
            Some(next) => self.saving.close_generation = next,
            None => {
                self.saving.shutting_down = true;
                self.message =
                    "Close ownership exhausted; buffers retained, restart the editor".into();
            }
        }
    }
    /// Interrupt shutdown settles real persistence before recovery snapshots
    /// capture the final saved baseline. Unapproved work is retired; an already
    /// authorized commit still publishes its receipt to the original model.
    pub fn settle_persistence(&mut self) {
        self.saving.shutting_down = true;
        self.saving.latest = None;
        self.cancel_save_formatting();
        self.cancel_save_continuations();
        if let Some(active) = &self.saving.active
            && !active.authorized
        {
            self.saving.worker.reject(active.id);
        }
        self.retire_unapproved_settings_write();
        while self.persistence_pending() {
            self.poll_settings_writes();
            self.poll_native_saves();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}
