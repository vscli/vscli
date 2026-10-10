//! Nested drawing uses the same bounded geometry that authorizes pane input.
use super::*;
use crate::{
    editor_layout::{Axis, Geometry, MAX_LEAVES},
    editor_presentation::PaneRects,
};
use anyhow::{Context, Result, ensure};

/// Stage model/group lookups before displaying any alternate Document view.
/// Topology and view creation belong to App; rendering never reconciles membership.
pub(super) fn draw(frame: &mut Frame, app: &mut App, area: Rect) -> Result<()> {
    let geometry = app.project_editor_layout(area)?;
    let active_document = app.active;
    let active_group = app
        .editor_groups()
        .active_group()
        .context("Editor layout has no active group")?;
    let active_view = active_group.value();
    let mut targets = [None; MAX_LEAVES];
    ensure!(
        app.panes.len() == app.editor_groups().groups().len(),
        "Pane projection is stale"
    );
    for (index, group) in app.editor_groups().groups().iter().enumerate() {
        ensure!(index < MAX_LEAVES, "Editor layout exceeds four groups");
        let member = group
            .active()
            .context("Editor layout contains an empty group")?;
        let placement =
            geometry.placements()[index].context("Editor layout is missing a placement")?;
        let pane = app
            .panes
            .get(index)
            .context("Editor layout is missing a pane")?;
        ensure!(
            placement.group == group.id()
                && pane.id == group.id().value()
                && pane.document == member.document(),
            "Editor pane model identity differs"
        );
        let document = app
            .documents
            .iter()
            .position(|doc| doc.id == member.document())
            .context("Editor pane model is not retained")?;
        targets[index] = Some((group.id(), document));
    }
    ensure!(
        app.documents.get(active_document).is_some_and(|doc| app
            .editor_groups()
            .active_membership()
            .is_some_and(|member| member.document == doc.id)),
        "Active document differs from active editor group"
    );
    let show_breadcrumbs = app.breadcrumbs_view().visible;
    let mut rectangles = [None; MAX_LEAVES];
    // All possible failure paths above precede alternate-view presentation.
    // `reserve` checks only geometry already validated by the allocator.
    for (index, target) in targets.iter().enumerate() {
        let Some((group, _)) = target else {
            continue;
        };
        let placement = geometry.placements()[index].context("Editor placement disappeared")?;
        rectangles[index] = Some(PaneRects::reserve(
            *group,
            placement.outer,
            *group == active_group && show_breadcrumbs,
        )?);
    }
    app.pane_areas.clear();
    for (index, target) in targets.iter().enumerate() {
        let Some((group, document)) = target else {
            continue;
        };
        let mut pane = rectangles[index].context("Editor rectangles disappeared")?;
        if pane.outer.width != 0 && pane.outer.height != 0 {
            app.active = *document;
            app.doc_mut().display_view(group.value());
            group_tabs::draw(frame, app, pane.strip, group.value(), index);
            if pane.breadcrumbs.width != 0 && pane.breadcrumbs.height != 0 {
                draw_breadcrumbs(frame, app, pane.breadcrumbs);
            }
            draw_editor(frame, app, pane.text, *group == active_group);
            // This captures line-number/gutter clipping rather than guessing it.
            pane.text = app.editor_area;
        }
        rectangles[index] = Some(pane);
        app.pane_areas.push(pane.text);
    }
    app.active = active_document;
    app.doc_mut().display_view(active_view);
    app.editor_area = rectangles
        .iter()
        .flatten()
        .find(|pane| pane.group == active_group)
        .map_or(Rect::default(), |pane| pane.text);
    draw_dividers(frame, &geometry, app.theme.colors.muted);
    // Only a fully drawn, restored frame can grant input authorization.
    app.present_editor_layout(geometry, rectangles)?;
    Ok(())
}

/// Overflow is an explicit recovery presentation, not an invented layout leaf.
/// Keep the active authoritative model/view and all retained models untouched.
/// No pane/tab/divider pointer map is granted by this drawer.
pub(super) fn draw_recovery(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(document) = app.active_document() else {
        return;
    };
    let label = group_tabs::bounded_label(&document.name());
    let dirty = document.dirty();
    let colors = app.theme.colors;
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    frame.render_widget(
        Paragraph::new(format!(
            " Recovery {}/{} · {label}{} · keyboard navigation ",
            app.active + 1,
            app.documents.len(),
            if dirty { " ●" } else { "" }
        ))
        .style(Style::default().bg(colors.panel).fg(colors.foreground)),
        rows[0],
    );
    draw_editor(frame, app, rows[1], true);
}

fn draw_dividers(frame: &mut Frame, geometry: &Geometry, color: Color) {
    let style = Style::default().fg(color);
    for divider in geometry.dividers().iter().flatten() {
        let glyph = match divider.axis {
            Axis::Columns => "│",
            Axis::Rows => "─",
        };
        for y in divider.rect.y..divider.rect.bottom() {
            for x in divider.rect.x..divider.rect.right() {
                if let Some(cell) = frame.buffer_mut().cell_mut((x, y)) {
                    cell.set_symbol(glyph).set_style(style);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        editor_groups::Groups,
        editor_layout::{Direction, Layout as EditorLayout},
    };
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn nested_divider_paint_matches_exact_allocator_cells_without_overwriting_panes() {
        let mut groups = Groups::default();
        groups.open(1).unwrap();
        let first = groups.active_group().unwrap();
        let mut layout = EditorLayout::default();
        let initial = layout.prepare_flat(&[first], Axis::Columns).unwrap();
        layout.commit(initial).unwrap();
        groups.split_active().unwrap();
        let second = groups.active_group().unwrap();
        let right = layout
            .prepare_split(first, second, Direction::Right)
            .unwrap();
        layout.commit(right).unwrap();
        groups.split_active().unwrap();
        let third = groups.active_group().unwrap();
        let down = layout
            .prepare_split(second, third, Direction::Down)
            .unwrap();
        layout.commit(down).unwrap();
        let geometry = layout
            .project(Rect::new(2, 1, 67, 20), groups.active_group())
            .unwrap();
        let mut terminal = Terminal::new(TestBackend::new(72, 24)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    Block::default().style(Style::default().bg(Color::Black)),
                    frame.area(),
                );
                for placement in geometry.placements().iter().flatten() {
                    frame.render_widget(
                        Paragraph::new("editor").style(Style::default().fg(Color::White)),
                        placement.outer,
                    );
                }
                draw_dividers(frame, &geometry, Color::DarkGray);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        for y in 0..24 {
            for x in 0..72 {
                let expected =
                    geometry.dividers().iter().flatten().find(|divider| {
                        divider.rect.contains(ratatui::layout::Position::new(x, y))
                    });
                let symbol = buffer[(x, y)].symbol();
                if let Some(divider) = expected {
                    assert_eq!(
                        symbol,
                        if divider.axis == Axis::Columns {
                            "│"
                        } else {
                            "─"
                        }
                    );
                } else {
                    assert_ne!(symbol, "│");
                    assert_ne!(symbol, "─");
                }
            }
        }
    }
}
