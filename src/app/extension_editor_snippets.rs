//! Public snippet insertion owns its exact native view and one retained receipt.
use super::*;
use crate::{
    document::Selection,
    extensions::editor_state::{Capture, Change},
    snippet::variables::{Cursor, Environment},
    snippet_clipboard::Worker,
};
use anyhow::{Context as _, bail, ensure};
#[derive(Default)]
pub(super) enum ClipboardState {
    #[default]
    Fresh,
    Reading {
        worker: Worker,
        fallback: String,
    },
    Ready(Option<String>),
}
fn view_selections(doc: &Document, view: u64) -> Vec<Selection> {
    let state = doc.view_state(Some(view));
    let mut selections = vec![Selection {
        cursor: state.cursor,
        anchor: state.anchor,
        desired_column: None,
    }];
    selections.extend(state.secondary.iter().map(|s| Selection {
        cursor: s.cursor,
        anchor: s.anchor,
        desired_column: None,
    }));
    selections
}
fn position(doc: &Document, point: crate::lsp::Position) -> usize {
    if point.line >= doc.line_count() {
        return doc.len();
    }
    let start = doc.line_start(point.line);
    let first = doc.text.char_to_utf16_cu(start);
    let last = doc.text.char_to_utf16_cu(doc.line_end(point.line));
    doc.text
        .utf16_cu_to_char(first + point.character.min(last - first))
}
impl App {
    pub(super) fn apply_extension_snippet(
        &mut self,
        host: &mut crate::extensions::Client,
        capture: &Capture,
        clipboard: &mut ClipboardState,
    ) -> Result<Option<Value>> {
        let Change::Snippet(snippet) = &capture.change else {
            bail!("Not a snippet service");
        };
        let editor = capture.editor.as_ref().context("Snippet editor missing")?;
        let receipt =
            |applied| Some(json!({"document":capture.document,"editor":editor,"applied":applied}));
        super::extension_services::Context::check_budget(self)?;
        let doc = self
            .documents
            .iter()
            .find(|doc| doc.id == capture.document)
            .context("Snippet document no longer visible")?;
        ensure!(
            host.editor_view_live(editor, doc),
            "Original snippet editor retired"
        );
        if !snippet.source_current
            || doc.is_read_only()
            || self.settings_error.is_some()
            || !capture.current(
                doc,
                &self.workspace.root,
                self.settings.extension_layers(),
                self.settings_profile_generation(),
            )
        {
            return Ok(receipt(false));
        }
        ensure!(
            self.prompt.is_none() && self.modal.is_none(),
            "Snippet cannot replace an active prompt or modal"
        );
        if snippet.template.uses_variable("CLIPBOARD") {
            if view_selections(doc, capture.view_id()) != snippet.original_selections {
                return Ok(receipt(false));
            }
            match clipboard {
                ClipboardState::Fresh => {
                    ensure!(
                        self.snippet_pending.is_none(),
                        "A native snippet is waiting for the clipboard"
                    );
                    ensure!(
                        self.clipboard.len() <= 1024 * 1024,
                        "Snippet clipboard fallback exceeds 1 MiB"
                    );
                    *clipboard = ClipboardState::Reading {
                        worker: Worker::start()?,
                        fallback: self.clipboard.clone(),
                    };
                    return Ok(None);
                }
                ClipboardState::Reading { worker, fallback } => {
                    let Some(result) = worker.try_finish() else {
                        return Ok(None);
                    };
                    *clipboard =
                        ClipboardState::Ready(result?.or_else(|| Some(std::mem::take(fallback))));
                }
                ClipboardState::Ready(_) => {}
            }
        }
        let pane = self
            .panes
            .iter()
            .position(|pane| pane.document == capture.document && pane.id == capture.view_id())
            .context("Snippet native pane retired")?;
        // Prepare group focus without exposing it before every expansion and
        // data-integrity check succeeds. Source insertion activates this view.
        let focus = if self.group_fallback {
            None
        } else {
            let group = self
                .editor_groups
                .groups()
                .iter()
                .find(|group| group.id().value() == capture.view_id())
                .context("Snippet group retired")?
                .id();
            let mut groups = self.editor_groups.clone();
            let change = groups.focus_group(group)?;
            Some((groups, change))
        };
        let other_bytes = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .filter(|doc| doc.id != capture.document)
            .map(|doc| doc.text.len_bytes())
            .sum::<usize>();
        let max_bytes = (4 * 1024 * 1024usize)
            .checked_sub(other_bytes)
            .context("Snippet mirror source byte budget exceeded")?;
        let other_model_selections = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .filter(|doc| doc.id != capture.document)
            .map(|doc| doc.secondary.len() + 1)
            .sum::<usize>();
        let other_pane_selections = self
            .panes
            .iter()
            .filter(|pane| pane.id != capture.view_id())
            .map(|pane| {
                self.documents
                    .iter()
                    .find(|doc| doc.id == pane.document)
                    .and_then(|doc| doc.editor_selection_count_for(pane.id))
                    .context("Snippet peer pane retired")
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .sum::<usize>();
        let max_selections = (16_384usize)
            .checked_sub(other_model_selections + other_pane_selections)
            .context("Snippet mirror selection budget exceeded")?
            / 2;
        let selections = snippet
            .ranges
            .iter()
            .map(|range| {
                let a = position(doc, range.start);
                let b = position(doc, range.end);
                Selection {
                    cursor: a.max(b),
                    anchor: Some(a.min(b)),
                    desired_column: None,
                }
            })
            .collect::<Vec<_>>();
        let count = selections.len();
        let language = doc
            .language_override()
            .unwrap_or_else(|| {
                doc.path
                    .as_deref()
                    .map_or("plaintext", crate::languages::language)
            })
            .to_owned();
        let environment = Environment {
            workspace: self.workspace.root.clone(),
            clipboard: if let ClipboardState::Ready(text) = clipboard {
                text.clone()
            } else {
                None
            },
            language,
            timestamp: chrono::Local::now().fixed_offset(),
        };
        let doc = self
            .documents
            .iter_mut()
            .find(|doc| doc.id == capture.document)
            .expect("validated snippet model");
        doc.insert_editor_snippet_resolved(
            capture.view_proof(),
            &snippet.template,
            selections,
            snippet.options,
            max_bytes,
            max_selections.min(4096),
            |document, selection, index, name, indent| {
                environment.resolve(
                    Cursor {
                        document,
                        selection,
                        index,
                        count,
                    },
                    name,
                    indent,
                )
            },
        )?;
        if let Some((groups, change)) = focus {
            self.editor_groups = groups;
            self.apply_group_change(change);
        } else {
            self.active_pane = pane;
            self.active = self
                .documents
                .iter()
                .position(|doc| doc.id == capture.document)
                .expect("retained snippet document");
        }
        self.focus = Focus::Editor;
        self.preview_edit_barrier();
        host.mark_api_editor_selections(editor);
        self.observe_navigation(navigation_history::Reason::EditorChange);
        self.observe_outline();
        self.observe_breadcrumbs();
        Ok(receipt(true))
    }
}
