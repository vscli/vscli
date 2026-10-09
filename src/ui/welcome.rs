//! Responsive native welcome page; image preparation lives in `brand`.
use super::*;
use std::path::PathBuf;

#[derive(Clone)]
pub(crate) enum Action {
    Command(&'static str),
    Recent(PathBuf),
}

pub(super) fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    let colors = app.theme.colors;
    let width = area.width.saturating_sub(4).min(88);
    let wide = width >= 70;
    let header = if area.height >= 28 {
        9
    } else if area.height >= 20 {
        5
    } else {
        2
    };
    let footer = if area.height >= 20 { 5 } else { 0 };
    let recent = if !wide && !app.recent_files.files.is_empty() && area.height >= 20 {
        2
    } else {
        0
    };
    let height = (header + 1 + 7 + recent + footer).min(area.height);
    let content = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    let logo = Rect::new(content.x, content.y, header * 2, header);
    if header >= 5 && app.modal.is_none() && app.prompt.is_none() {
        app.welcome_brand.render(frame, logo, colors.background);
    }
    let text_x = if header >= 5 {
        content.x + logo.width + 3
    } else {
        content.x
    };
    let text_width = content.right().saturating_sub(text_x);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                "VSCLI",
                Style::default()
                    .fg(colors.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Line::styled(
                "Your terminal. Your code.",
                Style::default().fg(colors.foreground),
            ),
            Line::styled(
                "Open a file and make it yours.",
                Style::default().fg(colors.muted),
            ),
        ]),
        Rect::new(
            text_x,
            content.y + u16::from(header >= 5),
            text_width,
            header,
        ),
    );
    let body_y = content.y + header + 1;
    let body_width = if wide { width / 2 - 2 } else { width };
    let context = app.context();
    for (i, (label, command)) in [
        ("New File", "workbench.action.files.newUntitledFile"),
        ("Open File", "workbench.action.files.openFile"),
        ("Open Recent File", "workbench.action.openRecent"),
        ("Command Palette", "workbench.action.showCommands"),
        ("Settings", "workbench.action.openSettings"),
        ("Extensions", "workbench.view.extensions"),
        ("Keyboard Inspector", "vscli.keyboardInspector"),
    ]
    .into_iter()
    .enumerate()
    {
        let row = Rect::new(content.x, body_y + i as u16, body_width, 1);
        if row.y >= content.bottom() {
            break;
        }
        let hint = app.keymap.shortcut_in_context(command, &context);
        let hint = if hint.is_empty() {
            let palette = app
                .keymap
                .shortcut_in_context("workbench.action.showCommands", &context);
            if palette.is_empty() {
                "Unbound".to_owned()
            } else {
                format!("via {palette}")
            }
        } else {
            hint
        };
        let padding = usize::from(body_width).saturating_sub(label.len() + hint.len() + 2);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(label, Style::default().fg(colors.foreground)),
                Span::raw(" ".repeat(padding.max(1))),
                Span::styled(hint, Style::default().fg(colors.accent)),
            ])),
            row,
        );
        app.welcome_actions.push((row, Action::Command(command)));
    }
    let recent_x = if wide {
        content.x + width / 2 + 2
    } else {
        content.x
    };
    let recent_y = if wide { body_y } else { body_y + 7 };
    if wide || recent > 0 {
        let recent_width = content.right().saturating_sub(recent_x);
        frame.render_widget(
            Paragraph::new("Recent files").style(Style::default().fg(colors.muted)),
            Rect::new(recent_x, recent_y, recent_width, 1),
        );
        if app.recent_files.files.is_empty() {
            frame.render_widget(
                Paragraph::new("Your files will appear here.")
                    .style(Style::default().fg(colors.muted)),
                Rect::new(recent_x, recent_y + 2, recent_width, 1),
            );
        }
        for (i, entry) in app
            .recent_files
            .files
            .iter()
            .take(if wide { 3 } else { 1 })
            .enumerate()
        {
            let y = recent_y + 1 + i as u16 * 2;
            let row = Rect::new(recent_x, y, recent_width, 1);
            let name = clean(&entry.path.file_name().unwrap_or_default().to_string_lossy());
            frame.render_widget(
                Paragraph::new(name).style(Style::default().fg(colors.accent)),
                row,
            );
            app.welcome_actions
                .push((row, Action::Recent(entry.path.clone())));
            if wide && let Some(parent) = entry.path.parent() {
                frame.render_widget(
                    Paragraph::new(clean(&parent.to_string_lossy()))
                        .style(Style::default().fg(colors.muted)),
                    Rect::new(recent_x, y + 1, recent_width, 1),
                );
            }
        }
    }
    if footer > 0 {
        let y = body_y + 7 + recent;
        if y + 5 <= area.bottom() {
            let user = app.user_settings_path().map_or_else(
                || "Unavailable · use --settings PATH".into(),
                |path| clean(&path.to_string_lossy()),
            );
            frame.render_widget(
                Paragraph::new("User settings · JSON").style(Style::default().fg(colors.muted)),
                Rect::new(content.x, y + 1, width, 1),
            );
            frame.render_widget(
                Paragraph::new(user)
                    .style(Style::default().fg(colors.foreground))
                    .wrap(Wrap { trim: false }),
                Rect::new(content.x, y + 2, width, 2),
            );
            frame.render_widget(
                Paragraph::new("Workspace overrides: .vscode/settings.json")
                    .style(Style::default().fg(colors.muted)),
                Rect::new(content.x, y + 4, width, 1),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend};
    fn click(app: &mut App, rect: Rect) {
        app.event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        }));
    }
    #[test]
    fn welcome_actions_preserve_hidden_dirty_buffers_and_resize_invalidates_hitboxes() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), crate::keys::Profile::Linux);
        let mut hidden = Document::from_text("saved");
        hidden.insert("dirty ", false);
        let id = hidden.id;
        app.hidden_documents.push(hidden);
        let settings = root.path().join("actual-profile/settings.json");
        app.configure_settings(Some(settings.clone())).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(150, 38)).unwrap();
        terminal.draw(|f| super::super::draw(f, &mut app)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains(&settings.to_string_lossy().to_string()));
        assert!(text.contains("ctrl+,"));
        assert!(app.documents.is_empty());
        let inspector = app
            .welcome_actions
            .iter()
            .find(|(_, a)| matches!(a, Action::Command("vscli.keyboardInspector")))
            .unwrap()
            .0;
        click(&mut app, inspector);
        assert!(matches!(app.modal, Some(Modal::Inspector)));
        assert_eq!(app.hidden_documents[0].id, id);
        assert_eq!(app.hidden_documents[0].text.to_string(), "dirty saved");
        app.modal = None;
        terminal.draw(|f| super::super::draw(f, &mut app)).unwrap();
        let new = app
            .welcome_actions
            .iter()
            .find(|(_, a)| matches!(a, Action::Command("workbench.action.files.newUntitledFile")))
            .unwrap()
            .0;
        app.event(Event::Resize(70, 20));
        click(&mut app, new);
        assert!(app.documents.is_empty());
        terminal.draw(|f| super::super::draw(f, &mut app)).unwrap();
        click(&mut app, new);
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.hidden_documents[0].id, id);
        assert!(app.documents[0].is_empty());
        assert!(!settings.exists());
    }
}
