//! App-owned resource model preparation and checked native publication.
//! Disk reads/diff preparation run on the one occupied, joined resource worker.
use super::*;
use crate::{
    document::{ResourceModelPrepared, ResourceModelSource},
    extensions::editor_state,
    resource_journal::Control,
    resource_worker::{Event, Outcome, Preparation, Transaction, Worker},
};
use anyhow::{Context as _, ensure};
use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[cfg(test)]
struct ReloadGate {
    entered: std::sync::mpsc::SyncSender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

struct Reload {
    source: ResourceModelSource,
    maximum: usize,
    #[cfg(test)]
    gate: Option<ReloadGate>,
}
struct Prepared {
    model: ResourceModelPrepared,
    deadline: Instant,
}
impl Transaction for Reload {
    type Prepared = Prepared;
    type Info = ();
    type Receipt = Prepared;
    fn prepare(self, preparation: &Preparation) -> Result<(Prepared, ())> {
        preparation.check()?;
        #[cfg(test)]
        if let Some(gate) = self.gate {
            gate.entered.send(())?;
            gate.release.recv_timeout(Duration::from_secs(10))?;
        }
        preparation.check()?;
        let loaded = Document::open_existing_bounded(self.source.resource(), self.maximum as u64)?;
        ensure!(
            loaded.path.as_deref() == Some(self.source.resource()),
            "Original reload resource resolved to another path"
        );
        let control = Control::new(preparation.cancel_flag(), preparation.deadline());
        let model = self.source.prepare_reload(loaded.text, &control)?;
        Ok((
            Prepared {
                model,
                deadline: preparation.deadline(),
            },
            (),
        ))
    }
    fn commit(prepared: Prepared) -> Result<Prepared> {
        Ok(prepared)
    }
}
struct Run {
    id: u64,
    source: ResourceModelSource,
    context: editor_state::Context,
    deadline: Instant,
    retired: bool,
}
#[derive(Default)]
pub(super) struct State {
    worker: Worker<Reload>,
    next: u64,
    run: Option<Run>,
    #[cfg(test)]
    gate: Option<ReloadGate>,
}
impl App {
    pub(super) fn resource_models_pending(&self) -> bool {
        self.resource_models.worker.busy()
    }
    pub(super) fn resource_model_pending(&self, document: u64) -> bool {
        self.resource_models
            .run
            .as_ref()
            .is_some_and(|run| run.source.proof().document == document)
    }
    pub(super) fn start_resource_model_reload(
        &mut self,
        original_document: u64,
        original_epoch: u64,
    ) {
        let result = (|| -> Result<()> {
            ensure!(
                !self.resource_models_pending(),
                "A native resource model reader is still occupied"
            );
            let document = self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .find(|document| document.id == original_document)
                .context("Original revert document was closed")?;
            let id = document.id;
            ensure!(
                document.resource_epoch() == original_epoch,
                "Original confirmed resource changed"
            );
            ensure!(
                !self.document_save_pending(id)
                    && !self.saves_pending()
                    && !self.api_discard_occupied()
                    && self.file_job.is_none()
                    && !self.file_participation_pending()
                    && !self.workspace_transactions_pending(),
                "Wait for pending document operations before reverting"
            );
            ensure!(
                self.settings_error.is_none(),
                "Original reload settings are unavailable"
            );
            let source = document.capture_resource_model()?;
            let context = editor_state::Context::new(
                self.workspace.root.clone(),
                self.settings.extension_layers().clone(),
                self.settings_profile_generation(),
            )?;
            let maximum = if self.extension_host.is_some() {
                let other: usize = self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .filter(|document| document.id != id)
                    .map(|document| document.text.len_bytes())
                    .sum();
                (4usize * 1024 * 1024).saturating_sub(other)
            } else {
                crate::document::MAX_FILE_BYTES as usize
            };
            let deadline = Instant::now() + Duration::from_secs(10);
            let next = self
                .resource_models
                .next
                .checked_add(1)
                .context("Native resource identity exhausted")?;
            self.resource_models.worker.start(
                next,
                Reload {
                    source: source.clone(),
                    maximum,
                    #[cfg(test)]
                    gate: self.resource_models.gate.take(),
                },
                deadline,
            )?;
            self.resource_models.next = next;
            self.resource_models.run = Some(Run {
                id: next,
                source,
                context,
                deadline,
                retired: false,
            });
            self.message = "Reloading file from disk…".into();
            Ok(())
        })();
        if let Err(error) = result {
            self.message = format!("Reload refused; original buffer retained: {error:#}");
        }
    }
    fn resource_reload_current(&self, run: &Run) -> Result<()> {
        ensure!(
            !run.retired
                && Instant::now() < run.deadline
                && self.settings_error.is_none()
                && run.context.current(
                    &self.workspace.root,
                    self.settings.extension_layers(),
                    self.settings_profile_generation()
                ),
            "Original reload workspace/settings/deadline changed"
        );
        let mut documents = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .filter(|document| document.id == run.source.proof().document);
        let document = documents.next().context("Original reload model retired")?;
        ensure!(
            documents.next().is_none(),
            "Original reload model identity is ambiguous"
        );
        run.source.check_current(document)
    }
    pub(super) fn poll_resource_models(&mut self) -> bool {
        if let Some(run) = &self.resource_models.run
            && !run.retired
            && self.resource_reload_current(run).is_err()
        {
            self.resource_models.worker.retire(run.id);
            self.resource_models
                .run
                .as_mut()
                .expect("original run")
                .retired = true;
        }
        let Some(event) = self.resource_models.worker.poll() else {
            return false;
        };
        match event {
            Event::Prepared { id, .. } => {
                let current =
                    self.resource_models.run.as_ref().is_some_and(|run| {
                        run.id == id && self.resource_reload_current(run).is_ok()
                    });
                if !current || self.resource_models.worker.authorize(id).is_err() {
                    self.resource_models.worker.retire(id);
                }
            }
            Event::Finished {
                id,
                retired,
                result,
                ..
            } => {
                let Some(run) = self.resource_models.run.take().filter(|run| run.id == id) else {
                    return true;
                };
                let result = (|| -> Result<()> {
                    ensure!(
                        !retired,
                        "Original reload source changed during preparation"
                    );
                    self.resource_reload_current(&run)?;
                    let prepared = match result.map_err(anyhow::Error::msg)? {
                        Outcome::Committed(prepared) => prepared,
                        _ => anyhow::bail!("Original reload retired or expired"),
                    };
                    ensure!(
                        prepared.deadline == run.deadline
                            && prepared.model.document() == run.source.proof().document,
                        "Reload receipt does not belong to original native source"
                    );
                    if self.extension_host.is_some() {
                        let total: usize = self
                            .documents
                            .iter()
                            .chain(&self.hidden_documents)
                            .map(|document| document.text.len_bytes())
                            .sum();
                        let original = self
                            .documents
                            .iter()
                            .chain(&self.hidden_documents)
                            .find(|document| document.id == run.source.proof().document)
                            .context("Original reload model retired")?;
                        ensure!(
                            total
                                .saturating_sub(original.text.len_bytes())
                                .saturating_add(prepared.model.bytes())
                                <= 4 * 1024 * 1024,
                            "Reload result exceeds current mirror budget"
                        );
                    }
                    self.invalidate_disk_watch_publications()?;
                    let document = self
                        .documents
                        .iter_mut()
                        .chain(&mut self.hidden_documents)
                        .find(|document| document.id == run.source.proof().document)
                        .context("Original reload model retired before publication")?;
                    let live = AtomicBool::new(false);
                    let control = Control::new(&live, run.deadline);
                    document
                        .prepare_resource_model_commit(prepared.model, &control, 8 * 1024 * 1024)?
                        .publish();
                    self.message = "Reloaded from disk".into();
                    Ok(())
                })();
                if let Err(error) = result {
                    self.message = format!("Reload refused; original buffer retained: {error:#}");
                }
            }
        }
        true
    }
    pub(super) fn settle_resource_models(&mut self) {
        if let Some(run) = self.resource_models.run.take() {
            self.resource_models.worker.retire(run.id);
        }
        let _ = self.resource_models.worker.settle();
    }
}

#[cfg(test)]
mod tests;
