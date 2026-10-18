//! Editor EOL preparation reuses bounded workspace ropes and native history.
use super::*;
use crate::{document::EditorViewProof, lsp::TextEdit};

pub(crate) struct EditorEolPrepared {
    source: WorkspaceSource,
    eol: WorkspaceTextPrepared,
    text: WorkspaceTextPrepared,
}
impl EditorEolPrepared {
    pub(crate) fn prepare(
        source: WorkspaceSource,
        mut edits: Vec<(TextEdit, bool)>,
        eol: &str,
        available: usize,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<Self> {
        ensure!(
            edits.len() <= MAX_WORKSPACE_EDITS,
            "Editor edit count exceeded"
        );
        let converted = source
            .clone()
            .prepare(Vec::new(), cancel, deadline)?
            .with_eol(eol, cancel, deadline)?;
        ensure!(
            converted.bytes() <= available,
            "Editor EOL exceeds mirror byte budget"
        );
        let mut intermediate = source.clone();
        intermediate.text = converted.text.clone();
        intermediate.eol = converted.eol.clone();
        edits.sort_by_key(|(edit, _)| {
            (
                edit.range.start.line,
                edit.range.start.character,
                edit.range.end.line,
                edit.range.end.character,
            )
        });
        let mut changes = Vec::with_capacity(edits.len());
        let mut markers = std::collections::BTreeMap::new();
        let mut bytes = 0usize;
        for (edit, force) in edits {
            guard(cancel, deadline)?;
            let start =
                intermediate.utf16_offset(edit.range.start.line, edit.range.start.character)?;
            let end = intermediate.utf16_offset(edit.range.end.line, edit.range.end.character)?;
            let replacement = normalize_replacement(
                &edit.new_text,
                eol,
                available.saturating_sub(bytes),
                cancel,
                deadline,
            )?;
            bytes += replacement.len();
            markers.insert((start, end), force);
            changes.push((start..end, replacement));
        }
        let mut text = intermediate.prepare(changes, cancel, deadline)?;
        ensure!(
            text.bytes() <= available,
            "Editor result exceeds mirror byte budget"
        );
        for change in &mut text.changes {
            change.decoration.force_move = markers[&(change.range.start, change.range.end)];
        }
        ensure!(
            converted.changes.len() + text.changes.len() + usize::from(converted.changed())
                <= MAX_WORKSPACE_EDITS,
            "Editor EOL and text exceed combined history budget"
        );
        Ok(Self {
            source,
            eol: converted,
            text,
        })
    }
    pub(crate) fn current(&self, document: &Document) -> bool {
        self.source.current(document)
    }
}

/// Both stages and every fallible reservation are ready before publication.
pub(crate) struct EditorEolCommit {
    source: WorkspaceSource,
    proof: EditorViewProof,
    original_head: bool,
    close_group: bool,
    intermediate: Option<Document>,
    final_document: Document,
}
impl EditorEolCommit {
    pub(crate) fn intermediate(&self) -> Option<&Document> {
        self.intermediate.as_ref()
    }
    pub(crate) fn final_document(&self) -> &Document {
        &self.final_document
    }
    pub(crate) fn current(&self, document: &Document) -> bool {
        self.source.current(document) && document.editor_view_current(&self.proof)
    }
    pub(crate) fn publish(mut self, document: &mut Document) {
        debug_assert!(self.current(document));
        let mut history = std::mem::take(&mut self.final_document.undo);
        let changed = history.len() > usize::from(self.original_head);
        if self.original_head {
            *document
                .undo
                .last_mut()
                .expect("retained editor history head") = history.remove(0);
        }
        document.undo.extend(history);
        if document.undo.len() > HISTORY_LIMIT {
            document.undo.drain(..document.undo.len() - HISTORY_LIMIT);
        }
        if changed {
            document.redo.clear();
        } else {
            if self.close_group {
                document.break_group();
            }
            return;
        }
        ResourceBody {
            body: self.final_document.workspace_body(),
            before: None,
            metadata: 0,
        }
        .publish(document);
    }
}

impl Document {
    pub(crate) fn prepare_editor_eol_commit(
        &mut self,
        prepared: EditorEolPrepared,
        proof: &EditorViewProof,
        before: bool,
        after: bool,
    ) -> Result<EditorEolCommit> {
        ensure!(
            prepared.current(self) && self.editor_view_current(proof),
            "Editor EOL source retired"
        );
        let count = prepared.eol.changes.len()
            + prepared.text.changes.len()
            + usize::from(prepared.eol.changed());
        let metadata = self.workspace_metadata(count, MAX_WORKSPACE_METADATA)?;
        let head_bytes = self
            .undo
            .last()
            .map(snapshot_bytes)
            .transpose()?
            .unwrap_or(0);
        ensure!(
            metadata.saturating_add(head_bytes.saturating_mul(3)) <= MAX_WORKSPACE_METADATA,
            "Editor EOL history metadata exceeds 8 MiB"
        );
        if (prepared.eol.changed() || (!before && prepared.text.changed()))
            && self.undo.last().is_some_and(|s| {
                !s.api_closed && self.api_history_current(s, &proof.owner, proof.id)
            })
        {
            ensure!(
                self.api_group_changes(self.undo.last().and_then(|s| s.api_history.as_ref()))
                    .saturating_add(count)
                    <= MAX_WORKSPACE_EDITS,
                "Editor EOL Undo group exceeds change budget"
            );
        }
        self.undo
            .try_reserve(2)
            .context("Editor EOL history reservation refused")?;
        let mut fork = self.workspace_fork();
        let original_head = !self.undo.is_empty();
        fork.undo.extend(self.undo.last().cloned());
        let eol_changed = prepared.eol.changed();
        // Upstream pushes EOL before applying the requested before/text/after stops.
        fork.apply_prepared_editor_stage(prepared.eol, proof, false, false)?;
        let intermediate = eol_changed.then(|| fork.workspace_fork());
        fork.apply_prepared_editor_stage(prepared.text, proof, before, after)?;
        Ok(EditorEolCommit {
            source: prepared.source,
            proof: proof.clone(),
            original_head,
            close_group: before || after,
            intermediate,
            final_document: fork,
        })
    }
    fn apply_prepared_editor_stage(
        &mut self,
        mut prepared: WorkspaceTextPrepared,
        proof: &EditorViewProof,
        before: bool,
        after: bool,
    ) -> Result<()> {
        if !prepared.changed() {
            if before || after {
                self.break_group();
            }
            return Ok(());
        }
        let displayed = self.active_view;
        self.display_view(proof.id);
        let join = !before
            && self.undo.last().is_some_and(|s| {
                !s.api_closed && self.api_history_current(s, &proof.owner, proof.id)
            });
        let group = if join {
            self.undo
                .last()
                .and_then(|s| s.api_history.clone())
                .unwrap_or_else(|| Arc::new(()))
        } else {
            Arc::new(())
        };
        // The prepared second stage belongs to this private first-stage fork.
        // Only the outer original source/view lease can authorize publication.
        prepared.source = self.capture_workspace_source()?;
        let eol = prepared.eol.clone();
        let mut body = self.prepare_resource_body(prepared, eol, MAX_WORKSPACE_METADATA)?;
        let snapshot = body
            .before
            .as_mut()
            .context("Missing editor history stage")?;
        snapshot.api_history = Some(group.clone());
        snapshot.api_open = !after;
        snapshot.api_closed = after;
        if join {
            if let Some(previous) = self.undo.last_mut() {
                previous.api_history = Some(group);
            }
        } else {
            self.break_group();
        }
        body.publish(self);
        self.display_view(displayed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::{Position, Range as LspRange};
    fn edit(line: usize, character: usize, text: &str) -> (TextEdit, bool) {
        (
            TextEdit {
                range: LspRange {
                    start: Position { line, character },
                    end: Position { line, character },
                },
                new_text: text.into(),
            },
            true,
        )
    }
    fn prepare(
        doc: &Document,
        edits: Vec<(TextEdit, bool)>,
        eol: &str,
    ) -> Result<EditorEolPrepared> {
        EditorEolPrepared::prepare(
            doc.capture_workspace_source()?,
            edits,
            eol,
            4 * 1024 * 1024,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(5),
        )
    }
    #[test]
    fn mixed_editor_eol_precedes_text_and_default_undo_stops_keep_two_real_history_steps() {
        let original = "λ🙂 a\nb\n";
        let mut doc = Document::from_text(original);
        let id = doc.id;
        let proof = doc.editor_view_identity();
        let prepared = prepare(&doc, vec![edit(0, 3, "\nZ")], "\r\n").unwrap();
        let commit = doc
            .prepare_editor_eol_commit(prepared, &proof, true, true)
            .unwrap();
        assert_eq!(
            commit.intermediate().unwrap().text.to_string(),
            "λ🙂 a\r\nb\r\n"
        );
        assert_eq!(
            doc.text.to_string(),
            original,
            "preparation must not publish EOL or text"
        );
        commit.publish(&mut doc);
        assert_eq!(doc.text.to_string(), "λ🙂\r\nZ a\r\nb\r\n");
        doc.undo();
        assert_eq!(doc.text.to_string(), "λ🙂 a\r\nb\r\n");
        assert_eq!(doc.eol, "\r\n");
        doc.undo();
        assert_eq!(doc.text.to_string(), original);
        assert_eq!(doc.eol, "\n");
        doc.redo();
        assert_eq!(doc.text.to_string(), "λ🙂 a\r\nb\r\n");
        doc.redo();
        assert_eq!(doc.text.to_string(), "λ🙂\r\nZ a\r\nb\r\n");
        assert_eq!(doc.id, id);
    }
    #[test]
    fn eol_without_line_breaks_versions_future_enter_and_same_eol_noop_retains_redo() {
        let mut doc = Document::from_text("base");
        doc.eol = "\n".into();
        let proof = doc.editor_view_identity();
        let prepared = prepare(&doc, Vec::new(), "\r\n").unwrap();
        let before = doc.text_epoch();
        doc.prepare_editor_eol_commit(prepared, &proof, true, true)
            .unwrap()
            .publish(&mut doc);
        assert!(doc.text_epoch() > before);
        assert_eq!(doc.eol, "\r\n");
        doc.undo();
        assert_eq!(doc.eol, "\n");
        let source = doc.capture_workspace_source().unwrap();
        let prepared = prepare(&doc, Vec::new(), "\n").unwrap();
        doc.prepare_editor_eol_commit(prepared, &proof, false, false)
            .unwrap()
            .publish(&mut doc);
        assert!(source.current(&doc));
        doc.redo();
        assert_eq!(doc.eol, "\r\n");
        doc.move_to(4, false);
        doc.newline();
        assert_eq!(doc.text.to_string(), "base\r\n");
    }
    #[test]
    fn malformed_late_editor_text_and_conversion_bounds_never_publish_an_eol_prefix() {
        let doc = Document::from_text("λ🙂 original\n");
        let source = doc.capture_workspace_source().unwrap();
        assert!(prepare(&doc, vec![edit(0, 2, "lost")], "\r\n").is_err());
        assert!(source.current(&doc));
        assert_eq!(doc.eol, "\n");
        let doc = Document::from_text(&"line\n".repeat(4096));
        let source = doc.capture_workspace_source().unwrap();
        assert!(prepare(&doc, Vec::new(), "\r\n").is_err());
        assert!(source.current(&doc));
    }
    #[test]
    fn editor_eol_all_stop_pairs_keep_eol_before_requested_boundary_and_native_typing() {
        for before in [false, true] {
            for after in [false, true] {
                let mut doc = Document::from_text("base\n");
                doc.move_to(4, false);
                doc.insert("P", true);
                let proof = doc.editor_view_identity();
                let prepared = prepare(&doc, vec![edit(0, 5, "A")], "\r\n").unwrap();
                doc.prepare_editor_eol_commit(prepared, &proof, before, after)
                    .unwrap()
                    .publish(&mut doc);
                doc.insert("B", true);
                assert_eq!(doc.text.to_string(), "basePAB\r\n");
                doc.undo();
                let expected = match (before, after) {
                    (_, true) => "basePA\r\n",
                    (true, false) => "baseP\r\n",
                    (false, false) => "base\n",
                };
                assert_eq!(
                    doc.text.to_string(),
                    expected,
                    "before={before},after={after}"
                );
                doc.redo();
                assert_eq!(doc.text.to_string(), "basePAB\r\n");
            }
        }
    }
    #[test]
    fn prepared_editor_eol_rejects_edit_undo_aba_and_preserves_original_redo() {
        let mut doc = Document::from_text("λ🙂\n");
        let proof = doc.editor_view_identity();
        let prepared = prepare(&doc, vec![edit(0, 0, "lost")], "\r\n").unwrap();
        doc.insert("retained", false);
        doc.undo();
        assert!(
            doc.prepare_editor_eol_commit(prepared, &proof, true, true)
                .is_err()
        );
        assert_eq!(doc.eol, "\n");
        doc.redo();
        assert_eq!(doc.text.to_string(), "retainedλ🙂\n");
    }

    #[test]
    fn inactive_eol_conversion_keeps_shared_line_end_carets_and_reversed_selection_through_history()
    {
        let mut doc = Document::from_text("λ🙂\nsecond\n");
        doc.activate_view(1);
        doc.set_selections(vec![Selection {
            cursor: 2,
            anchor: Some(6),
            desired_column: None,
        }]);
        let proof = doc.editor_view_identity();
        doc.activate_view(2);
        doc.move_to(2, false);
        let prepared = prepare(&doc, Vec::new(), "\r\n").unwrap();
        doc.prepare_editor_eol_commit(prepared, &proof, true, true)
            .unwrap()
            .publish(&mut doc);
        assert_eq!(doc.active_view, 2);
        assert_eq!(doc.cursor, 2);
        assert_eq!(doc.view_state(Some(1)).cursor, 2);
        assert_eq!(doc.view_state(Some(1)).anchor, Some(7));
        doc.undo();
        assert_eq!(doc.eol, "\n");
        assert_eq!(doc.cursor, 2);
        assert_eq!(doc.view_state(Some(1)).cursor, 2);
        assert_eq!(doc.view_state(Some(1)).anchor, Some(6));
        doc.redo();
        assert_eq!(doc.eol, "\r\n");
        assert_eq!(doc.cursor, 2);
        assert_eq!(doc.view_state(Some(1)).anchor, Some(7));
    }
}
