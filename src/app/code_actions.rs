//! User-invoked LSP actions. Validate every target before changing any shared buffer.
use super::workspace_edits::{
    WorkspaceEditPolicy, WorkspaceEditTarget, WorkspaceVersion, apply_workspace_edit,
};
use super::*;
use crate::lsp::{self, Request};
use anyhow::{Context, bail};

impl App {
    pub(super) fn request_code_actions(&mut self, kind: Option<&str>) {
        self.request_all_code_actions(kind);
    }

    pub(super) fn code_action_current(&self, request: &Request) -> Result<()> {
        self.request_current(request)?;
        self.native_action_context_current(request)?;
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
            self.lsp
                .as_ref()
                .context("Language server disconnected")?
                .ensure_command_available()?;
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
            let token = self
                .lsp
                .as_mut()
                .context("Language server disconnected")?
                .resolve_code_action(request, item.clone())?;
            self.track_native_action(request, token);
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
            let token = client.execute_action_command(request, json!({
                "command":command["command"], "arguments":command.get("arguments").cloned().unwrap_or_else(|| json!([]))
            }))?;
            self.track_native_action(request, token);
            self.message = "Code action command running…".into();
        }
        Ok(())
    }

    pub(super) fn apply_action_edit(&mut self, request: &Request, edit: Value) -> Result<()> {
        self.code_action_current(request)?;
        if self.prompt.is_some() || self.modal.is_some() {
            bail!("Input context changed; workspace edit was rejected");
        }
        let targets: Vec<_> = request
            .workspace
            .iter()
            .map(|(path, snapshot)| {
                Ok(WorkspaceEditTarget {
                    uri: lsp::file_uri(path)?,
                    path: Some(path.clone()),
                    document: snapshot.id,
                    revision: snapshot.revision,
                    text_epoch: snapshot.text_epoch,
                    version: WorkspaceVersion::Native(snapshot.version),
                })
            })
            .collect::<Result<_>>()?;
        let client = self.lsp.as_ref().context("Language server disconnected")?;
        let outcome = apply_workspace_edit(
            &mut self.documents,
            &mut self.hidden_documents,
            &edit,
            &targets,
            WorkspaceEditPolicy {
                require_versions: request.method == "workspace/executeCommand",
                max_total_document_bytes: None,
            },
            |target, _document| {
                let Some(path) = &target.path else {
                    return false;
                };
                let WorkspaceVersion::Native(version) = target.version else {
                    return false;
                };
                client.workspace_snapshot_current(
                    path,
                    &lsp::Snapshot {
                        id: target.document,
                        revision: target.revision,
                        text_epoch: target.text_epoch,
                        version,
                    },
                )
            },
        )?;
        self.preview_edit_barrier();
        self.finish_code_actions();
        let count = outcome.buffers;
        self.message =
            format!("Applied code action to {count} buffers; review and save (Undo is per file)");
        Ok(())
    }
}
