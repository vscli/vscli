use crate::{
    app::{App, COMMANDS, Focus, Modal, PromptKind},
    document::{Document, display_width, grapheme_width, graphemes},
};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use unicode_segmentation::UnicodeSegmentation;

fn clean(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { '�' } else { c })
        .collect()
}

fn clean_multiline(s: &str) -> String {
    s.lines().map(clean).collect::<Vec<_>>().join("\n")
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let colors = app.theme.colors;
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(colors.background).fg(colors.foreground)),
        area,
    );
    if area.width < 20 || area.height < 6 {
        frame.render_widget(
            Paragraph::new("VSCLI · enlarge terminal\nCtrl+Shift+W to exit")
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area);
    app.tab_area = rows[0];
    let mut tabs = vec![Span::styled(
        " VSCLI ",
        Style::default()
            .fg(colors.accent)
            .add_modifier(Modifier::BOLD),
    )];
    for (i, doc) in app.documents.iter().enumerate() {
        tabs.push(Span::styled(
            format!(
                " {}{} ",
                clean(&doc.name()),
                if doc.dirty() { " ●" } else { "" }
            ),
            Style::default()
                .fg(if i == app.active {
                    colors.foreground
                } else {
                    colors.muted
                })
                .bg(if i == app.active {
                    colors.selection
                } else {
                    colors.panel
                }),
        ));
        tabs.push(Span::raw(" "));
    }
    frame.render_widget(
        Paragraph::new(Line::from(tabs)).style(Style::default().bg(colors.panel)),
        rows[0],
    );
    let columns = Layout::horizontal([
        Constraint::Length(if app.sidebar && area.width >= 60 {
            26
        } else {
            0
        }),
        Constraint::Min(1),
    ])
    .split(rows[1]);
    if columns[0].width > 0 {
        draw_explorer(frame, app, columns[0]);
    } else {
        app.explorer_area = Rect::default();
    }
    let panes = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(if app.terminal_visible {
            (columns[1].height / 2).max(3).min(columns[1].height)
        } else {
            0
        }),
    ])
    .split(columns[1]);
    draw_editors(frame, app, panes[0]);
    app.terminal_area = Rect::default();
    if app.terminal_visible {
        draw_terminal(frame, app, panes[1]);
    }
    frame.render_widget(
        Paragraph::new(format!(" {}", clean(&app.message)))
            .style(Style::default().fg(colors.muted)),
        rows[2],
    );
    let status = if let Some(doc) = app.active_document() {
        format!(
            " {}{}  |  {}  |  Ln {}, Col {}  |  {} cursor(s)  |  UTF-8 {}  |  {}{}",
            clean(&doc.name()),
            if doc.dirty() { " *" } else { "" },
            app.language(),
            doc.row() + 1,
            doc.column() + 1,
            doc.secondary.len() + 1,
            if doc.eol == "\r\n" { "CRLF" } else { "LF" },
            if app.enhanced {
                "enhanced keys"
            } else {
                "legacy keys"
            },
            if app.workspace.indexing {
                "  · indexing…"
            } else {
                ""
            }
        )
    } else {
        format!(
            " VSCLI  |  {}  |  No open editors{}",
            clean(&app.workspace.root.file_name().map_or_else(
                || app.workspace.root.to_string_lossy(),
                |name| name.to_string_lossy()
            )),
            if app.workspace.indexing {
                "  · indexing…"
            } else {
                ""
            }
        )
    };
    frame.render_widget(
        Paragraph::new(status).style(Style::default().bg(colors.selection).fg(colors.foreground)),
        rows[3],
    );
    if app.prompt.is_some() {
        draw_prompt(frame, app);
    }
    if app.modal.is_some() {
        draw_modal(frame, app);
    }
}

fn draw_terminal(frame: &mut Frame, app: &mut App, area: Rect) {
    let colors = app.theme.colors;
    let count = app.terminals.len();
    let Some(terminal) = app.terminals.get_mut(app.active_terminal) else {
        return;
    };
    let title = format!(
        " TERMINAL {}/{} · {} · {} ",
        app.active_terminal + 1,
        count,
        clean(&terminal.title),
        clean(&terminal.status)
    );
    let block = Block::default()
        .borders(Borders::TOP)
        .title(title)
        .border_style(Style::default().fg(if app.focus == Focus::Terminal {
            colors.accent
        } else {
            colors.muted
        }));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.terminal_area = inner;
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if let Err(e) = terminal.resize(inner.height, inner.width) {
        app.message = format!("Terminal resize failed: {e}");
    }
    let screen = terminal.parser.screen();
    let color = |color, default| match color {
        vt100::Color::Default => default,
        vt100::Color::Idx(index) => Color::Indexed(index),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    };
    for row in 0..inner.height {
        for col in 0..inner.width {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            let mut style = Style::default()
                .fg(color(cell.fgcolor(), colors.foreground))
                .bg(color(cell.bgcolor(), colors.background));
            for (enabled, modifier) in [
                (cell.bold(), Modifier::BOLD),
                (cell.dim(), Modifier::DIM),
                (cell.italic(), Modifier::ITALIC),
                (cell.underline(), Modifier::UNDERLINED),
                (cell.inverse(), Modifier::REVERSED),
            ] {
                if enabled {
                    style = style.add_modifier(modifier);
                }
            }
            let text = if cell.contents().is_empty() {
                " "
            } else {
                cell.contents()
            };
            frame.buffer_mut().set_stringn(
                inner.x + col,
                inner.y + row,
                text,
                usize::from(inner.width - col),
                style,
            );
        }
    }
    if app.focus == Focus::Terminal
        && app.prompt.is_none()
        && app.modal.is_none()
        && !screen.hide_cursor()
        && screen.scrollback() == 0
    {
        let (row, col) = screen.cursor_position();
        frame.set_cursor_position((
            inner.x + col.min(inner.width - 1),
            inner.y + row.min(inner.height - 1),
        ));
    }
}

fn draw_explorer(frame: &mut Frame, app: &mut App, area: Rect) {
    let colors = app.theme.colors;
    let title = if app.focus == Focus::Explorer {
        " EXPLORER • "
    } else {
        " EXPLORER "
    };
    let block = Block::default()
        .borders(Borders::RIGHT | Borders::TOP)
        .title(title)
        .border_style(Style::default().fg(colors.muted))
        .style(Style::default().bg(colors.panel));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.explorer_area = inner;
    let offset = app
        .explorer_selected
        .saturating_sub(inner.height.saturating_sub(1) as usize);
    let lines: Vec<Line> = app
        .entries
        .iter()
        .enumerate()
        .skip(offset)
        .take(inner.height as usize)
        .map(|(i, entry)| {
            let name = entry
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let style = if i == app.explorer_selected && app.focus == Focus::Explorer {
                Style::default().bg(colors.selection).fg(colors.foreground)
            } else {
                Style::default().fg(if entry.directory {
                    colors.accent
                } else {
                    colors.foreground
                })
            };
            Line::styled(
                format!(
                    " {} {}",
                    if entry.directory { "▸" } else { " " },
                    clean(&name)
                ),
                style,
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_welcome(frame: &mut Frame, app: &App, area: Rect) {
    let colors = app.theme.colors;
    let logo = [
        "██╗   ██╗███████╗ ██████╗██╗     ██╗",
        "██║   ██║██╔════╝██╔════╝██║     ██║",
        "██║   ██║███████╗██║     ██║     ██║",
        "╚██╗ ██╔╝╚════██║██║     ██║     ██║",
        " ╚████╔╝ ███████║╚██████╗███████╗██║",
        "  ╚═══╝  ╚══════╝ ╚═════╝╚══════╝╚═╝",
    ];
    let mut lines = Vec::new();
    if area.width >= 42 && area.height >= 16 {
        lines.extend(
            logo.into_iter()
                .map(|line| Line::styled(line, Style::default().fg(colors.accent))),
        );
    } else {
        lines.push(Line::styled(
            "VSCLI",
            Style::default()
                .fg(colors.accent)
                .add_modifier(Modifier::BOLD),
        ));
    }
    lines.push(Line::default());
    lines.push(Line::styled(
        "Your terminal. Your workspace.",
        Style::default().fg(colors.muted),
    ));
    lines.push(Line::default());
    let context = app.context();
    for (label, command) in [
        ("New File", "workbench.action.files.newUntitledFile"),
        ("Open File", "workbench.action.files.openFile"),
        ("Quick Open", "workbench.action.quickOpen"),
        ("Open Recent File", "workbench.action.openRecent"),
        (
            "Reopen Closed Editor",
            "workbench.action.reopenClosedEditor",
        ),
        ("Command Palette", "workbench.action.showCommands"),
    ] {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{label}   "),
                Style::default().fg(colors.foreground),
            ),
            Span::styled(
                {
                    let key = app.keymap.shortcut_in_context(command, &context);
                    if key.is_empty() {
                        "Unbound".into()
                    } else {
                        key
                    }
                },
                Style::default().fg(colors.muted),
            ),
        ]));
    }
    if !app.recent_files.files.is_empty() {
        lines.push(Line::default());
        lines.push(Line::styled(
            "Recent files",
            Style::default().fg(colors.accent),
        ));
        for entry in app.recent_files.files.iter().take(5) {
            lines.push(Line::styled(
                clean(&entry.path.to_string_lossy()),
                Style::default().fg(colors.muted),
            ));
        }
    }
    let height = (lines.len() as u16).min(area.height);
    let content = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(height) / 2,
        area.width,
        height,
    );
    frame.render_widget(
        Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
        content,
    );
}

fn draw_editors(frame: &mut Frame, app: &mut App, area: Rect) {
    let colors = app.theme.colors;
    app.sync_pane();
    if app.documents.is_empty() {
        draw_welcome(frame, app, area);
        return;
    }
    let active_document = app.active;
    let active_view = app.panes[app.active_pane].id;
    let count = app.panes.len();
    let layout = Layout::default()
        .direction(if app.horizontal_split {
            ratatui::layout::Direction::Vertical
        } else {
            ratatui::layout::Direction::Horizontal
        })
        .constraints(vec![Constraint::Ratio(1, count as u32); count])
        .split(area);
    app.pane_areas.clear();
    for (index, pane_area) in layout.iter().enumerate() {
        let pane = app.panes[index].clone();
        app.active = app
            .documents
            .iter()
            .position(|doc| doc.id == pane.document)
            .unwrap_or(active_document);
        app.doc_mut().display_view(pane.id);
        let focused = index == app.active_pane;
        let inner = if count == 1 {
            *pane_area
        } else {
            let block = Block::default()
                .borders(Borders::TOP | Borders::RIGHT)
                .title(format!(" {} · {} ", index + 1, clean(&app.doc().name())))
                .border_style(Style::default().fg(if focused {
                    colors.accent
                } else {
                    colors.muted
                }));
            let inner = block.inner(*pane_area);
            frame.render_widget(block, *pane_area);
            inner
        };
        draw_editor(frame, app, inner, focused);
        app.pane_areas.push(app.editor_area);
    }
    app.active = active_document;
    app.doc_mut().display_view(active_view);
    app.editor_area = app.pane_areas[app.active_pane];
}

fn draw_editor(frame: &mut Frame, app: &mut App, area: Rect, focused: bool) {
    let colors = app.theme.colors;
    let digits = if app.doc().line_numbers == crate::settings::LineNumbers::Off {
        0
    } else {
        app.doc().line_count().to_string().len().max(3) as u16
    };
    let gutter = (digits + 2).min(area.width);
    let text_area = Rect::new(
        area.x + gutter,
        area.y,
        area.width.saturating_sub(gutter),
        area.height,
    );
    app.editor_area = text_area;
    if text_area.width == 0 || text_area.height == 0 {
        return;
    }
    let doc = app.doc_mut();
    let row = doc.row();
    let col = doc.visual_column();
    if row < doc.top {
        doc.top = row;
    } else if row >= doc.top + text_area.height as usize {
        doc.top = row + 1 - text_area.height as usize;
    }
    if col < doc.left {
        doc.left = col;
    } else if col >= doc.left + text_area.width as usize {
        doc.left = col + 1 - text_area.width as usize;
    }
    let doc = app.doc();
    let selections: Vec<_> = doc
        .selections()
        .iter()
        .map(|s| s.range())
        .filter(|range| !range.is_empty())
        .collect();
    let grammar = app.syntax.get(doc);
    let language = app.language();
    for y in 0..text_area.height {
        let row = doc.top + y as usize;
        if row >= doc.line_count() {
            break;
        }
        let active = row == doc.row();
        let base = Style::default().fg(colors.foreground).bg(if active {
            colors.current_line
        } else {
            colors.background
        });
        frame.render_widget(
            Block::default().style(base),
            Rect::new(area.x, area.y + y, area.width, 1),
        );
        let issue = app
            .current_diagnostics()
            .iter()
            .find(|d| d.range.start.line == row);
        let breakpoint = doc
            .path
            .as_ref()
            .and_then(|p| app.breakpoints.get(p))
            .is_some_and(|points| points.contains(&(row + 1)));
        let stopped = app
            .debugger
            .as_ref()
            .filter(|d| d.state == crate::debug::State::Stopped)
            .and_then(|d| d.frames.get(d.active_frame))
            .is_some_and(|f| {
                f.line == row + 1
                    && f.source.as_ref().and_then(|s| s.path.as_ref()) == doc.path.as_ref()
            });
        let label = match doc.line_numbers {
            crate::settings::LineNumbers::Off => String::new(),
            crate::settings::LineNumbers::Relative if !active => {
                row.abs_diff(doc.row()).to_string()
            }
            crate::settings::LineNumbers::Interval if !active && !(row + 1).is_multiple_of(10) => {
                String::new()
            }
            _ => (row + 1).to_string(),
        };
        let number = format!(
            "{:>width$}{}",
            label,
            if stopped {
                ">"
            } else if breakpoint {
                "●"
            } else if issue.is_some() {
                "!"
            } else {
                " "
            },
            width = digits as usize
        );
        frame.buffer_mut().set_string(
            area.x,
            area.y + y,
            number,
            Style::default().fg(if stopped {
                Color::Rgb(240, 200, 90)
            } else if issue.is_some() || breakpoint {
                Color::Rgb(244, 100, 100)
            } else if active {
                colors.accent
            } else {
                colors.muted
            }),
        );
        // The fallback lexer still needs the full line for word/string context.
        // Plain text and completed grammar results can use only the viewport
        // prefix, with enough lookahead for find matches crossing its edge.
        let line = if grammar.is_some() || language == "plaintext" {
            viewport_line(
                doc,
                row,
                doc.left + text_area.width as usize,
                app.find_query.len(),
            )
        } else {
            graphemes::as_text(doc.line_slice(row))
        };
        let styles = if grammar.is_none() {
            syntax_styles(&line, language, &app.theme)
        } else {
            Vec::new()
        };
        let start = doc.line_start(row);
        let start_byte = doc.text.char_to_byte(start);
        let mut char_pos = start;
        let mut visual = 0;
        let matches: Vec<_> = if app.find_query.is_empty() {
            Vec::new()
        } else {
            line.match_indices(&app.find_query)
                .map(|(b, _)| b..b + app.find_query.len())
                .collect()
        };
        for (byte, g) in line_graphemes(&line) {
            let width = doc.grapheme_width(g, visual);
            let next = visual + width;
            if visual >= doc.left + text_area.width as usize {
                break;
            }
            if next > doc.left && visual >= doc.left && next <= doc.left + text_area.width as usize
            {
                let color = grammar
                    .map(|h| {
                        h.style_at(start_byte + byte)
                            .map_or(colors.foreground, |style| app.theme.token(style))
                    })
                    .unwrap_or_else(|| styles.get(byte).copied().unwrap_or(colors.foreground));
                let mut style = base.fg(color);
                if matches.iter().any(|r| r.contains(&byte)) {
                    style = style.bg(Color::Rgb(96, 72, 21));
                }
                if selections
                    .iter()
                    .any(|r| r.start < char_pos + g.chars().count() && r.end > char_pos)
                {
                    style = style.bg(colors.selection);
                }
                paint_grapheme(
                    frame.buffer_mut(),
                    text_area.x + (visual - doc.left) as u16,
                    text_area.y + y,
                    g,
                    width,
                    style,
                );
            }
            char_pos += g.chars().count();
            visual = next;
        }
        if selections.iter().any(|r| r.contains(&doc.line_end(row)))
            && visual >= doc.left
            && visual < doc.left + text_area.width as usize
        {
            frame.buffer_mut().set_string(
                text_area.x + (visual - doc.left) as u16,
                text_area.y + y,
                " ",
                base.bg(colors.selection),
            );
        }
    }
    if focused && app.focus == Focus::Editor && app.prompt.is_none() && app.modal.is_none() {
        for selection in &doc.secondary {
            let row = doc.text.char_to_line(selection.cursor);
            let column =
                doc.display_width_slice(doc.text.slice(doc.line_start(row)..selection.cursor));
            if row >= doc.top
                && row < doc.top + text_area.height as usize
                && column >= doc.left
                && column < doc.left + text_area.width as usize
                && let Some(cell) = frame.buffer_mut().cell_mut((
                    text_area.x + (column - doc.left) as u16,
                    text_area.y + (row - doc.top) as u16,
                ))
            {
                cell.set_style(Style::default().bg(colors.foreground).fg(colors.background));
            }
        }
        frame.set_cursor_position((
            text_area.x + (col - doc.left) as u16,
            text_area.y + (doc.row() - doc.top) as u16,
        ));
    }
}

fn viewport_line(
    doc: &Document,
    row: usize,
    right: usize,
    lookahead_bytes: usize,
) -> std::borrow::Cow<'_, str> {
    let line = doc.line_slice(row);
    if line.len_bytes() <= right {
        return graphemes::as_text(line);
    }
    let mut end = 0;
    let mut column = 0;
    let mut clusters = graphemes::Graphemes::new(line);
    while column < right {
        let Some(g) = clusters.next() else {
            break;
        };
        column += doc.grapheme_width(&graphemes::as_text(g), column);
        end += g.len_bytes();
    }
    end = end.saturating_add(lookahead_bytes).min(line.len_bytes());
    // A byte-sized search query may leave lookahead inside a UTF-8 scalar.
    while end < line.len_bytes() && line.byte(end) & 0xc0 == 0x80 {
        end += 1;
    }
    graphemes::as_text(line.byte_slice(..end))
}

fn line_graphemes(line: &str) -> impl Iterator<Item = (usize, &str)> {
    // Printable ASCII and tabs are single-byte graphemes. Control sequences
    // such as CRLF must continue through the Unicode segmentation path.
    let ascii = line.bytes().all(|b| matches!(b, b'\t' | b' '..=b'~'));
    let mut bytes = 0..line.len();
    let mut unicode = line.grapheme_indices(true);
    std::iter::from_fn(move || {
        if ascii {
            bytes
                .next()
                .map(|offset| (offset, &line[offset..offset + 1]))
        } else {
            unicode.next()
        }
    })
}

fn paint_grapheme(buffer: &mut Buffer, x: u16, y: u16, g: &str, width: usize, style: Style) {
    match g.as_bytes() {
        // Layout already proved these one-cell symbols fit. Avoid allocating
        // an owned string and re-segmenting it inside Buffer::set_stringn.
        [b' '..=b'~'] => {
            buffer[(x, y)].set_symbol(g).set_style(style);
        }
        b"\t" => {
            for column in x..x + width as u16 {
                buffer[(column, y)].set_symbol(" ").set_style(style);
            }
        }
        _ => {
            // Retain Ratatui's width, zero-width and wide-cell reset behavior
            // for Unicode. Borrow the existing grapheme instead of copying it.
            let symbol = if g.chars().any(char::is_control) {
                "�"
            } else {
                g
            };
            buffer.set_stringn(x, y, symbol, width, style);
        }
    }
}

// Lightweight lexical colors for the first build. This is not a grammar engine.
fn syntax_styles(line: &str, language: &str, theme: &crate::theme::Theme) -> Vec<Color> {
    let colors = theme.colors;
    if language == "plaintext" {
        return Vec::new();
    }
    let mut styles = vec![colors.foreground; line.len()];
    let mut in_string = None;
    let mut escape = false;
    let mut comment = false;
    let chars: Vec<_> = line.char_indices().collect();
    for (idx, (byte, c)) in chars.iter().copied().enumerate() {
        let next = chars.get(idx + 1).map(|(_, c)| *c);
        if in_string.is_none()
            && ((c == '/' && next == Some('/'))
                || (c == '#' && matches!(language, "python" | "toml" | "shellscript")))
        {
            comment = true;
        }
        let color = if comment {
            theme.token(0)
        } else if let Some(quote) = in_string {
            if c == quote && !escape {
                in_string = None;
            }
            let prior = escape;
            escape = c == '\\' && !prior;
            theme.token(1)
        } else if matches!(c, '"' | '\'') {
            in_string = Some(c);
            theme.token(1)
        } else if c.is_ascii_digit() {
            theme.token(2)
        } else {
            colors.foreground
        };
        styles[byte..byte + c.len_utf8()].fill(color);
    }
    let keywords = [
        "fn",
        "pub",
        "use",
        "mod",
        "let",
        "mut",
        "struct",
        "enum",
        "impl",
        "self",
        "Self",
        "match",
        "if",
        "else",
        "return",
        "for",
        "while",
        "loop",
        "in",
        "const",
        "async",
        "await",
        "def",
        "class",
        "import",
        "from",
        "True",
        "False",
        "None",
        "function",
        "var",
        "export",
        "type",
        "interface",
        "true",
        "false",
        "null",
    ];
    for (start, word) in line.unicode_word_indices() {
        if keywords.contains(&word) && styles.get(start) == Some(&colors.foreground) {
            styles[start..start + word.len()].fill(theme.token(4));
        }
    }
    styles
}

fn popup(
    frame: &mut Frame,
    colors: crate::theme::Colors,
    title: &str,
    width: u16,
    height: u16,
) -> Rect {
    let area = frame.area();
    let w = width.min(area.width.saturating_sub(2));
    let h = height.min(area.height.saturating_sub(2));
    let rect = Rect::new(area.x + (area.width - w) / 2, area.y + 1, w, h);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(colors.accent))
        .style(Style::default().bg(colors.panel).fg(colors.foreground));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    inner
}
fn draw_prompt(frame: &mut Frame, app: &App) {
    let colors = app.theme.colors;
    let p = app.prompt.as_ref().unwrap();
    let title = match p.kind {
        PromptKind::InstallExtension => {
            " Install Extension from local VSIX · path · Enter installs without running code "
        }
        PromptKind::Palette => " Command Palette ",
        PromptKind::QuickOpen => " Go to File ",
        PromptKind::RecentFiles => " Open Recent File · file history only ",
        PromptKind::Snippet => " Insert Snippet · name, prefix or description ",
        PromptKind::Theme => " Color Theme · select or Load Color Theme File from commands ",
        PromptKind::StopExtension => " Stop Selected Extension ID (publisher.name) ",
        PromptKind::ThemeFile => " Load VS Code Color Theme (JSON/JSONC path) ",
        PromptKind::Open => " Open File (absolute or workspace-relative) ",
        PromptKind::SaveAs => " Save As (existing files are protected) ",
        PromptKind::DebugEvaluate => {
            " Evaluate in Selected Debug Frame · Enter executes expression "
        }
        PromptKind::CreateFile => " Create File · parent directory must exist ",
        PromptKind::CreateFolder => " Create Folder · parent directory must exist ",
        PromptKind::RenameFile(_) => " Rename Item · existing destinations are protected ",
        PromptKind::GitCommit => {
            " Commit Staged Changes · Enter commits (runs configured Git hooks) "
        }
        PromptKind::Rename => " Rename Symbol · Enter applies to unsaved buffers ",
        PromptKind::WorkspaceSearch => " Find in Files · Alt+C case · Alt+W word · Alt+R regex ",
        PromptKind::Find => " Find · Enter searches, F3 next ",
        PromptKind::ReplaceQuery => " Replace All · Find literal text ",
        PromptKind::ReplaceWith(_) => " Replace All · Replacement (Enter applies, Undo restores) ",
        PromptKind::Goto => " Go to Line · line:column ",
    };
    let list = matches!(
        p.kind,
        PromptKind::Palette
            | PromptKind::QuickOpen
            | PromptKind::RecentFiles
            | PromptKind::Snippet
            | PromptKind::Theme
    );
    let inner = popup(frame, colors, title, 84, if list { 19 } else { 5 });
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let cursor_width = display_width(&p.text[..p.cursor]);
    let available = inner.width.saturating_sub(3) as usize;
    let skip = cursor_width.saturating_sub(available);
    let mut visual = 0;
    let shown: String = p
        .text
        .graphemes(true)
        .filter_map(|g| {
            let old = visual;
            visual += grapheme_width(g, visual);
            if old >= skip { Some(clean(g)) } else { None }
        })
        .collect();
    frame.render_widget(
        Paragraph::new(format!("> {shown}")).style(
            Style::default()
                .bg(if p.select_all {
                    colors.selection
                } else {
                    colors.background
                })
                .fg(colors.foreground),
        ),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    frame.set_cursor_position((
        inner.x + 2 + (cursor_width - skip).min(available) as u16,
        inner.y,
    ));
    if matches!(p.kind, PromptKind::WorkspaceSearch) && inner.height > 1 {
        frame.render_widget(
            Paragraph::new(format!(
                "Case: {}   Whole word: {}   Regex: {}",
                app.search_options.case_sensitive,
                app.search_options.whole_word,
                app.search_options.regex
            )),
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );
    }
    if list && inner.height > 2 {
        let items: Vec<String> = if matches!(p.kind, PromptKind::Palette) {
            app.palette_items(&p.text)
                .into_iter()
                .map(|(name, id)| format!("{name}  {}", app.keymap.shortcut(id)))
                .collect()
        } else if matches!(p.kind, PromptKind::Snippet) {
            app.snippet_items(&p.text)
                .into_iter()
                .map(|entry| {
                    format!(
                        "{}  {}  {}  [{}]",
                        entry.name,
                        entry.prefixes.join(", "),
                        entry.description.replace('\n', " "),
                        entry
                            .source
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                    )
                })
                .collect()
        } else if matches!(p.kind, PromptKind::RecentFiles) {
            app.recent_items(&p.text)
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect()
        } else if matches!(p.kind, PromptKind::Theme) {
            app.theme_items(&p.text)
                .into_iter()
                .map(str::to_owned)
                .collect()
        } else {
            app.workspace
                .matches(&p.text)
                .iter()
                .map(|path| app.workspace.relative(path))
                .collect()
        };
        let selected = p.selected.min(items.len().saturating_sub(1));
        let height = inner.height.saturating_sub(2) as usize;
        let offset = selected.saturating_sub(height.saturating_sub(1));
        let lines: Vec<_> = items
            .iter()
            .enumerate()
            .skip(offset)
            .take(height)
            .map(|(i, item)| {
                Line::styled(
                    format!(" {}", clean(item)),
                    Style::default().bg(if i == selected {
                        colors.selection
                    } else {
                        colors.panel
                    }),
                )
            })
            .collect();
        frame.render_widget(
            Paragraph::new(if lines.is_empty() {
                vec![Line::from(" No matches")]
            } else {
                lines
            }),
            Rect::new(inner.x, inner.y + 2, inner.width, inner.height - 2),
        );
    }
}
fn draw_modal(frame: &mut Frame, app: &App) {
    let colors = app.theme.colors;
    let (title, text) = match app.modal.as_ref().unwrap() {
        Modal::ExtensionsLoading(_) => (
            " Extensions · Loading · Esc closes ",
            "Reading installed packages…".into(),
        ),
        Modal::Help => (
            " Getting Started · Esc to close ",
            format!(
                "VSCLI 0.1 · Native terminal editor\n\n{}  Command palette\n{}  Quick open (respects ignore files)\n{}  Open a path or create a file\n{}  Save    {}  Save As\n{}  Find    {}  Go to line\n{}  Toggle explorer\nCtrl+PageUp / Ctrl+PageDown  Switch tabs\nShift+arrows  Select · Mouse drag selects\n\nExplorer: arrows, Enter to open, Left for parent, Esc for editor.\n\nRecovery snapshots are written every two seconds.\nUTF-8 files up to 32 MiB. LSP: launch with --lsp PROGRAM.\nCtrl+D adds occurrences · Ctrl+Shift+F searches files.\nF1 → Extensions: Install from VSIX / Show Installed Extensions.\nCode extensions use an optional experimental Node host.\nF1 → Keyboard Inspector shows what your terminal sends.\nEnhanced shortcuts require a compatible terminal configuration.\n\n{}  Exit (unsaved changes are protected)",
                app.keymap.shortcut("workbench.action.showCommands"),
                app.keymap.shortcut("workbench.action.quickOpen"),
                app.keymap.shortcut("workbench.action.files.openFile"),
                app.keymap.shortcut("workbench.action.files.save"),
                app.keymap.shortcut("workbench.action.files.saveAs"),
                app.keymap.shortcut("actions.find"),
                app.keymap.shortcut("workbench.action.gotoLine"),
                app.keymap
                    .shortcut("workbench.action.toggleSidebarVisibility"),
                app.keymap.shortcut("workbench.action.closeWindow")
            ),
        ),
        Modal::ConfirmTask(task) => (
            " Run Workspace Task? ",
            format!(
                "{}\n\nThis workspace defines the command above.\n[Enter / Y] Trust tasks for this session and run\n[Esc / N] Cancel\n\n{}",
                clean_multiline(&task.preview),
                clean(&task.notice)
            ),
        ),
        Modal::Tasks { tasks, selected } => {
            let inner = popup(frame, colors, " Tasks · Enter runs · Esc closes ", 100, 20);
            let height = inner.height as usize;
            let offset = selected.saturating_sub(height.saturating_sub(1));
            let lines: Vec<_> = tasks
                .iter()
                .enumerate()
                .skip(offset)
                .take(height)
                .map(|(i, task)| {
                    Line::styled(
                        format!(
                            "{} · {}",
                            clean(&task.label),
                            clean(
                                task.definition["command"]
                                    .as_str()
                                    .unwrap_or("provider task")
                            )
                        ),
                        Style::default()
                            .fg(colors.foreground)
                            .bg(if i == *selected {
                                colors.selection
                            } else {
                                colors.panel
                            }),
                    )
                })
                .collect();
            frame.render_widget(Paragraph::new(lines), inner);
            return;
        }
        Modal::Text {
            title,
            text,
            scroll,
        } => {
            let inner = popup(
                frame,
                colors,
                title,
                110,
                frame.area().height.saturating_sub(2),
            );
            let lines: Vec<_> = text
                .lines()
                .skip(*scroll)
                .take(inner.height as usize)
                .map(|line| {
                    let color = if line.starts_with('+') {
                        Color::Rgb(130, 200, 145)
                    } else if line.starts_with('-') {
                        Color::Rgb(240, 130, 140)
                    } else {
                        colors.foreground
                    };
                    Line::styled(clean(line), Style::default().fg(color))
                })
                .collect();
            frame.render_widget(Paragraph::new(lines), inner);
            return;
        }
        Modal::Debug { section, selected } => {
            let title = ["Stack", "Scopes", "Variables", "Console"][*section];
            let inner = popup(
                frame,
                colors,
                &format!(
                    " DEBUG · {title} · Tab changes view · Enter expands · E evaluates · Esc closes "
                ),
                110,
                frame.area().height.saturating_sub(2),
            );
            let Some(debug) = &app.debugger else {
                frame.render_widget(
                    Paragraph::new("No debug session. Configure --debug-adapter and press F5."),
                    inner,
                );
                return;
            };
            let items: Vec<String> = match section {
                0 => debug
                    .frames
                    .iter()
                    .map(|f| {
                        format!(
                            "{} · {}:{}",
                            f.name,
                            f.source
                                .as_ref()
                                .and_then(|s| s.path.as_ref())
                                .map_or(String::new(), |p| app.workspace.relative(p)),
                            f.line
                        )
                    })
                    .collect(),
                1 => debug.scopes.iter().map(|s| s.name.clone()).collect(),
                2 => debug
                    .variables
                    .iter()
                    .map(|v| {
                        format!(
                            "{}{} = {} {}",
                            if v.reference > 0 { "▸ " } else { "  " },
                            v.name,
                            v.value,
                            v.kind.as_deref().unwrap_or("")
                        )
                    })
                    .collect(),
                _ => debug.console.iter().cloned().collect(),
            };
            let height = inner.height as usize;
            let offset = selected.saturating_sub(height.saturating_sub(1));
            let lines: Vec<_> = if items.is_empty() {
                vec![Line::raw(format!("{} · no items", clean(&debug.reason)))]
            } else {
                items
                    .iter()
                    .enumerate()
                    .skip(offset)
                    .take(height)
                    .map(|(i, s)| {
                        Line::styled(
                            clean(s),
                            Style::default()
                                .fg(colors.foreground)
                                .bg(if i == *selected {
                                    colors.selection
                                } else {
                                    colors.panel
                                }),
                        )
                    })
                    .collect()
            };
            frame.render_widget(Paragraph::new(lines), inner);
            return;
        }
        Modal::Git => {
            let inner = popup(
                frame,
                colors,
                " SOURCE CONTROL · Enter open · S stage · U unstage · D diff · Shift+D staged · R refresh · C commit ",
                frame.area().width.saturating_sub(2),
                frame.area().height.saturating_sub(2),
            );
            if inner.height == 0 {
                return;
            }
            let header = if let Some(status) = &app.git_status {
                format!(
                    "{} · {} changes{}",
                    clean(&status.branch),
                    status.entries.len(),
                    if app.git_job.is_some() {
                        " · working…"
                    } else {
                        ""
                    }
                )
            } else {
                "Loading Git status…".into()
            };
            frame.render_widget(
                Paragraph::new(header).style(Style::default().fg(colors.accent)),
                Rect::new(inner.x, inner.y, inner.width, 1),
            );
            if let Some(status) = &app.git_status {
                let height = inner.height.saturating_sub(2) as usize;
                let offset = app.git_selected.saturating_sub(height.saturating_sub(1));
                let lines: Vec<_> = status
                    .entries
                    .iter()
                    .enumerate()
                    .skip(offset)
                    .take(height)
                    .map(|(i, entry)| {
                        Line::styled(
                            format!(
                                "{}{} {}",
                                entry.index,
                                entry.worktree,
                                clean(&entry.path.to_string_lossy())
                            ),
                            Style::default()
                                .fg(colors.foreground)
                                .bg(if i == app.git_selected {
                                    colors.selection
                                } else {
                                    colors.panel
                                }),
                        )
                    })
                    .collect();
                frame.render_widget(
                    Paragraph::new(lines),
                    Rect::new(inner.x, inner.y + 2, inner.width, height as u16),
                );
            }
            return;
        }
        Modal::RunExtension(item) => {
            let inner = popup(
                frame,
                colors,
                " Run installed extension? · Enter runs · Esc cancels ",
                96,
                12,
            );
            frame.render_widget(Paragraph::new(format!("{}@{}\n{}\n\nRuns package code with your user permissions in the optional Node host.\nAdds or replaces this package in the selected session; restarts all selected packages.\nInstallation and package identity do not establish API compatibility.", clean(&item.id), clean(&item.version), clean(&item.compatibility))).wrap(Wrap {trim:false}), inner);
            return;
        }
        Modal::Extensions { items, selected } => {
            let inner = popup(
                frame,
                colors,
                " Installed Extensions · Enter run · S stop selected · H restart session · R rollback · Delete remove ",
                120,
                frame.area().height.saturating_sub(2),
            );
            let session_lines: Vec<_> = app
                .extension_packages
                .iter()
                .map(|package| {
                    Line::raw(format!(
                        "Session: {}@{} · {}",
                        clean(&package.id),
                        clean(&package.version),
                        clean(&app.extension_status(&package.id))
                    ))
                })
                .collect();
            let height = (inner.height as usize).saturating_sub(session_lines.len());
            let offset = selected.saturating_sub(height.saturating_sub(1));
            let lines: Vec<_> = if items.is_empty() {
                vec![Line::raw(
                    "No packages installed. F1 → Extensions: Install from VSIX",
                )]
            } else {
                items
                    .iter()
                    .enumerate()
                    .skip(offset)
                    .take(height)
                    .map(|(index, item)| {
                        Line::styled(
                            format!(
                                "{}@{}  {} · {}",
                                clean(&item.id),
                                clean(&item.version),
                                clean(&app.extension_status(&item.id)),
                                clean(&item.compatibility)
                            ),
                            Style::default()
                                .fg(colors.foreground)
                                .bg(if index == *selected {
                                    colors.selection
                                } else {
                                    colors.panel
                                }),
                        )
                    })
                    .collect()
            };
            frame.render_widget(
                Paragraph::new(session_lines.into_iter().chain(lines).collect::<Vec<_>>()),
                inner,
            );
            return;
        }
        Modal::Language {
            title,
            items,
            selected,
        } => {
            let inner = popup(
                frame,
                colors,
                title,
                110,
                frame.area().height.saturating_sub(2),
            );
            let height = inner.height as usize;
            let offset = selected.saturating_sub(height.saturating_sub(1));
            let lines: Vec<_> = if items.is_empty() {
                vec![Line::raw("No results")]
            } else {
                items
                    .iter()
                    .enumerate()
                    .skip(offset)
                    .take(height)
                    .map(|(i, item)| {
                        Line::styled(
                            clean(&item.label),
                            Style::default()
                                .fg(colors.foreground)
                                .bg(if i == *selected {
                                    colors.selection
                                } else {
                                    colors.panel
                                }),
                        )
                    })
                    .collect()
            };
            frame.render_widget(Paragraph::new(lines), inner);
            return;
        }
        Modal::Search => {
            let Some(search) = &app.search else {
                return;
            };
            let inner = popup(
                frame,
                colors,
                " Search Results · Enter opens · Esc closes/cancels ",
                110,
                frame.area().height.saturating_sub(2),
            );
            if inner.height == 0 {
                return;
            }
            let status = format!(
                "{} matches{} · {} skipped · {}{}",
                search.hits.len(),
                if search.truncated {
                    " (limit reached)"
                } else {
                    ""
                },
                search.skipped,
                clean(&search.query),
                if search.running {
                    " · searching…"
                } else {
                    ""
                }
            );
            frame.render_widget(
                Paragraph::new(status).style(Style::default().fg(colors.accent)),
                Rect::new(inner.x, inner.y, inner.width, 1),
            );
            let height = inner.height.saturating_sub(2) as usize;
            let offset = search.selected.saturating_sub(height.saturating_sub(1));
            let lines: Vec<_> = search
                .hits
                .iter()
                .enumerate()
                .skip(offset)
                .take(height)
                .map(|(i, hit)| {
                    Line::styled(
                        format!(
                            "{}:{}:{}  {}",
                            app.workspace.relative(&hit.path),
                            hit.row + 1,
                            hit.column + 1,
                            clean(&hit.line)
                        ),
                        Style::default()
                            .fg(colors.foreground)
                            .bg(if i == search.selected {
                                colors.selection
                            } else {
                                colors.panel
                            }),
                    )
                })
                .collect();
            frame.render_widget(
                Paragraph::new(lines),
                Rect::new(inner.x, inner.y + 2, inner.width, height as u16),
            );
            return;
        }
        Modal::Keys => {
            let mut text = String::from("Implemented command shortcuts · Esc to close\n\n");
            for (label, id) in COMMANDS {
                let key = app.keymap.shortcut(id);
                if !key.is_empty() {
                    text.push_str(&format!("{key:24} {label}\n"));
                }
            }
            text.push_str("\nAdditional movement/selection bindings: see docs/USAGE.md\nCustom bindings: --keybindings path/to/keybindings.json");
            (" Keyboard Shortcuts ", text)
        }
        Modal::Inspector => (
            " Keyboard Inspector · Esc to close ",
            format!(
                "Press a key combination to inspect the received event.\n\n{}\n\nProtocol: {}\nProfile: {:?}\n\nIf nothing changes, the terminal or OS may have intercepted it.\nThis inspector cannot reconstruct missing key events.",
                clean(&app.last_key),
                if app.enhanced { "enhanced" } else { "legacy" },
                app.keymap.profile
            ),
        ),
        Modal::Confirm(_) => (
            " Unsaved Changes ",
            format!(
                "Save changes to {}?\n\n[S / Enter] Save    [D] Discard    [Esc / C] Cancel",
                clean(&app.doc().name())
            ),
        ),
        Modal::Trash(path) => (
            " Move to System Trash? ",
            format!(
                "{}\n\nOpen buffers will be retained as unsaved copies.\n[Enter / Y] Move to trash    [Esc / N] Cancel",
                clean(&path.to_string_lossy())
            ),
        ),
        Modal::Revert => (
            " Revert File ",
            format!(
                "Discard in-memory edits and reload {} from disk?\n\n[Y] Revert    [N / Esc] Cancel",
                clean(&app.doc().name())
            ),
        ),
    };
    let inner = popup(frame, colors, title, 88, (text.lines().count() + 3) as u16);
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(colors.foreground)),
        inner,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    #[test]
    fn viewport_prefix_preserves_visible_clusters_and_search_matches() {
        for line in [
            "a\tbc def ".repeat(1000),
            "e\u{301} 猫 👩\u{200d}💻 🇬🇧🇺🇸 ".repeat(500),
            format!("a{}TAIL", "\u{301}".repeat(4000)),
        ] {
            let mut doc = Document::default();
            doc.insert(&format!("{line}\r\nsecond"), false);
            for right in [0, 1, 2, 7, 80, 499, 1003] {
                for query in ["", "bc def", "猫 👩\u{200d}💻", "🇬🇧🇺🇸", "TAIL"] {
                    let prefix = viewport_line(&doc, 0, right, query.len());
                    let mut column = 0;
                    let mut visible_end = 0;
                    let expected: Vec<_> = line
                        .grapheme_indices(true)
                        .take_while(|(byte, g)| {
                            if column >= right {
                                return false;
                            }
                            column += doc.grapheme_width(g, column);
                            visible_end = byte + g.len();
                            true
                        })
                        .collect();
                    assert_eq!(
                        prefix
                            .grapheme_indices(true)
                            .take(expected.len())
                            .collect::<Vec<_>>(),
                        expected
                    );
                    assert!(prefix.len() <= (visible_end + query.len() + 3).min(line.len()));
                    if !query.is_empty() {
                        let matches = |text: &str| {
                            text.match_indices(query)
                                .map(|(offset, _)| offset)
                                .take_while(|offset| *offset < visible_end)
                                .collect::<Vec<_>>()
                        };
                        assert_eq!(matches(&prefix), matches(&line));
                    }
                }
            }
            assert_eq!(viewport_line(&doc, 1, 80, 0), "second");
        }
    }

    #[test]
    fn theme_colors_reach_editor_cells_and_fallback_syntax() {
        use ratatui::{Terminal, backend::TestBackend};
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), crate::keys::Profile::Linux);
        app.execute(
            "workbench.action.files.newUntitledFile",
            serde_json::Value::Null,
        );
        app.sidebar = false;
        app.doc_mut().path = Some(root.path().join("main.rs"));
        app.doc_mut().insert("fn sample() {}", false);
        app.doc_mut().move_to(0, false);
        app.theme.colors.foreground = Color::Rgb(1, 2, 3);
        app.theme.colors.current_line = Color::Rgb(4, 5, 6);
        app.theme.tokens[4] = Color::Rgb(7, 8, 9);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let area = app.editor_area;
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(area.x, area.y)].symbol(), "f");
        assert_eq!(buffer[(area.x, area.y)].fg, Color::Rgb(7, 8, 9));
        assert_eq!(buffer[(area.x, area.y)].bg, Color::Rgb(4, 5, 6));
        assert_eq!(buffer[(area.x + 3, area.y)].fg, Color::Rgb(1, 2, 3));
    }

    #[test]
    fn fast_grapheme_iteration_matches_unicode_segmentation() {
        let ascii: String = (0..=127).map(char::from).collect();
        for line in [
            "ASCII words\tand tabs 0123456789",
            "",
            "\r\n",
            "\u{301}a e\u{301}",
            "猫🙂 👩\u{200d}💻 🇬🇧",
            "a\u{1b}b\u{7f}c",
            &ascii,
        ] {
            assert_eq!(
                line_graphemes(line).collect::<Vec<_>>(),
                line.grapheme_indices(true).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn fast_cell_painting_matches_previous_renderer() {
        let colors = crate::theme::Theme::default().colors;
        let doc = crate::document::Document::default();
        for g in [
            "a",
            " ",
            "\t",
            "\u{301}",
            "e\u{301}",
            "猫",
            "🙂",
            "👩\u{200d}💻",
            "🇬🇧",
            "\u{1b}",
            "\r\n",
        ] {
            for column in 0..8 {
                for style in [
                    Style::default(),
                    Style::default()
                        .fg(colors.accent)
                        .bg(colors.selection)
                        .add_modifier(Modifier::REVERSED),
                ] {
                    let mut expected =
                        Buffer::filled(Rect::new(0, 0, 20, 2), ratatui::buffer::Cell::new("?"));
                    let mut actual = expected.clone();
                    let width = doc.grapheme_width(g, column);
                    let symbol = if g == "\t" {
                        " ".repeat(width)
                    } else if g.chars().any(char::is_control) {
                        "�".into()
                    } else {
                        g.into()
                    };
                    expected.set_stringn(column as u16, 1, symbol, width, style);
                    paint_grapheme(&mut actual, column as u16, 1, g, width, style);
                    assert_eq!(actual, expected, "{g:?} at {column}");
                }
            }
        }
    }
    #[test]
    fn welcome_uses_overridden_shortcuts_and_themed_disambiguated_recent_paths() {
        let root = tempfile::tempdir().unwrap();
        let keys = root.path().join("keys.json");
        std::fs::write(
            &keys,
            r#"[{"key":"f8","command":"workbench.action.openRecent"}]"#,
        )
        .unwrap();
        let mut app = App::new(root.path().into(), crate::keys::Profile::Linux);
        app.keymap.load(&keys).unwrap();
        app.theme.colors.accent = Color::Rgb(1, 2, 3);
        app.recent_files.touch(root.path().join("left/same.txt"));
        app.recent_files.touch(root.path().join("right/same.txt"));
        let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let cells = &terminal.backend().buffer().content;
        let text: String = cells.iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("Open Recent File   f8"));
        assert!(text.contains("left/same.txt") && text.contains("right/same.txt"));
        assert!(
            cells
                .iter()
                .any(|cell| cell.symbol() == "R" && cell.fg == Color::Rgb(1, 2, 3))
        );
        assert!(app.documents.is_empty());
    }
    #[test]
    fn welcome_renders_without_creating_a_document_at_all_sizes() {
        let dir = tempfile::tempdir().unwrap();
        for profile in [
            crate::keys::Profile::Linux,
            crate::keys::Profile::Macos,
            crate::keys::Profile::Windows,
        ] {
            let mut app = App::new(dir.path().into(), profile);
            for (w, h) in [(110, 32), (40, 10), (20, 6), (1, 1)] {
                let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                terminal.draw(|frame| draw(frame, &mut app)).unwrap();
                assert!(app.documents.is_empty());
                assert!(app.panes.is_empty());
                let text: String = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect();
                assert!(!text.contains("Untitled"));
                if w == 110 {
                    assert!(text.contains("No open editors"));
                    assert!(text.contains("Command Palette"));
                    assert!(
                        text.contains(
                            &app.keymap
                                .shortcut("workbench.action.files.newUntitledFile")
                        )
                    );
                }
            }
        }
    }

    #[test]
    fn drawing_split_views_preserves_typing_groups_and_the_active_view() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(dir.path().into(), crate::keys::Profile::Linux);
        app.execute(
            "workbench.action.files.newUntitledFile",
            serde_json::Value::Null,
        );
        app.execute("workbench.action.splitEditor", serde_json::Value::Null);
        let mut terminal = Terminal::new(TestBackend::new(110, 32)).unwrap();
        for ch in ["a", "b", "c"] {
            app.doc_mut().insert(ch, true);
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            assert_eq!(app.active_pane, 1);
        }
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "");
        for (w, h) in [(20, 6), (40, 10), (100, 30)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        }
    }
    #[test]
    fn render_unicode_selection_and_tiny_sizes() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(dir.path().into(), crate::keys::Profile::Linux);
        app.execute(
            "workbench.action.files.newUntitledFile",
            serde_json::Value::Null,
        );
        app.doc_mut().insert("hello\n\t猫🙂 e\u{301}\n", false);
        app.doc_mut().select_all();
        for (w, h) in [(100, 30), (40, 10), (10, 3), (1, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal.draw(|f| draw(f, &mut app)).unwrap();
        }
    }
}
