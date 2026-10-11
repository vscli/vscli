//! Public tab close requests retain original native membership through dialogs,
//! save workers and mirror/transport backpressure. No close effect is replayed.
use super::*;
use crate::{
    editor_groups::{GroupId, Membership},
    extensions::{
        editor_tabs::Capture,
        services::{NativeService, Operation},
    },
};
use anyhow::{Context as _, ensure};
use std::{collections::VecDeque, sync::Arc};

pub(super) struct Completed {
    service: NativeService,
    capture: Arc<Capture>,
    remaining: VecDeque<Membership>,
    remaining_groups: VecDeque<GroupId>,
    waiting: Option<(Membership, u64)>,
    saved_close: Option<(Membership, u64)>,
    original_member: Option<Membership>,
    original_group: Option<GroupId>,
    original_focus: Focus,
    preserve_focus: bool,
    admitted: bool,
    result: Option<Result<Value, String>>,
}

impl App {
    pub(super) fn defer_api_tab_close_after_save(
        &mut self,
        member: Membership,
        generation: u64,
    ) -> bool {
        if let Some(done) = &mut self.extension_services.tabs_completed
            && done.waiting == Some((member, generation))
            && done.capture.engine_current(&self.editor_groups)
        {
            done.saved_close = Some((member, generation));
            return true;
        }
        false
    }
    pub(super) fn retire_extension_tab_close(&mut self) {
        if let Some(done) = self.extension_services.tabs_completed.take()
            && done.capture.engine_current(&self.editor_groups)
            && let Some((member, generation)) = done.waiting
        {
            let restore = self.active_tab_membership() == Some(member);
            self.retire_api_tab_close(member, generation);
            if restore {
                self.restore_extension_tab_focus(&done);
            }
        }
    }

    fn restore_extension_tab_focus(&mut self, done: &Completed) {
        if !done.preserve_focus || !done.capture.engine_current(&self.editor_groups) {
            return;
        }
        if let Some(member) = done
            .original_member
            .filter(|member| self.editor_groups.membership_current(*member))
        {
            let _ = self.focus_tab(member);
            self.focus = done.original_focus.clone();
        } else if let Some(group) = done
            .original_group
            .filter(|group| self.editor_groups.group(*group).is_some())
            && let Ok(change) = self.editor_groups.focus_group(group)
        {
            self.apply_group_change(change);
            self.focus = done.original_focus.clone();
        }
    }

    pub(super) fn start_extension_tab_close(
        &mut self,
        host: &mut crate::extensions::Client,
        service: NativeService,
    ) {
        let Operation::TabsClose {
            capture,
            preserve_focus,
            ..
        } = &service.operation
        else {
            return;
        };
        let mut members = VecDeque::new();
        for member in capture.memberships() {
            if !members.contains(member) {
                members.push_back(*member);
            }
        }
        self.extension_services.tabs_completed = Some(Completed {
            capture: capture.clone(),
            remaining: members,
            remaining_groups: capture.group_ids().iter().copied().collect(),
            waiting: None,
            saved_close: None,
            original_member: self.active_tab_membership(),
            original_group: self.editor_groups.active_group(),
            original_focus: self.focus.clone(),
            // The pinned VS Code main-thread group-close implementation ignores
            // preserveFocus. Tab closes retain the original surviving focus.
            preserve_focus: *preserve_focus && capture.kind() == "tabs",
            admitted: false,
            result: None,
            service,
        });
        self.poll_extension_tab_close(host);
    }

    fn advance_extension_tab_close(
        &mut self,
        host: &mut crate::extensions::Client,
        done: &mut Completed,
    ) -> Result<Option<bool>> {
        host.service_valid(&done.service)?;
        let Operation::TabsClose { context, .. } = &done.service.operation else {
            anyhow::bail!("Invalid native tab close operation");
        };
        ensure!(
            self.settings_error.is_none()
                && context.current(
                    &self.workspace.root,
                    self.settings.extension_layers(),
                    self.settings_profile_generation()
                ),
            "Original tab close workspace or settings retired"
        );
        ensure!(
            !self.group_fallback && done.capture.engine_current(&self.editor_groups),
            "Original native tab engine retired"
        );
        if !done.admitted {
            let refs = self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .collect::<Vec<_>>();
            self.extension_editor_tabs
                .validate(&self.editor_groups, &refs)?;
            ensure!(
                done.capture
                    .current(&self.extension_editor_tabs, &self.editor_groups),
                "Original native tab or group membership retired"
            );
            ensure!(
                self.prompt.is_none()
                    && self.modal.is_none()
                    && self.pending.is_none()
                    && self.close_membership.is_none()
                    && self.closing_group.is_none(),
                "Native tab close cannot replace another dialog or close request"
            );
            done.admitted = true;
        }
        // A group close only owns its original cohort. A newly opened tab is
        // never swept up by the continuation after a human Save decision.
        if done.capture.kind() == "groups" {
            for id in &done.remaining_groups {
                if let Some(group) = self.editor_groups.group(*id) {
                    ensure!(
                        group.tabs().iter().all(|tab| {
                            done.capture
                                .groups()
                                .iter()
                                .find(|proof| proof.group() == *id)
                                .is_some_and(|proof| {
                                    proof.members().contains(&Membership {
                                        group: *id,
                                        tab: tab.id(),
                                        document: tab.document(),
                                    })
                                })
                        }),
                        "Native tab group acquired an unselected membership"
                    );
                }
            }
        }
        if let Some((member, generation)) = done.waiting {
            if self.tab_close_generation() != generation {
                done.waiting = None;
                return Ok(Some(false));
            }
            if done.saved_close == Some((member, generation)) {
                let doc = self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .find(|doc| doc.id == member.document)
                    .context("Saved close model retired")?;
                if doc.dirty() {
                    self.retire_api_tab_close(member, generation);
                    done.waiting = None;
                    return Ok(Some(false));
                }
                // Publish the real save/resource transition while the original
                // model remains retained. FIFO sends guarantee didSave precedes
                // the later closed snapshot and terminal close receipt.
                host.set_editor_inventory(self.extension_editor_inventory.clone());
                host.set_editor_tabs(self.extension_editor_tabs.clone());
                if let Err(error) =
                    host.sync_with_hidden(&self.documents, &self.hidden_documents, self.active)
                {
                    self.message = format!("Saved tab close mirror retained: {error:#}");
                    return Ok(None);
                }
                self.close_tab_membership(member)?;
                self.close_membership = None;
                done.saved_close = None;
            }
            if self.editor_groups.membership_current(member) {
                if !self.tab_close_pending(member, generation) {
                    done.waiting = None;
                    return Ok(Some(false));
                }
                let human = matches!(self.modal, Some(Modal::Confirm(AfterSave::Close)))
                    || self
                        .prompt
                        .as_ref()
                        .is_some_and(|prompt| matches!(prompt.kind, PromptKind::SaveAs));
                host.set_document_save_path_wait(&done.service, human)?;
                return Ok(None);
            }
            done.waiting = None;
            done.remaining.pop_front();
            host.set_document_save_path_wait(&done.service, false)?;
        }
        // A bounded step per frame keeps the terminal responsive for large
        // extension batches and avoids running persistence inside rendering.
        if let Some(member) = done.remaining.front().copied() {
            ensure!(
                self.editor_groups.membership_current(member),
                "Original tab membership retired during close"
            );
            let doc = self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .find(|doc| doc.id == member.document)
                .context("Original tab model retired")?;
            if !doc.dirty() || self.editor_groups.memberships(member.document).count() > 1 {
                self.close_tab_membership(member)?;
                done.remaining.pop_front();
            } else {
                ensure!(
                    self.prompt.is_none() && self.modal.is_none() && self.pending.is_none(),
                    "Native tab close cannot replace another prompt"
                );
                self.focus_tab(member)?;
                self.close_membership = Some(member);
                self.close_eligibility = sticky_tabs::CloseEligibility::AnyMode;
                let generation = self.tab_close_generation();
                self.request_close(AfterSave::Close);
                done.waiting = Some((member, generation));
                let human = matches!(self.modal, Some(Modal::Confirm(AfterSave::Close)))
                    || self
                        .prompt
                        .as_ref()
                        .is_some_and(|prompt| matches!(prompt.kind, PromptKind::SaveAs));
                host.set_document_save_path_wait(&done.service, human)?;
            }
            return Ok(None);
        }
        if let Some(group) = done.remaining_groups.pop_front() {
            if self.editor_groups.group(group).is_some() {
                let proof = self.editor_groups.group_proof(group)?;
                ensure!(
                    proof.members().is_empty(),
                    "Unselected group membership retained"
                );
                let mut groups = self.editor_groups.clone();
                let change = groups.close_group(&proof)?;
                let layout = self.prepare_group_layout(&groups, &change, None)?;
                self.publish_group_layout(groups, layout);
                self.apply_group_change(change);
            }
            return Ok(None);
        }
        Ok(Some(true))
    }

    pub(super) fn poll_extension_tab_close(
        &mut self,
        host: &mut crate::extensions::Client,
    ) -> bool {
        let Some(mut done) = self.extension_services.tabs_completed.take() else {
            return false;
        };
        if done.service.request.session != host.session
            || !host.service().is_some_and(|service| {
                service.id == done.service.id && service.request.owner == done.service.request.owner
            })
        {
            if done.capture.engine_current(&self.editor_groups)
                && let Some((member, generation)) = done.waiting
            {
                let restore = self.active_tab_membership() == Some(member);
                self.retire_api_tab_close(member, generation);
                if restore {
                    self.restore_extension_tab_focus(&done);
                }
            }
            return true;
        }
        self.refresh_extension_editor_inventory();
        if done.result.is_none() {
            let before = self.editor_groups.proof();
            let had_modal = self.modal.is_some();
            let had_prompt = self.prompt.is_some();
            match self.advance_extension_tab_close(host, &mut done) {
                Ok(None) => {
                    let changed = !self.editor_groups.proof_current(&before)
                        || had_modal != self.modal.is_some()
                        || had_prompt != self.prompt.is_some();
                    self.extension_services.tabs_completed = Some(done);
                    return changed;
                }
                Ok(Some(closed)) => {
                    done.result = Some(Ok(json!({
                        "closed":closed,"kind":done.capture.kind(),"ids":done.capture.ids()
                    })))
                }
                Err(error) => {
                    done.result = Some(Err(crate::extensions::services::bounded_error(&error)))
                }
            }
            if done.capture.engine_current(&self.editor_groups)
                && let Some((member, generation)) = done.waiting.take()
            {
                self.retire_api_tab_close(member, generation);
            }
            self.restore_extension_tab_focus(&done);
            self.refresh_extension_editor_inventory();
        }
        host.set_editor_inventory(self.extension_editor_inventory.clone());
        host.set_editor_tabs(self.extension_editor_tabs.clone());
        if let Err(error) =
            host.sync_with_hidden(&self.documents, &self.hidden_documents, self.active)
        {
            self.message = format!("Tab close mirror receipt retained: {error:#}");
            self.extension_services.tabs_completed = Some(done);
            return false;
        }
        let result = host.service_valid(&done.service).and_then(|()| {
            done.result
                .clone()
                .expect("terminal close receipt")
                .map_err(anyhow::Error::msg)
        });
        if let Err(error) = host.answer_service(&done.service, result) {
            self.message = format!("Tab close acknowledgement retained: {error:#}");
            self.extension_services.tabs_completed = Some(done);
            return false;
        }
        true
    }
}
