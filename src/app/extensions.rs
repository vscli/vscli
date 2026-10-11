use super::*;

impl App {
    pub(super) fn refresh_extension_editor_inventory(&mut self) {
        let previous = self.extension_editor_inventory.identities();
        let views = self
            .panes
            .iter()
            .map(|pane| {
                let column = if self.group_fallback {
                    self.panes
                        .iter()
                        .position(|candidate| candidate.id == pane.id)
                        .unwrap_or(0)
                        + 1
                } else {
                    self.editor_groups
                        .groups()
                        .iter()
                        .position(|group| group.id().value() == pane.id)
                        .unwrap_or(0)
                        + 1
                };
                (pane.document, pane.id, column)
            })
            .collect::<Vec<_>>();
        let refs = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .collect::<Vec<_>>();
        let active = self
            .panes
            .get(self.active_pane)
            .map(|pane| (pane.document, pane.id));
        if let Err(error) = self
            .extension_editor_inventory
            .observe(&refs, &views, active)
        {
            self.extension_editor_inventory.invalidate();
            if self.extension_host.is_some() {
                self.message = format!("Extension editor inventory paused: {error:#}");
            }
        }
        // Retired public editors cannot revive styling when the same native
        // group later selects their retained model, including unpolled ABA.
        for (document, view, editor) in previous {
            if self
                .extension_editor_inventory
                .id_for_view(document, view)
                .as_deref()
                != Some(editor.as_str())
                && let Some(doc) = self
                    .documents
                    .iter_mut()
                    .chain(&mut self.hidden_documents)
                    .find(|doc| doc.id == document)
            {
                doc.clear_editor_decorations_for(view);
            }
        }
        self.refresh_extension_visible_ranges();
    }
    pub(super) fn poll_extensions(&mut self) -> bool {
        // Publish readiness only after the accepted-start worker releases its
        // operation slot. Otherwise a ready UI can reject the very next action.
        if self.extension_job.as_ref().is_some_and(|job| job.startup()) {
            return false;
        }
        let Some(mut host) = self.extension_host.take() else {
            return self.poll_extension_services();
        };
        self.refresh_extension_editor_inventory();
        host.set_editor_inventory(self.extension_editor_inventory.clone());
        if self.extension_packages.is_empty() {
            self.extension_packages = host.packages.clone();
        }
        let before: Vec<_> = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .map(|d| d.revision)
            .collect();
        let commands = host.commands.clone();
        let busy = host.busy();
        let diagnostics_epoch = host.diagnostics_epoch();
        let decorations_epoch = host.decorations_generation();
        host.set_workspace_edit_context(if self.settings_error.is_none() {
            workspace_transactions::CaptureContext::new(
                self.settings.extension_layers().clone(),
                self.settings_profile_generation(),
                self.workspace.root.clone(),
            )
            .ok()
        } else {
            None
        });
        host.set_file_search_context(if self.settings_error.is_none() {
            crate::workspace_file_search::Context::new(
                self.workspace.root.clone(),
                self.settings.extension_layers().clone(),
                self.settings_profile_generation(),
            )
            .ok()
        } else {
            None
        });
        host.set_untitled_context(if self.settings_error.is_none() {
            crate::extensions::untitled::CreationContext::new(
                self.workspace.root.clone(),
                self.settings.extension_layers().clone(),
                self.settings_profile_generation(),
            )
            .ok()
        } else {
            None
        });
        host.set_editor_state_context(if self.settings_error.is_none() {
            crate::extensions::editor_state::Context::new(
                self.workspace.root.clone(),
                self.settings.extension_layers().clone(),
                self.settings_profile_generation(),
            )
            .ok()
        } else {
            None
        });
        let result = host.poll_with_hidden(
            &mut self.documents,
            &mut self.hidden_documents,
            self.active,
            &self.settings,
        );
        // Promote successful native edits even if a later message failed, and
        // before a delegated Undo can restore the saved revision in this poll.
        let promoted = self.preview_edit_barrier();
        match result {
            Ok(messages) => {
                let changed = promoted
                    || !messages.is_empty()
                    || diagnostics_epoch != host.diagnostics_epoch()
                    || decorations_epoch != host.decorations_generation()
                    || busy != host.busy()
                    || commands != host.commands
                    || before
                        != self
                            .documents
                            .iter()
                            .chain(&self.hidden_documents)
                            .map(|d| d.revision)
                            .collect::<Vec<_>>();
                if messages.is_empty()
                    && busy
                    && !host.busy()
                    && self.message.starts_with("Running extension command:")
                {
                    self.message = "Extension command completed".into();
                }
                for message in messages {
                    self.message = message;
                }
                // Package validation is independent; one malformed contribution
                // cannot discard another selected package's defaults.
                if let Some(sets) = host.binding_sets.take() {
                    host.keybindings = None;
                    if self.retain_extension_bindings(sets.clone()) {
                        self.extension_host = Some(host);
                        return self.poll_extension_services()
                            | self.poll_extension_prompt()
                            | changed;
                    }
                    match self.keymap.set_extension_binding_sets(sets) {
                        Ok(errors) if !errors.is_empty() => {
                            self.message = format!(
                                "Extension commands ready; keybindings rejected: {}",
                                errors.join("; ")
                            )
                        }
                        Err(error) => {
                            self.message =
                                format!("Extension commands ready; keybindings rejected: {error:#}")
                        }
                        _ => {}
                    }
                }
                self.extension_host = Some(host);
                self.poll_extension_services() | self.poll_extension_prompt() | changed
            }
            Err(error) => {
                self.keymap.clear_extension_bindings();
                self.retire_extension_host(host);
                self.message = format!("Extension host stopped: {error:#}");
                true
            }
        }
    }
    pub(super) fn execute_extension(&mut self, command: &str, args: Option<Value>) {
        if !self.queue_extension_command(command, &args) {
            self.execute_extension_now(command, args);
        }
    }
    pub(super) fn execute_extension_now(&mut self, command: &str, args: Option<Value>) {
        self.refresh_extension_editor_inventory();
        if let Some(host) = &mut self.extension_host {
            host.set_editor_inventory(self.extension_editor_inventory.clone());
            if self
                .activation
                .owners
                .get(command)
                .is_some_and(|owner| host.command_owner(command) != Some(owner.as_str()))
            {
                self.message = format!(
                    "Extension command owner conflicts with the selected contribution: {command}"
                );
                return;
            }
            match host.execute_with_hidden(
                command,
                args,
                &self.documents,
                &self.hidden_documents,
                self.active,
                &self.settings,
            ) {
                Ok(()) => self.message = format!("Running extension command: {command}"),
                Err(error) => self.message = format!("Extension command failed: {error:#}"),
            }
        } else {
            self.message = format!("Command not implemented in this alpha: {command}");
        }
    }
}

#[cfg(test)]
mod decoration_retirement_tests {
    use super::*;
    use crate::decorations::{DecorationInput, DecorationType};
    use ratatui::{Terminal, backend::TestBackend, style::Color};
    fn colored(app: &mut App) -> bool {
        let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
        terminal.draw(|frame| crate::ui::draw(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .any(|cell| cell.fg == Color::Rgb(3, 77, 129))
    }
    #[test]
    fn unpolled_hide_show_retires_styling_without_replacing_shared_source_or_history() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.install_open_document(Document::from_text("猫🙂 alpha\r\n"))
            .unwrap();
        app.doc_mut().insert("dirty ", false);
        let id = app.doc().id;
        let member = app.editor_groups.active_membership().unwrap();
        let view = app.panes[0].id;
        let old = app
            .extension_editor_inventory
            .id_for_view(id, view)
            .unwrap();
        let kind = DecorationType::new(
            "decoration-1".into(),
            "test.owner".into(),
            json!({"color":"#034d81"}),
        )
        .unwrap();
        let proof = app.doc().editor_view_identity();
        let entries: Vec<DecorationInput> = serde_json::from_value(
            json!([{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":5}}}]),
        )
        .unwrap();
        app.doc_mut()
            .apply_editor_decorations(&proof, kind, entries)
            .unwrap();
        assert!(colored(&mut app));
        app.install_open_document(Document::from_text("other\r\n"))
            .unwrap();
        app.focus_tab(member).unwrap();
        assert_eq!(app.doc().id, id);
        assert_ne!(
            app.extension_editor_inventory
                .id_for_view(id, view)
                .as_deref(),
            Some(old.as_str())
        );
        assert!(
            !colored(&mut app),
            "old styling cannot revive before any host poll"
        );
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "猫🙂 alpha\r\n");
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), "dirty 猫🙂 alpha\r\n");
        assert!(
            !colored(&mut app),
            "Undo/Redo cannot revive an editor's retired styling"
        );
    }
}
