mod keyboard;
pub(crate) mod welcome;
use crate::{
    app::{App, COMMANDS, Focus, Modal, OutlineStatus, PromptKind},
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
    app.welcome_brand.begin_frame();
    app.welcome_actions.clear();
    // Frame presentation is rebuilt only for surfaces actually drawn below.
    // Full-screen Inspector and tiny-terminal early returns show none of them.
    app.extension_surfaces.output_area = Rect::default();
    app.extension_surfaces.status_hits.clear();
    app.extension_surfaces.presented_tree = None;
    app.outline_area = Rect::default();
    let colors = app.theme.colors;
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(colors.background).fg(colors.foreground)),
        area,
    );
    if matches!(app.modal, Some(Modal::Inspector)) {
        keyboard::draw(frame, app);
        return;
    }
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
    let output_visible = app.extension_surfaces.output.is_some();
    let panel_height = (columns[1].height
        / if output_visible && app.terminal_visible {
            3
        } else {
            2
        })
    .max(3)
    .min(columns[1].height);
    let panes = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(if app.terminal_visible {
            panel_height
        } else {
            0
        }),
        Constraint::Length(if output_visible { panel_height } else { 0 }),
    ])
    .split(columns[1]);
    draw_editors(frame, app, panes[0]);
    app.terminal_area = Rect::default();
    if app.terminal_visible {
        draw_terminal(frame, app, panes[1]);
    }
    app.extension_surfaces.output_area = Rect::default();
    if output_visible {
        draw_extension_output(frame, app, panes[2]);
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
            " VSCLI  |  No open editors  |  {}{}",
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
    draw_extension_status(frame, app, rows[3]);
    draw_signature(frame, app);
    draw_suggestions(frame, app);
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
    if app.outline_view().status != OutlineStatus::Hidden && area.height >= 6 {
        let height = (area.height / 3).clamp(4, 12).min(area.height - 2);
        let sections =
            Layout::vertical([Constraint::Min(2), Constraint::Length(height)]).split(area);
        draw_explorer_files(frame, app, sections[0]);
        draw_outline(frame, app, sections[1]);
    } else {
        draw_explorer_files(frame, app, area);
    }
}

fn draw_outline(frame: &mut Frame, app: &mut App, area: Rect) {
    let colors = app.theme.colors;
    let focused = app.focus == Focus::Outline;
    let block = Block::default()
        .borders(Borders::RIGHT | Borders::TOP)
        .title(if focused {
            " OUTLINE • "
        } else {
            " OUTLINE "
        })
        .border_style(Style::default().fg(if focused { colors.accent } else { colors.muted }))
        .style(Style::default().bg(colors.panel));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
    app.outline_area = rows[1];
    let view = app.outline_view();
    let status = match view.status {
        OutlineStatus::Loading => "Loading symbols…",
        OutlineStatus::Updating => "Updating symbols…",
        OutlineStatus::Error => "Symbols unavailable",
        OutlineStatus::NoDocument => "No open document",
        OutlineStatus::Unsupported => "No symbol provider",
        _ if view.source.is_empty() => "Document symbols",
        _ => view.source,
    };
    frame.render_widget(
        Paragraph::new(clean(status)).style(Style::default().fg(colors.muted)),
        rows[0],
    );
    let Some(tree) = view.tree else {
        frame.render_widget(
            Paragraph::new(clean(view.message))
                .style(Style::default().fg(colors.muted))
                .wrap(Wrap { trim: false }),
            rows[1],
        );
        return;
    };
    if view.visible.is_empty() {
        frame.render_widget(
            Paragraph::new("No symbols").style(Style::default().fg(colors.muted)),
            rows[1],
        );
        return;
    }
    let offset = app.outline_offset(rows[1].height);
    let lines: Vec<Line> = view
        .visible
        .iter()
        .skip(offset)
        .take(rows[1].height as usize)
        .map(|index| {
            let node = &tree.nodes[*index];
            let branch = tree
                .nodes
                .get(index + 1)
                .is_some_and(|child| child.parent == Some(*index));
            let expanded = branch && view.visible.contains(&(index + 1));
            let marker = if branch {
                if expanded { "▾" } else { "▸" }
            } else {
                " "
            };
            let kind = match node.kind {
                2..=4 => "◈",
                5 | 11 | 23 => "◇",
                6 | 9 | 12 => "ƒ",
                7 | 8 | 13 | 14 => "·",
                _ => "○",
            };
            let mut style = Style::default().fg(if view.actionable {
                if view.active == Some(*index) {
                    colors.accent
                } else {
                    colors.foreground
                }
            } else {
                colors.muted
            });
            if focused && view.selected == Some(*index) {
                style = style.bg(colors.selection);
            }
            if node.deprecated {
                style = style.add_modifier(Modifier::CROSSED_OUT);
            }
            Line::styled(
                format!(
                    "{}{} {} {}",
                    " ".repeat(node.depth as usize),
                    marker,
                    kind,
                    clean(&node.name)
                ),
                style,
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), rows[1]);
}

fn draw_explorer_files(frame: &mut Frame, app: &mut App, area: Rect) {
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

fn draw_editors(frame: &mut Frame, app: &mut App, area: Rect) {
    let colors = app.theme.colors;
    app.sync_pane();
    if app.documents.is_empty() {
        welcome::draw(frame, app, area);
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
    let extension_title = if let PromptKind::Extension(ref request) = p.kind {
        format!(
            " Extension {} · {} · {} · Esc cancels ",
            if request.spec.kind == crate::extensions::PromptType::QuickPick {
                "Quick Pick"
            } else {
                "Input Box"
            },
            clean(&request.spec.owner),
            clean(&request.spec.title)
        )
    } else {
        String::new()
    };
    let title = match p.kind {
        PromptKind::Extension(_) => &extension_title,
        PromptKind::InstallExtension => {
            " Install Extension from local VSIX · path · Enter installs without running code "
        }
        PromptKind::Palette => " Command Palette ",
        PromptKind::QuickOpen => " Go to File ",
        PromptKind::Symbols => {
            if app.symbols_workspace() {
                " Go to Symbol in Workspace · server query "
            } else {
                " Go to Symbol in Editor "
            }
        }
        PromptKind::RecentFiles => " Open Recent File · file history only ",
        PromptKind::Snippet => " Insert Snippet · name, prefix or description ",
        PromptKind::Theme => " Color Theme · select or Load Color Theme File from commands ",
        PromptKind::SearchExtensions => " Search Open VSX · Enter searches · Esc cancels ",
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
    let list = matches!(&p.kind, PromptKind::Extension(request) if request.spec.kind == crate::extensions::PromptType::QuickPick)
        || matches!(
            p.kind,
            PromptKind::Palette
                | PromptKind::QuickOpen
                | PromptKind::Symbols
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
    if let PromptKind::Extension(ref request) = p.kind
        && inner.height > 1
    {
        frame.render_widget(
            Paragraph::new(clean(&format!(
                "{}  {}",
                request.spec.prompt, request.spec.place_holder
            ))),
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );
    }
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
        let items: Vec<String> = if let PromptKind::Extension(ref request) = p.kind {
            request
                .matches(&p.text)
                .into_iter()
                .map(|(_, item)| format!("{}  {}  {}", item.label, item.description, item.detail))
                .collect()
        } else if matches!(p.kind, PromptKind::Palette) {
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
        } else if matches!(p.kind, PromptKind::Symbols) {
            app.symbol_items(&p.text)
                .iter()
                .map(|s| s.label.clone())
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
fn draw_modal(frame: &mut Frame, app: &mut App) {
    let colors = app.theme.colors;
    let (title, text) = match app.modal.as_ref().unwrap() {
        Modal::ExtensionSurfaces(_) | Modal::ExtensionTree => {
            draw_extension_modal(frame, app);
            return;
        }
        Modal::ExtensionsLoading(_) => (
            " Extensions · Loading · Esc closes ",
            "Reading installed packages…".into(),
        ),
        Modal::Help => (
            " Getting Started · Esc to close ",
            format!(
                "VSCLI 0.1 · Native terminal editor\n\n{}  Command palette\n{}  Quick open (respects ignore files)\n{}  Open a path or create a file\n{}  Save    {}  Save As\n{}  Find    {}  Go to line\n{}  Toggle explorer\nCtrl+PageUp / Ctrl+PageDown  Switch tabs\nShift+arrows  Select · Mouse drag selects\n\nExplorer: arrows, Enter to open, Left for parent, Esc for editor.\n\nRecovery snapshots are written every two seconds.\nUTF-8 files up to 32 MiB. Installed clangd / rust-analyzer start automatically.\nUse --lsp PROGRAM for a custom server or --no-lsp to disable.\nCtrl+D adds occurrences · Ctrl+Shift+F searches files.\nF1 → Extensions: Install from VSIX / Show Installed Extensions.\nCode extensions use an optional experimental Node host.\nF1 → Keyboard Inspector shows what your terminal sends.\nEnhanced shortcuts require a compatible terminal configuration.\n\n{}  Exit (unsaved changes are protected)",
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
        Modal::ExtensionRegistry { items, selected } => {
            let inner = popup(
                frame,
                colors,
                " Open VSX · Enter installs displayed version · / search again · Esc closes ",
                110,
                frame.area().height.saturating_sub(2),
            );
            let rows = inner.height.saturating_sub(4) as usize;
            let offset = selected.saturating_sub(rows.saturating_sub(1));
            let mut lines = vec![Line::raw(clean(&app.message))];
            if items.is_empty() {
                lines.push(Line::raw("No matching packages or updates."));
            }
            for (index, item) in items.iter().enumerate().skip(offset).take(rows) {
                lines.push(Line::styled(
                    format!(
                        "{}@{}  {}",
                        clean(&item.id),
                        clean(&item.version),
                        clean(&item.name)
                    ),
                    if index == *selected {
                        Style::default().fg(colors.foreground).bg(colors.selection)
                    } else {
                        Style::default().fg(colors.foreground)
                    },
                ));
            }
            if let Some(item) = items.get(*selected) {
                lines.push(Line::raw(format!(
                    "{} · {}",
                    clean(&item.platform),
                    clean(&item.license)
                )));
                lines.push(Line::raw(clean(&item.description)));
            }
            frame.render_widget(Paragraph::new(lines), inner);
            return;
        }
        Modal::Extensions { items, selected } => {
            let inner = popup(
                frame,
                colors,
                " Installed Extensions · e/d global · E/D workspace · Enter run once · / search · U updates ",
                120,
                frame.area().height.saturating_sub(2),
            );
            let mut session_lines: Vec<_> = app
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
            session_lines.insert(0, Line::raw(clean(&app.message)));
            session_lines.insert(
                1,
                Line::raw("S stop selected · H restart session · R rollback · Delete remove"),
            );
            session_lines.insert(1, Line::raw(app.language_configuration_catalog_status()));
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
                                "{}@{}  {} · {} · {}",
                                clean(&item.id),
                                clean(&item.version),
                                clean(&app.extension_status(&item.id)),
                                clean(&app.installed_extension_activation_status(item)),
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
        Modal::Inspector => {
            keyboard::draw(frame, app);
            return;
        }
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

fn draw_suggestions(frame: &mut Frame, app: &App) {
    let Some(model) = app.suggestion_model() else {
        return;
    };
    let doc = app.doc();
    let area = app.editor_area;
    if area.width < 12
        || area.height < 4
        || doc.row() < doc.top
        || doc.row() >= doc.top + area.height as usize
    {
        return;
    }
    let column = doc.visual_column();
    if column < doc.left || column >= doc.left + area.width as usize {
        return;
    }
    let caret_x = area.x + (column - doc.left) as u16;
    let caret_y = area.y + (doc.row() - doc.top) as u16;
    let selected = model.selected_item();
    let details = selected
        .is_some_and(|item| !item.documentation.is_empty() || !item.detail.is_empty())
        || app.suggestion_resolving();
    let (popup, detail_area) = suggestion_layout(area, caret_x, caret_y, model.len(), details);
    let height = popup.height;
    let colors = app.theme.colors;
    let first = model
        .selected()
        .saturating_sub(height.saturating_sub(3) as usize);
    let lines: Vec<_> = (first..model.len())
        .take(height.saturating_sub(2) as usize)
        .filter_map(|index| {
            model.item(index).map(|item| {
                Line::styled(
                    format!("{}  {}", clean(&item.label), clean(&item.detail)),
                    Style::default()
                        .fg(colors.foreground)
                        .bg(if index == model.selected() {
                            colors.selection
                        } else {
                            colors.panel
                        }),
                )
            })
        })
        .collect();
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().fg(colors.foreground).bg(colors.panel))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(if app.suggestion_acceptable() {
                        " Suggestions · Tab accepts "
                    } else {
                        " Suggestions · updating "
                    }),
            ),
        popup,
    );
    if let Some(area) = detail_area {
        let item = selected.unwrap();
        let mut lines = Vec::new();
        if !item.detail.is_empty() {
            lines.push(Line::styled(
                clean(&item.detail),
                Style::default().fg(colors.accent),
            ));
        }
        // The retained source is bounded, and rendering visits only this small
        // preview. Markdown remains inert text; terminal escapes are sanitized.
        let preview: String = item.documentation.chars().take(8192).collect();
        lines.extend(preview.lines().take(64).map(|line| Line::raw(clean(line))));
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(colors.foreground).bg(colors.panel))
                .block(Block::default().borders(Borders::ALL).title(
                    if app.suggestion_resolving() {
                        " Details · resolving "
                    } else {
                        " Details "
                    },
                )),
            area,
        );
    }
}

fn suggestion_layout(
    area: Rect,
    caret_x: u16,
    caret_y: u16,
    count: usize,
    details: bool,
) -> (Rect, Option<Rect>) {
    let list_height = (count.min(8) as u16 + 2).min(area.height);
    let side = details && area.width >= 90;
    let stacked = details && !side && area.width >= 40 && area.height >= list_height + 4;
    let width = if side {
        area.width.min(108)
    } else {
        area.width.min(64)
    };
    let detail_height = if stacked {
        (area.height - list_height).min(7)
    } else {
        area.height.min(10)
    };
    let height = if side {
        list_height.max(detail_height)
    } else if stacked {
        list_height + detail_height
    } else {
        list_height
    };
    let x = caret_x.min(area.right().saturating_sub(width)).max(area.x);
    let y = if caret_y.saturating_add(height).saturating_add(1) <= area.bottom() {
        caret_y + 1
    } else {
        caret_y.saturating_sub(height).max(area.y)
    };
    let list_width = if side { 60 } else { width };
    let popup = Rect::new(x, y, list_width, list_height);
    let detail = if side {
        Some(Rect::new(
            x + list_width,
            y,
            width - list_width,
            detail_height,
        ))
    } else if stacked {
        Some(Rect::new(x, y + list_height, width, detail_height))
    } else {
        None
    };
    (popup, detail)
}

fn draw_signature(frame: &mut Frame, app: &App) {
    let Some(hint) = app.signature_help() else {
        return;
    };
    let area = app.editor_area;
    if area.width < 12 || area.height < 4 {
        return;
    }
    let colors = app.theme.colors;
    let mut label = Vec::new();
    if let Some(range) = &hint.parameter {
        label.push(Span::raw(clean(&hint.label[..range.start])));
        label.push(Span::styled(
            clean(&hint.label[range.clone()]),
            Style::default()
                .fg(colors.accent)
                .add_modifier(Modifier::BOLD),
        ));
        label.push(Span::raw(clean(&hint.label[range.end..])));
    } else {
        label.push(Span::raw(clean(&hint.label)));
    }
    let mut lines = vec![Line::from(label)];
    lines.extend(
        clean_multiline(&hint.documentation)
            .lines()
            .take(4)
            .map(|line| Line::raw(line.to_owned())),
    );
    let height = (lines.len() as u16 + 2).min(area.height).min(7);
    let popup = Rect::new(area.x, area.y + area.height - height, area.width, height);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(colors.foreground).bg(colors.panel))
            .block(Block::default().borders(Borders::ALL).title(format!(
                " Parameter Hints {}/{} · {}Esc closes ",
                hint.signature + 1,
                hint.count,
                if hint.count > 1 {
                    "↑/↓ overload · "
                } else {
                    ""
                }
            ))),
        popup,
    );
}

fn draw_extension_output(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(channel) = app
        .extension_surfaces
        .output
        .as_ref()
        .and_then(|k| app.extension_host.as_ref()?.surfaces.channels.get(k))
    else {
        return;
    };
    let colors = app.theme.colors;
    let block = Block::default()
        .borders(Borders::TOP)
        .title(format!(
            " Output: {} · read-only · Esc closes ",
            clean(&channel.name)
        ))
        .border_style(Style::default().fg(if app.focus == Focus::Output {
            colors.accent
        } else {
            colors.muted
        }));
    let inner = block.inner(area);
    frame.render_widget(block.style(Style::default().bg(colors.panel)), area);
    app.extension_surfaces.output_area = area;
    let count = channel.text.len_lines();
    let height = inner.height as usize;
    app.extension_surfaces.output_scroll = app
        .extension_surfaces
        .output_scroll
        .min(count.saturating_sub(height));
    let start = count
        .saturating_sub(height)
        .saturating_sub(app.extension_surfaces.output_scroll);
    let lines: Vec<_> = channel
        .text
        .lines_at(start)
        .take(height)
        .map(|line| {
            Line::from(clean(
                line.chars()
                    .take((inner.width as usize).saturating_mul(4).max(1))
                    .collect::<String>()
                    .trim_end_matches(['\r', '\n']),
            ))
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(colors.foreground).bg(colors.panel)),
        inner,
    );
}
fn draw_extension_status(frame: &mut Frame, app: &mut App, area: Rect) {
    app.extension_surfaces.status_hits.clear();
    let Some(host) = &app.extension_host else {
        return;
    };
    let colors = app.theme.colors;
    let mut items: Vec<_> = host
        .surfaces
        .statuses
        .values()
        .filter(|v| v.visible && !v.text.is_empty())
        .collect();
    items.sort_by(|a, b| {
        a.alignment
            .cmp(&b.alignment)
            .then_with(|| {
                if a.alignment == 2 {
                    a.priority.total_cmp(&b.priority)
                } else {
                    b.priority.total_cmp(&a.priority)
                }
            })
            .then_with(|| a.key.cmp(&b.key))
    });
    // Reserve half the row for native document/language/cursor status.
    let start = area.x + area.width / 2;
    let mut left = start;
    let mut right = area.x + area.width;
    for item in items {
        let text = format!(" {} ", clean(&item.text));
        let width = (unicode_width::UnicodeWidthStr::width(text.as_str()) as u16)
            .min(right.saturating_sub(left));
        if width == 0 {
            break;
        }
        let x = if item.alignment == 2 {
            right -= width;
            right
        } else {
            let x = left;
            left += width;
            x
        };
        let rect = Rect::new(x, area.y, width, 1);
        let foreground = if item.color.starts_with('#') {
            u32::from_str_radix(item.color.trim_start_matches('#'), 16)
                .ok()
                .map(|rgb| Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8))
                .unwrap_or(colors.foreground)
        } else {
            colors.foreground
        };
        let background = match item.background.as_str() {
            "statusBarItem.errorBackground" => Color::Red,
            "statusBarItem.warningBackground" => Color::Yellow,
            _ => colors.selection,
        };
        frame.render_widget(
            Paragraph::new(text).style(Style::default().fg(foreground).bg(background)),
            rect,
        );
        if item.has_command {
            app.extension_surfaces.status_hits.push((
                rect,
                item.key.clone(),
                item.generation,
                host.session,
            ));
        }
    }
}
fn draw_extension_modal(frame: &mut Frame, app: &mut App) {
    let colors = app.theme.colors;
    app.extension_surfaces.presented_tree = if matches!(app.modal, Some(Modal::ExtensionTree)) {
        app.extension_surfaces.tree.as_ref().and_then(|key| {
            app.extension_host.as_ref().and_then(|host| {
                host.surfaces.trees.get(key).map(|tree| {
                    crate::app::extension_surfaces::TreePresentation {
                        session: host.session,
                        key: key.clone(),
                        generation: tree.generation,
                        rows: app.surface_tree_rows(),
                    }
                })
            })
        })
    } else {
        None
    };

    let (title, selected, rows) = match app.modal.as_ref().unwrap() {
        Modal::ExtensionSurfaces(picker) => {
            let title = match picker.kind {
                crate::app::extension_surfaces::PickerKind::Output => {
                    " Output Channels · Enter opens · Esc closes "
                }
                crate::app::extension_surfaces::PickerKind::Status => {
                    " Extension Status Items · Enter runs · Esc closes "
                }
                crate::app::extension_surfaces::PickerKind::Trees => {
                    " Extension Tree Views · Enter opens · Esc closes "
                }
            };
            (
                title.to_owned(),
                picker.selected,
                picker
                    .items
                    .iter()
                    .map(|i| clean(&i.label))
                    .collect::<Vec<_>>(),
            )
        }
        Modal::ExtensionTree => {
            let Some(tree) = app
                .extension_surfaces
                .tree
                .as_ref()
                .and_then(|k| app.extension_host.as_ref()?.surfaces.trees.get(k))
            else {
                return;
            };
            let rows = app
                .surface_tree_rows()
                .into_iter()
                .filter_map(|(id, depth)| {
                    tree.nodes.get(&id).map(|node| {
                        format!(
                            "{}{} {} {}",
                            "  ".repeat(depth),
                            if node.collapsible == 0 {
                                " "
                            } else if app.extension_surfaces.expanded.contains(&id)
                                || (node.collapsible == 2
                                    && !app.extension_surfaces.collapsed.contains(&id))
                            {
                                "▾"
                            } else {
                                "▸"
                            },
                            clean(&node.label),
                            clean(&node.description)
                        )
                    })
                })
                .collect();
            (
                format!(
                    " {} · ←/→ expand · Enter runs · R retries · Esc closes ",
                    clean(&tree.title)
                ),
                app.extension_surfaces.selected,
                rows,
            )
        }
        _ => return,
    };
    let inner = popup(frame, colors, &title, 110, 24);
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new("No items yet · loading or empty · R retries tree requests")
                .style(Style::default().fg(colors.muted)),
            inner,
        );
        return;
    }
    let selected = selected.min(rows.len().saturating_sub(1));
    let offset = selected.saturating_sub((inner.height as usize).saturating_sub(1));
    let lines: Vec<_> = rows
        .into_iter()
        .enumerate()
        .skip(offset)
        .take(inner.height as usize)
        .map(|(i, line)| {
            Line::styled(
                line,
                Style::default().fg(colors.foreground).bg(if i == selected {
                    colors.selection
                } else {
                    colors.panel
                }),
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    #[test]
    fn completion_details_fit_editor_at_all_caret_edges_and_terminal_sizes() {
        for (width, height) in [(12, 4), (39, 24), (48, 20), (76, 24), (100, 30), (160, 40)] {
            let area = Rect::new(7, 3, width, height);
            for x in [area.x, area.right() - 1] {
                for y in [area.y, area.bottom() - 1] {
                    for count in [1, 8, 300] {
                        let (list, details) = suggestion_layout(area, x, y, count, true);
                        assert_eq!(area.intersection(list), list);
                        if let Some(details) = details {
                            assert_eq!(area.intersection(details), details);
                            assert_eq!(list.intersection(details).area(), 0);
                            assert!(details.width >= 30);
                            assert!(details.height >= 4);
                        }
                    }
                }
            }
        }
    }
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
        // Long platform temporary prefixes must not hide the distinguishing parent.
        let workspace = root.path().join("long-temporary-root-prefix-".repeat(4));
        std::fs::create_dir(&workspace).unwrap();
        let mut app = App::new(workspace.clone(), crate::keys::Profile::Linux);
        app.keymap.load(&keys).unwrap();
        app.theme.colors.accent = Color::Rgb(1, 2, 3);
        app.recent_files.touch(workspace.join("left/same.txt"));
        app.recent_files.touch(workspace.join("right/same.txt"));
        let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let cells = &terminal.backend().buffer().content;
        let text: String = cells.iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("Open Recent File") && text.contains("f8"));
        assert!(text.contains("same.txt") && text.contains("left") && text.contains("right"));
        assert!(
            cells
                .iter()
                .any(|cell| cell.symbol() == "V" && cell.fg == Color::Rgb(1, 2, 3))
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
    fn outline_no_document_and_hidden_surfaces_never_leave_mouse_targets() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(dir.path().into(), crate::keys::Profile::Linux);
        app.execute("outline.focus", serde_json::Value::Null);
        let mut terminal = Terminal::new(TestBackend::new(110, 32)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("OUTLINE"));
        assert!(text.contains("No open document"));
        assert!(app.documents.is_empty());
        assert!(app.outline_area.height > 0);
        for (width, height) in [(40, 10), (1, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            assert_eq!(app.outline_area, Rect::default());
            assert!(app.documents.is_empty());
        }
        app.modal = Some(Modal::Inspector);
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        assert_eq!(app.outline_area, Rect::default());
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
