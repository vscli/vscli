use super::*;
use crate::{
    document::WorkspaceSource,
    editor_groups::{Membership, UiProof},
};
use std::{
    thread::{self, JoinHandle},
    time::Duration,
};

pub(super) struct Discard {
    thread: JoinHandle<Result<ropey::Rope>>,
    source: WorkspaceSource,
    language_epoch: u64,
    workspace: PathBuf,
    profile: u64,
    settings: std::sync::Arc<Vec<serde_json::Map<String, Value>>>,
    groups: UiProof,
    member: Option<Membership>,
    eligibility: sticky_tabs::CloseEligibility,
    generation: u64,
    action: AfterSave,
    deadline: Instant,
}

impl App {
    pub(in crate::app) fn api_discard_occupied(&self) -> bool {
        self.api_documents.discard.is_some()
    }
    pub(in crate::app) fn api_discard_pending(&self, member: Membership, generation: u64) -> bool {
        self.api_documents
            .discard
            .as_ref()
            .is_some_and(|job| job.member == Some(member) && job.generation == generation)
    }
    /// Explicit Discard of the last API-referenced view needs a genuine revert:
    /// retaining its dirty model or dropping a live resolver would be incorrect.
    pub(in crate::app) fn begin_api_discard(&mut self, action: AfterSave) -> bool {
        let Some(doc) = self
            .active_document()
            .filter(|doc| self.api_model_referenced(doc))
        else {
            return false;
        };
        let id = doc.id;
        let result = (|| -> Result<()> {
            ensure!(
                self.api_documents.discard.is_none(),
                "A native discard reader is still occupied"
            );
            let doc = self
                .documents
                .iter()
                .find(|doc| doc.id == id)
                .context("Discard model retired")?;
            if doc.path.is_none() && doc.virtual_uri().is_none() {
                let resource = resource_key(doc)?;
                let key = model_key(doc);
                let ledger = self
                    .api_documents
                    .ledger
                    .as_mut()
                    .context("Discard customer retired")?;
                ledger.untitled_revert(ledger.customer_epoch(), key, &resource)?;
                self.api_documents.managed.remove(&key);
                if matches!(action, AfterSave::Close) {
                    self.complete_close(action);
                } else {
                    self.remove_active();
                    self.complete_close(action);
                }
                return Ok(());
            }
            let path = doc
                .path
                .clone()
                .context("Discard has no native file backend")?;
            let source = doc.capture_workspace_source()?;
            let language_epoch = doc.language_epoch();
            let total: usize = self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .map(|doc| doc.text.len_bytes())
                .sum();
            let remaining =
                (4usize * 1024 * 1024).saturating_sub(total.saturating_sub(doc.text.len_bytes()));
            let deadline = Instant::now() + Duration::from_secs(3);
            let member = if matches!(action, AfterSave::Close) {
                self.close_membership
                    .or_else(|| self.active_tab_membership())
            } else {
                None
            };
            let eligibility = self.captured_close_eligibility(member);
            self.api_documents.discard = Some(Discard {
                thread: thread::Builder::new()
                    .name("vscli-api-discard".into())
                    .spawn(move || {
                        ensure!(Instant::now() < deadline, "Discard read expired");
                        let loaded = Document::open_existing_bounded(&path, remaining as u64)?;
                        ensure!(
                            loaded.path.as_ref() == Some(&path),
                            "Discard resource resolved to another path"
                        );
                        ensure!(Instant::now() < deadline, "Discard read expired");
                        Ok(loaded.text)
                    })?,
                source,
                language_epoch,
                workspace: self.workspace.root.clone(),
                profile: self.settings_profile_generation(),
                settings: self.settings.extension_layers().clone(),
                groups: self.editor_groups.proof(),
                member,
                eligibility,
                generation: self.tab_close_generation(),
                action,
                deadline,
            });
            self.message = "Reverting retained document before close…".into();
            Ok(())
        })();
        if let Err(error) = result {
            self.message = format!("Discard refused; original unsaved work retained: {error:#}");
        }
        true
    }
    pub(in crate::app) fn settle_api_discard(&mut self) {
        if let Some(job) = self.api_documents.discard.take() {
            let _ = job.thread.join();
        }
    }
    pub(in crate::app) fn poll_api_discard(&mut self) -> bool {
        if self
            .api_documents
            .discard
            .as_ref()
            .is_none_or(|job| !job.thread.is_finished())
        {
            return false;
        }
        let job = self
            .api_documents
            .discard
            .take()
            .expect("finished native discard");
        let result = (|| -> Result<()> {
            let next = job
                .thread
                .join()
                .map_err(|_| anyhow::anyhow!("Native discard reader panicked"))??;
            ensure!(Instant::now() < job.deadline, "Original discard expired");
            ensure!(
                self.modal.is_none() && self.prompt.is_none(),
                "Another native interaction replaced the discard"
            );
            ensure!(
                std::sync::Arc::ptr_eq(self.settings.extension_layers(), &job.settings)
                    && self.workspace.root == job.workspace
                    && self.settings_profile_generation() == job.profile,
                "Discard workspace/settings changed"
            );
            ensure!(
                self.tab_close_generation() == job.generation
                    && self.editor_groups.proof_current(&job.groups),
                "Original discard layout or close authority retired"
            );
            if let Some(member) = job.member {
                ensure!(
                    self.editor_groups.membership_current(member)
                        && self.close_target_eligible(member, job.eligibility),
                    "Original discard tab retired or protected"
                );
                self.prepare_api_tab_retention(member.document)?;
            } else {
                ensure!(
                    self.active_document()
                        .is_some_and(|doc| doc.id == job.source.document()),
                    "Original discard target is no longer active"
                );
            }
            let index = self
                .documents
                .iter()
                .position(|doc| doc.id == job.source.document())
                .context("Discard model no longer visible")?;
            ensure!(
                job.source.current(&self.documents[index])
                    && self.documents[index].language_epoch() == job.language_epoch,
                "Original discard text, resource, baseline or history changed"
            );
            let total: usize = self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .map(|doc| doc.text.len_bytes())
                .sum();
            ensure!(
                total
                    .saturating_sub(self.documents[index].text.len_bytes())
                    .saturating_add(next.len_bytes())
                    <= 4 * 1024 * 1024,
                "Discard result exceeds current mirror budget"
            );
            self.invalidate_disk_watch_publications()?;
            self.documents[index].reload_content_checked(next)?;
            if let Some(member) = job.member {
                self.finish_tab_close_eligible(member, job.eligibility);
            } else {
                if !matches!(job.action, AfterSave::Close) {
                    self.remove_active();
                }
                self.complete_close(job.action);
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.message = format!("Discard refused; original unsaved work retained: {error:#}");
        }
        true
    }
}
