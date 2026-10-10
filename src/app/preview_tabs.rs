//! Native preview admission and immediate shared-model promotion.
//!
//! Accepted transactions promote before subsequent UI work or save continuations.
//! Session restoration stays committed; sticky tabs are outside this module.
use super::*;
use crate::editor_groups::{Change, Membership, OpenMode, UiProof};
use anyhow::{Context, ensure};
use std::sync::Arc;

#[derive(Clone, Copy)]
struct Tracked {
    member: Membership,
    index: usize,
}
#[derive(Default)]
pub(super) struct State {
    generation: Option<u64>,
    records: [Option<Tracked>; crate::editor_groups::MAX_GROUPS],
}
#[derive(Clone, Copy)]
enum Pool {
    Visible,
    Hidden,
}
struct ModelProof {
    pool: Pool,
    index: usize,
    document: u64,
    path: Option<PathBuf>,
    revision: u64,
    epoch: u64,
    saved_revision: u64,
    save_generation: u64,
    dirty: bool,
}
impl ModelProof {
    fn capture(pool: Pool, index: usize, doc: &Document) -> Self {
        Self {
            pool,
            index,
            document: doc.id,
            path: doc.path.clone(),
            revision: doc.revision,
            epoch: doc.text_epoch(),
            saved_revision: doc.saved_revision,
            save_generation: doc.save_generation(),
            dirty: doc.dirty(),
        }
    }
    fn current(&self, app: &App) -> bool {
        let pool = match self.pool {
            Pool::Visible => &app.documents,
            Pool::Hidden => &app.hidden_documents,
        };
        pool.get(self.index).is_some_and(|doc| {
            doc.id == self.document
                && doc.path == self.path
                && doc.revision == self.revision
                && doc.text_epoch() == self.epoch
                && doc.saved_revision == self.saved_revision
                && doc.save_generation() == self.save_generation
        })
    }
}

/// Pure admission authorization. No model is removed before its final recheck.
/// Destination focus is part of the UiProof, including same-group A→B→A.
pub(super) struct Admission {
    document: u64,
    mode: OpenMode,
    groups: UiProof,
    workspace: PathBuf,
    settings: Arc<Vec<serde_json::Map<String, Value>>>,
    profile: u64,
    target: Option<ModelProof>,
    replacement: Option<(Membership, ModelProof)>,
}

impl App {
    /// Refresh only after structural/model changes, or a positively stale cache.
    /// Group::preview may inspect bounded tab lists here, never every text key.
    pub(super) fn refresh_preview_tabs(&mut self) -> Result<()> {
        let mut records = [None; crate::editor_groups::MAX_GROUPS];
        if !self.group_fallback {
            ensure!(
                self.editor_groups.groups().len() <= records.len(),
                "Preview group limit exceeded"
            );
            for (slot, group) in self.editor_groups.groups().iter().enumerate() {
                if let Some(member) = group.preview() {
                    let index = self
                        .documents
                        .iter()
                        .position(|doc| doc.id == member.document)
                        .context("Preview model is no longer retained")?;
                    records[slot] = Some(Tracked { member, index });
                }
            }
        }
        self.preview_tabs = State {
            generation: Some(self.editor_groups.generation()),
            records,
        };
        Ok(())
    }

    /// At most four cached model reads on ordinary input. Promotion is permanent
    /// tab metadata, independent of Undo snapshots and generated/snippet marks.
    pub(super) fn promote_dirty_preview_tabs(&mut self) -> Result<bool> {
        let stale_index = self.preview_tabs.records.iter().flatten().any(|record| {
            self.documents
                .get(record.index)
                .is_none_or(|doc| doc.id != record.member.document)
        });
        if stale_index || self.preview_tabs.generation != Some(self.editor_groups.generation()) {
            self.refresh_preview_tabs()?;
        }
        let mut dirty = [0_u64; crate::editor_groups::MAX_GROUPS];
        let mut dirty_count = 0;
        for record in self.preview_tabs.records.iter().flatten() {
            let doc = &self.documents[record.index]; // Exact ID was checked above.
            if doc.dirty() && !dirty[..dirty_count].contains(&doc.id) {
                dirty[dirty_count] = doc.id;
                dirty_count += 1;
            }
        }
        if dirty_count == 0 {
            return Ok(false);
        }
        let change = if dirty_count == 1 {
            // No engine clone for normal single-model typing/paste/Undo.
            self.editor_groups.promote_document(dirty[0])?
        } else {
            // A rare atomic workspace edit may dirty several previews. Stage
            // all checked counter transitions before publishing any promotion.
            let mut staged = self.editor_groups.clone();
            let mut merged = Change {
                previous: self.editor_groups.active_membership(),
                ..Change::default()
            };
            for id in &dirty[..dirty_count] {
                let change = staged.promote_document(*id)?;
                merged.changed |= change.changed;
                merged.promoted.extend(change.promoted);
            }
            merged.active = staged.active_membership();
            self.editor_groups = staged;
            merged
        };
        let changed = change.changed;
        self.apply_group_change(change);
        self.refresh_preview_tabs()?;
        Ok(changed)
    }

    fn preview_model_proof(&self, document: u64) -> Option<ModelProof> {
        self.documents
            .iter()
            .enumerate()
            .find(|(_, doc)| doc.id == document)
            .map(|(index, doc)| ModelProof::capture(Pool::Visible, index, doc))
            .or_else(|| {
                self.hidden_documents
                    .iter()
                    .enumerate()
                    .find(|(_, doc)| doc.id == document)
                    .map(|(index, doc)| ModelProof::capture(Pool::Hidden, index, doc))
            })
    }

    /// No path resolution, file read or full-Rope comparison on this UI path.
    fn preview_replacement_safe(&self, member: Membership) -> bool {
        !self.group_fallback
            && !self.preview_admission_failed
            && self
                .editor_groups
                .group(member.group)
                .is_some_and(|group| group.preview() == Some(member))
            && self.documents.iter().find(|doc| doc.id == member.document).is_some_and(|doc| {
                doc.path.is_some() && !doc.dirty()
            })
            && !self.document_save_pending(member.document)
            // Required root seam: saving::State retains a private SaveAs origin.
            && !self.save_as_document_pending(member.document)
            && !self.settings_writes_busy()
            && self.file_job.is_none()
            && self.close_membership != Some(member)
            && self.closing_group.as_ref().is_none_or(|closing| {
                !closing.proof.members().contains(&member)
            })
    }

    /// Preflight before moving a hidden target or touching the old preview.
    /// Caller chooses mode from the original immutable intent/settings proof.
    pub(super) fn prepare_preview_open(&self, document: u64, mode: OpenMode) -> Result<Admission> {
        ensure!(
            !self.group_fallback,
            "Preview layout is unavailable for recovered over-cap buffers"
        );
        ensure!(
            mode != OpenMode::Preview || !self.preview_admission_failed,
            "Preview admission disabled after promotion failure; all buffers retained, restart the editor"
        );
        let group = self
            .editor_groups
            .active_group()
            .and_then(|id| self.editor_groups.group(id));
        for group in self.editor_groups.groups() {
            if let Some(member) = group.preview() {
                ensure!(
                    self.documents.iter().any(|doc| doc.id == member.document),
                    "Preview model is no longer retained"
                );
            }
        }
        let target = self.preview_model_proof(document);
        // Dirty retained authority is committed before choosing a replacement.
        // A later promotion cannot restore a clean preview already discarded.
        let mode = if target.as_ref().is_some_and(|model| model.dirty) {
            OpenMode::Committed
        } else {
            mode
        };
        ensure!(
            mode != OpenMode::Preview || target.as_ref().is_none_or(|model| model.path.is_some()),
            "Untitled models must be committed"
        );
        let existing =
            group.is_some_and(|group| group.tabs().iter().any(|tab| tab.document() == document));
        let replacement = if mode == OpenMode::Preview && !existing {
            group
                .and_then(|group| group.preview())
                .filter(|member| self.preview_replacement_safe(*member))
        } else {
            None
        };
        self.editor_groups
            .can_open_mode(document, mode, replacement)?;
        let replacement = replacement.map(|member| {
            let model = self
                .preview_model_proof(member.document)
                .expect("Safe visible preview checked");
            (member, model)
        });
        Ok(Admission {
            document,
            mode,
            groups: self.editor_groups.proof(),
            workspace: self.workspace.root.clone(),
            settings: self.settings.extension_layers().clone(),
            profile: self.settings_profile_generation(),
            target,
            replacement,
        })
    }

    fn recheck_preview_admission(&self, admission: &Admission) -> Result<()> {
        ensure!(
            !self.group_fallback
                && self.editor_groups.proof_current(&admission.groups)
                && self.workspace.root == admission.workspace
                && Arc::ptr_eq(self.settings.extension_layers(), &admission.settings)
                && self.settings_profile_generation() == admission.profile,
            "Preview destination or settings changed before admission"
        );
        ensure!(
            admission
                .target
                .as_ref()
                .is_none_or(|target| target.current(self)),
            "Preview target model changed before admission"
        );
        if let Some((member, model)) = &admission.replacement {
            ensure!(
                model.current(self) && self.preview_replacement_safe(*member),
                "Original preview is no longer safe to replace"
            );
        }
        self.editor_groups.can_open_mode(
            admission.document,
            admission.mode,
            admission.replacement.as_ref().map(|(member, _)| *member),
        )
    }

    /// Engine-only publication for a root-owned staged install. Root must have
    /// reserved/validated the new model before this and finish installation in
    /// the same non-awaiting call, then apply Change and refresh the tracker.
    pub(super) fn commit_preview_open(&mut self, admission: Admission) -> Result<Change> {
        self.recheck_preview_admission(&admission)?;
        self.editor_groups.open_mode(
            admission.document,
            admission.mode,
            admission.replacement.map(|(member, _)| member),
        )
    }

    /// Existing authoritative model admission, without cloning its text/history.
    pub(super) fn open_preview_model(&mut self, document: u64, mode: OpenMode) -> Result<()> {
        ensure!(
            self.preview_model_proof(document).is_some(),
            "Preview target model is no longer retained"
        );
        self.install_preview_model(document, None, mode)
    }

    /// Loaded-document seam: reuse any retained authority before configuration,
    /// preserve hidden identities, and fail before any old model is removed.
    pub(super) fn install_preview_document(
        &mut self,
        mut doc: Document,
        mode: OpenMode,
    ) -> Result<()> {
        if let Some(id) = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .find(|old| {
                doc.path
                    .as_ref()
                    .is_some_and(|path| old.path.as_ref() == Some(path))
            })
            .map(|old| old.id)
        {
            return self.open_preview_model(id, mode);
        }
        let mode = if doc.dirty() {
            OpenMode::Committed
        } else {
            mode
        };
        ensure!(
            mode != OpenMode::Preview || doc.path.is_some(),
            "Untitled models must be committed"
        );
        self.settings.apply(&mut doc);
        let configuration_error = self.configure_document_language(&mut doc).err();
        self.install_preview_model(doc.id, Some(doc), mode)?;
        if let Some(error) = configuration_error {
            self.message.push_str(&format!(
                " · Native language configuration rejected: {error:#}"
            ));
        }
        Ok(())
    }

    fn install_preview_model(
        &mut self,
        document: u64,
        candidate: Option<Document>,
        mode: OpenMode,
    ) -> Result<()> {
        let admission = self.prepare_preview_open(document, mode)?;
        let hidden = admission
            .target
            .as_ref()
            .and_then(|target| match target.pool {
                Pool::Hidden => Some(target.index),
                Pool::Visible => None,
            });
        ensure!(
            candidate.is_some() || admission.target.is_some(),
            "No authoritative preview model to install"
        );
        if hidden.is_some() || candidate.is_some() {
            self.documents
                .try_reserve(1)
                .context("Cannot reserve preview model storage")?;
        }
        // Every fallible admission/configuration check is now complete. There is
        // no callback, await, I/O or further fallible edit until Change applies.
        let change = self.commit_preview_open(admission)?;
        if let Some(index) = hidden {
            let doc = self.hidden_documents.remove(index);
            self.documents.push(doc);
        } else if let Some(doc) = candidate {
            self.documents.push(doc);
        }
        for member in &change.removed {
            if self
                .editor_groups
                .memberships(member.document)
                .next()
                .is_none()
                && let Some(index) = self
                    .documents
                    .iter()
                    .position(|doc| doc.id == member.document)
            {
                // Only the exact safe preview authorized above can be removed.
                self.documents.remove(index);
            }
        }
        self.active = self
            .documents
            .iter()
            .position(|doc| doc.id == document)
            .expect("Prevalidated authoritative model installed");
        self.focus = Focus::Editor;
        self.apply_group_change(change);
        self.refresh_preview_tabs()?;
        self.message = format!("Opened {}", self.doc().name());
        self.remember_active_file();
        self.observe_navigation(navigation_history::Reason::EditorChange);
        Ok(())
    }

    /// Reconciliation/focus must not use the Committed open convenience wrapper.
    /// False means no current-group membership; caller may deliberately admit
    /// an unassigned model committed. This helper never admits another group.
    pub(super) fn retain_focused_preview(&mut self, document: u64) -> Result<bool> {
        if self.group_fallback {
            return Ok(false);
        }
        let Some(group) = self
            .editor_groups
            .active_group()
            .and_then(|id| self.editor_groups.group(id))
        else {
            return Ok(false);
        };
        let Some(tab) = group.tabs().iter().find(|tab| tab.document() == document) else {
            return Ok(false);
        };
        let member = Membership {
            group: group.id(),
            tab: tab.id(),
            document,
        };
        let change = self.editor_groups.focus(member)?;
        let changed = change.changed;
        // A no-op render reconciliation does not refresh preview records or
        // retire ownership. Root must call deliberate UI focus outside render.
        if changed {
            self.apply_group_change(change);
            self.refresh_preview_tabs()?;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, App, Vec<PathBuf>) {
        let root = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for name in ["a.cpp", "b.cpp", "c.cpp"] {
            let path = root.path().join(name);
            std::fs::write(&path, "猫🙂 alpha\r\nbody\r\n").unwrap();
            paths.push(std::fs::canonicalize(path).unwrap());
        }
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.extension_node = root.path().join("missing-node").display().to_string();
        app.install_preview_document(Document::open(&paths[0]).unwrap(), OpenMode::Preview)
            .unwrap();
        (root, app, paths)
    }
    #[test]
    fn focus_motion_keeps_preview_but_dirty_undo_never_demotes_it() {
        let (_root, mut app, paths) = fixture();
        let first = app.active_tab_membership().unwrap();
        let before = app.doc().text.to_string();
        app.doc_mut().move_to(2, false);
        assert!(app.retain_focused_preview(first.document).unwrap());
        assert_eq!(
            app.editor_groups.group(first.group).unwrap().preview(),
            Some(first)
        );
        assert_eq!(app.doc().cursor, 2);
        app.doc_mut().insert("X", false);
        assert!(app.promote_dirty_preview_tabs().unwrap());
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), before);
        assert!(!app.doc().dirty());
        assert_eq!(
            app.editor_groups.group(first.group).unwrap().preview(),
            None
        );
        app.install_preview_document(Document::open(&paths[1]).unwrap(), OpenMode::Preview)
            .unwrap();
        assert!(app.editor_groups.membership_current(first));
        app.open_preview_model(first.document, OpenMode::Preview)
            .unwrap();
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), "猫🙂X alpha\r\nbody\r\n");
        assert_eq!(std::fs::read(&paths[0]).unwrap(), before.as_bytes());
    }
    #[test]
    fn stale_cached_model_index_refreshes_before_dirty_promotion() {
        let (_root, mut app, paths) = fixture();
        let first = app.active_tab_membership().unwrap();
        app.install_preview_document(Document::open(&paths[1]).unwrap(), OpenMode::Committed)
            .unwrap();
        app.documents.swap(0, 1);
        app.active = 1;
        app.doc_mut().insert("X", false);
        assert!(app.promote_dirty_preview_tabs().unwrap());
        assert_eq!(
            app.editor_groups.group(first.group).unwrap().preview(),
            None
        );
        assert_eq!(app.documents[0].text.to_string(), "猫🙂 alpha\r\nbody\r\n");
        assert!(app.documents[1].dirty());
        assert_eq!(app.documents[1].id, first.document);
    }
    #[test]
    fn old_admission_rejects_same_byte_model_aba_without_mutation() {
        let (_root, mut app, paths) = fixture();
        let target = Document::open(&paths[1]).unwrap();
        let admission = app
            .prepare_preview_open(target.id, OpenMode::Preview)
            .unwrap();
        let first = app.active_tab_membership().unwrap();
        app.doc_mut().insert("X", false);
        app.doc_mut().undo();
        let groups = app.editor_groups.clone();
        let epoch = app.doc().text_epoch();
        assert!(app.commit_preview_open(admission).is_err());
        assert_eq!(app.editor_groups, groups);
        assert_eq!(app.doc().text_epoch(), epoch);
        assert!(app.editor_groups.membership_current(first));
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), "X猫🙂 alpha\r\nbody\r\n");
        assert_eq!(
            std::fs::read(&paths[0]).unwrap(),
            "猫🙂 alpha\r\nbody\r\n".as_bytes()
        );
    }
    #[test]
    fn exact_save_as_origin_prevents_clean_preview_eviction() {
        let (_root, mut app, paths) = fixture();
        let first = app.active_tab_membership().unwrap();
        app.capture_save_as_origin();
        app.install_preview_document(Document::open(&paths[1]).unwrap(), OpenMode::Preview)
            .unwrap();
        assert!(app.editor_groups.membership_current(first));
        assert_eq!(app.documents.len(), 2);
        assert_eq!(
            app.editor_groups.group(first.group).unwrap().tabs().len(),
            2
        );
        assert_eq!(
            app.editor_groups
                .group(first.group)
                .unwrap()
                .preview()
                .unwrap()
                .document,
            app.doc().id
        );
        assert_eq!(
            std::fs::read(&paths[0]).unwrap(),
            "猫🙂 alpha\r\nbody\r\n".as_bytes()
        );
    }
    #[test]
    fn focus_round_trip_cannot_publish_an_old_preview_admission() {
        let (_root, mut app, paths) = fixture();
        let first = app.active_tab_membership().unwrap();
        app.install_preview_document(Document::open(&paths[1]).unwrap(), OpenMode::Committed)
            .unwrap();
        let second = app.active_tab_membership().unwrap();
        app.open_preview_model(first.document, OpenMode::Preview)
            .unwrap();
        let third = Document::open(&paths[2]).unwrap();
        let admission = app
            .prepare_preview_open(third.id, OpenMode::Preview)
            .unwrap();
        app.open_preview_model(second.document, OpenMode::Committed)
            .unwrap();
        app.open_preview_model(first.document, OpenMode::Preview)
            .unwrap();
        let before = app.editor_groups.clone();
        assert!(app.commit_preview_open(admission).is_err());
        assert_eq!(app.editor_groups, before);
        assert_eq!(app.active_tab_membership(), Some(first));
        assert_eq!(
            app.editor_groups.group(first.group).unwrap().preview(),
            Some(first)
        );
    }
    #[test]
    fn protected_full_group_rejects_before_removing_a_hidden_target_or_redo() {
        let (_root, mut app, paths) = fixture();
        let first = app.active_tab_membership().unwrap();
        for _ in 1..crate::editor_groups::MAX_TABS_PER_GROUP {
            app.install_preview_document(Document::from_text("retained\r\n"), OpenMode::Committed)
                .unwrap();
        }
        app.open_preview_model(first.document, OpenMode::Preview)
            .unwrap();
        app.doc_mut().insert("X", false);
        app.doc_mut().undo();
        app.capture_save_as_origin();
        let hidden = Document::open(&paths[1]).unwrap();
        let hidden_id = hidden.id;
        app.hidden_documents.push(hidden);
        let before = app.editor_groups.clone();
        let epoch = app.doc().text_epoch();
        assert!(
            app.open_preview_model(hidden_id, OpenMode::Preview)
                .is_err()
        );
        assert_eq!(app.editor_groups, before);
        assert_eq!(app.active_tab_membership(), Some(first));
        assert_eq!(app.doc().text_epoch(), epoch);
        assert_eq!(app.hidden_documents.len(), 1);
        assert_eq!(app.hidden_documents[0].id, hidden_id);
        assert_eq!(
            app.documents.len(),
            crate::editor_groups::MAX_TABS_PER_GROUP
        );
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), "X猫🙂 alpha\r\nbody\r\n");
        assert_eq!(
            std::fs::read(&paths[0]).unwrap(),
            "猫🙂 alpha\r\nbody\r\n".as_bytes()
        );
    }
    #[test]
    fn shared_preview_replacement_preserves_authoritative_id_and_redo() {
        let (_root, mut app, paths) = fixture();
        let first = app.active_tab_membership().unwrap();
        app.doc_mut().insert("X", false);
        app.promote_dirty_preview_tabs().unwrap();
        app.doc_mut().undo();
        app.install_preview_document(Document::open(&paths[1]).unwrap(), OpenMode::Committed)
            .unwrap();
        app.split_editor(false);
        app.open_preview_model(first.document, OpenMode::Preview)
            .unwrap();
        let shared_preview = app.active_tab_membership().unwrap();
        assert_ne!(first.group, shared_preview.group);
        app.install_preview_document(Document::open(&paths[2]).unwrap(), OpenMode::Preview)
            .unwrap();
        assert!(!app.editor_groups.membership_current(shared_preview));
        assert!(app.editor_groups.membership_current(first));
        assert_eq!(app.editor_groups.memberships(first.document).count(), 1);
        let retained = app
            .documents
            .iter()
            .find(|doc| doc.id == first.document)
            .unwrap();
        assert_eq!(retained.text.to_string(), "猫🙂 alpha\r\nbody\r\n");
        app.focus_pane(0);
        app.open_preview_model(first.document, OpenMode::Committed)
            .unwrap();
        app.doc_mut().redo();
        assert_eq!(app.doc().id, first.document);
        assert_eq!(app.doc().text.to_string(), "X猫🙂 alpha\r\nbody\r\n");
        assert_eq!(
            std::fs::read(&paths[0]).unwrap(),
            "猫🙂 alpha\r\nbody\r\n".as_bytes()
        );
    }
    #[test]
    fn shared_dirty_target_is_committed_before_selecting_preview_replacement() {
        let (_root, mut app, paths) = fixture();
        let first = app.active_tab_membership().unwrap();
        app.doc_mut().insert("X", false);
        app.promote_dirty_preview_tabs().unwrap();
        app.install_preview_document(Document::open(&paths[1]).unwrap(), OpenMode::Committed)
            .unwrap();
        app.split_editor(false);
        app.install_preview_document(Document::open(&paths[2]).unwrap(), OpenMode::Preview)
            .unwrap();
        let preview = app.active_tab_membership().unwrap();
        app.open_preview_model(first.document, OpenMode::Preview)
            .unwrap();
        assert_eq!(
            app.editor_groups.group(preview.group).unwrap().preview(),
            Some(preview)
        );
        assert!(app.editor_groups.membership_current(preview));
        assert_eq!(app.editor_groups.memberships(first.document).count(), 2);
        assert_eq!(app.doc().id, first.document);
        assert_eq!(app.doc().text.to_string(), "X猫🙂 alpha\r\nbody\r\n");
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "猫🙂 alpha\r\nbody\r\n");
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), "X猫🙂 alpha\r\nbody\r\n");
        for path in paths {
            assert_eq!(
                std::fs::read(path).unwrap(),
                "猫🙂 alpha\r\nbody\r\n".as_bytes()
            );
        }
    }
    #[test]
    fn hidden_dirty_target_is_committed_without_discarding_current_preview() {
        let (_root, mut app, paths) = fixture();
        let preview = app.active_tab_membership().unwrap();
        let mut hidden = Document::open(&paths[1]).unwrap();
        let id = hidden.id;
        hidden.insert("X", false);
        app.hidden_documents.push(hidden);
        app.open_preview_model(id, OpenMode::Preview).unwrap();
        assert_eq!(
            app.editor_groups.group(preview.group).unwrap().preview(),
            Some(preview)
        );
        assert!(app.editor_groups.membership_current(preview));
        assert!(app.hidden_documents.is_empty());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), "X猫🙂 alpha\r\nbody\r\n");
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "猫🙂 alpha\r\nbody\r\n");
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), "X猫🙂 alpha\r\nbody\r\n");
        for path in paths {
            assert_eq!(
                std::fs::read(path).unwrap(),
                "猫🙂 alpha\r\nbody\r\n".as_bytes()
            );
        }
    }
    #[test]
    fn dirty_loaded_candidate_is_committed_without_discarding_current_preview() {
        let (_root, mut app, paths) = fixture();
        let preview = app.active_tab_membership().unwrap();
        let mut candidate = Document::open(&paths[1]).unwrap();
        let id = candidate.id;
        candidate.insert("X", false);
        app.install_preview_document(candidate, OpenMode::Preview)
            .unwrap();
        assert_eq!(
            app.editor_groups.group(preview.group).unwrap().preview(),
            Some(preview)
        );
        assert!(app.editor_groups.membership_current(preview));
        assert_eq!(app.doc().id, id);
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "猫🙂 alpha\r\nbody\r\n");
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), "X猫🙂 alpha\r\nbody\r\n");
        for path in paths {
            assert_eq!(
                std::fs::read(path).unwrap(),
                "猫🙂 alpha\r\nbody\r\n".as_bytes()
            );
        }
    }
    #[test]
    fn failed_multi_model_promotion_fences_undo_clean_replacement_and_preserves_redo() {
        let (_root, mut app, paths) = fixture();
        let a = app.active_tab_membership().unwrap();
        app.split_editor(false);
        app.install_preview_document(Document::open(&paths[1]).unwrap(), OpenMode::Preview)
            .unwrap();
        let b = app.active_tab_membership().unwrap();
        app.documents
            .iter_mut()
            .find(|doc| doc.id == a.document)
            .unwrap()
            .insert("A", false);
        app.documents
            .iter_mut()
            .find(|doc| doc.id == b.document)
            .unwrap()
            .insert("B", false);
        app.editor_groups
            .fixture_exhaust_membership_generation(a.group);
        let groups = app.editor_groups.clone();
        assert!(!app.preview_edit_barrier());
        assert!(app.preview_admission_failed);
        assert_eq!(app.editor_groups, groups);
        app.doc_mut().undo();
        assert!(!app.doc().dirty());
        assert_eq!(app.doc().text.to_string(), "猫🙂 alpha\r\nbody\r\n");
        let epoch = app.doc().text_epoch();
        app.refresh_preview_tabs().unwrap();
        assert!(app.preview_admission_failed);
        assert!(
            app.install_preview_document(Document::open(&paths[2]).unwrap(), OpenMode::Preview)
                .is_err()
        );
        assert_eq!(app.editor_groups, groups);
        assert_eq!(app.documents.len(), 2);
        assert_eq!(app.doc().text_epoch(), epoch);
        assert!(app.editor_groups.membership_current(a));
        assert!(app.editor_groups.membership_current(b));
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), "B猫🙂 alpha\r\nbody\r\n");
        assert!(app.doc().dirty());
        for path in paths {
            assert_eq!(
                std::fs::read(path).unwrap(),
                "猫🙂 alpha\r\nbody\r\n".as_bytes()
            );
        }
    }
    #[test]
    fn actual_pending_native_save_keeps_a_clean_preview_until_settlement() {
        let (_root, mut app, paths) = fixture();
        let first = app.active_tab_membership().unwrap();
        let original = app.doc().text.to_string();
        let generation = app.doc().save_generation();
        app.request_native_save(None).unwrap();
        assert!(app.document_save_pending(first.document));
        app.install_preview_document(Document::open(&paths[1]).unwrap(), OpenMode::Preview)
            .unwrap();
        assert!(app.editor_groups.membership_current(first));
        assert_eq!(
            app.editor_groups.group(first.group).unwrap().tabs().len(),
            2
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
        while app.saves_pending() {
            app.poll();
            assert!(
                std::time::Instant::now() < deadline,
                "Actual native save failed to settle"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let retained = app
            .documents
            .iter()
            .find(|doc| doc.id == first.document)
            .unwrap();
        assert_eq!(retained.text.to_string(), original);
        // Saving already-clean unchanged bytes settles as a real no-op.
        assert_eq!(retained.save_generation(), generation);
        assert!(!retained.dirty());
        assert_eq!(std::fs::read(&paths[0]).unwrap(), original.as_bytes());
        assert!(app.editor_groups.membership_current(first));
        assert_eq!(app.doc().path.as_ref(), Some(&paths[1]));
    }
}
