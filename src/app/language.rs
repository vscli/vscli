use super::*;
use crate::lsp::{self, Event, Request};
use anyhow::{Context, bail};

pub struct LanguageItem {
    pub label: String,
    pub action: LanguageAction,
}
pub enum LanguageAction {
    Provider {
        ticket: crate::extensions::providers::Ticket,
        item: Value,
    },
    Location {
        path: PathBuf,
        range: lsp::Range,
    },
    Completion {
        request: Request,
        item: Value,
    },
    CodeAction {
        request: std::sync::Arc<Request>,
        item: Value,
    },
}
impl App {
    pub(super) fn poll_language(&mut self) -> bool {
        let Some(client) = self.lsp.as_mut() else {
            return false;
        };
        let events = client.sync(&self.documents).and_then(|_| client.poll());
        match events {
            Err(error) => {
                self.language_service_failed(&format!("{error:#}"));
                true
            }
            Ok(events) => {
                let changed = !events.is_empty();
                for event in events {
                    match event {
                        Event::ApplyEdit(id, request, edit) => {
                            let result = request
                                .context("Unsolicited workspace edit; invoke a code action first")
                                .and_then(|request| {
                                    self.code_action_current(&request)?;
                                    self.apply_action_edit(&request, edit)
                                });
                            if let Err(error) = &result {
                                self.message = format!("Workspace edit rejected: {error:#}");
                            }
                            if let Some(client) = &self.lsp
                                && let Err(error) = client.acknowledge_edit(id, result)
                            {
                                self.message = format!("Workspace edit reply failed: {error:#}");
                            }
                        }
                        Event::Ready => self.message = "Language server ready".into(),
                        Event::Message(message) => self.message = message,
                        Event::Diagnostics(path, diagnostics) => {
                            if let Some(doc) = self
                                .documents
                                .iter()
                                .find(|d| d.path.as_ref() == Some(&path))
                            {
                                self.diagnostics
                                    .insert(path, (doc.id, doc.revision, diagnostics));
                            }
                        }
                        Event::Response(request, response) => {
                            if let Err(error) = self.language_response(request, response) {
                                self.message = format!("Language response: {error:#}");
                            }
                        }
                    }
                }
                changed
            }
        }
    }
    pub(super) fn language_request(&mut self, method: &str, extra: Value) {
        if method == "textDocument/completion" {
            self.request_suggestions(extra);
            return;
        }
        if self.extension_language_request(method, extra.clone()) {
            return;
        }
        let Some(client) = self.lsp.as_mut() else {
            self.message =
                "Language server not ready; use Language: Server Status or Language: Restart Server"
                    .into();
            return;
        };
        match client.sync(&self.documents).and_then(|_| {
            client.request_in_view(
                method,
                &self.documents[self.active],
                extra,
                self.panes.get(self.active_pane).map(|pane| pane.id),
            )
        }) {
            Ok(()) => self.message = format!("Language request: {method}"),
            Err(error) => self.message = format!("Language request failed: {error:#}"),
        }
    }
    pub(super) fn language_saved(&mut self) {
        if let Some(client) = self.lsp.as_mut()
            && let Err(error) = client
                .sync(&self.documents)
                .and_then(|_| client.saved(&self.documents[self.active]))
        {
            self.message = format!("File saved; language server notification failed: {error:#}");
        }
    }
    pub(super) fn request_current(&self, request: &Request) -> Result<()> {
        if self.active_document().is_none()
            || self.doc().id != request.document_id
            || self.doc().revision != request.revision
            || self.doc().path.as_ref() != Some(&request.path)
        {
            bail!("Document changed since this request; invoke the command again");
        }
        Ok(())
    }
    fn language_response(&mut self, request: Request, response: Value) -> Result<()> {
        if request.method == "completionItem/resolve" {
            return self.native_suggestion_resolve_response(request, response);
        }
        if request.method == "textDocument/completion" {
            return self.native_suggestion_response(request, response);
        }
        if matches!(
            request.method.as_str(),
            "textDocument/documentSymbol" | "workspace/symbol"
        ) {
            return self.symbol_response(&request, &response);
        }
        if request.method == "workspace/executeCommand" {
            // Command results are not WorkspaceEdits; mutations arrive through applyEdit.
            return Ok(());
        }
        if request.method == "textDocument/signatureHelp" {
            return self.signature_response(&request, &response);
        }
        self.request_current(&request)?;
        if response.is_null() {
            self.message = "Language server returned no results".into();
            return Ok(());
        }
        match request.method.as_str() {
            "textDocument/codeAction" => {
                self.code_action_current(&request)?;
                if self.prompt.is_some() || self.modal.is_some() {
                    bail!("Input context changed; request code actions again");
                }
                let Value::Array(mut actions) = response else {
                    bail!("Invalid code action response");
                };
                if actions.len() > 300 {
                    bail!("Code action picker exceeds 300 items; narrow the selection");
                }
                let request = std::sync::Arc::new(request);
                actions.sort_by_key(|item| !item["isPreferred"].as_bool().unwrap_or(false));
                let items = actions
                    .into_iter()
                    .take(300)
                    .map(|item| {
                        let label = format!(
                            "{}{}",
                            item["title"]
                                .as_str()
                                .unwrap_or("Untitled action")
                                .chars()
                                .take(1024)
                                .collect::<String>(),
                            item["disabled"]["reason"]
                                .as_str()
                                .map_or(String::new(), |r| format!(" (disabled: {r})"))
                        );
                        LanguageItem {
                            label,
                            action: LanguageAction::CodeAction {
                                request: request.clone(),
                                item,
                            },
                        }
                    })
                    .collect::<Vec<_>>();
                if items.is_empty() {
                    self.message = "No code actions available".into();
                } else {
                    self.modal = Some(Modal::Language {
                        title: " Code Actions · Enter applies · Esc closes ".into(),
                        items,
                        selected: 0,
                    });
                }
            }
            "codeAction/resolve" => self.apply_code_action(&request, &response, true)?,
            "textDocument/hover" => {
                if self.doc().cursor != request.cursor {
                    return Ok(());
                }
                self.modal = Some(Modal::Text {
                    title: " Hover · Esc closes ".into(),
                    scroll: 0,
                    text: content_text(&response["contents"]),
                });
            }
            "textDocument/definition" | "textDocument/references" => {
                let values = response
                    .as_array()
                    .cloned()
                    .unwrap_or_else(|| vec![response]);
                let mut items = Vec::new();
                for location in values.into_iter().take(5000) {
                    let uri = location["uri"]
                        .as_str()
                        .or_else(|| location["targetUri"].as_str())
                        .context("Missing location URI")?;
                    let path = lsp::uri_path(uri)?;
                    let range: lsp::Range = serde_json::from_value(
                        location
                            .get("range")
                            .or_else(|| location.get("targetSelectionRange"))
                            .context("Missing location range")?
                            .clone(),
                    )?;
                    items.push(LanguageItem {
                        label: format!(
                            "{}:{}:{}",
                            self.workspace.relative(&path),
                            range.start.line + 1,
                            range.start.character + 1
                        ),
                        action: LanguageAction::Location { path, range },
                    });
                }
                if items.len() == 1 {
                    self.language_action(&items.remove(0).action)?;
                } else {
                    self.modal = Some(Modal::Language {
                        title: " Locations · Enter opens · Esc closes ".into(),
                        items,
                        selected: 0,
                    });
                }
            }
            "textDocument/formatting" => {
                let changes = lsp::edits(self.doc(), serde_json::from_value(response)?)?;
                let count = changes.len();
                self.doc_mut().apply_changes(changes);
                self.message = format!("Applied {count} formatting edits · Undo restores");
            }
            "textDocument/rename" => {
                self.apply_workspace_edit(response)?;
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn language_action(&mut self, action: &LanguageAction) -> Result<()> {
        match action {
            LanguageAction::Provider { ticket, item } => {
                self.apply_provider_action(ticket, item)?
            }
            LanguageAction::CodeAction { request, item } => {
                self.apply_code_action(request, item, false)?
            }
            LanguageAction::Location { path, range } => {
                self.open_with_intent(path, super::navigation::OpenIntent::Location(*range))?;
            }
            LanguageAction::Completion { request, item } => {
                self.request_current(request)?;
                if self.doc().cursor != request.cursor {
                    bail!("Cursor moved; request completion again");
                }
                self.apply_completion_item(item, request.cursor)?;
            }
        }
        Ok(())
    }
    fn apply_workspace_edit(&mut self, response: Value) -> Result<()> {
        // Stage every edit before changing a buffer. Files remain unsaved for review.
        if response.get("documentChanges").is_some() {
            bail!("Versioned workspace edits are not supported yet");
        }
        let changes = response["changes"]
            .as_object()
            .context("No workspace text edits")?;
        let mut staged = Vec::new();
        for (uri, edits) in changes {
            let path = lsp::uri_path(uri)?;
            if let Some(index) = self
                .documents
                .iter()
                .position(|d| d.path.as_ref() == Some(&path))
            {
                // Until workspace snapshots are part of requests, refuse edits to other dirty buffers.
                if index != self.active && self.documents[index].dirty() {
                    bail!("Rename touches another unsaved buffer; save it and retry");
                }
                let edits = lsp::edits(
                    &self.documents[index],
                    serde_json::from_value(edits.clone())?,
                )?;
                staged.push((Some(index), None, edits));
            } else {
                let doc = Document::open(&path)?;
                let edits = lsp::edits(&doc, serde_json::from_value(edits.clone())?)?;
                staged.push((None, Some(doc), edits));
            }
        }
        let count = staged.len();
        for (index, doc, edits) in staged {
            let index = index.unwrap_or_else(|| {
                self.documents.push(doc.unwrap());
                self.documents.len() - 1
            });
            self.documents[index].apply_changes(edits);
        }
        self.message =
            format!("Renamed across {count} buffers; review and save each file (Undo is per file)");
        Ok(())
    }
    pub fn current_diagnostics(&self) -> &[lsp::Diagnostic] {
        self.active_document()
            .and_then(|d| d.path.as_ref())
            .and_then(|p| self.diagnostics.get(p))
            .filter(|(id, rev, _)| *id == self.doc().id && *rev == self.doc().revision)
            .map_or(&[], |(_, _, items)| items.as_slice())
    }
    pub(super) fn show_problems(&mut self) {
        let mut items = Vec::new();
        for doc in &self.documents {
            if let Some(path) = &doc.path
                && let Some((id, revision, diagnostics)) = self.diagnostics.get(path)
                && *id == doc.id
                && *revision == doc.revision
            {
                for diagnostic in diagnostics {
                    items.push(LanguageItem {
                        label: format!(
                            "{}:{}  {}",
                            self.workspace.relative(path),
                            diagnostic.range.start.line + 1,
                            diagnostic.message
                        ),
                        action: LanguageAction::Location {
                            path: path.clone(),
                            range: diagnostic.range,
                        },
                    });
                }
            }
        }
        items.sort_by(|a, b| a.label.cmp(&b.label));
        self.modal = Some(Modal::Language {
            title: " Problems · Enter opens · Esc closes ".into(),
            items,
            selected: 0,
        });
    }
}
pub(super) fn content_text(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        text.to_owned()
    } else if let Some(items) = value.as_array() {
        items
            .iter()
            .map(content_text)
            .collect::<Vec<_>>()
            .join("\n\n")
    } else {
        value["value"].as_str().unwrap_or("").to_owned()
    }
}
