use crate::{
    file_edit_review::{
        Change,
        dialog::{Dialog, Focus, Phase, plain},
    },
    theme::Colors,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

pub(super) fn draw(frame: &mut Frame, dialog: &Dialog, colors: Colors) {
    let screen = frame.area();
    let width = screen.width.saturating_sub(2).min(116);
    let height = screen
        .height
        .saturating_sub(2)
        .min(if dialog.phase() == Phase::Ask { 15 } else { 36 });
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(if dialog.phase() == Phase::Ask {
            " Additional file edits "
        } else {
            " Review proposed file edits "
        })
        .style(Style::default().bg(colors.panel).fg(colors.foreground));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if dialog.phase() == Phase::Ask {
        draw_ask(frame, dialog, colors, inner);
    } else {
        draw_preview(frame, dialog, colors, inner);
    }
}

fn contributors(dialog: &Dialog, max: usize) -> String {
    if dialog.contributors().is_empty() {
        return "File operation participants".into();
    }
    let mut result = dialog
        .contributors()
        .iter()
        .take(3)
        .map(|name| plain(name, max / 3))
        .collect::<Vec<_>>()
        .join(", ");
    if dialog.contributors().len() > 3 {
        result.push_str(&format!(" +{} more", dialog.contributors().len() - 3));
    }
    result
}
fn button(
    dialog: &Dialog,
    focus: Focus,
    text: String,
    colors: Colors,
    disabled: bool,
) -> Span<'static> {
    let focused = dialog.focus() == focus;
    let style = if disabled {
        Style::default().fg(colors.muted)
    } else if focused {
        Style::default()
            .fg(colors.accent)
            .add_modifier(Modifier::BOLD)
            .bg(colors.selection)
    } else {
        Style::default().fg(colors.foreground)
    };
    Span::styled(
        format!("{} {text} ", if focused { "›" } else { " " }),
        style,
    )
}
fn draw_ask(frame: &mut Frame, dialog: &Dialog, colors: Colors, area: Rect) {
    let mut lines = vec![
        Line::from(format!(" {}", contributors(dialog, area.width as usize))),
        Line::from(format!(
            " proposed {} additional edits for this file operation.",
            dialog.overview().rows().len()
        )),
        Line::from(""),
        Line::from(if dialog.requires_preview() {
            " Confirmation requested: review the proposed edits before applying."
        } else {
            " Apply all edits, review and select them, or skip the additional edits."
        }),
        Line::from(""),
        Line::from(vec![
            button(
                dialog,
                Focus::Apply,
                if dialog.requires_preview() {
                    "[Apply — review first]".into()
                } else {
                    "[Apply]".into()
                },
                colors,
                dialog.requires_preview(),
            ),
            button(dialog, Focus::Preview, "[Preview]".into(), colors, false),
            button(dialog, Focus::Skip, "[Skip]".into(), colors, false),
        ]),
        Line::from(""),
    ];
    if !dialog.requires_preview() {
        lines.push(Line::from(button(
            dialog,
            Focus::Remember,
            format!(
                "[{}] Remember this choice",
                if dialog.remember_checked() { "x" } else { " " }
            ),
            colors,
            false,
        )));
        lines.push(Line::styled(
            " Preview choice is remembered even if the later review is canceled.",
            Style::default().fg(colors.muted),
        ));
    } else {
        lines.push(Line::styled(
            " Required confirmation cannot be remembered.",
            Style::default().fg(colors.muted),
        ));
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        " Tab / arrows: focus   Enter / Space: choose   Esc: skip",
        Style::default().fg(colors.muted),
    ));
    frame.render_widget(Paragraph::new(lines), area);
}
fn kind(change: &Change) -> &'static str {
    match change {
        Change::Create => "Create",
        Change::Rename => "Rename",
        Change::Copy => "Copy",
        Change::Delete => "Delete",
        Change::Text => "Text",
        Change::Snippet => "Snippet",
        Change::EndOfLine => "EOL",
        Change::TextAndEndOfLine => "Text + EOL",
        Change::Notebook => "Notebook",
    }
}
fn draw_preview(frame: &mut Frame, dialog: &Dialog, colors: Colors, area: Rect) {
    let sections = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(3),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!(" {}", contributors(dialog, area.width as usize))),
            Line::styled(
                " Proposed changes only · baseline diff is not loaded",
                Style::default().fg(colors.muted),
            ),
        ]),
        sections[0],
    );
    let panes = if area.width >= 68 {
        Layout::horizontal([Constraint::Percentage(43), Constraint::Percentage(57)])
            .split(sections[1])
    } else {
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(sections[1])
    };
    let rowblock = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " Edits · {} / {} selected ",
            dialog.selected_count(),
            dialog.overview().rows().len()
        ))
        .border_style(Style::default().fg(if dialog.focus() == Focus::Rows {
            colors.accent
        } else {
            colors.muted
        }));
    let rows_area = rowblock.inner(panes[0]);
    frame.render_widget(rowblock, panes[0]);
    let count = rows_area.height as usize;
    let start = dialog
        .cursor()
        .saturating_sub(count / 2)
        .min(dialog.overview().rows().len().saturating_sub(count));
    let lines = dialog
        .overview()
        .rows()
        .iter()
        .enumerate()
        .skip(start)
        .take(count)
        .map(|(position, row)| {
            let marker = if dialog.row_selected(position) {
                "[x]"
            } else {
                "[ ]"
            };
            let confirmation = if row.needs_confirmation { " !" } else { "" };
            let resource = plain(&row.resource, rows_area.width as usize);
            let label = row
                .label
                .as_deref()
                .map(|label| format!(" · {}", plain(label, 60)))
                .unwrap_or_default();
            let text = format!(
                "{marker} {}. {}{confirmation} {resource}{label}",
                row.index + 1,
                kind(&row.change)
            );
            Line::styled(
                text,
                if position == dialog.cursor() {
                    Style::default()
                        .bg(colors.selection)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(colors.foreground)
                },
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), rows_area);
    let detailsblock = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Details · edit {} ", dialog.cursor() + 1))
        .border_style(Style::default().fg(if dialog.focus() == Focus::Details {
            colors.accent
        } else {
            colors.muted
        }));
    let details_area = detailsblock.inner(panes[1]);
    frame.render_widget(detailsblock, panes[1]);
    let lines = dialog
        .details()
        .iter()
        .skip(dialog.details_offset())
        .take(details_area.height as usize)
        .map(|text| Line::from(text.as_str()))
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), details_area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                button(
                    dialog,
                    Focus::Apply,
                    format!("[Apply {} selected]", dialog.selected_count()),
                    colors,
                    dialog.selected_count() == 0,
                ),
                button(dialog, Focus::Skip, "[Skip]".into(), colors, false),
            ]),
            Line::styled(
                " Tab: pane/action   ↑↓: row/scroll   Space: select   Enter: activate",
                Style::default().fg(colors.muted),
            ),
            Line::styled(
                " PgUp/PgDn, Home/End: navigate   Esc: skip · Complete original edits retained",
                Style::default().fg(colors.muted),
            ),
        ]),
        sections[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        file_edit_review::{Initial, Overview},
        theme::Theme,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::json;
    use std::sync::Arc;
    fn text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }
    fn dialog(initial: Initial) -> Dialog {
        let overview = Arc::new(Overview::prepare(Arc::new(json!({"edits":[
            {"newResource":"file:///created","metadata":{"label":"File"}},
            {"resource":"file:///snippet","textEdit":{"text":"\u{001b}[31m猫🙂","insertAsSnippet":true}},
            {"resource":"file:///eol","textEdit":{"text":"","eol":1}},
            {"resource":"notebook:///cells","cellEdit":{"editType":5}}
        ]}))).unwrap());
        Dialog::new(1, overview, vec!["Extension 猫🙂".into()], initial).unwrap()
    }
    #[test]
    fn initial_choice_renders_focus_and_visible_remember_checkbox() {
        let dialog = dialog(Initial::Ask {
            requires_preview: false,
        });
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| draw(frame, &dialog, Theme::default().colors))
            .unwrap();
        let output = text(&terminal);
        assert!(output.contains("[Apply]"));
        assert!(output.contains("[Preview]"));
        assert!(output.contains("[ ] Remember this choice"));
        assert!(output.contains("Extension"));
        assert!(output.contains("猫"));
        assert!(output.contains("🙂"));
    }
    #[test]
    fn preview_lists_rich_rows_and_proposed_details_without_baseline_claims() {
        let mut dialog = dialog(Initial::Apply { show_preview: true });
        dialog.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let mut terminal = Terminal::new(TestBackend::new(116, 36)).unwrap();
        terminal
            .draw(|frame| draw(frame, &dialog, Theme::default().colors))
            .unwrap();
        let output = text(&terminal);
        for expected in [
            "Create",
            "Snippet",
            "EOL",
            "Notebook",
            "insertAsSnippet",
            "baseline diff is not loaded",
            "[Apply 4 selected]",
        ] {
            assert!(output.contains(expected), "{expected}");
        }
        assert!(!output.contains('\u{001b}'));
    }
    #[test]
    fn small_terminal_and_long_inventory_render_bounded_without_panicking() {
        let original = Arc::new(json!({"edits": (0..1000).map(|index|
            json!({"resource":"file:///a","textEdit":{"text":format!("row {index}")}})).collect::<Vec<_>>()}));
        let overview = Arc::new(Overview::prepare(original).unwrap());
        let mut dialog =
            Dialog::new(1, overview, vec![], Initial::Apply { show_preview: true }).unwrap();
        dialog.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        assert_eq!(dialog.cursor(), 999);
        for (width, height) in [(1, 1), (20, 6), (42, 15), (100, 30)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| draw(frame, &dialog, Theme::default().colors))
                .unwrap();
            if width == 100 {
                assert!(text(&terminal).contains("row 999"));
            }
        }
    }
}
