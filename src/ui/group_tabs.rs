//! Bounded, observational editor-group tab presentation.
use super::*;
use crate::{app::TabHit, editor_groups::Membership};

const LABEL_BYTES: usize = 256;
const TAB_CELLS: usize = 28;

// Bound segmentation even for a hostile combining-character filename. If the
// byte prefix is incomplete, discard its last cluster rather than split it.
pub(super) fn bounded_label(name: &str) -> String {
    let mut end = name.len().min(LABEL_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    let prefix = &name[..end];
    let clipped = end < name.len();
    let last = clipped
        .then(|| prefix.grapheme_indices(true).next_back().map(|(i, _)| i))
        .flatten();
    let prefix = &prefix[..last.unwrap_or(prefix.len())];
    let mut result = String::new();
    for cluster in prefix.graphemes(true) {
        result.push_str(if cluster.chars().any(char::is_control) {
            "�"
        } else {
            cluster
        });
    }
    if clipped {
        result.push('…');
    }
    result
}

fn document_label(doc: &Document) -> String {
    doc.path
        .as_ref()
        .and_then(|path| path.file_name())
        .map_or_else(
            || "Untitled".into(),
            |name| bounded_label(&name.to_string_lossy()),
        )
}

struct Label {
    member: Membership,
    text: String,
    dirty: bool,
    active: bool,
}
impl Label {
    fn width(&self) -> usize {
        (display_width(&self.text) + if self.dirty { 4 } else { 2 }).min(TAB_CELLS)
    }
    fn fit(&self, width: usize) -> String {
        if width <= 2 {
            let text = breadcrumb_label(&self.text, width);
            return if text.is_empty() && width > 0 {
                "…".into()
            } else {
                text
            };
        }
        let dirty = self.dirty && width >= 5;
        let budget = width - 2 - if dirty { 2 } else { 0 };
        let mut text = breadcrumb_label(&self.text, budget);
        if display_width(&self.text) > budget && budget > 0 {
            text = breadcrumb_label(&self.text, budget - 1);
            text.push('…');
        }
        format!(" {text}{} ", if dirty { " ●" } else { "" })
    }
}

pub(super) fn draw(frame: &mut Frame, app: &mut App, area: Rect, pane: u64, index: usize) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let colors = app.theme.colors;
    let proof = app.editor_groups().proof();
    let group = app
        .editor_groups()
        .groups()
        .iter()
        .find(|group| group.id().value() == pane);
    let Some(group) = group else {
        // Over-cap recovery remains usable without inventing engine memberships.
        frame.render_widget(
            Paragraph::new(format!(" {} · {}", index + 1, document_label(app.doc())))
                .style(Style::default().bg(colors.panel).fg(colors.foreground)),
            area,
        );
        return;
    };
    let active = group.active().map(|tab| tab.id());
    let labels: Vec<_> = group
        .tabs()
        .iter()
        .filter_map(|tab| {
            let doc = app.documents.iter().find(|doc| doc.id == tab.document())?;
            Some(Label {
                member: Membership {
                    group: group.id(),
                    tab: tab.id(),
                    document: tab.document(),
                },
                text: document_label(doc),
                dirty: doc.dirty(),
                active: Some(tab.id()) == active,
            })
        })
        .collect();
    let Some(selected) = labels.iter().position(|label| label.active) else {
        return;
    };
    let prefix = if area.width >= 12 {
        format!("{} ·", index + 1)
    } else {
        String::new()
    };
    let prefix_width = display_width(&prefix);
    let budget = area.width as usize - prefix_width;
    let mut start = selected;
    let mut used = labels[selected].width().min(budget);
    while start > 0 && used + labels[start - 1].width() <= budget {
        start -= 1;
        used += labels[start].width();
    }
    let mut spans = vec![Span::styled(prefix, Style::default().fg(colors.muted))];
    let mut column = prefix_width;
    let mut hits = Vec::new();
    for label in labels.iter().skip(start) {
        let remaining = area.width as usize - column;
        if remaining == 0 {
            break;
        }
        // Partial trailing tabs are useful only if they have a visible label.
        let width = label.width().min(remaining);
        if !label.active && width < 3 {
            break;
        }
        let text = label.fit(width);
        let rendered = display_width(&text);
        if rendered == 0 {
            continue;
        }
        spans.push(Span::styled(
            text,
            Style::default()
                .fg(if label.active {
                    colors.foreground
                } else {
                    colors.muted
                })
                .bg(if label.active {
                    colors.selection
                } else {
                    colors.panel
                })
                .add_modifier(if label.active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ));
        hits.push(TabHit {
            area: Rect::new(area.x + column as u16, area.y, rendered as u16, 1),
            membership: label.member,
            proof,
        });
        column += rendered;
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(colors.panel)),
        area,
    );
    app.tab_hits.extend(hits);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::Value;
    #[test]
    fn resize_retires_old_tab_geometry_before_another_frame_is_drawn() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), crate::keys::Profile::Linux);
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| super::super::draw(frame, &mut app))
            .unwrap();
        assert!(!app.tab_hits.is_empty());
        let membership = app.editor_groups().active_membership();
        app.event(crossterm::event::Event::Resize(80, 24));
        assert!(app.tab_hits.is_empty());
        assert_eq!(app.editor_groups().active_membership(), membership);
    }

    #[test]
    fn bounded_filename_keeps_complete_clusters_and_removes_controls() {
        assert_eq!(
            bounded_label("猫e\u{301}\n👩\u{200d}💻"),
            "猫e\u{301}�👩\u{200d}💻"
        );
        let hostile = format!("a{}tail", "\u{301}".repeat(100_000));
        assert_eq!(bounded_label(&hostile), "…");
        let boundary = format!("{}👩\u{200d}💻", "x".repeat(252));
        assert_eq!(bounded_label(&boundary), format!("{}…", "x".repeat(252)));
    }

    #[test]
    fn both_group_strips_preserve_shared_views_and_undo_ownership() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), crate::keys::Profile::Linux);
        app.sidebar = false;
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.doc_mut().insert("猫🙂\r\nbody", false);
        app.doc_mut().move_to(1, false);
        app.execute("workbench.action.splitEditor", Value::Null);
        app.doc_mut().move_to(6, false);
        let left = app.panes[0].id;
        let right = app.panes[1].id;
        let proof = app.editor_groups().proof();
        let id = app.doc().id;
        let epoch = app.doc().text_epoch();
        let revision = app.doc().revision;
        let saved = app.doc().save_generation();
        let text = app.doc().text.to_string();
        for (width, height) in [(110, 32), (20, 6), (40, 10)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| super::super::draw(frame, &mut app))
                .unwrap();
            assert_eq!(app.editor_groups().proof(), proof);
            assert_eq!(app.active_pane, 1);
            assert_eq!(app.doc().id, id);
            assert_eq!(app.doc().cursor, 6);
            assert_eq!(app.doc().view_state(Some(left)).cursor, 1);
            assert_eq!(app.doc().view_state(Some(right)).cursor, 6);
            assert_eq!(app.doc().revision, revision);
            assert_eq!(app.doc().text_epoch(), epoch);
            assert_eq!(app.doc().save_generation(), saved);
            assert_eq!(app.doc().text.to_string(), text);
            assert_eq!(app.tab_hits.len(), 2);
            assert_ne!(
                app.tab_hits[0].membership.tab,
                app.tab_hits[1].membership.tab
            );
            for hit in &app.tab_hits {
                assert_eq!(hit.membership.document, id);
                assert!(app.editor_groups().proof_current(&hit.proof));
                assert!(app.editor_groups().membership_current(hit.membership));
                let bounds = Rect::new(0, 0, width, height);
                assert_eq!(hit.area.intersection(bounds), hit.area);
                assert!(
                    app.pane_areas
                        .iter()
                        .all(|editor| editor.intersection(hit.area).area() == 0)
                );
            }
        }
        app.doc_mut().undo();
        assert!(app.doc().is_empty());
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), text);
        assert!(app.doc().dirty());
        assert!(!root.path().join("Untitled").exists());
    }

    #[test]
    fn crowded_strip_keeps_selected_tab_visible_even_at_one_cell() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), crate::keys::Profile::Linux);
        for i in 0..12 {
            app.execute("workbench.action.files.newUntitledFile", Value::Null);
            app.doc_mut().path = Some(
                root.path()
                    .join(format!("猫🙂e\u{301}-{i}-{}.cpp", "long".repeat(30))),
            );
        }
        app.doc_mut().insert("dirty\r\n", false);
        app.sync_pane();
        let selected = app.editor_groups().active_membership().unwrap();
        let proof = app.editor_groups().proof();
        let pane = selected.group.value();
        for width in [1, 2, 3, 8, 12, 20, 40, 80] {
            app.tab_hits.clear();
            let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
            terminal
                .draw(|frame| draw(frame, &mut app, Rect::new(0, 0, width, 1), pane, 0))
                .unwrap();
            let hit = app
                .tab_hits
                .iter()
                .find(|hit| hit.membership == selected)
                .unwrap();
            assert!(hit.area.width > 0);
            assert_eq!(app.editor_groups().proof(), proof);
            let bounds = Rect::new(0, 0, width, 1);
            for hit in &app.tab_hits {
                assert_eq!(hit.area.intersection(bounds), hit.area);
                assert!(app.editor_groups().proof_current(&hit.proof));
            }
            for pair in app.tab_hits.windows(2) {
                assert_eq!(pair[0].area.intersection(pair[1].area).area(), 0);
            }
            let cells = &terminal.backend().buffer().content;
            assert!(
                cells
                    .iter()
                    .any(|cell| cell.bg == app.theme.colors.selection)
            );
            if width >= 8 {
                assert!(cells.iter().any(|cell| cell.symbol() == "●"));
            }
        }
    }

    #[test]
    fn hidden_frame_surfaces_clear_previous_tab_hit_rectangles() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), crate::keys::Profile::Linux);
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| super::super::draw(frame, &mut app))
            .unwrap();
        assert!(!app.tab_hits.is_empty());
        app.modal = Some(Modal::Inspector);
        terminal
            .draw(|frame| super::super::draw(frame, &mut app))
            .unwrap();
        assert!(app.tab_hits.is_empty());
        app.modal = None;
        terminal
            .draw(|frame| super::super::draw(frame, &mut app))
            .unwrap();
        assert!(!app.tab_hits.is_empty());
        let mut tiny = Terminal::new(TestBackend::new(1, 1)).unwrap();
        tiny.draw(|frame| super::super::draw(frame, &mut app))
            .unwrap();
        assert!(app.tab_hits.is_empty());
    }
}
