//! Group topology is staged with membership; cell geometry never owns documents.
use super::*;
use crate::{
    editor_groups::{Change, GroupId, Groups, Membership, OpenMode},
    editor_layout::{Axis, Direction, Geometry, Layout, Plan, SavedNode},
    editor_presentation::{GeometryProof, PaneRects},
};
use anyhow::{Context, ensure};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct GeometryInputs {
    sidebar: bool,
    terminal: bool,
    output: bool,
    modal: bool,
    prompt: bool,
    breadcrumbs: bool,
    panes: [Option<PaneInput>; 4],
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PaneInput {
    group: GroupId,
    document: u64,
    gutter_digits: u16,
    tab_size: usize,
    text_epoch: u64,
    top: usize,
    left: usize,
}

const RESIZE_DEADLINE: Duration = Duration::from_secs(2);

pub(super) struct PendingResize {
    command: &'static str,
    axis: Axis,
    delta: i16,
    member: Membership,
    groups: crate::editor_groups::UiProof,
    layout: Geometry,
    started: Instant,
}

impl App {
    pub(crate) fn editor_group_overflow(&self) -> bool {
        self.group_fallback
    }

    pub fn editor_layout(&self) -> &Layout {
        &self.editor_layout
    }

    pub fn project_editor_layout(&self, area: Rect) -> Result<Geometry> {
        ensure!(
            !self.group_fallback,
            "Nested layout unavailable in recovery overflow"
        );
        self.editor_layout
            .validate(&group_ids(&self.editor_groups))?;
        self.editor_layout
            .project(area, self.editor_groups.active_group())
    }

    pub(crate) fn present_editor_layout(
        &mut self,
        geometry: Geometry,
        panes: [Option<PaneRects>; 4],
    ) -> Result<GeometryProof> {
        // All maps were drawn in this frame. Retire the old sealed proof on
        // draw-time viewport clamps without clearing those freshly drawn maps.
        let next = self.editor_geometry_inputs();
        if self.editor_geometry_inputs.is_some_and(|old| old != next) {
            self.editor_presentation.invalidate()?;
        }
        self.editor_geometry_inputs = Some(next);
        self.editor_presentation
            .present(geometry, &self.editor_groups, panes)
    }

    /// Fixed-size shape and displayed-content inputs; no Rope/selection clones.
    fn editor_geometry_inputs(&self) -> GeometryInputs {
        let panes = std::array::from_fn(|index| {
            let group = self.editor_groups.groups().get(index)?;
            let document = group.active()?.document();
            let doc = self.documents.iter().find(|doc| doc.id == document)?;
            let digits = if doc.line_numbers == crate::settings::LineNumbers::Off {
                0
            } else {
                (doc.line_count().max(1).ilog10() + 1).max(3) as u16
            };
            let view = doc.view_state(Some(group.id().value()));
            Some(PaneInput {
                group: group.id(),
                document,
                gutter_digits: digits,
                tab_size: doc.tab_size,
                text_epoch: doc.text_epoch(),
                top: view.top,
                left: view.left,
            })
        });
        GeometryInputs {
            sidebar: self.sidebar,
            terminal: self.terminal_visible,
            output: self.extension_surfaces.output.is_some(),
            modal: self.modal.is_some(),
            prompt: self.prompt.is_some(),
            breadcrumbs: self.breadcrumbs_view().visible,
            panes,
        }
    }

    /// Retire pointer authorization before any later event in the same batch.
    pub(super) fn observe_editor_geometry(&mut self) -> bool {
        let next = self.editor_geometry_inputs();
        let changed = self.editor_geometry_inputs.is_some_and(|old| old != next);
        if changed {
            self.invalidate_editor_presentation();
        }
        self.editor_geometry_inputs = Some(next);
        changed
    }

    pub(super) fn editor_geometry_current(&self) -> bool {
        self.editor_presentation.proof().is_some_and(|proof| {
            self.editor_presentation
                .current(proof, &self.editor_layout, &self.editor_groups)
        })
    }

    /// Terminal-cell resizing owns no document edit. A palette command may
    /// wait once for a real repaint after its overlay retired pointer geometry.
    pub(super) fn execute_editor_layout_command(&mut self, command: &str) -> bool {
        let resize = match command {
            "workbench.action.increaseViewWidth" => {
                Some(("workbench.action.increaseViewWidth", Axis::Columns, 4))
            }
            "workbench.action.decreaseViewWidth" => {
                Some(("workbench.action.decreaseViewWidth", Axis::Columns, -4))
            }
            "workbench.action.increaseViewHeight" => {
                Some(("workbench.action.increaseViewHeight", Axis::Rows, 2))
            }
            "workbench.action.decreaseViewHeight" => {
                Some(("workbench.action.decreaseViewHeight", Axis::Rows, -2))
            }
            _ => None,
        };
        let reset = command == "workbench.action.evenEditorWidths";
        if resize.is_none() && !reset {
            return false;
        }
        if self.pending_editor_resize.is_some() {
            self.message = "An editor resize is already waiting for repaint".into();
            return true;
        }
        let result = (|| -> Result<Option<bool>> {
            ensure!(
                !self.group_fallback,
                "Layout resizing unavailable in recovery overflow"
            );
            let member = self
                .editor_groups
                .active_membership()
                .context("No editor group to resize")?;
            if let Some((command, axis, delta)) = resize {
                ensure!(
                    self.focus == Focus::Editor,
                    "Focus an editor group to resize"
                );
                if !self.editor_geometry_current() {
                    self.editor_layout
                        .validate(&group_ids(&self.editor_groups))?;
                    let layout = self
                        .editor_layout
                        .project(Rect::default(), Some(member.group))?;
                    self.pending_editor_resize = Some(PendingResize {
                        command,
                        axis,
                        delta,
                        member,
                        groups: self.editor_groups.proof(),
                        layout,
                        started: Instant::now(),
                    });
                    return Ok(None);
                }
                return self
                    .apply_editor_resize(member.group, axis, delta)
                    .map(Some);
            }
            let plan = self.editor_layout.prepare_reset()?;
            self.commit_editor_layout_plan(plan).map(Some)
        })();
        match result {
            Ok(None) => self.message = "Editor resize queued for the next editor repaint".into(),
            Ok(Some(true)) => self.message = "Editor layout resized".into(),
            Ok(Some(false)) => self.message = "Editor layout unchanged".into(),
            Err(error) => self.message = format!("Editor layout resize rejected: {error:#}"),
        }
        true
    }

    fn apply_editor_resize(&mut self, group: GroupId, axis: Axis, delta: i16) -> Result<bool> {
        ensure!(
            self.editor_geometry_current(),
            "Editor geometry changed; redraw before resizing"
        );
        let geometry = self
            .editor_presentation
            .geometry()
            .context("Editor geometry is not drawn")?;
        let plan = self
            .editor_layout
            .prepare_resize_group(group, axis, delta, geometry)?;
        self.commit_editor_layout_plan(plan)
    }

    /// One captured intent, consumed only after an actual editor seal.
    /// Logical expiry consumes metadata; no worker/request capacity is fabricated.
    pub(super) fn poll_editor_resize(&mut self) -> bool {
        let Some(pending) = self.pending_editor_resize.as_ref() else {
            return false;
        };
        let current = !self.group_fallback
            && self.focus == Focus::Editor
            && self.editor_groups.proof_current(&pending.groups)
            && self.editor_groups.active_membership() == Some(pending.member)
            && self.editor_layout.geometry_current(&pending.layout);
        let expired = pending.started.elapsed() >= RESIZE_DEADLINE;
        if !current || expired {
            let command = pending.command;
            self.pending_editor_resize = None;
            self.message = format!(
                "Editor resize retired ({command}): {}",
                if expired {
                    "no editor repaint before deadline"
                } else {
                    "the original group or layout changed"
                }
            );
            return true;
        }
        if self.prompt.is_some() || self.modal.is_some() || !self.editor_geometry_current() {
            return false;
        }
        let pending = self
            .pending_editor_resize
            .take()
            .expect("resize intent was checked");
        match self.apply_editor_resize(pending.member.group, pending.axis, pending.delta) {
            Ok(true) => self.message = "Editor layout resized".into(),
            Ok(false) => self.message = "Editor layout unchanged".into(),
            Err(error) => self.message = format!("Editor layout resize rejected: {error:#}"),
        }
        true
    }

    /// Stage a topology update before any model/view/closed-history publication.
    /// All runtime group identities come from the already-staged Groups engine.
    pub(super) fn prepare_group_layout(
        &self,
        groups: &Groups,
        change: &Change,
        split: Option<Direction>,
    ) -> Result<Layout> {
        ensure!(
            !self.group_fallback,
            "Nested layout unavailable in recovery overflow"
        );
        self.editor_layout
            .validate(&group_ids(&self.editor_groups))?;
        let mut next = self.editor_layout.clone();
        for group in &change.removed_groups {
            let plan = next.prepare_remove(*group)?;
            next.commit(plan)?;
        }
        for group in &change.created_groups {
            let plan = if next.groups().is_empty() {
                next.prepare_flat(&[*group], Axis::Columns)?
            } else {
                let direction = split
                    .context("Creating another editor group requires an explicit local split")?;
                ensure!(
                    matches!(direction, Direction::Right | Direction::Down),
                    "Left/Up group splitting is not supported yet"
                );
                let source = change.previous.context("Split source was closed")?.group;
                next.prepare_split(source, *group, direction)?
            };
            next.commit(plan)?;
        }
        next.validate(&group_ids(groups))?;
        Ok(next)
    }

    /// Fresh session imports map bounded saved indices onto new native IDs.
    /// Existing/recovered session append must use prepare_group_layout instead.
    pub(super) fn prepare_import_group_layout(
        &self,
        groups: &Groups,
        saved: Option<&SavedNode>,
        flat_axis: Axis,
    ) -> Result<Layout> {
        let ids = group_ids(groups);
        let mut next = self.editor_layout.clone();
        let plan = match saved {
            Some(saved) => next.prepare_import(&ids, Some(saved))?,
            None => next.prepare_flat(&ids, flat_axis)?,
        };
        next.commit(plan)?;
        next.validate(&ids)?;
        Ok(next)
    }

    /// The prepared tree carries all fallible work. No await/callback occurs
    /// between preparation and these two assignments; callers apply Change once.
    pub(super) fn publish_group_layout(&mut self, groups: Groups, layout: Layout) {
        self.editor_groups = groups;
        self.editor_layout = layout;
        self.invalidate_editor_presentation();
    }

    pub(super) fn open_editor_group(
        &mut self,
        document: u64,
        mode: OpenMode,
        replacement: Option<Membership>,
    ) -> Result<Change> {
        if self.editor_groups.active_group().is_some() {
            self.editor_layout
                .validate(&group_ids(&self.editor_groups))?;
            return self.editor_groups.open_mode(document, mode, replacement);
        }
        let mut groups = self.editor_groups.clone();
        let change = groups.open_mode(document, mode, replacement)?;
        let layout = self.prepare_group_layout(&groups, &change, None)?;
        self.publish_group_layout(groups, layout);
        Ok(change)
    }

    pub(super) fn open_editor_in_group(&mut self, group: GroupId, document: u64) -> Result<Change> {
        self.editor_layout
            .validate(&group_ids(&self.editor_groups))?;
        self.editor_groups.open_in_group(group, document)
    }

    pub(super) fn close_editor_membership(&mut self, member: Membership) -> Result<Change> {
        let group = self
            .editor_groups
            .group(member.group)
            .context("Original editor group was closed")?;
        self.editor_layout
            .validate(&group_ids(&self.editor_groups))?;
        if group.tabs().len() != 1 {
            return self.editor_groups.close(member);
        }
        let mut groups = self.editor_groups.clone();
        let change = groups.close(member)?;
        let layout = self.prepare_group_layout(&groups, &change, None)?;
        self.publish_group_layout(groups, layout);
        Ok(change)
    }

    pub(super) fn split_editor_layout(&mut self, direction: Direction) -> Result<Change> {
        ensure!(
            matches!(direction, Direction::Right | Direction::Down),
            "Left/Up group splitting is not supported yet"
        );
        let mut groups = self.editor_groups.clone();
        let change = groups.split_active()?;
        let layout = self.prepare_group_layout(&groups, &change, Some(direction))?;
        let source = change.previous.context("Split folding source retired")?;
        let target = change.active.context("Split folding target missing")?;
        let index = self
            .documents
            .iter()
            .position(|doc| doc.id == source.document)
            .context("Split model retired")?;
        let copy_folding = self.copy_split_folding_intent(source);
        if copy_folding {
            self.check_folding_view_copy_reservation(source)?;
        }
        let lease = if copy_folding {
            self.documents[index]
                .prepare_expanded_folding_view(target.group.value(), source.group.value())?
        } else {
            self.documents[index]
                .prepare_expanded_editor_view(target.group.value(), source.group.value())?
        };
        // Every fallible group/layout/view operation has finished. Publication
        // touches disjoint fields under the exclusive Document view lease.
        self.editor_groups = groups;
        self.editor_layout = layout;
        lease.publish();
        self.invalidate_editor_presentation();
        Ok(change)
    }

    /// Geometry-only changes do not retire provider/save/model/view ownership.
    pub(super) fn commit_editor_layout_plan(&mut self, plan: Plan) -> Result<bool> {
        plan.projected().validate(&group_ids(&self.editor_groups))?;
        let changed = self.editor_layout.commit(plan)?;
        if changed {
            self.invalidate_editor_presentation();
        }
        Ok(changed)
    }

    pub(crate) fn invalidate_editor_presentation(&mut self) {
        let result = self.editor_presentation.invalidate();
        self.clear_editor_hit_maps();
        if let Err(error) = result {
            self.message = format!("Editor geometry interaction disabled: {error:#}");
        }
    }

    pub(super) fn clear_editor_hit_maps(&mut self) {
        self.tab_hits.clear();
        self.pane_areas.clear();
        // Preserve last drawn viewport dimensions for PageUp/PageDown in the
        // same input batch. Native pointer mapping requires a sealed frame;
        // this rectangle alone grants no source-hit authorization.
        self.breadcrumbs_area = Rect::default();
        self.breadcrumbs_picker_area = Rect::default();
        self.breadcrumbs_hits.clear();
        self.breadcrumbs_presented = None;
    }
}

fn group_ids(groups: &Groups) -> Vec<GroupId> {
    groups.groups().iter().map(|group| group.id()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn app() -> (tempfile::TempDir, App) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("shared.txt");
        std::fs::write(&path, "猫🙂 alpha\r\nsecond\r\n").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.open(&path).unwrap();
        (root, app)
    }
    fn draw(app: &mut App) {
        let mut terminal = Terminal::new(TestBackend::new(110, 35)).unwrap();
        terminal.draw(|frame| crate::ui::draw(frame, app)).unwrap();
    }

    #[test]
    fn local_right_then_down_preserves_left_region_and_shared_undo_across_collapse() {
        let (_root, mut app) = app();
        app.doc_mut().move_to(2, false);
        app.doc_mut().insert("Z", false);
        let document = app.doc().id;
        let original = "猫🙂 alpha\r\nsecond\r\n";
        let dirty = app.doc().text.clone();
        let first = app.editor_groups.active_group().unwrap();
        app.split_editor(false);
        let second = app.editor_groups.active_group().unwrap();
        let before = app.project_editor_layout(Rect::new(0, 0, 90, 30)).unwrap();
        let left = before.placement(first).unwrap().outer;
        app.doc_mut().move_to(4, false);
        app.split_editor(true);
        let third = app.editor_groups.active_group().unwrap();
        let nested = app.project_editor_layout(Rect::new(0, 0, 90, 30)).unwrap();
        assert_eq!(nested.placement(first).unwrap().outer, left);
        assert_eq!(
            nested.placement(second).unwrap().outer.x,
            nested.placement(third).unwrap().outer.x
        );
        assert!(
            nested.placement(second).unwrap().outer.y < nested.placement(third).unwrap().outer.y
        );
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.doc().id, document);
        assert_eq!(app.doc().text, dirty);
        let closed = app.active_tab_membership().unwrap();
        app.close_tab_membership(closed).unwrap();
        assert_eq!(app.editor_layout.groups(), vec![first, second]);
        assert_eq!(app.doc().id, document);
        assert!(app.doc().dirty());
        assert_eq!(app.doc().text, dirty);
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), original);
        app.doc_mut().redo();
        assert_eq!(app.doc().text, dirty);
    }

    #[test]
    fn rejected_actual_split_reservation_preserves_tree_views_focus_and_redo() {
        let (_root, mut app) = app();
        app.doc_mut().insert("seed", false);
        app.doc_mut().undo();
        app.doc_mut().move_to(2, true);
        let before = app.editor_groups.clone();
        let layout = app.editor_layout.clone();
        let selection = app.doc().selections();
        let text = app.doc().text.clone();
        let epoch = app.doc().text_epoch();
        app.editor_groups.fail_next_recent_reservation();
        app.split_editor(true);
        assert!(app.message.contains("MRU"), "{}", app.message);
        assert_eq!(app.editor_groups, before);
        assert_eq!(app.editor_layout, layout);
        assert_eq!(app.doc().selections(), selection);
        assert_eq!(app.doc().text, text);
        assert_eq!(app.doc().text_epoch(), epoch);
        app.doc_mut().redo();
        assert!(app.doc().text.to_string().starts_with("seed"));
        // The staged clone consumes the shared one-shot; valid retry succeeds.
        app.split_editor(true);
        assert_eq!(app.editor_groups.groups().len(), 2);
        app.editor_layout
            .validate(&group_ids(&app.editor_groups))
            .unwrap();
    }

    #[test]
    fn rejected_late_layout_preflight_does_not_publish_staged_memberships_or_models() {
        let (_root, app) = app();
        let before = app.editor_groups.clone();
        let layout = app.editor_layout.clone();
        let original = app.doc().selections();
        let epoch = app.doc().text_epoch();
        let mut staged = app.editor_groups.clone();
        let change = staged.split_active().unwrap();
        let error = app
            .prepare_group_layout(&staged, &change, None)
            .unwrap_err();
        assert!(error.to_string().contains("explicit local split"));
        assert_eq!(app.editor_groups, before);
        assert_eq!(app.editor_layout, layout);
        assert_eq!(app.doc().selections(), original);
        assert_eq!(app.doc().text_epoch(), epoch);
        assert_eq!(app.documents.len(), 1);
    }

    #[test]
    fn redraw_and_ratio_resize_preserve_membership_views_and_document_history() {
        let (_root, mut app) = app();
        app.doc_mut().insert("unsaved", false);
        let dirty = app.doc().text.clone();
        let epoch = app.doc().text_epoch();
        app.split_editor(false);
        draw(&mut app);
        let proof = app.editor_groups.proof();
        let group = app.editor_groups.active_group().unwrap();
        let selection = app.doc().selections();
        let geometry = app.editor_presentation.geometry().unwrap().clone();
        let rect = geometry.placement(group).unwrap().outer;
        let plan = app
            .editor_layout
            .prepare_resize_group(group, Axis::Columns, 4, &geometry)
            .unwrap();
        app.commit_editor_layout_plan(plan).unwrap();
        assert!(app.editor_groups.proof_current(&proof));
        assert_eq!(app.doc().text_epoch(), epoch);
        assert_eq!(app.doc().text, dirty);
        assert_eq!(app.doc().selections(), selection);
        assert!(!app.editor_geometry_current());
        let now = app.project_editor_layout(geometry.area()).unwrap();
        assert!(now.placement(group).unwrap().outer.width > rect.width);
        draw(&mut app);
        let sealed = app.editor_presentation.proof().unwrap().clone();
        draw(&mut app);
        assert_eq!(app.editor_presentation.proof(), Some(&sealed));
        assert!(app.editor_groups.proof_current(&proof));
        app.doc_mut().undo();
        assert!(!app.doc().text.to_string().starts_with("unsaved"));
        app.doc_mut().redo();
        assert_eq!(app.doc().text, dirty);
    }

    #[test]
    fn geometry_sidebar_aba_and_unpainted_text_retire_mouse_authorization() {
        let (_root, mut app) = app();
        draw(&mut app);
        let original = app.editor_presentation.proof().unwrap().clone();
        app.execute("workbench.action.toggleSidebarVisibility", Value::Null);
        app.execute("workbench.action.toggleSidebarVisibility", Value::Null);
        assert!(!app.editor_presentation.current(
            &original,
            &app.editor_layout,
            &app.editor_groups
        ));
        draw(&mut app);
        let pane = app
            .editor_presentation
            .panes()
            .iter()
            .flatten()
            .next()
            .unwrap()
            .clone();
        app.doc_mut().insert("unpainted", false);
        let cursor = app.doc().cursor;
        let text = app.doc().text.clone();
        app.event(Event::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: pane.text.x,
            row: pane.text.y,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(app.doc().cursor, cursor);
        assert_eq!(app.doc().text, text);
        assert!(!app.editor_geometry_current());
    }
    fn queue_from_actual_palette(app: &mut App, label: &str, expected_command: &str) {
        app.event(Event::Key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)));
        assert!(matches!(
            app.prompt.as_ref().map(|prompt| &prompt.kind),
            Some(PromptKind::Palette)
        ));
        app.event(Event::Paste(label.into()));
        let items = app.palette_items(label);
        assert_eq!(items.first().map(|item| item.1), Some(expected_command));
        draw(app); // A real overlay repaint retires the editor pointer seal.
        assert!(!app.editor_geometry_current());
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(app.prompt.is_none());
        assert!(app.pending_editor_resize.is_some());
    }

    #[test]
    fn actual_palette_resize_waits_for_real_repaint_then_poll_and_cannot_be_overwritten() {
        let (_root, mut app) = app();
        app.doc_mut().insert("dirty", false);
        let dirty = app.doc().text.clone();
        let epoch = app.doc().text_epoch();
        app.split_editor(false);
        draw(&mut app);
        let member = app.active_tab_membership().unwrap();
        let proof = app.editor_groups.proof();
        let geometry = app.editor_presentation.geometry().unwrap().clone();
        let width = geometry.placement(member.group).unwrap().outer.width;
        queue_from_actual_palette(
            &mut app,
            "View: Increase Current View Width",
            "workbench.action.increaseViewWidth",
        );
        assert!(!app.poll_editor_resize());
        app.execute("workbench.action.decreaseViewWidth", Value::Null);
        assert_eq!(app.pending_editor_resize.as_ref().unwrap().delta, 4);
        draw(&mut app);
        // Drawing seals geometry, but cannot itself execute the resize.
        assert_eq!(
            app.editor_presentation
                .geometry()
                .unwrap()
                .placement(member.group)
                .unwrap()
                .outer
                .width,
            width
        );
        app.poll();
        assert!(app.pending_editor_resize.is_none());
        let after = app.project_editor_layout(geometry.area()).unwrap();
        assert_eq!(
            after.placement(member.group).unwrap().outer.width,
            width + 4
        );
        assert!(app.editor_groups.proof_current(&proof));
        assert_eq!(app.active_tab_membership(), Some(member));
        assert_eq!(app.doc().text, dirty);
        assert_eq!(app.doc().text_epoch(), epoch);
        assert!(!app.editor_geometry_current());
        app.doc_mut().undo();
        assert!(!app.doc().text.to_string().starts_with("dirty"));
        app.doc_mut().redo();
        assert_eq!(app.doc().text, dirty);
    }

    #[test]
    fn queued_resize_rejects_group_focus_and_layout_aba_without_retargeting() {
        for change_layout in [false, true] {
            let (_root, mut app) = app();
            app.split_editor(false);
            draw(&mut app);
            let member = app.active_tab_membership().unwrap();
            queue_from_actual_palette(
                &mut app,
                "View: Increase Current View Width",
                "workbench.action.increaseViewWidth",
            );
            if change_layout {
                let geometry = app.project_editor_layout(Rect::new(0, 0, 90, 30)).unwrap();
                let plan = app
                    .editor_layout
                    .prepare_resize_group(member.group, Axis::Columns, 4, &geometry)
                    .unwrap();
                app.commit_editor_layout_plan(plan).unwrap();
                let reset = app.editor_layout.prepare_reset().unwrap();
                app.commit_editor_layout_plan(reset).unwrap();
            } else {
                let other = app.editor_groups.groups()[0].active().unwrap();
                let group = app.editor_groups.groups()[0].id();
                let other = Membership {
                    group,
                    tab: other.id(),
                    document: other.document(),
                };
                app.focus_tab(other).unwrap();
                app.focus_tab(member).unwrap();
            }
            draw(&mut app);
            let layout = app.editor_layout.clone();
            let groups = app.editor_groups.clone();
            let text = app.doc().text.clone();
            app.poll();
            assert!(app.pending_editor_resize.is_none());
            assert!(
                app.message.contains("original group or layout changed"),
                "{}",
                app.message
            );
            assert_eq!(app.editor_layout, layout);
            assert_eq!(app.editor_groups, groups);
            assert_eq!(app.doc().text, text);
            assert_eq!(app.active_tab_membership(), Some(member));
        }
    }

    #[test]
    fn queued_resize_expires_without_a_repaint_and_never_retries_later() {
        let (_root, mut app) = app();
        app.split_editor(false);
        draw(&mut app);
        queue_from_actual_palette(
            &mut app,
            "View: Increase Current View Width",
            "workbench.action.increaseViewWidth",
        );
        app.pending_editor_resize.as_mut().unwrap().started =
            Instant::now() - RESIZE_DEADLINE - Duration::from_millis(1);
        let original = app.editor_layout.clone();
        assert!(app.poll_editor_resize());
        assert!(app.pending_editor_resize.is_none());
        assert!(app.message.contains("deadline"));
        draw(&mut app);
        assert!(!app.poll_editor_resize());
        assert_eq!(app.editor_layout, original);
    }
    #[test]
    fn indentation_width_change_and_observed_aba_reject_unpainted_source_hits() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("tabbed.txt");
        let disk = "\t猫🙂 X\r\n";
        std::fs::write(&path, disk).unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.open(&path).unwrap();
        app.doc_mut().move_to(3, false);
        app.doc_mut().insert("!", false);
        let dirty = app.doc().text.clone();
        app.doc_mut().move_to(0, false);
        draw(&mut app);
        let pane = app
            .editor_presentation
            .panes()
            .iter()
            .flatten()
            .next()
            .unwrap()
            .clone();
        let seal = pane.proof.clone();
        let groups = app.editor_groups.proof();
        let epoch = app.doc().text_epoch();
        let revision = app.doc().revision;
        let selection = app.doc().selections();
        let click = Event::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: pane.text.x + 3,
            row: pane.text.y,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.doc().tab_size, 4);
        assert_eq!(app.doc().position_at(0, 3), 0);
        app.doc_mut().set_indentation(2, true);
        assert_ne!(app.doc().position_at(0, 3), 0);
        app.event(click.clone());
        assert!(
            !app.editor_presentation
                .current(&seal, &app.editor_layout, &app.editor_groups)
        );
        assert_eq!(app.doc().selections(), selection);
        assert_eq!(app.doc().text, dirty);
        assert_eq!(app.doc().text_epoch(), epoch);
        assert_eq!(app.doc().revision, revision);
        assert!(app.doc().dirty());
        // Returning to width4 after the observed width2 cannot revive its seal.
        app.doc_mut().set_indentation(4, true);
        app.event(click);
        assert!(!app.editor_geometry_current());
        assert!(app.editor_groups.proof_current(&groups));
        assert_eq!(app.doc().selections(), selection);
        assert_eq!(app.doc().text, dirty);
        assert_eq!(app.doc().text_epoch(), epoch);
        assert_eq!(std::fs::read(&path).unwrap(), disk.as_bytes());
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), disk);
        app.doc_mut().redo();
        assert_eq!(app.doc().text, dirty);
        assert_eq!(std::fs::read(&path).unwrap(), disk.as_bytes());
    }
}
