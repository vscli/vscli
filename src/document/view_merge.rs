//! Historical-view staging for a joint group/layout merge; no focus or text I/O.
use super::{Document, Selection, ViewState, typing};
use anyhow::{Context, Result, ensure};
use std::mem::size_of;

const MAX_SELECTIONS: usize = 10_000;
const MAX_PREPARED_PAYLOAD: usize = 512 * 1024;

fn state_payload(secondary_capacity: usize) -> Result<usize> {
    // New owner Arc<()> and empty generated-pair Arc<Vec<_>> allocations.
    secondary_capacity
        .checked_mul(size_of::<Selection>())
        .and_then(|bytes| bytes.checked_add(size_of::<ViewState>()))
        .and_then(|bytes| bytes.checked_add(4 * size_of::<usize>() + size_of::<Vec<()>>()))
        .context("Historical merge payload overflow")
}

/// Explicit caller policy; this type does not prove live group membership or
/// identify which source editor VS Code would show during a merge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMergeProjection {
    /// Copy exact retained source geometry; retire destination private sessions.
    CopySource,
    /// Keep an exact retained destination, including its private sessions.
    KeepTarget,
    /// Admit a genuinely absent destination at a fresh native origin.
    Origin,
}

/// Exclusive preparation lease. The caller validates and publishes the entire
/// Groups/Layout transaction before infallibly publishing disjoint model leases.
pub struct ViewMerge<'a> {
    document: &'a mut Document,
    source: u64,
    target: u64,
    prepared: Option<ViewState>,
    prepared_payload: usize,
}

fn validate_view(view: &ViewState, len: usize) -> Result<()> {
    ensure!(
        view.secondary.len() < MAX_SELECTIONS,
        "Historical view merge exceeds 10,000 selections"
    );
    ensure!(
        view.cursor <= len
            && view.anchor.is_none_or(|position| position <= len)
            && view.secondary.iter().all(|selection| {
                selection.cursor <= len && selection.anchor.is_none_or(|position| position <= len)
            }),
        "Historical merge selection is outside current source"
    );
    Ok(())
}

impl Document {
    /// Exact retained group view; unlike view_state(), never substitutes the
    /// current view for a missing group. Neutral zero is not a group identity.
    pub fn retained_view_state(&self, id: u64) -> Option<&ViewState> {
        if id == 0 {
            None
        } else if id == self.active_view {
            Some(&self.view)
        } else {
            self.other_views.get(&id)
        }
    }

    pub fn prepare_historical_view_merge(
        &mut self,
        source: u64,
        target: u64,
        projection: ViewMergeProjection,
    ) -> Result<ViewMerge<'_>> {
        ensure!(
            source != 0 && target != 0 && source != target,
            "Invalid historical merge view identity"
        );
        // Unlike view_state(), exact lookup must not fall back to another view.
        let source_view = self.retained_view_state(source);
        let target_view = self.retained_view_state(target);
        // All visited public cohorts are checked before allocation or mutation.
        validate_view(&self.view, self.len())?;
        if let Some(view) = source_view {
            validate_view(view, self.len())?;
        }
        if let Some(view) = target_view {
            validate_view(view, self.len())?;
        }
        ensure!(
            source_view.is_some() || self.active_view != 0,
            "Missing historical source has ambiguous neutral view ownership"
        );
        let (prepared, prepared_payload) = match projection {
            ViewMergeProjection::CopySource => {
                let original = source_view.context("Historical merge source view is absent")?;
                let snippet_generation = target_view.map_or(Ok(0), |view| {
                    view.snippet_generation
                        .checked_add(1)
                        .context("Historical merge target snippet generation exhausted")
                })?;
                let pair_generation = target_view.map_or(Ok(0), |view| {
                    view.pairs
                        .generation
                        .checked_add(1)
                        .context("Historical merge target pair generation exhausted")
                })?;
                ensure!(
                    state_payload(original.secondary.len())? <= MAX_PREPARED_PAYLOAD,
                    "Historical merge exceeds preparation payload budget"
                );
                let mut secondary = Vec::new();
                secondary
                    .try_reserve_exact(original.secondary.len())
                    .context("Historical merge selection allocation failed")?;
                secondary.extend_from_slice(&original.secondary);
                let payload = state_payload(secondary.capacity())?;
                ensure!(
                    payload <= MAX_PREPARED_PAYLOAD,
                    "Historical merge exceeds preparation payload budget"
                );
                let mut pairs = typing::Pairs::default();
                pairs.generation = pair_generation;
                let state = ViewState {
                    cursor: original.cursor,
                    anchor: original.anchor,
                    secondary,
                    top: original.top,
                    left: original.left,
                    desired_column: original.desired_column,
                    snippet_generation,
                    pairs,
                    // Fresh owner, no source/target snippet, pairs or history.
                    ..ViewState::default()
                };
                (Some(state), payload)
            }
            ViewMergeProjection::KeepTarget => {
                ensure!(
                    target_view.is_some(),
                    "Historical merge target view is absent"
                );
                (None, 0)
            }
            ViewMergeProjection::Origin => {
                ensure!(
                    target_view.is_none(),
                    "Historical merge target already exists"
                );
                (Some(ViewState::default()), state_payload(0)?)
            }
        };
        // A new inactive destination needs one insertion. Source-active installs
        // directly into the live view; existing destinations reuse their slot.
        if self.active_view != source && self.active_view != target && target_view.is_none() {
            self.other_views
                .try_reserve(1)
                .context("Historical merge view allocation failed")?;
        }
        Ok(ViewMerge {
            document: self,
            source,
            target,
            prepared,
            prepared_payload,
        })
    }
}

impl ViewMerge<'_> {
    /// New ViewState, actual secondary capacity, owner/pair Arc payload. Shared
    /// retained source/history, map allocation and allocator headers are excluded.
    pub fn prepared_payload(&self) -> usize {
        self.prepared_payload
    }

    pub fn publish(self) {
        let document = self.document;
        if document.active_view == self.source {
            let destination = self.prepared.unwrap_or_else(|| {
                document
                    .other_views
                    .remove(&self.target)
                    .expect("Exclusive lease retains its checked target")
            });
            document.other_views.remove(&self.source);
            document.other_views.remove(&self.target);
            document.view = destination;
            document.active_view = self.target;
        } else {
            document.other_views.remove(&self.source);
            if let Some(destination) = self.prepared {
                if document.active_view == self.target {
                    document.view = destination;
                } else {
                    document.other_views.insert(self.target, destination);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{editing_profile::TypingOptions, snippet::Template};
    use std::{collections::BTreeMap, sync::Arc};

    fn options() -> TypingOptions {
        TypingOptions {
            profile: crate::editing_profile::ProfileId::Cpp,
            ..Default::default()
        }
    }

    fn snippet(document: &mut Document, value: &str) {
        document
            .insert_snippet(
                &Template::parse(&format!("${{1:{value}}}$0")).unwrap(),
                &BTreeMap::new(),
            )
            .unwrap();
    }

    #[test]
    fn inactive_source_copies_exact_unicode_crlf_view_without_activating_or_editing() {
        let mut document = Document::from_text("猫🙂 first\r\nsecond e\u{301}\r\n");
        document.activate_view(1);
        document.set_selections(vec![
            Selection {
                cursor: 0,
                anchor: Some(2),
                desired_column: Some(9),
            },
            Selection::caret(12),
        ]);
        document.top = 1;
        document.left = 3;
        let original = document.view.clone();
        document.activate_view(3);
        document.move_to(4, false);
        document.insert("λ", false);
        // The retained secondary caret is after "se" on the second line.
        // Establish the real multicursor transaction before testing its Redo.
        assert_eq!(
            document.text.to_string(),
            "猫🙂 fλirst\r\nseλcond e\u{301}\r\n"
        );
        document.undo();
        assert_eq!(
            document.text.to_string(),
            "猫🙂 first\r\nsecond e\u{301}\r\n"
        );
        let active = document.view.clone();
        let source = document.other_views[&1].clone();
        let epoch = document.text_epoch();
        let revision = document.revision;
        let dirty = document.dirty();
        let directory = tempfile::tempdir().unwrap();
        let receipt = document
            .capture_save(directory.path().join("saved.txt"))
            .unwrap();
        assert!(document.retained_view_state(0).is_none());
        assert!(document.retained_view_state(1).is_some());
        assert!(document.retained_view_state(2).is_none());
        let lease = document
            .prepare_historical_view_merge(1, 2, ViewMergeProjection::CopySource)
            .unwrap();
        assert!(lease.prepared_payload() <= MAX_PREPARED_PAYLOAD);
        lease.publish();
        assert_eq!(document.active_view, 3);
        assert!(Arc::ptr_eq(&document.session_owner, &active.session_owner));
        assert_eq!(document.cursor, active.cursor);
        assert_eq!(document.anchor, active.anchor);
        assert_eq!(document.text_epoch(), epoch);
        assert_eq!(document.revision, revision);
        assert_eq!(document.dirty(), dirty);
        document.check_save_snapshot(&receipt).unwrap();
        assert!(document.retained_view_state(1).is_none());
        let target = document.retained_view_state(2).unwrap();
        assert_eq!(
            (target.cursor, target.anchor),
            (source.cursor, source.anchor)
        );
        assert_eq!(target.secondary, source.secondary);
        assert_eq!((target.top, target.left), (original.top, original.left));
        assert!(!Arc::ptr_eq(&target.session_owner, &source.session_owner));
        assert!(target.snippet.is_none());
        document.redo();
        assert_eq!(
            document.text.to_string(),
            "猫🙂 fλirst\r\nseλcond e\u{301}\r\n"
        );
        document.undo();
        assert_eq!(
            document.text.to_string(),
            "猫🙂 first\r\nsecond e\u{301}\r\n"
        );
    }

    #[test]
    fn kept_active_duplicate_retains_real_pair_undo_and_private_owner() {
        let mut document = Document::from_text("\r\n");
        document.activate_view(1);
        document.activate_view(2);
        document.type_character('(', options(), false).unwrap();
        document.type_character('x', options(), false).unwrap();
        let owner = document.session_owner.clone();
        let generation = document.pairs.generation;
        let epoch = document.text_epoch();
        let lease = document
            .prepare_historical_view_merge(1, 2, ViewMergeProjection::KeepTarget)
            .unwrap();
        assert_eq!(lease.prepared_payload(), 0);
        lease.publish();
        assert_eq!(document.active_view, 2);
        assert!(Arc::ptr_eq(&document.session_owner, &owner));
        assert_eq!(document.pairs.generation, generation);
        assert_eq!(document.text_epoch(), epoch);
        document.undo();
        assert_eq!(document.text.to_string(), "()\r\n");
        document.type_character(')', options(), false).unwrap();
        assert_eq!(document.text.to_string(), "()\r\n");
        assert_eq!(document.cursor, 2);
        document.redo();
        assert_eq!(document.text.to_string(), "(x)\r\n");
    }

    #[test]
    fn kept_inactive_duplicate_preserves_snippet_and_unrelated_current_view() {
        let mut document = Document::from_text("\r\n");
        document.activate_view(1);
        document.activate_view(2);
        snippet(&mut document, "x");
        document
            .type_character('y', Default::default(), false)
            .unwrap();
        let target_owner = document.session_owner.clone();
        document.activate_view(3);
        let unrelated_owner = document.session_owner.clone();
        let unrelated_cursor = document.cursor;
        document
            .prepare_historical_view_merge(1, 2, ViewMergeProjection::KeepTarget)
            .unwrap()
            .publish();
        assert_eq!(document.active_view, 3);
        assert_eq!(document.cursor, unrelated_cursor);
        assert!(Arc::ptr_eq(&document.session_owner, &unrelated_owner));
        assert!(Arc::ptr_eq(
            &document.other_views[&2].session_owner,
            &target_owner
        ));
        document.activate_view(2);
        assert!(document.in_snippet());
        document.undo();
        assert_eq!(document.text.to_string(), "x\r\n");
        assert!(document.in_snippet());
        document.redo();
        assert_eq!(document.text.to_string(), "y\r\n");
        assert!(document.in_snippet());
    }

    #[test]
    fn removed_current_source_installs_exact_kept_target_without_history_change() {
        let mut document = Document::from_text("猫🙂\r\n");
        document.activate_view(2);
        document.move_to(2, true);
        document.top = 7;
        document.left = 5;
        let target = document.view.clone();
        document.activate_view(1);
        document.move_to(0, false);
        document.insert("λ", false);
        document.undo();
        let epoch = document.text_epoch();
        let lengths = (document.undo.len(), document.redo.len());
        document
            .prepare_historical_view_merge(1, 2, ViewMergeProjection::KeepTarget)
            .unwrap()
            .publish();
        assert_eq!(document.active_view, 2);
        assert_eq!(
            (document.cursor, document.anchor),
            (target.cursor, target.anchor)
        );
        assert_eq!((document.top, document.left), (7, 5));
        assert!(Arc::ptr_eq(&document.session_owner, &target.session_owner));
        assert!(!document.other_views.contains_key(&1));
        assert!(!document.other_views.contains_key(&2));
        assert_eq!(document.text_epoch(), epoch);
        assert_eq!((document.undo.len(), document.redo.len()), lengths);
        document.redo();
        assert_eq!(document.text.to_string(), "λ猫🙂\r\n");
    }

    #[test]
    fn explicit_origin_is_fresh_and_does_not_clone_removed_active_source() {
        let mut document = Document::from_text("猫🙂\r\n");
        document.activate_view(1);
        document.move_to(2, true);
        document.top = 4;
        document.left = 9;
        let source_owner = document.session_owner.clone();
        let epoch = document.text_epoch();
        document
            .prepare_historical_view_merge(1, 2, ViewMergeProjection::Origin)
            .unwrap()
            .publish();
        assert_eq!(document.active_view, 2);
        assert_eq!(
            (
                document.cursor,
                document.anchor,
                document.top,
                document.left
            ),
            (0, None, 0, 0)
        );
        assert!(document.secondary.is_empty());
        assert!(!Arc::ptr_eq(&document.session_owner, &source_owner));
        assert_eq!(document.text_epoch(), epoch);
        assert_eq!(document.text.to_string(), "猫🙂\r\n");
    }

    #[test]
    fn removed_current_source_copy_preserves_reversed_cohort_receipt_and_text_redo() {
        let mut document = Document::from_text("猫🙂 first\r\n");
        document.activate_view(1);
        document.set_selections(vec![
            Selection {
                cursor: 2,
                anchor: Some(7),
                desired_column: None,
            },
            Selection::caret(8),
        ]);
        document.top = 4;
        document.left = 5;
        document.insert("λ", false);
        assert_eq!(document.text.to_string(), "猫🙂λtλ\r\n");
        document.undo();
        assert_eq!(document.text.to_string(), "猫🙂 first\r\n");
        let selections = document.selections();
        let source_owner = document.session_owner.clone();
        let epoch = document.text_epoch();
        let id = document.id;
        let lengths = (document.undo.len(), document.redo.len());
        let directory = tempfile::tempdir().unwrap();
        let receipt = document
            .capture_save(directory.path().join("saved.txt"))
            .unwrap();
        document
            .prepare_historical_view_merge(1, 2, ViewMergeProjection::CopySource)
            .unwrap()
            .publish();
        assert_eq!(document.active_view, 2);
        assert_eq!(document.id, id);
        assert_eq!(document.selections(), selections);
        assert_eq!((document.top, document.left), (4, 5));
        assert!(!Arc::ptr_eq(&document.session_owner, &source_owner));
        assert!(!document.other_views.contains_key(&1));
        assert!(!document.other_views.contains_key(&2));
        assert_eq!(document.text_epoch(), epoch);
        assert_eq!((document.undo.len(), document.redo.len()), lengths);
        document.check_save_snapshot(&receipt).unwrap();
        document.redo();
        assert_eq!(document.text.to_string(), "猫🙂λtλ\r\n");
        document.undo();
        assert_eq!(document.text.to_string(), "猫🙂 first\r\n");
    }

    #[test]
    fn missing_source_is_never_guessed_but_keep_and_origin_preserve_current_view() {
        let mut document = Document::from_text("猫🙂\r\n");
        document.activate_view(3);
        document.move_to(2, false);
        let owner = document.session_owner.clone();
        assert!(
            document
                .prepare_historical_view_merge(1, 2, ViewMergeProjection::CopySource)
                .is_err()
        );
        document
            .prepare_historical_view_merge(1, 3, ViewMergeProjection::KeepTarget)
            .unwrap()
            .publish();
        assert!(Arc::ptr_eq(&document.session_owner, &owner));
        document
            .prepare_historical_view_merge(1, 2, ViewMergeProjection::Origin)
            .unwrap()
            .publish();
        assert_eq!(document.active_view, 3);
        assert_eq!(document.cursor, 2);
        assert!(Arc::ptr_eq(&document.session_owner, &owner));
        assert_eq!(document.other_views[&2].cursor, 0);
        assert!(!document.other_views.contains_key(&1));
    }

    #[test]
    fn missing_source_with_neutral_private_state_refuses_without_touching_history() {
        let mut document = Document::from_text("\r\n");
        document.activate_view(2);
        document.activate_view(1);
        snippet(&mut document, "x");
        document
            .type_character('y', Default::default(), false)
            .unwrap();
        document.remove_view(1);
        assert_eq!(document.active_view, 0);
        assert!(document.in_snippet());
        let owner = document.session_owner.clone();
        let selections = document.selections();
        let epoch = document.text_epoch();
        let revision = document.revision;
        let lengths = (document.undo.len(), document.redo.len());
        for (target, mode) in [
            (2, ViewMergeProjection::KeepTarget),
            (3, ViewMergeProjection::Origin),
        ] {
            assert!(
                document
                    .prepare_historical_view_merge(1, target, mode)
                    .is_err()
            );
        }
        assert_eq!(document.active_view, 0);
        assert!(Arc::ptr_eq(&document.session_owner, &owner));
        assert_eq!(document.selections(), selections);
        assert_eq!(document.text_epoch(), epoch);
        assert_eq!(document.revision, revision);
        assert_eq!((document.undo.len(), document.redo.len()), lengths);
        assert!(document.in_snippet());
        assert!(document.other_views.contains_key(&2));
        assert!(!document.other_views.contains_key(&3));
        assert_eq!(document.text.to_string(), "y\r\n");
        // The rejected lease must not consume the real text history.
        document.undo();
        assert_eq!(document.text.to_string(), "x\r\n");
        document.redo();
        assert_eq!(document.text.to_string(), "y\r\n");
    }

    #[test]
    fn copying_over_old_target_retires_pair_history_even_when_old_target_is_current() {
        let mut document = Document::from_text("\r\n");
        document.activate_view(1);
        document.activate_view(2);
        document.type_character('(', options(), false).unwrap();
        document.type_character('x', options(), false).unwrap();
        let old_target_owner = document.session_owner.clone();
        document
            .prepare_historical_view_merge(1, 2, ViewMergeProjection::CopySource)
            .unwrap()
            .publish();
        assert_eq!(document.active_view, 2);
        assert!(!Arc::ptr_eq(&document.session_owner, &old_target_owner));
        document.undo();
        assert_eq!(document.text.to_string(), "()\r\n");
        assert_eq!(document.cursor, 1);
        document.type_character(')', options(), false).unwrap();
        assert_eq!(document.text.to_string(), "())\r\n");
        document.undo();
        document.redo();
        assert_eq!(document.text.to_string(), "())\r\n");
    }

    #[test]
    fn dropped_leases_and_invalid_mode_identity_leave_projection_and_redo_inert() {
        let mut document = Document::from_text("猫🙂\r\n");
        document.activate_view(1);
        document.activate_view(2);
        document.activate_view(3);
        document.insert("λ", false);
        document.undo();
        let owner = document.session_owner.clone();
        let epoch = document.text_epoch();
        let lengths = (document.undo.len(), document.redo.len());
        for mode in [
            ViewMergeProjection::CopySource,
            ViewMergeProjection::KeepTarget,
        ] {
            drop(document.prepare_historical_view_merge(1, 2, mode).unwrap());
        }
        drop(
            document
                .prepare_historical_view_merge(1, 4, ViewMergeProjection::Origin)
                .unwrap(),
        );
        for (source, target, mode) in [
            (0, 2, ViewMergeProjection::CopySource),
            (1, 0, ViewMergeProjection::CopySource),
            (1, 1, ViewMergeProjection::CopySource),
            (1, 2, ViewMergeProjection::Origin),
            (1, 4, ViewMergeProjection::KeepTarget),
        ] {
            assert!(
                document
                    .prepare_historical_view_merge(source, target, mode)
                    .is_err()
            );
        }
        assert_eq!(document.active_view, 3);
        assert!(Arc::ptr_eq(&document.session_owner, &owner));
        assert!(document.other_views.contains_key(&1));
        assert!(document.other_views.contains_key(&2));
        assert!(!document.other_views.contains_key(&4));
        assert_eq!(document.text_epoch(), epoch);
        assert_eq!((document.undo.len(), document.redo.len()), lengths);
        document.redo();
        assert_eq!(document.text.to_string(), "λ猫🙂\r\n");
    }

    #[test]
    fn copy_refuses_retirement_overflow_keep_requires_no_gratuitous_clock_increment() {
        let mut document = Document::from_text("x");
        document.activate_view(1);
        document.activate_view(2);
        document.activate_view(3);
        document.other_views.get_mut(&2).unwrap().snippet_generation = u64::MAX;
        assert!(
            document
                .prepare_historical_view_merge(1, 2, ViewMergeProjection::CopySource)
                .is_err()
        );
        document.other_views.get_mut(&2).unwrap().snippet_generation = 0;
        document.other_views.get_mut(&2).unwrap().pairs.generation = u64::MAX;
        assert!(
            document
                .prepare_historical_view_merge(1, 2, ViewMergeProjection::CopySource)
                .is_err()
        );
        let owner = document.other_views[&2].session_owner.clone();
        document
            .prepare_historical_view_merge(1, 2, ViewMergeProjection::KeepTarget)
            .unwrap()
            .publish();
        assert_eq!(document.other_views[&2].pairs.generation, u64::MAX);
        assert!(Arc::ptr_eq(&document.other_views[&2].session_owner, &owner));
        assert_eq!(document.active_view, 3);
    }

    #[test]
    fn cohort_bounds_precede_copy_and_late_invalid_selection_preserves_all_state() {
        let mut document = Document::from_text("猫🙂\r\n");
        document.activate_view(1);
        document.secondary = vec![Selection::caret(1); MAX_SELECTIONS - 1];
        document.activate_view(3);
        let lease = document
            .prepare_historical_view_merge(1, 2, ViewMergeProjection::CopySource)
            .unwrap();
        assert!(lease.prepared_payload() <= MAX_PREPARED_PAYLOAD);
        drop(lease);
        let source = document.other_views.get_mut(&1).unwrap();
        source.secondary.push(Selection::caret(1));
        assert!(
            document
                .prepare_historical_view_merge(1, 2, ViewMergeProjection::CopySource)
                .is_err()
        );
        document.other_views.get_mut(&1).unwrap().secondary.pop();
        document
            .other_views
            .get_mut(&1)
            .unwrap()
            .secondary
            .last_mut()
            .unwrap()
            .cursor = 99;
        assert!(
            document
                .prepare_historical_view_merge(1, 2, ViewMergeProjection::CopySource)
                .is_err()
        );
        assert_eq!(document.active_view, 3);
        assert!(!document.other_views.contains_key(&2));
        assert!(document.other_views.contains_key(&1));
        assert_eq!(document.text.to_string(), "猫🙂\r\n");
    }

    #[test]
    fn rejected_second_document_lease_cannot_publish_first_document_or_lose_redo() {
        let mut first = Document::from_text("猫🙂\r\n");
        first.activate_view(1);
        first.insert("λ", false);
        first.undo();
        let epoch = first.text_epoch();
        let mut second = Document::from_text("other\r\n");
        second.activate_view(1);
        second.activate_view(2);
        second.snippet_generation = u64::MAX;
        let first_lease = first
            .prepare_historical_view_merge(1, 2, ViewMergeProjection::CopySource)
            .unwrap();
        assert!(
            second
                .prepare_historical_view_merge(1, 2, ViewMergeProjection::CopySource)
                .is_err()
        );
        drop(first_lease);
        assert_eq!(first.active_view, 1);
        assert!(!first.other_views.contains_key(&2));
        assert_eq!(first.text_epoch(), epoch);
        assert_eq!(second.active_view, 2);
        assert_eq!(second.text.to_string(), "other\r\n");
        first.redo();
        assert_eq!(first.text.to_string(), "λ猫🙂\r\n");
    }
}
