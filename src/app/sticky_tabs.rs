//! Native sticky metadata and bounded, captured close subsets.
use super::*;
use crate::editor_groups::{Change, GroupProof, Membership};
use anyhow::ensure;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum CloseEligibility {
    #[default]
    AnyMode,
    NonSticky,
}

impl App {
    pub fn active_editor_is_sticky(&self) -> bool {
        self.active_tab_membership()
            .is_some_and(|member| self.editor_membership_is_sticky(member))
    }

    pub(super) fn editor_membership_is_sticky(&self, member: Membership) -> bool {
        self.editor_groups.group(member.group).is_some_and(|group| {
            group.tabs().iter().any(|tab| {
                tab.id() == member.tab && tab.document() == member.document && tab.is_sticky()
            })
        })
    }

    fn keyboard_protects_sticky(&self) -> bool {
        matches!(
            self.settings.prevent_pinned_editor_close(),
            crate::settings::PreventPinnedEditorClose::Keyboard
                | crate::settings::PreventPinnedEditorClose::KeyboardAndMouse
        )
    }

    pub(super) fn set_active_editor_sticky(&mut self, sticky: bool) {
        let Some(member) = self.active_tab_membership() else {
            return;
        };
        self.sync_pane();
        match self.editor_groups.set_sticky(member, sticky) {
            Ok(change) => {
                let changed = change.changed;
                self.apply_group_change(change);
                if changed && self.closing_group.take().is_some() {
                    // A metadata change retires only the remaining subset. An
                    // accepted save receipt still owns its exact target.
                    self.message =
                        "Close batch retired: tab modes changed; buffers retained".into();
                }
            }
            Err(error) => {
                self.message = format!("Pin editor rejected; buffers retained: {error:#}")
            }
        }
    }

    pub(super) fn close_active_editor(&mut self, forced: bool) {
        let Some(member) = self.active_tab_membership() else {
            if self.group_fallback {
                self.request_close(AfterSave::Close);
            }
            return;
        };
        let protected = !forced && self.keyboard_protects_sticky();
        if protected && self.editor_membership_is_sticky(member) {
            let fallback = self
                .editor_groups
                .next_nonsticky_recent(member.group)
                .or_else(|| self.editor_groups.next_nonsticky_recent_any_group());
            if let Some(target) = fallback {
                if let Err(error) = self.focus_tab(target) {
                    self.message = format!("Protected editor focus rejected: {error:#}");
                }
            } else {
                self.message = "Pinned editor retained; no unpinned editor to focus".into();
            }
            return;
        }
        self.close_membership = Some(member);
        self.close_eligibility = if protected {
            CloseEligibility::NonSticky
        } else {
            CloseEligibility::AnyMode
        };
        self.request_close(AfterSave::Close);
    }

    pub(super) fn captured_close_eligibility(
        &self,
        member: Option<Membership>,
    ) -> CloseEligibility {
        if member.is_some() && member == self.close_membership {
            self.close_eligibility
        } else {
            CloseEligibility::AnyMode
        }
    }

    pub(super) fn close_target_eligible(
        &self,
        member: Membership,
        eligibility: CloseEligibility,
    ) -> bool {
        self.editor_groups.membership_current(member)
            && (eligibility == CloseEligibility::AnyMode
                || !self.editor_membership_is_sticky(member))
    }

    pub(super) fn reject_protected_close(&mut self, member: Membership, saved: bool) {
        if self.close_membership == Some(member) {
            self.close_membership = None;
        }
        self.closing_group = None;
        self.message = if saved {
            "Saved; close retired because the original editor is now pinned".into()
        } else {
            "Close retired because the original editor is now pinned; buffer retained".into()
        };
    }

    fn close_batch_current(&self, closing: &ClosingGroup) -> bool {
        self.workspace.root == closing.workspace
            && self.settings_profile_generation() == closing.profile
            && std::sync::Arc::ptr_eq(self.settings.extension_layers(), &closing.settings)
            && std::iter::once(&closing.proof)
                .chain(&closing.others)
                .all(|proof| self.editor_groups.group_proof_current(proof))
            && closing
                .remaining
                .iter()
                .all(|member| self.close_target_eligible(*member, CloseEligibility::NonSticky))
    }

    pub(super) fn close_batch_owns(&self, member: Membership) -> bool {
        self.closing_group.as_ref().is_some_and(|closing| {
            closing.remaining.contains(&member) && self.close_batch_current(closing)
        })
    }

    pub(super) fn begin_close_editor_batch(&mut self, all: bool) {
        if self.group_fallback {
            if all {
                self.request_close(AfterSave::CloseAll);
            } else {
                self.message =
                    "Close group unavailable in recovery overflow; buffers retained".into();
            }
            return;
        }
        if self.workspace.root.as_os_str().len() > 4096 {
            self.message =
                "Close batch rejected: workspace path exceeds 4 KiB; buffers retained".into();
            return;
        }
        self.sync_pane();
        let proofs: Result<Vec<GroupProof>> = self
            .editor_groups
            .groups()
            .iter()
            .filter(|group| all || Some(group.id()) == self.editor_groups.active_group())
            .map(|group| self.editor_groups.group_proof(group.id()))
            .collect();
        let result = proofs.and_then(|mut proofs| {
            ensure!(proofs.len() <= 4, "Close batch exceeds four groups");
            if proofs.is_empty() {
                return Ok(None);
            }
            let remaining: Vec<_> = proofs
                .iter()
                .flat_map(|proof| proof.members().iter().copied())
                .filter(|member| !self.editor_membership_is_sticky(*member))
                .collect();
            ensure!(
                remaining.len() <= 512,
                "Close batch exceeds 512 memberships"
            );
            if remaining.is_empty() {
                return Ok(None);
            }
            let proof = proofs.remove(0);
            Ok(Some(ClosingGroup {
                proof,
                others: proofs,
                remaining,
                settings: self.settings.extension_layers().clone(),
                workspace: self.workspace.root.clone(),
                profile: self.settings_profile_generation(),
            }))
        });
        match result {
            Ok(closing) => {
                self.closing_group = closing;
                self.advance_close_editor_group();
            }
            Err(error) => {
                self.message = format!("Close batch rejected; buffers retained: {error:#}")
            }
        }
    }

    pub(super) fn advance_close_editor_group(&mut self) {
        let Some(closing) = self.closing_group.take() else {
            return;
        };
        if !self.close_batch_current(&closing) {
            if matches!(self.modal, Some(Modal::Confirm(AfterSave::Close)))
                && self
                    .close_membership
                    .is_some_and(|member| closing.remaining.contains(&member))
            {
                self.modal = None;
                self.close_membership = None;
            }
            self.message = "Close batch retired: its tabs changed; buffers retained".into();
            return;
        }
        // A dirty model with a membership outside this captured subset stays
        // authoritative and requires no Save/Discard prompt for removed views.
        let dirty = closing.remaining.iter().copied().find(|member| {
            let targeted = closing
                .remaining
                .iter()
                .filter(|target| target.document == member.document)
                .count();
            let all = self.editor_groups.memberships(member.document).count();
            targeted == all
                && (self.document_save_pending(member.document)
                    || self.save_as_document_pending(member.document)
                    || self
                        .documents
                        .iter()
                        .chain(&self.hidden_documents)
                        .any(|doc| doc.id == member.document && doc.dirty()))
        });
        if let Some(member) = dirty {
            if self.save_as_document_pending(member.document) {
                self.message =
                    "Close batch retired: finish the original Save As first; buffers retained"
                        .into();
                return;
            }
            match self.focus_tab(member) {
                Ok(()) => {
                    self.closing_group = Some(closing);
                    self.close_membership = Some(member);
                    self.close_eligibility = CloseEligibility::NonSticky;
                    self.request_close(AfterSave::Close);
                }
                Err(error) => {
                    self.message = format!("Close batch retired; buffers retained: {error:#}")
                }
            }
            return;
        }
        // Stage every group's subset and counter checks on one bounded engine
        // clone. No model/view/closed-editor side effect occurs before success.
        let staged = (|| -> Result<(crate::editor_groups::Groups, crate::editor_layout::Layout, Change)> {
            let mut groups = self.editor_groups.clone();
            let mut aggregate = Change {
                previous: groups.active_membership(),
                ..Change::default()
            };
            for proof in std::iter::once(&closing.proof).chain(&closing.others) {
                let targets: Vec<_> = closing
                    .remaining
                    .iter()
                    .copied()
                    .filter(|member| member.group == proof.group())
                    .collect();
                let change = groups.close_memberships(proof, &targets)?;
                aggregate.changed |= change.changed;
                aggregate.removed.extend(change.removed);
                aggregate.removed_groups.extend(change.removed_groups);
            }
            aggregate.active = groups.active_membership();
            let layout = self.prepare_group_layout(&groups, &aggregate, None)?;
            Ok((groups, layout, aggregate))
        })();
        match staged {
            Ok((groups, layout, change)) => {
                for member in &change.removed {
                    self.record_closed_tab(*member);
                }
                let removed: Vec<_> = change
                    .removed
                    .iter()
                    .map(|member| member.document)
                    .collect();
                self.publish_group_layout(groups, layout);
                self.close_membership = None;
                self.apply_group_change(change);
                self.documents.retain(|doc| {
                    !removed.contains(&doc.id)
                        || self.editor_groups.memberships(doc.id).next().is_some()
                });
                self.project_editor_groups();
                if self.documents.is_empty() {
                    self.session_closed_all();
                }
            }
            Err(error) => {
                self.message = format!("Close batch retired; buffers retained: {error:#}")
            }
        }
    }

    pub(super) fn refresh_closing_group(&mut self) {
        let Some(mut closing) = self.closing_group.take() else {
            return;
        };
        // Called only after an owned removal whose complete pre-removal proofs
        // were current. Never recapture new targets from a changed group.
        closing
            .remaining
            .retain(|member| self.editor_groups.membership_current(*member));
        if closing.remaining.is_empty() {
            return;
        }
        let mut proofs = Vec::new();
        for old in std::iter::once(&closing.proof).chain(&closing.others) {
            if self.editor_groups.group(old.group()).is_some() {
                match self.editor_groups.group_proof(old.group()) {
                    Ok(proof) => proofs.push(proof),
                    Err(error) => {
                        self.message = format!("Close batch retired: {error:#}");
                        return;
                    }
                }
            }
        }
        if proofs.is_empty() {
            return;
        }
        closing.proof = proofs.remove(0);
        closing.others = proofs;
        self.closing_group = Some(closing);
        self.advance_close_editor_group();
    }
}
