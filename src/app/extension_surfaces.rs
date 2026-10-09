use super::*;
use crate::extensions::{SurfaceKey, SurfaceReveal};
use std::collections::BTreeSet;

#[derive(Clone, Copy, PartialEq)]
pub enum PickerKind {
    Output,
    Status,
    Trees,
}
pub struct Picker {
    pub kind: PickerKind,
    pub session: u64,
    pub items: Vec<Item>,
    pub selected: usize,
}
pub struct Item {
    pub key: SurfaceKey,
    pub generation: u64,
    pub label: String,
}
pub struct TreePresentation {
    pub session: u64,
    pub key: SurfaceKey,
    pub generation: u64,
    pub rows: Vec<(String, usize)>,
}
#[derive(Default)]
pub struct State {
    session: Option<u64>,
    generation: u64,
    pub output: Option<SurfaceKey>,
    pub output_area: Rect,
    pub output_scroll: usize,
    pub tree: Option<SurfaceKey>,
    tree_generation: u64,
    pub expanded: BTreeSet<String>,
    pub collapsed: BTreeSet<String>,
    pub selected: usize,
    pub presented_tree: Option<TreePresentation>,
    requested: BTreeSet<Option<String>>,
    pub status_hits: Vec<(Rect, SurfaceKey, u64, u64)>,
}
impl App {
    pub(super) fn poll_extension_surfaces(&mut self) -> bool {
        let previous_focus = self.focus.clone();
        let session = self.extension_host.as_ref().map(|h| h.session);
        let mut changed = false;
        if self.extension_surfaces.session != session {
            self.extension_surfaces = State {
                session,
                ..State::default()
            };
            if self.focus == Focus::Output {
                self.focus = Focus::Editor;
            }
            if matches!(
                self.modal,
                Some(Modal::ExtensionTree | Modal::ExtensionSurfaces(_))
            ) {
                self.modal = None;
            }
            changed = true;
        }
        let Some(host) = &mut self.extension_host else {
            return changed;
        };
        changed |= self.extension_surfaces.generation != host.surfaces.generation;
        self.extension_surfaces.generation = host.surfaces.generation;
        while let Some(reveal) = host.surfaces.reveal.pop_front() {
            match reveal {
                SurfaceReveal::Output(key, preserve)
                    if host.surfaces.channels.contains_key(&key) =>
                {
                    self.extension_surfaces.output = Some(key);
                    self.extension_surfaces.output_scroll = 0;
                    if !preserve && self.prompt.is_none() && self.modal.is_none() {
                        self.focus = Focus::Output;
                    }
                }
                SurfaceReveal::HideOutput(key)
                    if self.extension_surfaces.output.as_ref() == Some(&key) =>
                {
                    self.extension_surfaces.output = None
                }
                _ => {}
            }
            changed = true;
        }
        if self
            .extension_surfaces
            .output
            .as_ref()
            .is_some_and(|k| !host.surfaces.channels.contains_key(k))
        {
            self.extension_surfaces.output = None;
        }
        if self.extension_surfaces.output.is_none() && self.focus == Focus::Output {
            self.focus = Focus::Editor;
        }
        if let Some(key) = &self.extension_surfaces.tree {
            if let Some(tree) = host.surfaces.trees.get(key) {
                if tree.generation != self.extension_surfaces.tree_generation {
                    self.extension_surfaces.tree_generation = tree.generation;
                    self.extension_surfaces.expanded.clear();
                    self.extension_surfaces.collapsed.clear();
                    self.extension_surfaces.requested.clear();
                    self.extension_surfaces.selected = 0;
                }
            } else {
                self.extension_surfaces.tree = None;
                if matches!(self.modal, Some(Modal::ExtensionTree)) {
                    self.modal = None;
                }
            }
        }
        if self.focus != previous_focus {
            self.session_interaction();
            self.clear_signature();
            self.cancel_navigation();
            self.invalidate_symbol_context();
        }
        if matches!(self.modal, Some(Modal::ExtensionTree)) {
            self.load_surface_tree();
        } else if self.extension_surfaces.tree.is_some() {
            self.close_surface_tree();
        }
        changed
    }
    pub(super) fn surface_picker(&mut self, kind: PickerKind) {
        let Some(host) = &self.extension_host else {
            self.message = "No extension session is running".into();
            return;
        };
        let items = match kind {
            PickerKind::Output => host
                .surfaces
                .channels
                .values()
                .map(|v| Item {
                    key: v.key.clone(),
                    generation: 0,
                    label: format!("{} · {}", v.name, v.key.owner),
                })
                .collect(),
            PickerKind::Status => host
                .surfaces
                .statuses
                .values()
                .filter(|v| v.visible)
                .map(|v| Item {
                    key: v.key.clone(),
                    generation: v.generation,
                    label: format!("{} · {} · {}", v.text, v.tooltip, v.key.owner),
                })
                .collect(),
            PickerKind::Trees => host
                .surfaces
                .trees
                .values()
                .map(|v| Item {
                    key: v.key.clone(),
                    generation: v.generation,
                    label: format!("{} · {}", v.title, v.key.owner),
                })
                .collect(),
        };
        self.modal = Some(Modal::ExtensionSurfaces(Picker {
            kind,
            session: host.session,
            items,
            selected: 0,
        }));
    }
    pub(super) fn surface_picker_key(&mut self, key: KeyEvent, mut picker: Picker) {
        if self.extension_host.as_ref().map(|h| h.session) != Some(picker.session) {
            return;
        }
        match key.code {
            KeyCode::Up => picker.selected = picker.selected.saturating_sub(1),
            KeyCode::Down => {
                picker.selected = (picker.selected + 1).min(picker.items.len().saturating_sub(1))
            }
            KeyCode::Enter => {
                if let Some(item) = picker.items.get(picker.selected) {
                    match picker.kind {
                        PickerKind::Output => {
                            if self
                                .extension_host
                                .as_ref()
                                .is_some_and(|h| h.surfaces.channels.contains_key(&item.key))
                            {
                                self.extension_surfaces.output = Some(item.key.clone());
                                self.extension_surfaces.output_scroll = 0;
                                self.focus = Focus::Output;
                            }
                        }
                        PickerKind::Status => self.surface_action(&item.key, item.generation, None),
                        PickerKind::Trees => {
                            if self
                                .extension_host
                                .as_ref()
                                .and_then(|h| h.surfaces.trees.get(&item.key))
                                .is_some_and(|v| v.generation == item.generation)
                            {
                                self.close_surface_tree();
                                self.extension_surfaces.tree = Some(item.key.clone());
                                self.extension_surfaces.tree_generation = item.generation;
                                self.extension_surfaces.expanded.clear();
                                self.extension_surfaces.collapsed.clear();
                                self.extension_surfaces.requested.clear();
                                self.extension_surfaces.selected = 0;
                                self.modal = Some(Modal::ExtensionTree);
                                self.surface_tree_event("visibility", None, true);
                                self.load_surface_tree();
                            } else {
                                self.message = "Tree view changed; reopen the view picker".into();
                            }
                        }
                    }
                }
                return;
            }
            _ => {}
        }
        self.modal = Some(Modal::ExtensionSurfaces(picker));
    }
    pub fn surface_tree_rows(&self) -> Vec<(String, usize)> {
        let Some(tree) = self
            .extension_surfaces
            .tree
            .as_ref()
            .and_then(|k| self.extension_host.as_ref()?.surfaces.trees.get(k))
        else {
            return vec![];
        };
        let mut rows = Vec::new();
        let mut stack: Vec<_> = tree
            .children
            .get(&None)
            .into_iter()
            .flatten()
            .rev()
            .map(|id| (id.clone(), 0))
            .collect();
        while let Some((id, depth)) = stack.pop() {
            if self.extension_surfaces.expanded.contains(&id)
                || (tree.nodes.get(&id).is_some_and(|n| n.collapsible == 2)
                    && !self.extension_surfaces.collapsed.contains(&id))
            {
                stack.extend(
                    tree.children
                        .get(&Some(id.clone()))
                        .into_iter()
                        .flatten()
                        .rev()
                        .map(|child| (child.clone(), depth + 1)),
                );
            }
            rows.push((id, depth));
        }
        rows
    }
    fn load_surface_tree(&mut self) {
        let Some(key) = self.extension_surfaces.tree.clone() else {
            return;
        };
        let rows = self.surface_tree_rows();
        let Some(host) = &mut self.extension_host else {
            return;
        };
        if host.surfaces.tree_busy() {
            return;
        }
        let Some(tree) = host.surfaces.trees.get(&key) else {
            return;
        };
        let parent = if !tree.children.contains_key(&None) {
            Some(None)
        } else {
            rows.into_iter().find_map(|(id, _)| {
                let expanded = self.extension_surfaces.expanded.contains(&id)
                    || (tree.nodes.get(&id).is_some_and(|n| n.collapsible == 2)
                        && !self.extension_surfaces.collapsed.contains(&id));
                (expanded
                    && !tree.children.contains_key(&Some(id.clone()))
                    && !self
                        .extension_surfaces
                        .requested
                        .contains(&Some(id.clone())))
                .then_some(Some(id))
            })
        };
        let Some(parent) = parent else {
            return;
        };
        if self.extension_surfaces.requested.contains(&parent) {
            return;
        }
        match host.surface_tree_children_with_hidden(
            &key,
            parent.clone(),
            &self.documents,
            &self.hidden_documents,
            self.active,
            &self.settings,
        ) {
            Ok(()) => {
                self.extension_surfaces.requested.insert(parent);
            }
            Err(e) => self.message = format!("Tree request failed: {e:#}"),
        }
    }
    fn surface_tree_event(&mut self, event: &str, node: Option<&str>, visible: bool) {
        if let Some(key) = &self.extension_surfaces.tree
            && let Some(host) = &self.extension_host
        {
            let _ = host.surface_tree_event(
                key,
                self.extension_surfaces.tree_generation,
                event,
                node,
                visible,
            );
        }
    }
    pub(super) fn close_surface_tree(&mut self) {
        self.surface_tree_event("visibility", None, false);
        self.extension_surfaces.tree = None;
    }
    pub(super) fn surface_tree_key(&mut self, key: KeyEvent) {
        let rows = self.surface_tree_rows();
        let current = self.extension_surfaces.tree.as_ref().and_then(|key| {
            self.extension_host.as_ref().and_then(|host| {
                host.surfaces
                    .trees
                    .get(key)
                    .map(|tree| (host.session, key, tree.generation))
            })
        });
        let presented = self
            .extension_surfaces
            .presented_tree
            .as_ref()
            .is_some_and(|previous| {
                current == Some((previous.session, &previous.key, previous.generation))
                    && previous.rows == rows
            });
        if !presented && key.code != KeyCode::Char('r') && key.code != KeyCode::Char('R') {
            self.message = "Tree view changed; review the refreshed items".into();
            self.modal = Some(Modal::ExtensionTree);
            return;
        }

        let selected = self
            .extension_surfaces
            .selected
            .min(rows.len().saturating_sub(1));
        self.extension_surfaces.selected = selected;
        match key.code {
            KeyCode::Up => self.extension_surfaces.selected = selected.saturating_sub(1),
            KeyCode::Down => {
                self.extension_surfaces.selected = (selected + 1).min(rows.len().saturating_sub(1))
            }
            KeyCode::Char('r' | 'R') => {
                self.extension_surfaces.requested.clear();
            }
            KeyCode::Left => {
                if let Some((id, _)) = rows.get(selected) {
                    self.extension_surfaces.expanded.remove(id);
                    self.extension_surfaces.collapsed.insert(id.clone());
                    self.surface_tree_event("collapse", Some(id), true);
                }
            }
            KeyCode::Right | KeyCode::Enter => {
                if let Some((id, _)) = rows.get(selected) {
                    let tree_key = self.extension_surfaces.tree.clone().unwrap();
                    let item = self
                        .extension_host
                        .as_ref()
                        .and_then(|h| h.surfaces.trees.get(&tree_key))
                        .and_then(|v| v.nodes.get(id));
                    if key.code == KeyCode::Enter && item.is_some_and(|n| n.has_command) {
                        self.surface_action(
                            &tree_key,
                            self.extension_surfaces.tree_generation,
                            Some(id),
                        );
                        self.close_surface_tree();
                        return;
                    } else if item.is_some_and(|n| n.collapsible > 0) {
                        self.extension_surfaces.expanded.insert(id.clone());
                        self.extension_surfaces.collapsed.remove(id);
                        self.surface_tree_event("expand", Some(id), true);
                    }
                }
            }
            _ => {}
        }
        self.modal = Some(Modal::ExtensionTree);
        self.load_surface_tree();
    }
    fn surface_action(&mut self, key: &SurfaceKey, generation: u64, node: Option<&str>) {
        if let Some(host) = &mut self.extension_host {
            match host.surface_action_with_hidden(
                key,
                generation,
                node,
                &self.documents,
                &self.hidden_documents,
                self.active,
                &self.settings,
            ) {
                Ok(()) => self.message = "Running extension surface action".into(),
                Err(e) => self.message = format!("Extension surface action rejected: {e:#}"),
            }
        }
    }
    pub(super) fn surface_mouse(&mut self, mouse: crossterm::event::MouseEvent) -> bool {
        let point = ratatui::layout::Position::new(mouse.column, mouse.row);
        if self.extension_surfaces.output.is_some()
            && self.extension_surfaces.output_area.contains(point)
        {
            self.focus = Focus::Output;
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.extension_surfaces.output_scroll =
                        self.extension_surfaces.output_scroll.saturating_add(3)
                }
                MouseEventKind::ScrollDown => {
                    self.extension_surfaces.output_scroll =
                        self.extension_surfaces.output_scroll.saturating_sub(3)
                }
                _ => {}
            }
            return true;
        }
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && let Some((_, key, generation, session)) = self
                .extension_surfaces
                .status_hits
                .iter()
                .find(|(r, _, _, _)| r.contains(point))
                .cloned()
        {
            if self.extension_host.as_ref().map(|h| h.session) == Some(session) {
                self.surface_action(&key, generation, None);
            }
            return true;
        }
        false
    }
    pub(super) fn output_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.extension_surfaces.output = None;
                self.focus = Focus::Editor;
            }
            KeyCode::Up | KeyCode::PageUp => {
                self.extension_surfaces.output_scroll = self
                    .extension_surfaces
                    .output_scroll
                    .saturating_add(if key.code == KeyCode::Up { 1 } else { 10 })
            }
            KeyCode::Down | KeyCode::PageDown => {
                self.extension_surfaces.output_scroll = self
                    .extension_surfaces
                    .output_scroll
                    .saturating_sub(if key.code == KeyCode::Down { 1 } else { 10 })
            }
            KeyCode::End => self.extension_surfaces.output_scroll = 0,
            KeyCode::Home => self.extension_surfaces.output_scroll = usize::MAX,
            _ => {}
        }
    }
}
