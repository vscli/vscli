//! Typed metadata/selection publication and retained mirror-before-answer receipt.
use super::*;
use crate::extensions::{
    editor_state::{Capture, Change},
    services::{NativeService, Operation},
};
use anyhow::{Context as _, ensure};

pub(super) struct Completed {
    service: NativeService,
    result: Result<Value, String>,
}
impl App {
    fn known_document_language(&self, host: &crate::extensions::Client, language: &str) -> bool {
        matches!(
            language,
            "plaintext"
                | "c"
                | "cpp"
                | "rust"
                | "python"
                | "javascript"
                | "typescript"
                | "javascriptreact"
                | "typescriptreact"
                | "tsx"
                | "go"
                | "json"
                | "jsonc"
                | "sql"
                | "markdown"
                | "toml"
                | "shellscript"
                | "html"
                | "css"
        ) || host.selected_language_known(language)
            || self.registered_document_language(language)
    }
    fn apply_editor_state(
        &mut self,
        host: &mut crate::extensions::Client,
        capture: &Capture,
    ) -> Result<Value> {
        ensure!(
            self.documents
                .len()
                .saturating_add(self.hidden_documents.len())
                <= 128,
            "Editor state inventory exceeds 128 models"
        );
        ensure!(
            self.documents
                .iter()
                .chain(&self.hidden_documents)
                .map(|doc| doc.text.len_bytes())
                .sum::<usize>()
                <= 4 * 1024 * 1024
                && self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .all(|doc| doc.secondary.len() < 4096),
            "Editor state mirror source or selection budget exceeded"
        );
        let doc = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .find(|doc| doc.id == capture.document)
            .context("Editor state model closed")?;
        ensure!(
            self.settings_error.is_none()
                && capture.current(
                    doc,
                    &self.workspace.root,
                    self.settings.extension_layers(),
                    self.settings_profile_generation()
                ),
            "Original editor state source or scope changed"
        );
        if let Some(editor) = &capture.editor {
            ensure!(
                host.editor_view_live(editor, doc),
                "Original visible native editor retired"
            );
        }
        let model_selections = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .map(|doc| doc.secondary.len() + 1)
            .sum::<usize>();
        let explicit = host.editor_inventory_explicit();
        ensure!(
            !explicit || self.panes.len() <= 4,
            "Visible editor inventory exceeds four groups"
        );
        let pane_selections = if explicit {
            self.panes
                .iter()
                .map(|pane| {
                    self.documents
                        .iter()
                        .find(|doc| doc.id == pane.document)
                        .and_then(|doc| doc.editor_selection_count_for(pane.id))
                        .context("Visible native editor view retired")
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .sum::<usize>()
        } else {
            0
        };
        let mut prospective_selections = model_selections.saturating_add(pane_selections);
        if let Change::Selections(requested) = &capture.change {
            let target_count = doc
                .editor_selection_count_for(capture.view_id())
                .context("Captured editor view retired")?;
            // Models mirror their internally active view; the visible inventory
            // additionally mirrors every pane. An inactive sibling only changes
            // its own pane entry, while the current model view changes both.
            if doc.editor_view_stamp().0 == capture.view_id() {
                prospective_selections = prospective_selections
                    .saturating_sub(target_count)
                    .saturating_add(requested.len());
            }
            if explicit {
                ensure!(
                    self.panes
                        .iter()
                        .any(|pane| pane.document == capture.document
                            && pane.id == capture.view_id()),
                    "Captured visible editor retired"
                );
                prospective_selections = prospective_selections
                    .saturating_sub(target_count)
                    .saturating_add(requested.len());
            }
        }
        ensure!(
            prospective_selections <= 16_384,
            "Editor state aggregate selection budget exceeded"
        );
        match &capture.change {
            Change::Language(language) => {
                ensure!(
                    self.known_document_language(host, language),
                    "Unknown document language: {language}"
                );
                let current = doc.language_override().unwrap_or_else(|| {
                    doc.path
                        .as_deref()
                        .map_or("plaintext", crate::languages::language)
                });
                if current != language {
                    let configuration = self.language_configuration(language);
                    let settings = self.settings.clone();
                    let doc = self
                        .documents
                        .iter_mut()
                        .chain(&mut self.hidden_documents)
                        .find(|doc| doc.id == capture.document)
                        .expect("validated retained model");
                    // Configuration validation and the checked language clock
                    // precede all mutation. Both constructors are bounded.
                    doc.check_language_override(Some(language))?;
                    doc.set_language_configuration(configuration)?;
                    doc.set_language_override(Some(language.clone()))?;
                    settings.apply(doc);
                    // Current source/options keys retire stale syntax/outline
                    // projections. Automatic LSP lifecycle observes the override
                    // in the next ordinary poll without blocking this reply.
                }
            }
            Change::Selections(_) => {
                let doc = self
                    .documents
                    .iter_mut()
                    .chain(&mut self.hidden_documents)
                    .find(|doc| doc.id == capture.document)
                    .expect("validated retained model");
                capture.apply_selections(doc)?;
                if let Some(editor) = &capture.editor {
                    host.mark_api_editor_selections(editor);
                } else {
                    host.mark_api_selections(capture.document);
                }
            }
        }
        Ok(if let Some(editor) = &capture.editor {
            json!({"document":capture.document,"editor":editor})
        } else {
            json!({"document":capture.document})
        })
    }
    pub(super) fn start_editor_state_service(
        &mut self,
        host: &mut crate::extensions::Client,
        service: NativeService,
    ) {
        let result = match &service.operation {
            Operation::EditorState(capture) => self.apply_editor_state(host, capture),
            _ => Err(anyhow::anyhow!("Invalid editor state service")),
        };
        self.extension_services.editor_state_completed = Some(Completed {
            service,
            result: result.map_err(|error| crate::extensions::services::bounded_error(&error)),
        });
        self.poll_editor_state_completion(host);
    }
    pub(super) fn poll_editor_state_completion(
        &mut self,
        host: &mut crate::extensions::Client,
    ) -> bool {
        let Some(done) = self.extension_services.editor_state_completed.take() else {
            return false;
        };
        if done.service.request.session != host.session {
            return true;
        }
        let exact = host.service().is_some_and(|service| {
            service.id == done.service.id && service.request.owner == done.service.request.owner
        });
        if !exact {
            return true;
        }
        self.refresh_extension_editor_inventory();
        host.set_editor_inventory(self.extension_editor_inventory.clone());
        if let Err(error) =
            host.sync_with_hidden(&self.documents, &self.hidden_documents, self.active)
        {
            self.message = format!("Editor state mirror receipt retained: {error:#}");
            self.extension_services.editor_state_completed = Some(done);
            return false;
        }
        let result = host
            .service_valid(&done.service)
            .and_then(|()| done.result.clone().map_err(anyhow::Error::msg));
        if let Err(error) = host.answer_service(&done.service, result) {
            self.message = format!("Editor state acknowledgement retained: {error:#}");
            self.extension_services.editor_state_completed = Some(done);
            return false;
        }
        true
    }
}
