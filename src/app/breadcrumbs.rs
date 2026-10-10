//! Breadcrumbs consumes the guarded Outline publication; rendering never requests symbols.
use super::*;
use crate::{
    breadcrumbs::{Element, ElementKind, Options, PathMode},
    document::Selection,
};
use anyhow::{Context as _, bail};

pub struct BreadcrumbRow {
    pub label: String,
    pub kind: u8,
    pub node: usize,
}
pub struct BreadcrumbPickerView<'a> {
    pub rows: &'a [BreadcrumbRow],
    pub selected: usize,
}
pub struct BreadcrumbsView<'a> {
    pub possible: bool,
    pub visible: bool,
    pub elements: &'a [Element],
    pub focused: Option<usize>,
    pub picker: Option<BreadcrumbPickerView<'a>>,
    pub updating: bool,
    pub message: &'a str,
    pub generation: u64,
}
#[derive(Clone, PartialEq)]
struct Context {
    document: u64,
    epoch: u64,
    saved: u64,
    path: PathBuf,
    workspace: PathBuf,
    pane: u64,
    primary: Selection,
    secondary: Vec<Selection>,
    options: Options,
    publication: u64,
    current: bool,
    has_tree: bool,
    active: Option<usize>,
}
impl Context {
    fn matches(&self, app: &App, options: Options) -> bool {
        let Some(doc) = app.active_document() else {
            return false;
        };
        let symbols = app.document_symbols_view();
        self.document == doc.id
            && self.epoch == doc.text_epoch()
            && self.saved == doc.save_generation()
            && doc.path.as_ref() == Some(&self.path)
            && self.workspace == app.workspace.root
            && self.pane == app.panes.get(app.active_pane).map_or(0, |p| p.id)
            && self.primary.cursor == doc.cursor
            && self.primary.anchor == doc.anchor
            && self.secondary == doc.secondary
            && self.options == options
            && self.publication == symbols.generation
            && self.current == symbols.current
            && self.has_tree == symbols.tree.is_some()
            && self.active == symbols.active
    }
    fn capture(app: &App, options: Options) -> Result<Self> {
        super::extension_services::Context::check_budget(app)?;
        let doc = app
            .active_document()
            .context("No active Breadcrumbs document")?;
        let path = doc
            .path
            .as_ref()
            .context("Breadcrumbs requires a saved file")?;
        if path.as_os_str().len() > 4096 || app.workspace.root.as_os_str().len() > 4096 {
            bail!("Breadcrumbs supports paths up to 4 KiB");
        }
        let symbols = app.document_symbols_view();
        Ok(Self {
            document: doc.id,
            epoch: doc.text_epoch(),
            saved: doc.save_generation(),
            path: path.clone(),
            workspace: app.workspace.root.clone(),
            pane: app.panes.get(app.active_pane).map_or(0, |p| p.id),
            primary: Selection {
                cursor: doc.cursor,
                anchor: doc.anchor,
                desired_column: None,
            },
            secondary: doc.secondary.clone(),
            options,
            publication: symbols.generation,
            current: symbols.current,
            has_tree: symbols.tree.is_some(),
            active: symbols.active,
        })
    }
}
fn list_key(command: &str) -> Option<KeyCode> {
    match command {
        "list.select" => Some(KeyCode::Enter),
        "list.focusDown" => Some(KeyCode::Down),
        "list.focusUp" => Some(KeyCode::Up),
        "list.focusFirst" => Some(KeyCode::Home),
        "list.focusLast" => Some(KeyCode::End),
        "list.focusPageUp" => Some(KeyCode::PageUp),
        "list.focusPageDown" => Some(KeyCode::PageDown),
        _ => None,
    }
}
struct Picker {
    context: Context,
    rows: Vec<BreadcrumbRow>,
    selected: usize,
}
#[derive(Default)]
pub(super) struct State {
    enabled_override: Option<bool>,
    observed: Option<Context>,
    elements: Vec<Element>,
    focused: Option<usize>,
    picker: Option<Picker>,
    generation: u64,
    exhausted: bool,
    message: String,
    path_budget_error: bool,
}
impl State {
    fn advance(&mut self) {
        if let Some(next) = self.generation.checked_add(1) {
            self.generation = next;
        } else {
            self.exhausted = true;
            self.picker = None;
            self.focused = None;
            self.message = "Breadcrumbs identity exhausted; restart the editor".into();
        }
    }
}
impl App {
    fn breadcrumbs_options(&self) -> Options {
        let mut options = self
            .settings
            .breadcrumbs(if self.active_document().is_some() {
                self.language()
            } else {
                "plaintext"
            });
        options.enabled = self.breadcrumbs.enabled_override.unwrap_or(options.enabled);
        options
    }
    pub(super) fn breadcrumbs_enabled(&self) -> bool {
        self.breadcrumbs_options().enabled
    }
    pub(super) fn breadcrumbs_demand(&self) -> bool {
        let options = self.breadcrumbs_options();
        options.enabled
            && options.symbol_path != PathMode::Off
            && !self.breadcrumbs.exhausted
            && self.active_document().is_some_and(|doc| doc.path.is_some())
    }
    fn breadcrumbs_current(&self) -> bool {
        self.breadcrumbs
            .observed
            .as_ref()
            .is_some_and(|context| context.matches(self, self.breadcrumbs_options()))
    }
    pub fn breadcrumbs_view(&self) -> BreadcrumbsView<'_> {
        let possible = self.active_document().is_some_and(|doc| doc.path.is_some());
        let visible = possible && self.breadcrumbs_enabled() && !self.breadcrumbs.exhausted;
        let current = self.breadcrumbs_current();
        BreadcrumbsView {
            possible,
            visible,
            elements: if current {
                &self.breadcrumbs.elements
            } else {
                &[]
            },
            focused: if current && self.focus == Focus::Breadcrumbs {
                self.breadcrumbs.focused
            } else {
                None
            },
            picker: if current {
                self.breadcrumbs
                    .picker
                    .as_ref()
                    .filter(|p| p.context.matches(self, self.breadcrumbs_options()))
                    .map(|p| BreadcrumbPickerView {
                        rows: &p.rows,
                        selected: p.selected,
                    })
            } else {
                None
            },
            updating: visible
                && self.breadcrumbs_options().symbol_path != PathMode::Off
                && matches!(
                    self.document_symbols_view().status,
                    OutlineStatus::Loading | OutlineStatus::Updating
                ),
            message: &self.breadcrumbs.message,
            generation: self.breadcrumbs.generation,
        }
    }
    pub(super) fn observe_breadcrumbs(&mut self) -> bool {
        if self.breadcrumbs.exhausted {
            return false;
        }
        let options = self.breadcrumbs_options();
        let possible = self.active_document().is_some_and(|doc| doc.path.is_some());
        if !possible || !options.enabled {
            if self.breadcrumbs.observed.is_none()
                && self.breadcrumbs.elements.is_empty()
                && self.breadcrumbs.focused.is_none()
            {
                return false;
            }
            self.breadcrumbs.observed = None;
            self.breadcrumbs.elements.clear();
            self.breadcrumbs.picker = None;
            self.breadcrumbs.focused = None;
            self.breadcrumbs.message.clear();
            self.breadcrumbs.advance();
            if self.focus == Focus::Breadcrumbs {
                self.focus = Focus::Editor;
            }
            return true;
        }
        let oversized = self.workspace.root.as_os_str().len() > 4096
            || self
                .active_document()
                .and_then(|doc| doc.path.as_ref())
                .is_some_and(|path| path.as_os_str().len() > 4096);
        if oversized {
            if self.breadcrumbs.path_budget_error {
                return false;
            }
            self.breadcrumbs.path_budget_error = true;
            self.breadcrumbs.observed = None;
            self.breadcrumbs.elements.clear();
            self.breadcrumbs.picker = None;
            self.breadcrumbs.focused = None;
            self.breadcrumbs.message = "Breadcrumbs supports paths up to 4 KiB".into();
            self.breadcrumbs.advance();
            if self.focus == Focus::Breadcrumbs {
                self.focus = Focus::Editor;
            }
            return true;
        }
        self.breadcrumbs.path_budget_error = false;
        if self
            .breadcrumbs
            .observed
            .as_ref()
            .is_some_and(|c| c.matches(self, options))
        {
            return false;
        }
        // Cursor/selection or same-resource edits need a new interaction proof,
        // but never new file labels or path allocations when the projection is
        // unchanged. Keep caller-ordered secondary storage if it is identical.
        let symbols = self.document_symbols_view();
        let projection = (
            symbols.generation,
            symbols.current,
            symbols.tree.is_some(),
            symbols.active,
        );
        let stable = self.breadcrumbs.observed.as_ref().is_some_and(|old| {
            self.active_document().is_some_and(|doc| {
                old.document == doc.id
                    && doc.path.as_ref() == Some(&old.path)
                    && old.workspace == self.workspace.root
                    && old.options == options
                    && (old.publication, old.current, old.has_tree, old.active) == projection
            })
        });
        if stable {
            let doc = &self.documents[self.active];
            if doc.secondary.len() < 16_384 {
                let context = self.breadcrumbs.observed.as_mut().unwrap();
                context.epoch = doc.text_epoch();
                context.saved = doc.save_generation();
                context.pane = self.panes.get(self.active_pane).map_or(0, |pane| pane.id);
                context.primary.cursor = doc.cursor;
                context.primary.anchor = doc.anchor;
                if context.secondary != doc.secondary {
                    context.secondary.clone_from(&doc.secondary);
                }
                self.breadcrumbs.picker = None;
                self.breadcrumbs.advance();
                return true;
            }
        }
        self.breadcrumbs.picker = None;
        let result = (|| -> Result<(Context, Vec<Element>)> {
            let context = Context::capture(self, options)?;
            let mut elements = crate::breadcrumbs::file_trail(
                &context.path,
                &context.workspace,
                options.file_path,
            )?;
            let symbols = self.document_symbols_view();
            if let Some(tree) = symbols.tree {
                elements.extend(crate::breadcrumbs::symbol_trail(
                    tree,
                    symbols.active,
                    options.symbol_path,
                )?);
            }
            Ok((context, elements))
        })();
        match result {
            Ok((context, elements)) => {
                let focused = self
                    .breadcrumbs
                    .focused
                    .and_then(|i| self.breadcrumbs.elements.get(i))
                    .and_then(|previous| {
                        elements.iter().position(|item| {
                            item.kind == previous.kind
                                && item.node == previous.node
                                && item.label == previous.label
                        })
                    });
                self.breadcrumbs.observed = Some(context);
                self.breadcrumbs.elements = elements;
                self.breadcrumbs.focused = focused;
                self.breadcrumbs.message.clear();
            }
            Err(error) => {
                self.breadcrumbs.observed = None;
                self.breadcrumbs.elements.clear();
                self.breadcrumbs.focused = None;
                self.breadcrumbs.message = format!("{error:#}");
            }
        }
        self.breadcrumbs.advance();
        true
    }
    pub(super) fn poll_breadcrumbs(&mut self) -> bool {
        self.observe_breadcrumbs()
    }
    fn dismiss_breadcrumbs(&mut self, editor: bool) {
        if self.breadcrumbs.picker.take().is_some() || self.breadcrumbs.focused.take().is_some() {
            self.breadcrumbs.advance();
        }
        if editor && self.focus == Focus::Breadcrumbs {
            self.focus = Focus::Editor;
        }
    }
    pub(super) fn breadcrumbs_ui_event(&mut self, event: &Event) {
        if matches!(event, Event::Key(key) if key.kind == KeyEventKind::Release) {
            return;
        }
        let owned = self.focus == Focus::Breadcrumbs
            && match event {
                Event::Key(_) | Event::Paste(_) => true,
                Event::Resize(..) => true,
                Event::Mouse(mouse) => mouse.kind == crossterm::event::MouseEventKind::Moved,
                _ => false,
            };
        if self.focus == Focus::Breadcrumbs && matches!(event, Event::Paste(_)) {
            self.message =
                "Breadcrumbs: picker filtering is not supported yet; editor text unchanged".into();
        }
        // Click admission is guarded against the rendered generation by App;
        // retire the previous popup only when this is an unrelated click.
        let own_click = matches!(event,Event::Mouse(mouse) if self.breadcrumbs_area.contains(ratatui::layout::Position::new(mouse.column,mouse.row)) || self.breadcrumbs_picker_area.contains(ratatui::layout::Position::new(mouse.column,mouse.row)));
        if !owned && !own_click {
            self.dismiss_breadcrumbs(true);
        }
    }
    pub(super) fn breadcrumbs_ui_command(&mut self, command: &str) {
        if !command.starts_with("breadcrumbs.")
            && !(list_key(command).is_some()
                && self.focus == Focus::Breadcrumbs
                && self.breadcrumbs.picker.is_some())
        {
            self.dismiss_breadcrumbs(true);
        }
    }
    fn focus_breadcrumbs(&mut self, select: bool) {
        self.observe_breadcrumbs();
        if !self.breadcrumbs_view().visible
            || self.breadcrumbs.elements.is_empty()
            || self.prompt.is_some()
            || self.modal.is_some()
        {
            self.message = "Breadcrumbs requires a saved file and a visible trail".into();
            return;
        }
        self.focus = Focus::Breadcrumbs;
        self.breadcrumbs.focused = Some(self.breadcrumbs.elements.len() - 1);
        self.breadcrumbs.advance();
        if select {
            self.pick_breadcrumb()
                .unwrap_or_else(|error| self.message = format!("Breadcrumbs: {error:#}"));
        }
    }
    fn move_breadcrumb(&mut self, forward: bool, pick: bool) {
        if !self.breadcrumbs_current() {
            self.dismiss_breadcrumbs(true);
            return;
        }
        let Some(index) = self.breadcrumbs.focused else {
            return;
        };
        let next = if forward {
            (index + 1).min(self.breadcrumbs.elements.len().saturating_sub(1))
        } else {
            index.saturating_sub(1)
        };
        self.breadcrumbs.focused = Some(next);
        self.breadcrumbs.picker = None;
        self.breadcrumbs.advance();
        if pick {
            self.pick_breadcrumb()
                .unwrap_or_else(|error| self.message = format!("Breadcrumbs: {error:#}"));
        }
    }
    fn pick_breadcrumb(&mut self) -> Result<()> {
        if self.focus != Focus::Breadcrumbs
            || !self.breadcrumbs_current()
            || self.prompt.is_some()
            || self.modal.is_some()
        {
            bail!("Breadcrumbs context changed");
        }
        let element = self
            .breadcrumbs
            .elements
            .get(self.breadcrumbs.focused.context("No focused breadcrumb")?)
            .context("No focused breadcrumb")?;
        if !matches!(element.kind, ElementKind::Symbol | ElementKind::RootSymbols) {
            bail!("Folder and file dropdowns are not supported yet");
        }
        let symbols = self.document_symbols_view();
        if !symbols.current {
            bail!("Document symbols changed or are still updating");
        }
        let tree = symbols.tree.context("No current document symbols")?;
        let node = element.node;
        let parent = node.and_then(|index| tree.nodes[index].parent);
        let mut indices: Vec<usize> = tree
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| (n.parent == parent).then_some(i))
            .collect();
        indices.sort_by_key(|i| {
            (
                tree.nodes[*i].range.start.line,
                tree.nodes[*i].range.start.character,
            )
        });
        let selected = node
            .and_then(|node| indices.iter().position(|i| *i == node))
            .unwrap_or(0);
        let rows = indices
            .into_iter()
            .map(|index| BreadcrumbRow {
                label: tree.nodes[index].name.clone(),
                kind: tree.nodes[index].kind,
                node: index,
            })
            .collect();
        self.breadcrumbs.picker = Some(Picker {
            context: Context::capture(self, self.breadcrumbs_options())?,
            rows,
            selected,
        });
        self.breadcrumbs.advance();
        Ok(())
    }
    fn reveal_breadcrumb(&mut self, picker: bool) -> Result<()> {
        if self.focus != Focus::Breadcrumbs
            || !self.breadcrumbs_current()
            || self.prompt.is_some()
            || self.modal.is_some()
        {
            bail!("Breadcrumbs context changed");
        }
        let (node, publication) = if picker {
            let picker = self
                .breadcrumbs
                .picker
                .as_ref()
                .context("No Breadcrumbs picker")?;
            if !picker.context.matches(self, self.breadcrumbs_options()) {
                bail!("Breadcrumbs picker changed");
            }
            (
                picker
                    .rows
                    .get(picker.selected)
                    .context("No selected symbol")?
                    .node,
                picker.context.publication,
            )
        } else {
            (
                self.breadcrumbs
                    .elements
                    .get(self.breadcrumbs.focused.context("No focused breadcrumb")?)
                    .and_then(|item| item.node)
                    .context("This breadcrumb has no symbol to reveal")?,
                self.document_symbols_view().generation,
            )
        };
        self.reveal_document_symbol(node, publication)?;
        self.dismiss_breadcrumbs(true);
        self.observe_breadcrumbs();
        Ok(())
    }
    pub(super) fn execute_breadcrumbs_command(&mut self, command: &str) -> bool {
        if self.focus == Focus::Breadcrumbs
            && self.breadcrumbs.picker.is_some()
            && let Some(code) = list_key(command)
        {
            self.breadcrumbs_key(KeyEvent::new(code, KeyModifiers::NONE));
            return true;
        }
        match command {
            "breadcrumbs.toggle" => {
                self.breadcrumbs.enabled_override = Some(!self.breadcrumbs_enabled());
                self.dismiss_breadcrumbs(true);
                self.observe_outline();
                self.observe_breadcrumbs();
            }
            "breadcrumbs.toggleToOn" => {
                self.breadcrumbs.enabled_override = Some(true);
                self.observe_outline();
                self.observe_breadcrumbs();
                self.focus_breadcrumbs(true);
            }
            "breadcrumbs.focus" => self.focus_breadcrumbs(false),
            "breadcrumbs.focusAndSelect" => self.focus_breadcrumbs(true),
            "breadcrumbs.focusNext" => self.move_breadcrumb(true, false),
            "breadcrumbs.focusPrevious" => self.move_breadcrumb(false, false),
            "breadcrumbs.focusNextWithPicker" => self.move_breadcrumb(true, true),
            "breadcrumbs.focusPreviousWithPicker" => self.move_breadcrumb(false, true),
            "breadcrumbs.selectFocused" => {
                if let Err(error) = self.pick_breadcrumb() {
                    self.message = format!("Breadcrumbs: {error:#}");
                }
            }
            "breadcrumbs.revealFocusedFromTreeAside" => {
                self.message =
                    "Breadcrumbs: opening symbols to the side is not supported yet".into();
            }
            "breadcrumbs.revealFocused" => {
                if let Err(error) = self.reveal_breadcrumb(false) {
                    self.message = format!("Breadcrumbs: {error:#}");
                }
            }
            "breadcrumbs.selectEditor" => self.dismiss_breadcrumbs(true),
            _ => return false,
        };
        true
    }
    pub(super) fn breadcrumbs_key(&mut self, key: KeyEvent) {
        if !self.breadcrumbs_current() || self.prompt.is_some() || self.modal.is_some() {
            self.dismiss_breadcrumbs(true);
            return;
        }
        if self.breadcrumbs.picker.is_some() {
            match key.code {
                KeyCode::Enter => {
                    if let Err(error) = self.reveal_breadcrumb(true) {
                        self.message = format!("Breadcrumbs: {error:#}");
                        self.dismiss_breadcrumbs(true);
                    }
                }
                KeyCode::Esc => self.dismiss_breadcrumbs(true),
                KeyCode::Up
                | KeyCode::Down
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::PageUp
                | KeyCode::PageDown => {
                    let picker = self.breadcrumbs.picker.as_mut().unwrap();
                    let last = picker.rows.len().saturating_sub(1);
                    picker.selected = match key.code {
                        KeyCode::Up => picker.selected.saturating_sub(1),
                        KeyCode::Down => (picker.selected + 1).min(last),
                        KeyCode::Home => 0,
                        KeyCode::End => last,
                        KeyCode::PageUp => picker.selected.saturating_sub(10),
                        _ => (picker.selected + 10).min(last),
                    };
                    self.breadcrumbs.advance();
                }
                _ => {
                    self.message =
                        "Breadcrumbs: picker filtering is not supported yet; editor text unchanged"
                            .into();
                }
            }
        } else {
            match key.code {
                KeyCode::Left => self.move_breadcrumb(false, false),
                KeyCode::Right => self.move_breadcrumb(true, false),
                KeyCode::Down | KeyCode::Enter => {
                    let _ = self.execute_breadcrumbs_command("breadcrumbs.selectFocused");
                }
                KeyCode::Char(' ') => {
                    let _ = self.execute_breadcrumbs_command("breadcrumbs.revealFocused");
                }
                KeyCode::Esc => self.dismiss_breadcrumbs(true),
                _ => {
                    self.message =
                        "Breadcrumbs: picker filtering is not supported yet; editor text unchanged"
                            .into();
                }
            }
        }
    }
    pub(super) fn breadcrumbs_click(&mut self, index: usize) {
        if !self.breadcrumbs_current()
            || !self.breadcrumbs_view().visible
            || self.prompt.is_some()
            || self.modal.is_some()
            || index >= self.breadcrumbs.elements.len()
        {
            return;
        }
        self.focus = Focus::Breadcrumbs;
        self.breadcrumbs.focused = Some(index);
        self.breadcrumbs.picker = None;
        self.breadcrumbs.advance();
        if let Err(error) = self.pick_breadcrumb() {
            self.message = format!("Breadcrumbs: {error:#}");
        }
    }
    pub(super) fn breadcrumbs_picker_click(&mut self, row: usize) {
        if !self.breadcrumbs_current() {
            return;
        }
        let Some(picker) = self.breadcrumbs.picker.as_mut() else {
            return;
        };
        if row >= picker.rows.len() {
            return;
        }
        picker.selected = row;
        if let Err(error) = self.reveal_breadcrumb(true) {
            self.message = format!("Breadcrumbs: {error:#}");
            self.dismiss_breadcrumbs(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, App) {
        let root = tempfile::tempdir().unwrap();
        // Match CLI startup: saved documents and workspace use canonical roots,
        // including macOS /var aliases and Windows extended-length prefixes.
        let workspace = std::fs::canonicalize(root.path()).unwrap();
        let path = workspace.join("猫.cpp");
        std::fs::write(&path, "猫🙂\r\nbody\r\n").unwrap();
        let mut app = App::new(workspace, Profile::Linux);
        app.open(&path).unwrap();
        (root, app)
    }
    #[test]
    fn native_file_trail_focus_toggle_preserves_dirty_identity_redo_and_disk() {
        let (_root, mut app) = fixture();
        let id = app.doc().id;
        app.doc_mut().insert("seed", false);
        app.doc_mut().undo();
        let epoch = app.doc().text_epoch();
        app.observe_breadcrumbs();
        assert_eq!(
            app.breadcrumbs_view().elements.last().unwrap().label,
            "猫.cpp"
        );
        app.execute_breadcrumbs_command("breadcrumbs.focus");
        assert!(app.focus == Focus::Breadcrumbs);
        assert_eq!(app.breadcrumbs_view().focused, Some(0));
        app.execute_breadcrumbs_command("breadcrumbs.selectFocused");
        assert!(app.message.contains("not supported"));
        assert!(app.breadcrumbs_view().picker.is_none());
        app.execute_breadcrumbs_command("breadcrumbs.selectEditor");
        app.execute_breadcrumbs_command("breadcrumbs.toggle");
        assert!(!app.breadcrumbs_view().visible);
        assert!(!app.breadcrumbs_demand());
        app.execute_breadcrumbs_command("breadcrumbs.toggle");
        assert!(app.breadcrumbs_view().visible);
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text_epoch(), epoch);
        assert_eq!(app.doc().text.to_string(), "猫🙂\r\nbody\r\n");
        assert_eq!(
            std::fs::read(app.doc().path.as_ref().unwrap()).unwrap(),
            "猫🙂\r\nbody\r\n".as_bytes()
        );
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), "seed猫🙂\r\nbody\r\n");
    }
    #[test]
    fn observed_workspace_aba_edit_undo_and_primary_selection_retire_hitmaps() {
        let (_root, mut app) = fixture();
        app.execute_breadcrumbs_command("breadcrumbs.focus");
        let initial = app.breadcrumbs_view().generation;
        let workspace = app.workspace.root.clone();
        app.workspace.root = workspace.join("other");
        assert!(app.observe_breadcrumbs());
        app.workspace.root = workspace;
        assert!(app.observe_breadcrumbs());
        assert!(app.breadcrumbs_view().generation > initial);
        let before = app.breadcrumbs_view().generation;
        app.doc_mut().insert("x", false);
        app.doc_mut().undo();
        assert!(app.breadcrumbs_view().elements.is_empty());
        app.observe_breadcrumbs();
        assert!(app.breadcrumbs_view().generation > before);
        let before = app.breadcrumbs_view().generation;
        app.doc_mut().anchor = Some(2);
        assert!(app.observe_breadcrumbs());
        assert!(app.breadcrumbs_view().generation > before);
        app.breadcrumbs_ui_event(&Event::Paste("x".into()));
        assert!(app.focus == Focus::Breadcrumbs);
        app.breadcrumbs_ui_command("workbench.action.quickOpen");
        assert!(app.focus != Focus::Breadcrumbs);
        assert_eq!(app.doc().text.to_string(), "猫🙂\r\nbody\r\n");
    }
    #[test]
    fn untitled_has_no_fake_file_trail_but_enabled_setting_remains_true() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        assert!(app.breadcrumbs_enabled());
        assert!(!app.breadcrumbs_view().possible);
        assert!(!app.breadcrumbs_view().visible);
        assert!(!app.breadcrumbs_demand());
        app.execute_breadcrumbs_command("breadcrumbs.focus");
        assert!(app.focus != Focus::Breadcrumbs);
    }
    #[test]
    fn ordinary_cursor_proofs_reuse_cached_labels_paths_and_secondary_storage() {
        let (_root, mut app) = fixture();
        app.doc_mut()
            .set_selections(vec![Selection::caret(0), Selection::caret(4)]);
        app.observe_breadcrumbs();
        let generation = app.breadcrumbs_view().generation;
        let elements = app.breadcrumbs.elements.as_ptr();
        let label = app.breadcrumbs.elements[0].label.as_ptr();
        let path = app
            .breadcrumbs
            .observed
            .as_ref()
            .unwrap()
            .path
            .as_os_str()
            .as_encoded_bytes()
            .as_ptr();
        let secondary = app
            .breadcrumbs
            .observed
            .as_ref()
            .unwrap()
            .secondary
            .as_ptr();
        app.doc_mut().move_to(1, false);
        assert!(app.observe_breadcrumbs());
        assert!(app.breadcrumbs_view().generation > generation);
        assert_eq!(app.breadcrumbs.elements.as_ptr(), elements);
        assert_eq!(app.breadcrumbs.elements[0].label.as_ptr(), label);
        let context = app.breadcrumbs.observed.as_ref().unwrap();
        assert_eq!(context.path.as_os_str().as_encoded_bytes().as_ptr(), path);
        assert_eq!(context.secondary.as_ptr(), secondary);
        assert_eq!(context.primary.cursor, 1);
        assert_eq!(context.secondary, app.doc().secondary);
        assert!(!app.observe_breadcrumbs());
        assert_eq!(app.doc().text.to_string(), "猫🙂\r\nbody\r\n");
    }
    #[test]
    fn oversized_path_failure_is_inert_until_budget_recovers() {
        let (_root, mut app) = fixture();
        let path = app.doc().path.clone();
        let epoch = app.doc().text_epoch();
        app.doc_mut().path = Some(PathBuf::from("x".repeat(4097)));
        assert!(app.observe_breadcrumbs());
        let generation = app.breadcrumbs_view().generation;
        let error = app.breadcrumbs.message.as_ptr();
        for _ in 0..32 {
            assert!(!app.observe_breadcrumbs());
            assert_eq!(app.breadcrumbs_view().generation, generation);
            assert_eq!(app.breadcrumbs.message.as_ptr(), error);
            assert!(app.breadcrumbs_view().elements.is_empty());
        }
        app.doc_mut().path = path;
        assert!(app.observe_breadcrumbs());
        assert_eq!(app.breadcrumbs_view().elements[0].label, "猫.cpp");
        assert_eq!(app.doc().text_epoch(), epoch);
        assert_eq!(app.doc().text.to_string(), "猫🙂\r\nbody\r\n");
    }
}
