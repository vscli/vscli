use super::*;
use crate::lsp::{self, Event, Request, TextEdit};
use anyhow::{Context, bail};

pub struct LanguageItem {
    pub label: String,
    pub action: LanguageAction,
}
pub enum LanguageAction {
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
            "textDocument/completion" => {
                if self.doc().cursor != request.cursor {
                    return Ok(());
                }
                let mut items = response
                    .as_array()
                    .or_else(|| response["items"].as_array())
                    .cloned()
                    .unwrap_or_default();
                items.sort_by(|a, b| {
                    a["sortText"]
                        .as_str()
                        .or(a["label"].as_str())
                        .cmp(&b["sortText"].as_str().or(b["label"].as_str()))
                });
                let items = items
                    .into_iter()
                    .take(300)
                    .map(|item| LanguageItem {
                        label: format!(
                            "{}  {}",
                            item["label"].as_str().unwrap_or("?"),
                            item["detail"].as_str().unwrap_or("")
                        ),
                        action: LanguageAction::Completion {
                            request: request.clone(),
                            item,
                        },
                    })
                    .collect();
                self.modal = Some(Modal::Language {
                    title: " Completion · arrows select · Enter applies · Esc closes ".into(),
                    items,
                    selected: 0,
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
            LanguageAction::CodeAction { request, item } => {
                self.apply_code_action(request, item, false)?
            }
            LanguageAction::Location { path, range } => {
                self.open(path)?;
                let start = lsp::offset(self.doc(), range.start)?;
                let end = lsp::offset(self.doc(), range.end)?;
                self.doc_mut().clear_secondary();
                self.doc_mut().move_to(start, false);
                self.doc_mut().move_to(end, true);
            }
            LanguageAction::Completion { request, item } => {
                self.request_current(request)?;
                if self.doc().cursor != request.cursor {
                    bail!("Cursor moved; request completion again");
                }
                if item["insertTextFormat"].as_u64() == Some(2) {
                    bail!("Server sent a snippet despite snippetSupport=false");
                }
                let mut changes: Vec<TextEdit> = if let Some(edit) = item.get("textEdit") {
                    if edit.get("range").is_none() {
                        bail!("Server returned an unadvertised insert/replace edit");
                    }
                    vec![serde_json::from_value(edit.clone())?]
                } else {
                    let mut start = request.cursor;
                    while start > 0
                        && (self.doc().text.char(start - 1).is_alphanumeric()
                            || self.doc().text.char(start - 1) == '_')
                    {
                        start -= 1;
                    }
                    vec![TextEdit {
                        range: lsp::Range {
                            start: lsp::position(self.doc(), start),
                            end: lsp::position(self.doc(), request.cursor),
                        },
                        new_text: item["insertText"]
                            .as_str()
                            .or_else(|| item["label"].as_str())
                            .context("Completion has no text")?
                            .into(),
                    }]
                };
                let primary_end = lsp::offset(self.doc(), changes[0].range.end)?;
                if let Some(additional) = item.get("additionalTextEdits") {
                    changes.extend(serde_json::from_value::<Vec<TextEdit>>(additional.clone())?);
                }
                let changes = lsp::edits(self.doc(), changes)?;
                self.doc_mut().clear_secondary();
                self.doc_mut().move_to(primary_end, false);
                self.doc_mut().apply_changes(changes);
                self.message = "Completion applied".into();
                if item.get("command").is_some() {
                    self.message
                        .push_str("; follow-up server command is not supported yet");
                }
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
fn content_text(value: &Value) -> String {
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
