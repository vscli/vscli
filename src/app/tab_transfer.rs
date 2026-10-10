//! Original keyed cross-group moves stage membership, topology and source view.
use super::*;
use crate::{editor_groups::TransferDestination, editor_layout::Direction};
use anyhow::{Context, ensure};

impl App {
    pub(super) fn execute_tab_transfer_command(&mut self, command: &str) -> bool {
        let destination = match command {
            "workbench.action.moveEditorToPreviousGroup" => 0,
            "workbench.action.moveEditorToNextGroup" => 1,
            "workbench.action.moveEditorToFirstGroup" => 2,
            "workbench.action.moveEditorToLastGroup" => 3,
            _ => return false,
        };
        if self.group_fallback {
            self.message =
                "Group transfer unavailable in recovery overflow; buffers retained".into();
            return true;
        }
        let Some(source) = self.active_tab_membership() else {
            return true;
        };
        let result = (|| -> Result<Option<crate::editor_groups::Change>> {
            let groups = &self.editor_groups;
            let index = groups
                .groups()
                .iter()
                .position(|group| group.id() == source.group)
                .context("Transfer source group was closed")?;
            let target_index = match destination {
                0 => match index.checked_sub(1) {
                    Some(index) => Some(index),
                    None => return Ok(None),
                },
                1 => (index + 1 < groups.groups().len()).then_some(index + 1),
                2 => Some(0),
                _ => Some(groups.groups().len() - 1),
            };
            let target = match target_index {
                Some(index) => {
                    TransferDestination::Existing(groups.group_proof(groups.groups()[index].id())?)
                }
                None => TransferDestination::NewAfterSource,
            };
            let plan = groups.prepare_transfer(
                groups.proof(),
                groups.group_proof(source.group)?,
                source,
                target,
            )?;
            if !plan.change().changed {
                // Original endpoint wrappers still return focus to this group.
                if self.focus != Focus::Editor {
                    self.breadcrumbs_ui_command(command);
                    self.navigation_input_interaction();
                    self.session_interaction();
                    self.cancel_suggestions();
                    self.clear_signature();
                    self.cancel_symbols();
                    self.focus = Focus::Editor;
                    self.observe_navigation(navigation_history::Reason::Ordinary);
                    self.invalidate_pending_extension_commands();
                    self.invalidate_editor_presentation();
                }
                return Ok(None);
            }
            let direction = match self.settings.tab_transfer_direction()? {
                crate::settings::EditorSideBySideDirection::Right => Direction::Right,
                crate::settings::EditorSideBySideDirection::Down => Direction::Down,
            };
            let projected = plan.projected(groups)?;
            let expected: Vec<_> = projected.groups().iter().map(|group| group.id()).collect();
            let target = plan
                .change()
                .active
                .context("Transfer destination is missing")?;
            let mut layout = self.editor_layout.clone();
            layout.validate(
                &groups
                    .groups()
                    .iter()
                    .map(|group| group.id())
                    .collect::<Vec<_>>(),
            )?;
            // Keep source leaf alive until the new leaf is anchored to it.
            for created in &plan.change().created_groups {
                let split = layout.prepare_split(source.group, *created, direction)?;
                layout.commit(split)?;
            }
            for removed in &plan.change().removed_groups {
                let removal = layout.prepare_remove(*removed)?;
                layout.commit(removal)?;
            }
            layout.validate(&expected)?;
            let document = self
                .documents
                .iter()
                .position(|document| document.id == source.document)
                .context("Transfer document is not retained")?;
            ensure!(self.active == document, "Transfer document is not active");
            // Exclusive view lease prevents any change between source validation
            // and publication. Groups can still reject its stale plan before
            // either Layout or this Document projection is published.
            let view = self.documents[document]
                .prepare_view_transfer(source.group.value(), target.group.value())?;
            let change = self.editor_groups.commit_transfer(plan)?;
            self.editor_layout = layout;
            view.publish();
            Ok(Some(change))
        })();
        match result {
            Ok(Some(change)) => {
                let target = change.active.unwrap();
                self.breadcrumbs_ui_command(command);
                self.navigation_input_interaction();
                self.session_interaction();
                self.cancel_suggestions();
                self.clear_signature();
                self.cancel_symbols();
                // Retire remaining batch only; accepted Save/SaveAs receipts and
                // exact original close continuations retain their own identity.
                let retired = self.closing_group.take().is_some();
                if self
                    .close_membership
                    .is_some_and(|member| change.removed.contains(&member))
                {
                    if matches!(self.modal, Some(Modal::Confirm(AfterSave::Close))) {
                        self.modal = None;
                    }
                    self.close_membership = None;
                }
                self.apply_transferred_group_change(change, target);
                self.focus = Focus::Editor;
                self.observe_navigation(navigation_history::Reason::Ordinary);
                self.invalidate_pending_extension_commands();
                self.message = if retired {
                    "Editor transferred; remaining close batch retired, approved original save retained".into()
                } else {
                    "Editor transferred to group".into()
                };
            }
            Ok(None) => {}
            Err(error) => {
                self.message = format!("Group transfer rejected; buffers retained: {error:#}")
            }
        }
        true
    }
}
