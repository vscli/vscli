use super::*;
use crate::lsp::{self, Event, Request};
use anyhow::{Context, bail};

/// A borrowed view of current diagnostics from independently owned sources.
/// Iteration does not clone messages or allocate on the rendering path.
pub struct Diagnostics<'a> {
    native: &'a [lsp::Diagnostic],
    host: Option<&'a crate::extensions::Client>,
    document: Option<&'a Document>,
}
impl<'a> Diagnostics<'a> {
    pub fn iter(&self) -> impl Iterator<Item = &'a lsp::Diagnostic> + use<'a> {
        let host = self.host;
        let document = self.document;
        self.native.iter().chain(
            host.into_iter()
                .zip(document)
                .flat_map(|(host, doc)| host.diagnostics_for(doc)),
        )
    }
    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }
    pub fn len(&self) -> usize {
        self.iter().count()
    }
}
impl std::ops::Index<usize> for Diagnostics<'_> {
    type Output = lsp::Diagnostic;
    fn index(&self, index: usize) -> &Self::Output {
        self.iter()
            .nth(index)
            .expect("Diagnostic index out of bounds")
    }
}
impl std::fmt::Debug for Diagnostics<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

pub struct LanguageItem {
    pub label: String,
    pub action: LanguageAction,
}
pub enum LanguageAction {
    ExtensionCodeAction {
        ticket: crate::extensions::providers::Ticket,
        item: Value,
    },
    Provider {
        ticket: crate::extensions::providers::Ticket,
        item: Value,
    },
    Location {
        path: PathBuf,
        range: lsp::Range,
    },
    DiagnosticLocation {
        document: u64,
        text_epoch: u64,
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
                let before = self.diagnostics.len();
                self.prune_native_diagnostics();
                let changed = !events.is_empty() || before != self.diagnostics.len();
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
                        Event::SignatureFailure(request, error) => {
                            self.signature_failure(&request, &error);
                        }
                        Event::SymbolFailure(request, error) => {
                            if self.symbol_request_owned(&request) {
                                self.symbol_failure(&request, error);
                            } else {
                                self.outline_native_failure(&request, &error);
                            }
                        }
                        Event::Diagnostics(publication) => {
                            if let Some(doc) = self
                                .documents
                                .iter()
                                .find(|d| d.path.as_ref() == Some(&publication.path))
                                && self.lsp.as_ref().is_some_and(|client| {
                                    client.diagnostic_current(&publication, doc)
                                })
                            {
                                self.prune_native_diagnostics();
                                if let Err(error) = self.retain_diagnostic_publication(publication)
                                {
                                    self.message =
                                        format!("Diagnostic publication rejected: {error:#}");
                                }
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
        if method == "textDocument/signatureHelp" {
            self.request_signature();
            return;
        }
        if method == "textDocument/codeAction" {
            let kind = extra["context"]["only"]
                .as_array()
                .and_then(|only| only.first())
                .and_then(Value::as_str);
            self.request_all_code_actions(kind);
            return;
        }
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
            || self.doc().text_epoch() != request.text_epoch
            || self.doc().path.as_ref() != Some(&request.path)
            || self
                .lsp
                .as_ref()
                .is_none_or(|client| !client.request_current(request))
        {
            bail!("Document changed since this request; invoke the command again");
        }
        Ok(())
    }
    fn language_response(&mut self, request: Request, response: Value) -> Result<()> {
        if matches!(
            request.method.as_str(),
            "textDocument/codeAction" | "codeAction/resolve"
        ) {
            return self.native_action_reply(request, response);
        }
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
            if self.outline_native_owned(&request) {
                return self.outline_native_response(&request, &response);
            }
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
            LanguageAction::ExtensionCodeAction { ticket, item } => {
                self.apply_extension_code_action(ticket, item, false)?;
            }
            LanguageAction::Provider { ticket, item } => {
                self.apply_provider_action(ticket, item)?
            }
            LanguageAction::CodeAction { request, item } => {
                self.select_native_action(request)?;
                self.apply_code_action(request, item, false)?
            }
            LanguageAction::Location { path, range } => {
                self.open_with_intent(path, super::navigation::OpenIntent::Location(*range))?;
            }
            LanguageAction::DiagnosticLocation {
                document,
                text_epoch,
                range,
            } => {
                let doc = self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .find(|doc| doc.id == *document)
                    .context("Problem document was closed; reopen Problems")?;
                if doc.text_epoch() != *text_epoch {
                    bail!("Problem document changed; reopen Problems");
                }
                let offset = lsp::offset(doc, range.start)?;
                let previous = self.suspend_navigation_observation();
                self.cancel_navigation();
                if let Some(index) = self
                    .hidden_documents
                    .iter()
                    .position(|doc| doc.id == *document)
                {
                    self.documents.push(self.hidden_documents.remove(index));
                }
                self.active = self
                    .documents
                    .iter()
                    .position(|doc| doc.id == *document)
                    .unwrap();
                self.focus = Focus::Editor;
                self.sync_pane();
                self.doc_mut().clear_secondary();
                self.doc_mut().move_to(offset, false);
                self.sync_pane();
                self.remember_active_file();
                self.resume_navigation_observation(previous, navigation_history::Reason::Jump);
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
    pub fn current_diagnostics(&self) -> Diagnostics<'_> {
        self.diagnostics_for_document(self.active_document())
    }
    fn prune_native_diagnostics(&mut self) {
        self.diagnostics.retain(|_, publication| {
            self.documents
                .iter()
                .find(|doc| doc.id == publication.snapshot.id)
                .is_some_and(|doc| {
                    self.lsp
                        .as_ref()
                        .is_some_and(|client| client.diagnostic_current(publication, doc))
                })
        });
    }
    fn retain_diagnostic_publication(
        &mut self,
        publication: lsp::DiagnosticPublication,
    ) -> Result<()> {
        if publication.items.is_empty() {
            self.diagnostics.remove(&publication.path);
            return Ok(());
        }
        let mut resources = 1;
        let mut items = publication.items.len();
        let mut bytes = publication.serialized_bytes;
        for old in self
            .diagnostics
            .values()
            .filter(|old| old.path != publication.path)
        {
            resources += 1;
            items += old.items.len();
            bytes += old.serialized_bytes;
        }
        if resources > 128 || items > 5000 || bytes > 2 * 1024 * 1024 {
            bail!("Native diagnostic cache exceeds 128 resources, 5,000 diagnostics or 2 MiB");
        }
        self.diagnostics
            .insert(publication.path.clone(), publication);
        Ok(())
    }
    fn diagnostics_for_document<'a>(&'a self, document: Option<&'a Document>) -> Diagnostics<'a> {
        let native = document
            .and_then(|doc| {
                let publication = self.diagnostics.get(doc.path.as_ref()?)?;
                self.lsp
                    .as_ref()?
                    .diagnostic_current(publication, doc)
                    .then_some(publication.items.as_slice())
            })
            .unwrap_or(&[]);
        Diagnostics {
            native,
            host: self.extension_host.as_ref(),
            document,
        }
    }
    pub(super) fn show_problems(&mut self) {
        let mut items = Vec::new();
        let mut truncated = false;
        'documents: for doc in self.documents.iter().chain(&self.hidden_documents) {
            for diagnostic in self.diagnostics_for_document(Some(doc)).iter() {
                if items.len() == 5000 {
                    truncated = true;
                    break 'documents;
                }
                items.push(LanguageItem {
                    label: format!(
                        "{} · {}:{}:{}  {}",
                        match diagnostic.severity {
                            Some(1) => "Error",
                            Some(2) => "Warning",
                            Some(3) => "Information",
                            Some(4) => "Hint",
                            _ => "Problem",
                        },
                        doc.path
                            .as_ref()
                            .map_or_else(|| doc.name(), |path| self.workspace.relative(path)),
                        diagnostic.range.start.line + 1,
                        diagnostic.range.start.character + 1,
                        diagnostic.message
                    ),
                    action: LanguageAction::DiagnosticLocation {
                        document: doc.id,
                        text_epoch: doc.text_epoch(),
                        range: diagnostic.range,
                    },
                });
            }
        }
        items.sort_by(|a, b| a.label.cmp(&b.label));
        self.modal = Some(Modal::Language {
            title: if truncated {
                " Problems · first 5,000 · Enter opens · Esc closes "
            } else {
                " Problems · Enter opens · Esc closes "
            }
            .into(),
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

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    fn publication(path: PathBuf, count: usize, bytes: usize) -> lsp::DiagnosticPublication {
        lsp::DiagnosticPublication {
            uri: lsp::file_uri(&path).unwrap(),
            path,
            server: std::sync::Arc::new(()),
            snapshot: lsp::Snapshot {
                id: 1,
                revision: 0,
                text_epoch: 0,
                version: 1,
            },
            versioned: true,
            serialized_bytes: bytes,
            items: vec![
                lsp::Diagnostic {
                    range: lsp::Range {
                        start: lsp::Position {
                            line: 0,
                            character: 0
                        },
                        end: lsp::Position {
                            line: 0,
                            character: 1
                        }
                    },
                    severity: Some(2),
                    message: "warning".into(),
                    extra: Default::default(),
                };
                count
            ],
        }
    }
    #[test]
    fn native_cache_replacement_is_atomic_under_global_budgets() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        let first = root.path().join("one.rs");
        let second = root.path().join("two.rs");
        app.retain_diagnostic_publication(publication(first.clone(), 3000, 1024))
            .unwrap();
        assert!(
            app.retain_diagnostic_publication(publication(second.clone(), 2001, 1024))
                .is_err()
        );
        assert_eq!(app.diagnostics.len(), 1);
        assert_eq!(app.diagnostics[&first].items.len(), 3000);
        app.retain_diagnostic_publication(publication(first.clone(), 1, 2 * 1024 * 1024))
            .unwrap();
        assert!(
            app.retain_diagnostic_publication(publication(second, 1, 1))
                .is_err()
        );
        assert_eq!(app.diagnostics[&first].serialized_bytes, 2 * 1024 * 1024);
        app.retain_diagnostic_publication(publication(first.clone(), 0, 2))
            .unwrap();
        assert!(!app.diagnostics.contains_key(&first));
        app.retain_diagnostic_publication(publication(root.path().join("absent.rs"), 0, 2))
            .unwrap();
        assert!(app.diagnostics.is_empty());
        app.diagnostics.clear();
        for index in 0..128 {
            app.retain_diagnostic_publication(publication(
                root.path().join(format!("{index}.rs")),
                1,
                2,
            ))
            .unwrap();
        }
        assert!(
            app.retain_diagnostic_publication(publication(first, 1, 2))
                .is_err()
        );
        assert_eq!(app.diagnostics.len(), 128);
    }
    #[test]
    fn problem_navigation_preserves_hidden_untitled_identity_and_rejects_edit_undo() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        let hidden = Document::from_text("line\nproblem\n");
        let id = hidden.id;
        let epoch = hidden.text_epoch();
        app.hidden_documents.push(hidden);
        let action = LanguageAction::DiagnosticLocation {
            document: id,
            text_epoch: epoch,
            range: lsp::Range {
                start: lsp::Position {
                    line: 1,
                    character: 0,
                },
                end: lsp::Position {
                    line: 1,
                    character: 1,
                },
            },
        };
        app.language_action(&action).unwrap();
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().row(), 1);
        assert!(app.hidden_documents.is_empty());
        let text = app.doc().text.to_string();
        app.doc_mut().insert("edit", false);
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), text);
        let selections = app.doc().selections();
        assert!(
            app.language_action(&action)
                .unwrap_err()
                .to_string()
                .contains("changed")
        );
        assert_eq!(app.doc().selections(), selections);
        assert_eq!(app.doc().id, id);
        assert!(app.doc().path.is_none());
        app.documents.clear();
        assert!(
            app.language_action(&action)
                .unwrap_err()
                .to_string()
                .contains("closed")
        );
    }
}
