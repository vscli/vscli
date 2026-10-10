//! Joint Close Group membership, topology and historical-view publication.
use super::*;
use crate::{document::ViewMergeProjection, editor_groups::Change};
use anyhow::{Context, ensure};
use std::mem::size_of;

const PREPARED_VIEW_BYTES: usize = 8 * 1024 * 1024;

impl App {
    pub(super) fn execute_group_merge_command(
        &mut self,
        command: &str,
        args: Option<&Value>,
    ) -> bool {
        if command != "workbench.action.closeGroup" {
            return false;
        }
        if args.is_some() {
            self.message = "Close Group accepts only the current group; buffers retained".into();
            return true;
        }
        if self.group_fallback {
            self.message = "Close Group unavailable in recovery overflow; buffers retained".into();
            return true;
        }
        let Some(source) = self.editor_groups.active_group() else {
            return true;
        };
        let result = (|| -> Result<Option<Change>> {
            let groups = &self.editor_groups;
            let plan =
                groups.prepare_close_group_merge(groups.proof(), groups.group_proof(source)?)?;
            if !plan.change().changed {
                return Ok(None);
            }
            self.settings.check_group_merge_policy()?;
            ensure!(
                plan.moves().len() <= crate::editor_groups::MAX_TABS_PER_GROUP,
                "Close Group exceeds 128 view mappings"
            );
            for (index, row) in plan.moves().iter().enumerate() {
                ensure!(
                    row.source.group == source
                        && row.source.document == row.target.document
                        && groups.membership_current(row.source)
                        && plan.moves()[..index]
                            .iter()
                            .all(|other| other.source.document != row.source.document),
                    "Close Group has an invalid or duplicate document mapping"
                );
            }
            let projected = plan.projected(groups)?;
            let mut expected = Vec::new();
            expected.try_reserve_exact(projected.groups().len())?;
            expected.extend(projected.groups().iter().map(|group| group.id()));
            let mut current = Vec::new();
            current.try_reserve_exact(groups.groups().len())?;
            current.extend(groups.groups().iter().map(|group| group.id()));
            let mut layout = self.editor_layout.clone();
            layout.validate(&current)?;
            let removal = layout.prepare_remove(source)?;
            layout.commit(removal)?;
            layout.validate(&expected)?;

            // All leases borrow distinct retained models. No historical view is
            // activated, model removed or live group/layout changed in preflight.
            let mut views = Vec::new();
            views.try_reserve_exact(plan.moves().len())?;
            let mut payload = views
                .capacity()
                .checked_mul(size_of::<crate::document::ViewMerge<'_>>())
                .context("Close Group lease payload overflow")?;
            ensure!(
                payload <= PREPARED_VIEW_BYTES,
                "Close Group view payload exceeds 8 MiB"
            );
            // Reject duplicate retained IDs before creating leases, even when a
            // malformed recovery fixture supplies the same source model twice.
            for row in plan.moves() {
                ensure!(
                    self.documents
                        .iter()
                        .chain(&self.hidden_documents)
                        .filter(|document| document.id == row.source.document)
                        .count()
                        == 1,
                    "Close Group source document is not uniquely retained"
                );
            }
            for document in self.documents.iter_mut().chain(&mut self.hidden_documents) {
                let Some(row) = plan
                    .moves()
                    .iter()
                    .find(|row| row.source.document == document.id)
                else {
                    continue;
                };
                // Existing inactive targets retain every private session. New
                // inactive targets start at native origin (memento-cache gap).
                let projection = if row.active {
                    ViewMergeProjection::CopySource
                } else if document
                    .retained_view_state(row.target.group.value())
                    .is_some()
                {
                    ViewMergeProjection::KeepTarget
                } else {
                    ViewMergeProjection::Origin
                };
                let view = document.prepare_historical_view_merge(
                    row.source.group.value(),
                    row.target.group.value(),
                    projection,
                )?;
                payload = payload
                    .checked_add(view.prepared_payload())
                    .context("Close Group aggregate view payload overflow")?;
                ensure!(
                    payload <= PREPARED_VIEW_BYTES,
                    "Close Group view payload exceeds 8 MiB"
                );
                views.push(view);
            }
            ensure!(
                views.len() == plan.moves().len(),
                "Close Group source document is not uniquely retained"
            );
            let change = self.editor_groups.commit_close_group_merge(plan)?;
            self.editor_layout = layout;
            for view in views {
                view.publish();
            }
            Ok(Some(change))
        })();
        match result {
            Ok(Some(change)) => {
                self.breadcrumbs_ui_command(command);
                self.navigation_input_interaction();
                self.session_interaction();
                self.cancel_suggestions();
                self.clear_signature();
                self.cancel_symbols();
                // Accepted saves retain exact Document and original Membership
                // proof. Source retirement can never retarget a close receipt.
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
                self.apply_merged_group_change(change);
                self.focus = Focus::Editor;
                self.observe_navigation(navigation_history::Reason::Ordinary);
                self.invalidate_pending_extension_commands();
                self.message = if retired {
                    "Group merged; remaining close batch retired, approved original save retained"
                        .into()
                } else {
                    "Group merged; all documents retained".into()
                };
            }
            Ok(None) => {}
            Err(error) => {
                self.message = format!("Close Group rejected; buffers retained: {error:#}")
            }
        }
        true
    }
}
