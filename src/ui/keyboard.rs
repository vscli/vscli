//! A reference keyboard for received terminal events, never inferred held state.
use super::*;
use crate::keys::{Profile, Resolution};
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, ModifierKeyCode,
};
use ratatui::{layout::Alignment, widgets::BorderType};

struct Cap {
    label: &'static str,
    code: KeyCode,
    alternate: Option<char>,
    width: u16,
}
fn cap(label: &'static str, code: KeyCode, width: u16) -> Cap {
    Cap {
        label,
        code,
        alternate: None,
        width,
    }
}
fn chars(labels: &'static str) -> Vec<Cap> {
    labels
        .chars()
        .map(|c| Cap {
            label: match c {
                'Q' => "Q",
                'W' => "W",
                'E' => "E",
                'R' => "R",
                'T' => "T",
                'Y' => "Y",
                'U' => "U",
                'I' => "I",
                'O' => "O",
                'P' => "P",
                'A' => "A",
                'S' => "S",
                'D' => "D",
                'F' => "F",
                'G' => "G",
                'H' => "H",
                'J' => "J",
                'K' => "K",
                'L' => "L",
                'Z' => "Z",
                'X' => "X",
                'C' => "C",
                'V' => "V",
                'B' => "B",
                'N' => "N",
                'M' => "M",
                _ => "?",
            },
            code: KeyCode::Char(c.to_ascii_lowercase()),
            alternate: None,
            width: 5,
        })
        .collect()
}
fn punctuation(label: &'static str, plain: char, shifted: char, width: u16) -> Cap {
    Cap {
        label,
        code: KeyCode::Char(plain),
        alternate: Some(shifted),
        width,
    }
}
fn selected(cap: &Cap, event: Option<KeyEvent>) -> bool {
    event.is_some_and(|event| match (cap.code, event.code) {
        (KeyCode::Char(expected), KeyCode::Char(actual)) => {
            expected.eq_ignore_ascii_case(&actual) || cap.alternate == Some(actual)
        }
        (KeyCode::Tab, KeyCode::BackTab) => true,
        _ => cap.code == event.code,
    })
}
fn key_name(code: KeyCode) -> String {
    match code {
        KeyCode::Char(' ') => "Space".into(),
        KeyCode::Char(c) => clean(&c.to_string()),
        KeyCode::Enter => "Enter".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => "Back Tab".into(),
        KeyCode::Backspace => "Backspace".into(),
        KeyCode::Delete => "Delete".into(),
        KeyCode::Insert => "Insert".into(),
        KeyCode::Esc => "Escape".into(),
        KeyCode::Left => "Left arrow".into(),
        KeyCode::Right => "Right arrow".into(),
        KeyCode::Up => "Up arrow".into(),
        KeyCode::Down => "Down arrow".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "Page Up".into(),
        KeyCode::PageDown => "Page Down".into(),
        KeyCode::F(n) => format!("F{n}"),
        KeyCode::Null => "NUL (legacy ambiguity)".into(),
        KeyCode::CapsLock => "Caps Lock".into(),
        KeyCode::NumLock => "Num Lock".into(),
        KeyCode::ScrollLock => "Scroll Lock".into(),
        KeyCode::PrintScreen => "Print Screen".into(),
        KeyCode::Pause => "Pause".into(),
        KeyCode::Menu => "Menu".into(),
        KeyCode::KeypadBegin => "Keypad Begin".into(),
        KeyCode::Media(_) => "Media key".into(),
        KeyCode::Modifier(_) => "Modifier key".into(),
    }
}
fn shortcut(sequence: &str) -> String {
    sequence
        .split_whitespace()
        .map(|stroke| {
            let mut parts = stroke
                .split('+')
                .filter(|part| !part.is_empty())
                .map(|part| match part {
                    "ctrl" => "Ctrl".into(),
                    "cmd" => "Cmd".into(),
                    "alt" => "Alt".into(),
                    "shift" => "Shift".into(),
                    "space" => "Space".into(),
                    "escape" => "Esc".into(),
                    other => {
                        let mut chars = other.chars();
                        chars.next().map_or_else(String::new, |first| {
                            first.to_uppercase().collect::<String>() + chars.as_str()
                        })
                    }
                })
                .collect::<Vec<_>>();
            if stroke.ends_with('+') {
                parts.push("Plus".into());
            }
            parts.join(" + ")
        })
        .collect::<Vec<_>>()
        .join("  →  ")
}
fn modifiers(event: Option<KeyEvent>) -> Vec<(&'static str, bool)> {
    let mods = event.map_or(KeyModifiers::NONE, |key| key.modifiers);
    [
        ("Ctrl", KeyModifiers::CONTROL),
        ("Shift", KeyModifiers::SHIFT),
        ("Alt", KeyModifiers::ALT),
        ("Super", KeyModifiers::SUPER),
        ("Meta", KeyModifiers::META),
        ("Hyper", KeyModifiers::HYPER),
    ]
    .into_iter()
    .map(|(name, flag)| (name, mods.contains(flag)))
    .collect()
}
fn row(frame: &mut Frame, app: &App, x: u16, y: u16, caps: &[Cap]) {
    let colors = app.theme.colors;
    let mut left = x;
    for key in caps {
        let active = selected(key, app.last_key);
        let style = Style::default()
            .fg(if active {
                colors.background
            } else {
                colors.foreground
            })
            .bg(if active { colors.accent } else { colors.panel });
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(if active { colors.accent } else { colors.muted }));
        frame.render_widget(
            Paragraph::new(key.label)
                .alignment(Alignment::Center)
                .style(style)
                .block(block),
            Rect::new(left, y, key.width, 3),
        );
        left += key.width;
    }
}
pub(super) fn draw(frame: &mut Frame, app: &App) {
    let colors = app.theme.colors;
    let area = frame.area();
    let wide = area.width >= 104 && area.height >= 30;
    let compact = area.width >= 78 && area.height >= 25;
    let target = if wide {
        Rect::new(
            area.x + (area.width - 104) / 2,
            area.y + (area.height - 30) / 2,
            104,
            30,
        )
    } else {
        area
    };
    frame.render_widget(Clear, target);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Keyboard Inspector · Esc closes ")
        .style(Style::default().fg(colors.foreground).bg(colors.panel))
        .border_style(Style::default().fg(colors.accent));
    let inner = block.inner(target);
    frame.render_widget(block, target);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let event = app.last_key;
    let resolution = if app.keyboard.sequence.is_empty() {
        Resolution::None
    } else {
        app.keymap
            .resolve(&app.keyboard.sequence, &app.keyboard.context)
    };
    let mapping = match resolution {
        Resolution::Command(id, _) => format!("Mapped command: {id}"),
        Resolution::Chord => "Mapped command: chord prefix · waiting for next key".into(),
        Resolution::None => {
            if event.is_none() {
                "Mapped command: awaiting input".into()
            } else {
                "Mapped command: unbound in captured context".into()
            }
        }
    };
    let profile = match app.keymap.profile {
        Profile::Linux => "Linux",
        Profile::Windows => "Windows",
        Profile::Macos => "macOS",
    };
    let kind = event.map_or("Awaiting input", |key| match key.kind {
        KeyEventKind::Press => "Press",
        KeyEventKind::Repeat => "Repeat",
        KeyEventKind::Release => "Release",
    });
    let key = event.map_or_else(
        || "Press a shortcut to inspect it".into(),
        |key| key_name(key.code),
    );
    let normalized = if app.keyboard.sequence.is_empty() {
        "—".into()
    } else {
        shortcut(&app.keyboard.sequence)
    };
    let event_state = event
        .map(|event| {
            [
                (KeyEventState::CAPS_LOCK, "Caps Lock"),
                (KeyEventState::NUM_LOCK, "Num Lock"),
                (KeyEventState::KEYPAD, "Keypad"),
            ]
            .into_iter()
            .filter_map(|(flag, label)| event.state.contains(flag).then_some(label))
            .collect::<Vec<_>>()
            .join(", ")
        })
        .filter(|v| !v.is_empty());
    let information = vec![
        Line::styled(
            format!(
                " Received: {key} · {kind}{}",
                event_state.map_or(String::new(), |s| format!(" · {s}"))
            ),
            Style::default()
                .fg(colors.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Line::from(format!(" Shortcut: {normalized}")),
        Line::from(format!(" {mapping}")),
        Line::styled(
            format!(
                " Protocol: {} · Profile: {profile} · Context: {}",
                if app.enhanced { "enhanced" } else { "legacy" },
                app.keyboard.source
            ),
            Style::default().fg(colors.muted),
        ),
    ];
    frame.render_widget(
        Paragraph::new(information),
        Rect::new(inner.x, inner.y, inner.width, 4.min(inner.height)),
    );
    if inner.height <= 4 {
        return;
    }
    let mut spans = vec![Span::styled(
        " Reported modifiers: ",
        Style::default().fg(colors.muted),
    )];
    for (label, active) in modifiers(event) {
        spans.push(Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(if active {
                    colors.background
                } else {
                    colors.muted
                })
                .bg(if active { colors.accent } else { colors.panel }),
        ));
        spans.push(Span::raw(" "));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(inner.x, inner.y + 4, inner.width, 1),
    );
    if wide || compact {
        let x = inner.x + 1;
        let y = inner.y + 6;
        if wide {
            let mut functions = vec![cap("Esc", KeyCode::Esc, 6)];
            const LABELS: [&str; 12] = [
                "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
            ];
            functions.extend(
                LABELS
                    .into_iter()
                    .enumerate()
                    .map(|(i, label)| cap(label, KeyCode::F(i as u8 + 1), 5)),
            );
            row(frame, app, x, y, &functions);
            row(
                frame,
                app,
                x + 77,
                y,
                &[
                    cap("Prt", KeyCode::PrintScreen, 7),
                    cap("Scr", KeyCode::ScrollLock, 7),
                    cap("Pause", KeyCode::Pause, 8),
                ],
            );
        }
        let y = y + if wide { 3 } else { 0 };
        let mut number = vec![punctuation("` ~", '`', '~', 5)];
        for (label, plain, shifted) in [
            ("1 !", '1', '!'),
            ("2 @", '2', '@'),
            ("3 #", '3', '#'),
            ("4 $", '4', '$'),
            ("5 %", '5', '%'),
            ("6 ^", '6', '^'),
            ("7 &", '7', '&'),
            ("8 *", '8', '*'),
            ("9 (", '9', '('),
            ("0 )", '0', ')'),
            ("- _", '-', '_'),
            ("= +", '=', '+'),
        ] {
            number.push(punctuation(label, plain, shifted, 5));
        }
        number.push(cap("Backsp", KeyCode::Backspace, 9));
        row(frame, app, x, y, &number);
        let mut q = vec![cap("Tab", KeyCode::Tab, 7)];
        q.extend(chars("QWERTYUIOP"));
        q.extend([
            punctuation("[ {", '[', '{', 5),
            punctuation("] }", ']', '}', 5),
            punctuation("\\ |", '\\', '|', 7),
        ]);
        row(frame, app, x, y + 3, &q);
        let mut a = vec![cap("Caps", KeyCode::CapsLock, 9)];
        a.extend(chars("ASDFGHJKL"));
        a.extend([
            punctuation("; :", ';', ':', 5),
            punctuation("' \"", '\'', '"', 5),
            cap("Enter", KeyCode::Enter, 10),
        ]);
        row(frame, app, x, y + 6, &a);
        let mut z = vec![cap(
            "Shift",
            KeyCode::Modifier(ModifierKeyCode::LeftShift),
            12,
        )];
        z.extend(chars("ZXCVBNM"));
        z.extend([
            punctuation(", <", ',', '<', 5),
            punctuation(". >", '.', '>', 5),
            punctuation("/ ?", '/', '?', 5),
            cap("Shift", KeyCode::Modifier(ModifierKeyCode::RightShift), 12),
        ]);
        row(frame, app, x, y + 9, &z);
        row(
            frame,
            app,
            x,
            y + 12,
            &[
                cap("Ctrl", KeyCode::Modifier(ModifierKeyCode::LeftControl), 7),
                cap("Super", KeyCode::Modifier(ModifierKeyCode::LeftSuper), 7),
                cap("Alt", KeyCode::Modifier(ModifierKeyCode::LeftAlt), 7),
                cap("Space", KeyCode::Char(' '), 32),
                cap("Alt", KeyCode::Modifier(ModifierKeyCode::RightAlt), 7),
                cap("Menu", KeyCode::Menu, 7),
                cap("Ctrl", KeyCode::Modifier(ModifierKeyCode::RightControl), 7),
            ],
        );
        if wide {
            row(
                frame,
                app,
                x + 77,
                y,
                &[
                    cap("Ins", KeyCode::Insert, 7),
                    cap("Home", KeyCode::Home, 7),
                    cap("PgUp", KeyCode::PageUp, 8),
                ],
            );
            row(
                frame,
                app,
                x + 77,
                y + 3,
                &[
                    cap("Del", KeyCode::Delete, 7),
                    cap("End", KeyCode::End, 7),
                    cap("PgDn", KeyCode::PageDown, 8),
                ],
            );
            row(frame, app, x + 84, y + 9, &[cap("↑", KeyCode::Up, 7)]);
            row(
                frame,
                app,
                x + 77,
                y + 12,
                &[
                    cap("←", KeyCode::Left, 7),
                    cap("↓", KeyCode::Down, 7),
                    cap("→", KeyCode::Right, 8),
                ],
            );
        }
        let footer = y + 15;
        if footer < inner.bottom() {
            frame.render_widget(Paragraph::new(" US reference layout · highlight = last event, not held keys\n Modifier sides/layout cannot be inferred from combined terminal reports.").style(Style::default().fg(colors.muted)),Rect::new(inner.x,footer,inner.width,inner.bottom().saturating_sub(footer)));
        }
    } else if inner.height > 6 {
        frame.render_widget(Paragraph::new(" Compact view · enlarge to 104×30 for the keyboard diagram.\n Commands are previewed; editor text is protected.\n Only received events are known; no held/left/right state is inferred.").wrap(Wrap{trim:false}).style(Style::default().fg(colors.muted)),Rect::new(inner.x,inner.y+6,inner.width,inner.height-6));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::Event;
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
    use serde_json::Value;

    fn render(app: &mut App, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| super::super::draw(frame, app))
            .unwrap();
        terminal.backend().buffer().clone()
    }
    fn text(buffer: &Buffer) -> String {
        buffer.content.iter().map(|cell| cell.symbol()).collect()
    }
    fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
        app.event(Event::Key(KeyEvent::new(code, modifiers)));
    }

    #[test]
    fn keyboard_shortcut_names_preserve_plus_and_explicit_sides() {
        assert_eq!(shortcut("ctrl++"), "Ctrl + Plus");
        assert_eq!(shortcut("+"), "Plus");
        let event = Some(KeyEvent::new(
            KeyCode::Modifier(ModifierKeyCode::RightControl),
            KeyModifiers::CONTROL,
        ));
        assert!(selected(
            &cap("Ctrl", KeyCode::Modifier(ModifierKeyCode::RightControl), 7),
            event
        ));
        assert!(!selected(
            &cap("Ctrl", KeyCode::Modifier(ModifierKeyCode::LeftControl), 7),
            event
        ));
        assert!(
            !modifiers(Some(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::NONE)))
                .into_iter()
                .any(|(_, active)| active)
        );
    }

    #[test]
    fn keyboard_layout_highlights_received_key_and_combined_modifiers_without_sides() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.execute("vscli.keyboardInspector", Value::Null);
        app.theme.colors.accent = Color::Rgb(7, 19, 41);
        app.theme.colors.background = Color::Rgb(11, 23, 47);
        app.enhanced = true;
        press(
            &mut app,
            KeyCode::Char('p'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        let buffer = render(&mut app, 104, 30);
        let screen = text(&buffer);
        assert!(screen.contains("Ctrl + Shift + P"));
        assert!(screen.contains("workbench.action.showCommands"));
        assert!(screen.contains("Protocol: enhanced · Profile: Linux · Context: Editor"));
        assert_eq!(buffer[(56, 14)].symbol(), "P");
        assert_eq!(buffer[(56, 14)].bg, app.theme.colors.accent);
        assert_eq!(buffer[(56, 14)].fg, app.theme.colors.background);
        assert_eq!(buffer[(22, 5)].bg, app.theme.colors.accent); // aggregate Ctrl
        assert_eq!(buffer[(29, 5)].bg, app.theme.colors.accent); // aggregate Shift
        assert_eq!(buffer[(5, 23)].bg, app.theme.colors.panel); // left Ctrl
        assert_eq!(buffer[(72, 23)].bg, app.theme.colors.panel); // right Ctrl
        assert!(screen.contains("F12") && screen.contains("PgDn") && screen.contains('↑'));
        assert!(!screen.contains("KeyEvent") && !screen.contains("CONTROL"));
    }

    #[test]
    fn keyboard_preview_chords_release_and_resize_preserve_unsaved_document() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("kept.txt");
        std::fs::write(&path, "disk\r\n猫").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.open(&path).unwrap();
        app.doc_mut().insert("unsaved🙂", false);
        let id = app.doc().id;
        let revision = app.doc().revision;
        let selections = app.doc().selections();
        let original = app.doc().text.to_string();
        app.execute("vscli.keyboardInspector", Value::Null);
        press(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert!(text(&render(&mut app, 104, 30)).contains("chord prefix"));
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(text(&render(&mut app, 104, 30)).contains("editor.action.addCommentLine"));
        app.event(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            KeyEventKind::Release,
        )));
        assert!(text(&render(&mut app, 104, 30)).contains("Release"));
        press(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        app.event(Event::Paste("must not enter document".into()));
        for (w, h) in [(1, 1), (12, 4), (60, 15), (78, 25), (104, 30), (160, 50)] {
            app.event(Event::Resize(w, h));
            let buffer = render(&mut app, w, h);
            if w == 60 {
                assert!(text(&buffer).contains("Compact view"));
            }
        }
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().revision, revision);
        assert_eq!(app.doc().selections(), selections);
        assert_eq!(app.doc().text.to_string(), original);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "disk\r\n猫");
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(app.modal.is_none());
        assert_eq!(app.doc().text.to_string(), original);
        app.execute("workbench.action.files.save", Value::Null);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn inspector_hides_surface_presentations_without_discarding_selected_models() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        let key = crate::extensions::SurfaceKey {
            owner: "test.extension".into(),
            id: "output".into(),
        };
        app.extension_surfaces.output = Some(key.clone());
        app.extension_surfaces.tree = Some(key.clone());
        app.execute("vscli.keyboardInspector", Value::Null);
        for (width, height) in [(140, 40), (50, 16), (1, 1)] {
            app.extension_surfaces.output_area = Rect::new(5, 5, 20, 10);
            app.extension_surfaces
                .status_hits
                .push((Rect::new(0, 39, 20, 1), key.clone(), 7, 9));
            app.extension_surfaces.presented_tree =
                Some(crate::app::extension_surfaces::TreePresentation {
                    session: 9,
                    key: key.clone(),
                    generation: 7,
                    rows: vec![("previous".into(), 0)],
                });
            render(&mut app, width, height);
            assert_eq!(app.extension_surfaces.output_area, Rect::default());
            assert!(app.extension_surfaces.status_hits.is_empty());
            assert!(app.extension_surfaces.presented_tree.is_none());
            assert_eq!(app.extension_surfaces.output.as_ref(), Some(&key));
            assert_eq!(app.extension_surfaces.tree.as_ref(), Some(&key));
        }
        assert!(app.documents.is_empty());
    }

    #[test]
    fn keyboard_welcome_profiles_and_unmapped_unicode_are_honest() {
        let root = tempfile::tempdir().unwrap();
        for (profile, name, modifier) in [
            (Profile::Linux, "Linux", KeyModifiers::CONTROL),
            (Profile::Windows, "Windows", KeyModifiers::CONTROL),
            (Profile::Macos, "macOS", KeyModifiers::SUPER),
        ] {
            let mut app = App::new(root.path().into(), profile);
            app.execute("vscli.keyboardInspector", Value::Null);
            press(&mut app, KeyCode::Char('s'), modifier);
            let screen = text(&render(&mut app, 104, 30));
            assert!(screen.contains(name));
            assert!(screen.contains("workbench.action.files.save"));
            press(&mut app, KeyCode::Char('猫'), KeyModifiers::NONE);
            let screen = text(&render(&mut app, 104, 30));
            assert!(screen.contains("Received: 猫"));
            assert!(screen.contains("· Press"));
            assert_eq!(app.last_key.unwrap().code, KeyCode::Char('猫'));
            assert!(screen.contains("unbound in captured context"));
            assert!(app.documents.is_empty());
            assert!(!selected(
                &cap("Ctrl", KeyCode::Modifier(ModifierKeyCode::LeftControl), 7),
                app.last_key
            ));
        }
    }
}
