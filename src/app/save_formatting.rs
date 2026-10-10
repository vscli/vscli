//! One save participant; actual formatter capacity belongs to the LSP client.
use super::*;
use crate::{lsp::Request, settings::SaveFormatting};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_millis(1500);
#[cfg(test)]
mod tests;

struct Pending {
    intent: u64,
    profile: u64,
    token: u64,
    server: Arc<()>,
    document: u64,
    path: PathBuf,
    revision: u64,
    epoch: u64,
    saved_revision: u64,
    save_generation: u64,
    settings: Arc<Vec<serde_json::Map<String, Value>>>,
    workspace: PathBuf,
    started: Instant,
}
#[derive(Default)]
pub(super) struct State {
    active: Option<Pending>,
}
impl State {
    pub(super) fn pending(&self) -> bool {
        self.active.is_some()
    }
    pub(super) fn document_pending(&self, id: u64) -> bool {
        self.active
            .as_ref()
            .is_some_and(|pending| pending.document == id)
    }
}
impl App {
    pub(super) fn cancel_save_formatting(&mut self) {
        if let Some(pending) = self.saving.formatting.active.take()
            && let Some(client) = &mut self.lsp
            && Arc::ptr_eq(&pending.server, &client.identity())
            && let Err(error) = client.cancel_formatting_request(pending.token)
        {
            self.message = format!("Formatting cancellation failed: {error:#}");
        }
    }
    pub(super) fn retire_save_formatting(&mut self, reason: &str) -> bool {
        if !self.saving.formatting.pending() {
            return false;
        }
        self.cancel_save_formatting();
        self.saving.latest = None;
        self.message = format!("Save retired; unsaved work retained: {reason}");
        true
    }
    pub(super) fn escape_save_formatting(&mut self) -> bool {
        if !self.retire_save_formatting("formatting canceled") {
            return false;
        }
        self.cancel_save_continuations();
        true
    }
    fn formatting_model_current(&self, pending: &Pending) -> bool {
        self.settings_error.is_none()
            && Arc::ptr_eq(&pending.settings, self.settings.extension_layers())
            && pending.profile == self.settings_profile_generation()
            && pending.workspace == self.workspace.root
            && self.saving.latest.as_ref().is_some_and(|intent| {
                intent.id == pending.intent
                    && intent.document == pending.document
                    && intent.stage == saving::Stage::Formatting
            })
            && self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .any(|doc| {
                    doc.id == pending.document
                        && doc.path.as_ref() == Some(&pending.path)
                        && doc.revision == pending.revision
                        && doc.text_epoch() == pending.epoch
                        && doc.saved_revision == pending.saved_revision
                        && doc.save_generation() == pending.save_generation
                })
    }
    fn formatting_server_current(&self, pending: &Pending) -> bool {
        self.lsp.as_ref().is_some_and(|client| {
            Arc::ptr_eq(&pending.server, &client.identity())
                && client.formatting_request_current(pending.token)
        })
    }
    fn formatting_skip(&mut self, notice: impl Into<String>) {
        if let Some(intent) = &mut self.saving.latest {
            intent.stage = saving::Stage::Capture;
            intent.add_notice(format!("Formatting skipped: {}", notice.into()));
        }
    }
    pub(super) fn begin_save_formatting(&mut self, intent: &saving::Intent) -> Result<bool> {
        if intent.automatic.is_some() {
            return Ok(false);
        }
        let Some(doc) = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .find(|doc| doc.id == intent.document)
        else {
            return Ok(false);
        };
        let language = doc
            .path
            .as_deref()
            .map_or("plaintext", crate::languages::language);
        match self.settings.save_formatting(language) {
            SaveFormatting::Off => return Ok(false),
            SaveFormatting::Unavailable(notice) => {
                self.formatting_skip(notice);
                return Ok(false);
            }
            SaveFormatting::File => {}
        }
        if self.settings_error.is_some() {
            self.formatting_skip("settings failed to load");
            return Ok(false);
        }
        if intent.destination.is_some()
            || doc.path.is_none()
            || !self.documents.iter().any(|visible| visible.id == doc.id)
        {
            self.formatting_skip("first save, Save As and hidden targets are not qualified yet");
            return Ok(false);
        }
        let Some(client) = &mut self.lsp else {
            self.formatting_skip("no native language server");
            return Ok(false);
        };
        if !client.supports_document_formatting() || !client.formatting_available() {
            let notice = if !client.supports_document_formatting() {
                "native server has no document formatter"
            } else if client.formatting_channel_closed() {
                "formatter timed out; awaiting actual reply or server restart"
            } else {
                "formatter is not ready or a previous callback is still running"
            };
            self.formatting_skip(notice);
            return Ok(false);
        }
        let server = client.identity();
        let token = match client.request_document_formatting(doc) {
            Ok(token) => token,
            Err(error) => {
                self.formatting_skip(format!(
                    "native formatter unavailable: {}",
                    format!("{error:#}").chars().take(500).collect::<String>()
                ));
                return Ok(false);
            }
        };
        self.saving.formatting.active = Some(Pending {
            intent: intent.id,
            profile: self.settings_profile_generation(),
            token,
            server,
            document: doc.id,
            path: doc.path.clone().expect("named target checked"),
            revision: doc.revision,
            epoch: doc.text_epoch(),
            saved_revision: doc.saved_revision,
            save_generation: doc.save_generation(),
            settings: self.settings.extension_layers().clone(),
            workspace: self.workspace.root.clone(),
            started: Instant::now(),
        });
        self.message = "Formatting before save… Escape cancels".into();
        Ok(true)
    }
    pub(super) fn save_formatting_owned(&self, request: &Request) -> bool {
        self.saving
            .formatting
            .active
            .as_ref()
            .is_some_and(|pending| {
                pending.token == request.token && Arc::ptr_eq(&pending.server, &request.server)
            })
    }
    pub(super) fn save_formatting_result(
        &mut self,
        request: Request,
        result: Result<Value, String>,
    ) {
        let current = self
            .saving
            .formatting
            .active
            .as_ref()
            .is_some_and(|pending| {
                self.formatting_model_current(pending)
                    && self
                        .lsp
                        .as_ref()
                        .is_some_and(|client| client.request_current(&request))
                    && request.document_id == pending.document
                    && request.text_epoch == pending.epoch
            });
        if !current {
            self.retire_save_formatting("document or server ownership changed");
            return;
        }
        if self
            .saving
            .formatting
            .active
            .as_ref()
            .is_some_and(|pending| {
                Instant::now().saturating_duration_since(pending.started) >= DEADLINE
            })
        {
            self.saving.formatting.active = None;
            self.formatting_skip("1500 ms save deadline elapsed; late formatter result discarded after actual settlement");
            return;
        }
        self.saving.formatting.active = None;
        let result = result.map_err(anyhow::Error::msg).and_then(|edits| {
            let doc = self
                .documents
                .iter_mut()
                .chain(&mut self.hidden_documents)
                .find(|doc| doc.id == request.document_id)
                .ok_or_else(|| anyhow::anyhow!("Formatting model retired"))?;
            let changes = crate::save_formatting_edits::stage(doc, &edits)?;
            doc.apply_changes(changes);
            Ok(())
        });
        if let Some(intent) = &mut self.saving.latest {
            intent.stage = saving::Stage::Capture;
        }
        if let Err(error) = result {
            self.formatting_skip(format!("{error:#}").chars().take(500).collect::<String>());
        }
    }
    pub(super) fn poll_save_formatting(&mut self, now: Instant) -> bool {
        let Some(pending) = self.saving.formatting.active.as_ref() else {
            return false;
        };
        if !self.formatting_model_current(pending) {
            return self.retire_save_formatting("document or settings changed during formatting");
        }
        if !self.formatting_server_current(pending) {
            return self
                .retire_save_formatting("native server synchronization changed during formatting");
        }
        if now.saturating_duration_since(pending.started) >= DEADLINE {
            self.cancel_save_formatting();
            self.formatting_skip(
                "1500 ms save deadline elapsed; previous callback remains occupied until its reply",
            );
            return true;
        }
        false
    }
}
