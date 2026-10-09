//! User-invoked LSP actions. Validate every target before changing any shared buffer.
use super::*;
use crate::lsp::{self, Request, TextEdit};
use anyhow::{Context, bail};
use std::collections::HashSet;

impl App {
    pub(super) fn request_code_actions(&mut self, kind: Option<&str>) {
        if self.active_document().is_none() {
            self.message = "Open a file before requesting code actions".into();
            return;
        }
        if !self.lsp.as_ref().is_some_and(|client| {
            client.ready
                && (client.capabilities["codeActionProvider"].is_object()
                    || client.capabilities["codeActionProvider"] == true)
        }) {
            self.message = "The configured language server does not provide code actions".into();
            return;
        }
        let selection = self
            .doc()
            .selection()
            .unwrap_or(self.doc().cursor..self.doc().cursor);
        let range = lsp::Range {
            start: lsp::position(self.doc(), selection.start),
            end: lsp::position(self.doc(), selection.end),
        };
        let diagnostics: Vec<_> = self
            .current_diagnostics()
            .iter()
            .filter(|d| {
                (d.range.start.line, d.range.start.character)
                    <= (range.end.line, range.end.character)
                    && (d.range.end.line, d.range.end.character)
                        >= (range.start.line, range.start.character)
            })
            .take(129)
            .collect();
        if diagnostics.len() > 128 {
            self.message = "More than 128 diagnostics intersect this selection; narrow it before requesting code actions".into();
            return;
        }
        let mut context = json!({"diagnostics":diagnostics,"triggerKind":1});
        if let Some(kind) = kind {
            context["only"] = json!([kind]);
        }
        self.focus = Focus::Editor;
        self.language_request(
            "textDocument/codeAction",
            json!({"range":range,"context":context}),
        );
    }

    pub(super) fn code_action_current(&self, request: &Request) -> Result<()> {
        self.request_current(request)?;
        if self.doc().selections() != request.selections
            || request.view.is_some_and(|view| {
                self.panes
                    .get(self.active_pane)
                    .is_none_or(|pane| pane.id != view)
            })
            || self.focus != Focus::Editor
        {
            bail!("Editor context changed; request code actions again");
        }
        Ok(())
    }

    pub(super) fn apply_code_action(
        &mut self,
        request: &Request,
        item: &Value,
        resolved: bool,
    ) -> Result<()> {
        self.code_action_current(request)?;
        if self.prompt.is_some() || self.modal.is_some() {
            bail!("Input context changed; request code actions again");
        }
        if let Some(disabled) = item.get("disabled") {
            bail!(
                "Code action is disabled: {}",
                disabled["reason"].as_str().unwrap_or("no reason given")
            );
        }
        let command = if item["command"].is_string() {
            Some(item)
        } else {
            item.get("command")
        };
        if command.is_some() && item.get("edit").is_some() {
            bail!("Combined edit-and-command actions are not supported; no edits were applied");
        }
        if let Some(command) = command {
            if !self.lsp.as_ref().is_some_and(|c| c.command_available()) {
                bail!("A language server command is already running or the request queue is full");
            }
            command["command"]
                .as_str()
                .context("Invalid code action command")?;
            if command
                .get("arguments")
                .is_some_and(|args| !args.is_array())
            {
                bail!("Invalid code action command arguments");
            }
        }
        if !resolved
            && item.get("edit").is_none()
            && item.get("data").is_some()
            && self
                .lsp
                .as_ref()
                .is_some_and(|c| c.capabilities["codeActionProvider"]["resolveProvider"] == true)
        {
            self.lsp
                .as_mut()
                .context("Language server disconnected")?
                .follow_up(request, "codeAction/resolve", item.clone())?;
            self.message = "Resolving code action…".into();
            return Ok(());
        }
        if let Some(edit) = item.get("edit") {
            self.apply_action_edit(request, edit.clone())?;
        } else if command.is_none() {
            bail!("Code action contains no supported edit or command");
        }
        if let Some(command) = command {
            let client = self.lsp.as_mut().context("Language server disconnected")?;
            client.sync(&self.documents)?;
            client.follow_up(request, "workspace/executeCommand", json!({
                "command":command["command"], "arguments":command.get("arguments").cloned().unwrap_or_else(|| json!([]))
            }))?;
            self.message = "Code action command running…".into();
        }
        Ok(())
    }

    pub(super) fn apply_action_edit(&mut self, request: &Request, edit: Value) -> Result<()> {
        self.code_action_current(request)?;
        if self.prompt.is_some() || self.modal.is_some() {
            bail!("Input context changed; workspace edit was rejected");
        }
        let object = edit
            .as_object()
            .context("Workspace edit must be an object")?;
        if object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "changes" | "documentChanges" | "changeAnnotations"
            )
        }) {
            bail!("Unsupported workspace edit fields");
        }
        if edit
            .get("changeAnnotations")
            .is_some_and(|v| v.as_object().is_none_or(|a| !a.is_empty()))
        {
            bail!("Annotated workspace edits require a review UI and are not supported yet");
        }
        let mut edits = Vec::new();
        if let Some(changes) = edit.get("documentChanges") {
            if edit.get("changes").is_some() {
                bail!("Ambiguous workspace edit contains both edit forms");
            }
            for change in changes.as_array().context("Invalid documentChanges")? {
                if change.get("kind").is_some() {
                    bail!(
                        "Workspace file creation, rename and deletion actions are not supported yet"
                    );
                }
                if change.as_object().is_none_or(|object| {
                    object
                        .keys()
                        .any(|key| !matches!(key.as_str(), "textDocument" | "edits"))
                }) {
                    bail!("Unsupported document edit fields");
                }
                let doc = change
                    .get("textDocument")
                    .context("Missing workspace edit document")?;
                let version = match doc.get("version") {
                    Some(Value::Null) => None,
                    Some(value) => Some(value.as_i64().context("Invalid workspace edit version")?),
                    None => bail!("Missing workspace edit version"),
                };
                edits.push((
                    doc["uri"]
                        .as_str()
                        .context("Missing workspace edit URI")?
                        .to_owned(),
                    version,
                    change["edits"].clone(),
                ));
            }
        } else if let Some(changes) = edit.get("changes") {
            for (uri, changes) in changes
                .as_object()
                .context("Invalid workspace edit changes")?
            {
                edits.push((uri.clone(), None, changes.clone()));
            }
        } else {
            bail!("No workspace text edits");
        }
        if edits.len() > 128 {
            bail!("Code action exceeds 128 edited buffers");
        }
        let mut count_edits = 0usize;
        let mut replacement_bytes = 0usize;
        let mut seen = HashSet::new();
        let mut staged = Vec::new();
        for (uri, version, edits) in edits {
            // URI decoding and lexical normalization only: do not perform filesystem I/O here.
            let path = url::Url::parse(&uri)?
                .to_file_path()
                .map_err(|_| anyhow::anyhow!("Unsupported document URI"))?;
            let (path, snapshot) = request.workspace.iter().find(|(p, _)| {
                lsp::file_uri(p).ok().as_deref() == Some(uri.as_str()) || **p == path
            }).context("Code action touches a file outside the synchronized open buffers; open it and retry")?;
            if !seen.insert(path.clone()) {
                bail!("Repeated workspace edit target is not supported");
            }
            let index = self
                .documents
                .iter()
                .position(|d| d.path.as_ref() == Some(path) && d.id == snapshot.id)
                .context("Workspace document was closed or replaced; request code actions again")?;
            let doc = &self.documents[index];
            if doc.revision != snapshot.revision || version.is_some_and(|v| v != snapshot.version) {
                bail!("Workspace document changed since this request; request code actions again");
            }
            let values = edits.as_array().context("Invalid workspace text edits")?;
            count_edits += values.len();
            if count_edits > 4096 {
                bail!("Code action exceeds 4096 text edits");
            }
            for value in values {
                if value.get("annotationId").is_some() {
                    bail!("Annotated workspace text edits are not supported yet");
                }
                if value.as_object().is_none_or(|object| {
                    object
                        .keys()
                        .any(|key| !matches!(key.as_str(), "range" | "newText"))
                }) {
                    bail!("Unsupported workspace text edit fields");
                }
                replacement_bytes += value["newText"]
                    .as_str()
                    .context("Invalid replacement text")?
                    .len();
                if replacement_bytes > 4 * 1024 * 1024 {
                    bail!("Code action exceeds 4 MiB replacement text");
                }
            }
            let changes = lsp::edits(doc, serde_json::from_value::<Vec<TextEdit>>(edits)?)?;
            staged.push((index, changes));
        }
        let count = staged.len();
        for (index, changes) in staged {
            self.documents[index].apply_changes(changes);
        }
        self.message =
            format!("Applied code action to {count} buffers; review and save (Undo is per file)");
        Ok(())
    }
}
