//! Measured viewports are display metadata, never document-hit authorization.
use super::*;
use crate::{document::EditorViewProof, editor_layout::Geometry, editor_presentation::PaneRects};
use anyhow::{Context as _, ensure};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Shape {
    sidebar: bool,
    terminal: bool,
    output: bool,
    breadcrumbs: bool,
    active: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    fn draw(app: &mut App, width: u16, height: u16) {
        Terminal::new(TestBackend::new(width, height))
            .unwrap()
            .draw(|frame| crate::ui::draw(frame, app))
            .unwrap();
    }
    #[test]
    fn measured_viewport_rejects_replaced_native_view_owner_without_losing_shared_history() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.install_open_document(Document::from_text("猫🙂\r\nsecond\r\n"))
            .unwrap();
        app.doc_mut().insert("dirty ", false);
        let id = app.doc().id;
        let view = app.panes[0].id;
        assert!(app.extension_reveal_viewport(id, view).is_none());
        draw(&mut app, 120, 32);
        let measured = app.extension_reveal_viewport(id, view).unwrap();
        assert!(measured.width > 0 && measured.height > 0);
        let old = app
            .extension_editor_inventory
            .id_for_view(id, view)
            .unwrap();
        app.doc_mut().remove_view(view);
        app.doc_mut().activate_view(view);
        app.refresh_extension_editor_inventory();
        assert_ne!(
            app.extension_editor_inventory
                .id_for_view(id, view)
                .as_deref(),
            Some(old.as_str())
        );
        assert!(app.extension_reveal_viewport(id, view).is_none());
        draw(&mut app, 120, 32);
        assert_eq!(app.extension_reveal_viewport(id, view), Some(measured));
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "猫🙂\r\nsecond\r\n");
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), "dirty 猫🙂\r\nsecond\r\n");
    }
    #[test]
    fn resize_retires_old_measured_dimensions_until_actual_repaint() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.install_open_document(Document::from_text("猫🙂\r\nsecond\r\n"))
            .unwrap();
        let id = app.doc().id;
        let view = app.panes[0].id;
        draw(&mut app, 120, 32);
        let large = app.extension_reveal_viewport(id, view).unwrap();
        app.event(Event::Resize(120, 18));
        assert!(app.extension_reveal_viewport(id, view).is_none());
        draw(&mut app, 120, 18);
        let small = app.extension_reveal_viewport(id, view).unwrap();
        assert!(small.height < large.height);
        assert_eq!(app.doc().text.to_string(), "猫🙂\r\nsecond\r\n");
        assert!(!app.doc().dirty());
    }
}
impl Shape {
    fn capture(app: &App) -> Self {
        Self {
            sidebar: app.sidebar,
            terminal: app.terminal_visible,
            output: app.extension_surfaces.output.is_some(),
            breadcrumbs: app.breadcrumbs_view().visible,
            active: app
                .breadcrumbs_view()
                .visible
                .then(|| app.editor_groups.active_group().map(|group| group.value()))
                .flatten(),
        }
    }
}
struct Viewport {
    editor: String,
    document: u64,
    proof: EditorViewProof,
    text: Rect,
    gutter: u16,
}
#[derive(Default)]
pub(super) struct Viewports {
    shape: Option<Shape>,
    geometry: Option<Geometry>,
    views: Vec<Viewport>,
}
impl Viewports {
    fn current(&self, app: &App) -> bool {
        self.shape == Some(Shape::capture(app))
            && self.geometry.as_ref().is_some_and(|geometry| {
                app.editor_layout.geometry_current(geometry)
                    && app
                        .editor_layout
                        .project(geometry.area(), app.editor_groups.active_group())
                        .is_ok_and(|current| current == *geometry)
            })
    }
    fn view<'a>(&'a self, app: &App, document: u64, view: u64) -> Option<&'a Viewport> {
        if !self.current(app) {
            return None;
        }
        let id = app.extension_editor_inventory.id_for_view(document, view)?;
        let doc = app.documents.iter().find(|doc| doc.id == document)?;
        self.views.iter().find(|viewport| {
            viewport.document == document
                && viewport.proof.view_id() == view
                && viewport.editor == id
                && doc.editor_view_current(&viewport.proof)
        })
    }
}
impl App {
    pub(super) fn capture_extension_editor_viewports(
        &mut self,
        geometry: Geometry,
        panes: &[Option<PaneRects>; 4],
    ) {
        self.refresh_extension_editor_inventory();
        let views = panes
            .iter()
            .flatten()
            .filter_map(|pane| {
                let membership = self
                    .editor_groups
                    .groups()
                    .iter()
                    .find(|group| group.id() == pane.group)?
                    .active()?;
                let doc = self
                    .documents
                    .iter()
                    .find(|doc| doc.id == membership.document())?;
                let view = pane.group.value();
                Some(Viewport {
                    editor: self.extension_editor_inventory.id_for_view(doc.id, view)?,
                    document: doc.id,
                    proof: doc.editor_view_identity_for(view)?,
                    text: pane.text,
                    gutter: pane.text.x.saturating_sub(pane.outer.x),
                })
            })
            .collect();
        self.extension_editor_viewports = Viewports {
            shape: Some(Shape::capture(self)),
            geometry: Some(geometry),
            views,
        };
        self.refresh_extension_visible_ranges();
    }
    pub(super) fn refresh_extension_visible_ranges(&mut self) {
        let ranges = self
            .panes
            .iter()
            .filter_map(|pane| {
                let viewport =
                    self.extension_editor_viewports
                        .view(self, pane.document, pane.id)?;
                let doc = self.documents.iter().find(|doc| doc.id == pane.document)?;
                let ranges = if viewport.text.width == 0 || viewport.text.height == 0 {
                    json!([])
                } else {
                    let first = doc.view_state(Some(pane.id)).top.min(doc.line_count() - 1);
                    let last = first
                        .saturating_add(usize::from(viewport.text.height) - 1)
                        .min(doc.line_count() - 1);
                    // Rope metadata resolves the complete line's UTF-16 length in
                    // logarithmic time, including very long non-ASCII lines.
                    json!([{"start":{"line":first,"character":0},
                    "end":{"line":last,"character":doc.line_slice(last).len_utf16_cu()}}])
                };
                Some((viewport.editor.clone(), ranges))
            })
            .collect::<Vec<_>>();
        self.extension_editor_inventory.set_visible_ranges(&ranges);
    }
    pub(super) fn extension_reveal_viewport(&self, document: u64, view: u64) -> Option<Rect> {
        let viewport = self.extension_editor_viewports.view(self, document, view)?;
        let doc = self.documents.iter().find(|doc| doc.id == document)?;
        let options = doc.editor_options_for(view)?;
        let digits = if options["lineNumbers"] == 0 {
            0
        } else {
            (doc.line_count().max(1).ilog10() + 1).max(3) as u16
        };
        if (digits + 2).min(viewport.text.width.saturating_add(viewport.gutter)) != viewport.gutter
            || viewport.text.width == 0
            || viewport.text.height == 0
        {
            return None;
        }
        Some(viewport.text)
    }
    pub(super) fn apply_extension_reveal(
        &mut self,
        capture: &crate::extensions::editor_state::Capture,
        range: crate::lsp::Range,
        kind: u8,
        area: Rect,
    ) -> Result<()> {
        let doc = self
            .documents
            .iter_mut()
            .find(|doc| doc.id == capture.document)
            .context("Reveal editor model retired")?;
        ensure!(
            kind <= 3
                && (range.start.line, range.start.character)
                    <= (range.end.line, range.end.character),
            "Invalid editor reveal range or type"
        );
        let clamp = |point: crate::lsp::Position| {
            let row = point.line.min(doc.line_count() - 1);
            let line = doc.line_slice(row);
            let column = point.character.min(line.len_utf16_cu());
            (row, doc.line_start(row) + line.utf16_cu_to_char(column))
        };
        let (first, start) = clamp(range.start);
        let (last, end) = clamp(range.end);
        let view = doc.view_state(Some(capture.view_id()));
        let height = usize::from(area.height);
        let width = usize::from(area.width);
        ensure!(height > 0 && width > 0, "Reveal editor viewport is empty");
        let outside = first < view.top || last >= view.top.saturating_add(height);
        let top = match kind {
            1 => first
                .saturating_add((last - first) / 2)
                .saturating_sub(height / 2),
            2 if outside => first
                .saturating_add((last - first) / 2)
                .saturating_sub(height / 2),
            3 => first,
            0 if last - first >= height || first < view.top => first,
            0 if last >= view.top.saturating_add(height) => {
                last.saturating_add(1).saturating_sub(height)
            }
            _ => view.top,
        };
        // Visual columns honor actual tab stops and grapheme widths, without
        // allocating a flattened line or letting huge prefixes block input.
        let visual = |row, offset| -> Result<usize> {
            let prefix = doc.text.slice(doc.line_start(row)..offset);
            ensure!(
                prefix.len_bytes() <= 256 * 1024,
                "Reveal horizontal prefix exceeds 256 KiB budget"
            );
            Ok(doc.display_width_slice(prefix))
        };
        let begin = visual(first, start)?;
        let finish = if first == last {
            visual(last, end)?
        } else {
            begin
        };
        let left = if begin < view.left || finish.saturating_sub(begin) >= width {
            begin
        } else if finish >= view.left.saturating_add(width) {
            finish.saturating_add(1).saturating_sub(width)
        } else {
            view.left
        };
        doc.reveal_editor_view(capture.view_proof(), top, left)
    }
}
