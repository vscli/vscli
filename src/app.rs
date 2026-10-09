mod debugger;
mod extension_management;
mod extensions;
mod files;
mod language;
mod navigation;
mod panes;
mod snippet_catalogs;
mod snippets;
mod source_control;
mod tasks;
mod terminals;
mod themes;
mod watching;
use crate::{
    document::Document,
    keys::{self, Keymap, Profile, Resolution},
    workspace::{Entry, Workspace, directory_entries, score},
};
use anyhow::Result;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
pub use language::{LanguageAction, LanguageItem};
use ratatui::layout::Rect;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) fn native_command_ids() -> Vec<String> {
    let mut ids: Vec<_> = COMMANDS
        .iter()
        .map(|(_, id)| *id)
        .chain([
            "cancelSelection",
            "deleteLeft",
            "deleteRight",
            "deleteWordLeft",
            "deleteWordRight",
            "editor.action.addCommentLine",
            "editor.action.changeAll",
            "editor.action.indentLines",
            "editor.action.nextMatchFindAction",
            "editor.action.outdentLines",
            "editor.action.previousMatchFindAction",
            "editor.action.removeCommentLine",
            "expandLineSelection",
            "git.openChange",
            "git.refresh",
            "git.stage",
            "git.unstage",
            "jumpToNextSnippetPlaceholder",
            "jumpToPrevSnippetPlaceholder",
            "leaveSnippet",
            "lineBreakInsert",
            "outdent",
            "tab",
            "type",
            "workbench.action.closeWindow",
            "workbench.action.debug.continue",
            "workbench.action.focusFirstEditorGroup",
            "workbench.action.focusFourthEditorGroup",
            "workbench.action.focusSecondEditorGroup",
            "workbench.action.focusThirdEditorGroup",
            "workbench.action.openSettings",
            "workbench.action.showCommands",
            "workbench.action.splitEditor",
            "workbench.action.terminal.focus",
            "workbench.action.togglePanel",
            "workbench.view.extensions",
        ])
        .map(str::to_owned)
        .collect();
    for profile in [Profile::Linux, Profile::Macos, Profile::Windows] {
        ids.extend(
            Keymap::new(profile)
                .bindings
                .into_iter()
                .map(|binding| binding.command),
        );
    }
    ids.sort();
    ids.dedup();
    ids
}

pub const COMMANDS: &[(&str, &str)] = &[
    ("Open Recent File", "workbench.action.openRecent"),
    (
        "Reopen Closed Editor",
        "workbench.action.reopenClosedEditor",
    ),
    ("Insert Snippet", "editor.action.insertSnippet"),
    (
        "Extensions: Install from VSIX",
        "workbench.extensions.action.installVSIX",
    ),
    (
        "Extensions: Show Installed Extensions",
        "workbench.extensions.action.showInstalledExtensions",
    ),
    ("Preferences: Color Theme", "workbench.action.selectTheme"),
    ("Preferences: Color Theme Report", "vscli.theme.report"),
    ("Preferences: Load Color Theme File", "vscli.theme.load"),
    ("Extensions: Stop Host", "vscli.extensions.stop"),
    (
        "Extensions: Stop Selected Package",
        "vscli.extensions.stopSelected",
    ),
    (
        "Extensions: Restart Selected Session",
        "vscli.extensions.restart",
    ),
    ("Settings: Compatibility Report", "vscli.settings.report"),
    (
        "Preferences: Open User Settings (JSON)",
        "workbench.action.openSettingsJson",
    ),
    (
        "File: New Text File",
        "workbench.action.files.newUntitledFile",
    ),
    ("File: Open File…", "workbench.action.files.openFile"),
    ("Explorer: New File", "explorer.newFile"),
    ("Explorer: New Folder", "explorer.newFolder"),
    ("Explorer: Rename Selected Item", "renameFile"),
    ("Explorer: Move Selected Item to Trash", "deleteFile"),
    (
        "Explorer: Refresh Files",
        "workbench.files.action.refreshFilesExplorer",
    ),
    ("File: Save", "workbench.action.files.save"),
    ("File: Save As…", "workbench.action.files.saveAs"),
    ("File: Close Editor", "workbench.action.closeActiveEditor"),
    (
        "File: Close All Editors",
        "workbench.action.closeAllEditors",
    ),
    ("File: Revert File", "workbench.action.files.revert"),
    ("View: Quick Open", "workbench.action.quickOpen"),
    (
        "View: Split Editor Right",
        "workbench.action.splitEditorRight",
    ),
    (
        "View: Split Editor Down",
        "workbench.action.splitEditorDown",
    ),
    (
        "View: Focus Next Editor Group",
        "workbench.action.focusNextGroup",
    ),
    (
        "View: Close Editor Group",
        "workbench.action.closeEditorsInGroup",
    ),
    ("Tasks: Run Task", "workbench.action.tasks.runTask"),
    ("Tasks: Run Build Task", "workbench.action.tasks.build"),
    (
        "Tasks: Terminate Active Task",
        "workbench.action.tasks.terminate",
    ),
    ("Git: Source Control", "workbench.view.scm"),
    ("Debug: Start / Continue", "workbench.action.debug.start"),
    (
        "Debug: Toggle Breakpoint",
        "editor.debug.action.toggleBreakpoint",
    ),
    ("Debug: Stop", "workbench.action.debug.stop"),
    ("Debug: Pause", "workbench.action.debug.pause"),
    ("Debug: Step Over", "workbench.action.debug.stepOver"),
    ("Debug: Step Into", "workbench.action.debug.stepInto"),
    ("Debug: Step Out", "workbench.action.debug.stepOut"),
    ("Debug: Stack and Variables", "workbench.view.debug"),
    ("Debug: Console", "workbench.debug.action.toggleRepl"),
    ("Debug: Evaluate Expression", "vscli.debug.evaluate"),
    ("Git: Commit Staged Changes", "git.commit"),
    ("Git: History", "git.viewHistory"),
    (
        "Terminal: Toggle",
        "workbench.action.terminal.toggleTerminal",
    ),
    ("Terminal: New Terminal", "workbench.action.terminal.new"),
    (
        "Terminal: Kill Active Terminal",
        "workbench.action.terminal.kill",
    ),
    ("Terminal: Next", "workbench.action.terminal.focusNext"),
    (
        "Terminal: Previous",
        "workbench.action.terminal.focusPrevious",
    ),
    (
        "View: Focus Editor",
        "workbench.action.focusActiveEditorGroup",
    ),
    (
        "View: Toggle Sidebar",
        "workbench.action.toggleSidebarVisibility",
    ),
    ("View: Explorer", "workbench.view.explorer"),
    ("View: Next Editor", "workbench.action.nextEditor"),
    ("View: Previous Editor", "workbench.action.previousEditor"),
    ("Edit: Undo", "undo"),
    ("Edit: Redo", "redo"),
    ("Edit: Copy", "editor.action.clipboardCopyAction"),
    ("Edit: Cut", "editor.action.clipboardCutAction"),
    ("Edit: Paste", "editor.action.clipboardPasteAction"),
    ("Edit: Select All", "editor.action.selectAll"),
    ("Edit: Toggle Line Comment", "editor.action.commentLine"),
    ("Edit: Delete Line", "editor.action.deleteLines"),
    (
        "Edit: Add Next Occurrence",
        "editor.action.addSelectionToNextFindMatch",
    ),
    (
        "Edit: Select All Occurrences",
        "editor.action.selectHighlights",
    ),
    ("Edit: Add Cursor Above", "editor.action.insertCursorAbove"),
    ("Edit: Add Cursor Below", "editor.action.insertCursorBelow"),
    (
        "Edit: Cursors at Line Ends",
        "editor.action.insertCursorAtEndOfEachLineSelected",
    ),
    ("Edit: Insert Line Above", "editor.action.insertLineBefore"),
    ("Edit: Insert Line Below", "editor.action.insertLineAfter"),
    ("Edit: Copy Line Up", "editor.action.copyLinesUpAction"),
    ("Edit: Copy Line Down", "editor.action.copyLinesDownAction"),
    ("Edit: Move Line Up", "editor.action.moveLinesUpAction"),
    ("Edit: Move Line Down", "editor.action.moveLinesDownAction"),
    ("Edit: Jump to Bracket", "editor.action.jumpToBracket"),
    ("Edit: Undo Cursor", "cursorUndo"),
    ("Find: Find in File", "actions.find"),
    ("Search: Find in Files", "workbench.action.findInFiles"),
    (
        "Find: Replace All in File…",
        "editor.action.startFindReplaceAction",
    ),
    ("Go: Go to Line…", "workbench.action.gotoLine"),
    ("Language: Hover", "editor.action.showHover"),
    ("Language: Complete", "editor.action.triggerSuggest"),
    (
        "Language: Go to Definition",
        "editor.action.revealDefinition",
    ),
    ("Language: Find References", "editor.action.goToReferences"),
    ("Language: Format Document", "editor.action.formatDocument"),
    ("Language: Rename Symbol", "editor.action.rename"),
    ("View: Problems", "workbench.actions.view.problems"),
    (
        "Preferences: Keyboard Shortcuts",
        "workbench.action.openGlobalKeybindings",
    ),
    ("Developer: Keyboard Inspector", "vscli.keyboardInspector"),
    ("Help: Getting Started", "vscli.help"),
    ("File: Exit", "workbench.action.quit"),
];

#[derive(Clone, PartialEq)]
pub enum Focus {
    Editor,
    Explorer,
    Terminal,
}
#[derive(Clone)]
pub enum PromptKind {
    InstallExtension,
    StopExtension,
    Palette,
    QuickOpen,
    RecentFiles,
    Snippet,
    Theme,
    ThemeFile,
    Open,
    SaveAs,
    Find,
    WorkspaceSearch,
    Rename,
    GitCommit,
    DebugEvaluate,
    CreateFile,
    CreateFolder,
    RenameFile(PathBuf),
    ReplaceQuery,
    ReplaceWith(String),
    Goto,
}
pub struct Prompt {
    pub kind: PromptKind,
    pub text: String,
    pub cursor: usize,
    pub selected: usize,
    pub select_all: bool,
}
impl Prompt {
    fn new(kind: PromptKind, text: String) -> Self {
        let cursor = text.len();
        Self {
            kind,
            text,
            cursor,
            selected: 0,
            select_all: true,
        }
    }
    pub fn insert(&mut self, text: &str) {
        if self.select_all {
            self.text.clear();
            self.cursor = 0;
            self.select_all = false;
        }
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.selected = 0;
    }
}
#[derive(Clone)]
pub enum AfterSave {
    Close,
    Quit,
    CloseAll,
}
pub enum Modal {
    ExtensionsLoading(u64),
    Extensions {
        items: Vec<crate::extension_store::Installed>,
        selected: usize,
    },
    RunExtension(crate::extension_store::Installed),
    Help,
    Keys,
    Inspector,
    Search,
    Text {
        title: String,
        text: String,
        scroll: usize,
    },
    Git,
    Debug {
        section: usize,
        selected: usize,
    },
    Language {
        title: String,
        items: Vec<LanguageItem>,
        selected: usize,
    },
    Confirm(AfterSave),
    Revert,
    Trash(PathBuf),
    Tasks {
        tasks: Vec<crate::tasks::Task>,
        selected: usize,
    },
    ConfirmTask(crate::tasks::PreparedTask),
}

#[derive(Clone)]
pub struct Pane {
    pub id: u64,
    pub document: u64,
}

pub struct App {
    pub panes: Vec<Pane>,
    pub active_pane: usize,
    pub pane_areas: Vec<Rect>,
    pub horizontal_split: bool,
    next_pane_id: u64,
    pub documents: Vec<Document>,
    pub active: usize,
    pub workspace: Workspace,
    pub keymap: Keymap,
    pub focus: Focus,
    pub sidebar: bool,
    pub explorer_dir: PathBuf,
    pub entries: Vec<Entry>,
    pub explorer_selected: usize,
    pub prompt: Option<Prompt>,
    pub modal: Option<Modal>,
    pub message: String,
    pub running: bool,
    pub chord: Option<String>,
    pub last_key: String,
    pub enhanced: bool,
    pub find_query: String,
    pub search: Option<crate::search::Search>,
    pub search_options: crate::search::Options,
    pub debug_configuration: Option<crate::debug::Configuration>,
    pub debugger: Option<crate::debug::Client>,
    pub breakpoints: std::collections::BTreeMap<PathBuf, std::collections::BTreeSet<usize>>,
    pub file_job: Option<crate::files::Job>,
    pub(super) watch: crate::watch::State,
    pub git_job: Option<crate::git::Job>,
    pub git_status: Option<crate::git::Status>,
    pub git_selected: usize,
    pub trusted_tasks: bool,
    pub terminals: Vec<crate::terminal::Session>,
    pub active_terminal: usize,
    pub terminal_visible: bool,
    pub terminal_area: Rect,
    pub extensions_directory: Option<PathBuf>,
    pub extension_node: String,
    extension_job: Option<extension_management::Job>,
    extension_retirement: Option<std::sync::mpsc::Receiver<()>>,
    extension_epoch: u64,
    pub extension_packages: Vec<crate::extensions::Package>,
    pub extension_host: Option<crate::extensions::Client>,
    pub lsp: Option<crate::lsp::Client>,
    pub syntax: crate::syntax::Engine,
    pub theme: crate::theme::Theme,
    pub recent_files: crate::recent::State,
    navigation: navigation::State,
    theme_state: themes::State,
    pub settings: crate::settings::Settings,
    pub imported_keybinding_notices: Vec<String>,
    settings_loader: Option<crate::settings::Loader>,
    settings_error: Option<String>,
    settings_user: Option<PathBuf>,
    pub diagnostics: HashMap<PathBuf, (u64, u64, Vec<crate::lsp::Diagnostic>)>,
    pub editor_area: Rect,
    pub explorer_area: Rect,
    pub tab_area: Rect,
    pub pending: Option<AfterSave>,
    snippet_pending: Option<snippets::Pending>,
    snippet_catalog: snippet_catalogs::State,
    clipboard: String,
    pub clipboard_line: bool,
}
impl Drop for App {
    fn drop(&mut self) {
        self.shutdown_extensions();
    }
}
impl App {
    pub fn new(root: PathBuf, profile: Profile) -> Self {
        let entries = directory_entries(&root);
        Self {
            panes: Vec::new(),
            active_pane: 0,
            pane_areas: Vec::new(),
            horizontal_split: false,
            next_pane_id: 2,
            documents: Vec::new(),
            active: 0,
            workspace: Workspace::new(root.clone()),
            watch: crate::watch::State::new(root.clone()),
            keymap: Keymap::new(profile),
            focus: Focus::Editor,
            sidebar: true,
            explorer_dir: root,
            entries,
            explorer_selected: 0,
            prompt: None,
            modal: None,
            message: "F1 commands · Ctrl+P open · Ctrl+S save · Ctrl+Shift+W exit".into(),
            running: true,
            chord: None,
            last_key: String::new(),
            enhanced: false,
            find_query: String::new(),
            search: None,
            search_options: crate::search::Options::default(),
            debug_configuration: None,
            debugger: None,
            breakpoints: std::collections::BTreeMap::new(),
            file_job: None,
            git_job: None,
            git_status: None,
            git_selected: 0,
            trusted_tasks: false,
            terminals: Vec::new(),
            active_terminal: 0,
            terminal_visible: false,
            terminal_area: Rect::default(),
            extension_host: None,
            extensions_directory: crate::extension_store::default_directory(),
            extension_node: "node".into(),
            extension_job: None,
            extension_retirement: None,
            extension_epoch: 0,
            extension_packages: Vec::new(),
            lsp: None,
            syntax: crate::syntax::Engine::default(),
            theme: crate::theme::Theme::default(),
            recent_files: crate::recent::State::default(),
            navigation: navigation::State::default(),
            theme_state: themes::State::default(),
            settings: crate::settings::Settings::default(),
            imported_keybinding_notices: Vec::new(),
            settings_loader: None,
            settings_error: None,
            settings_user: None,
            diagnostics: HashMap::new(),
            editor_area: Rect::default(),
            explorer_area: Rect::default(),
            tab_area: Rect::default(),
            pending: None,
            snippet_pending: None,
            snippet_catalog: snippet_catalogs::State::default(),
            clipboard: String::new(),
            clipboard_line: false,
        }
    }
    pub fn poll(&mut self) -> bool {
        let changed = self.workspace.poll();
        let changed = self.search.as_mut().is_some_and(|s| s.poll()) || changed;
        let mut changed = self.poll_language() || changed;
        if let Some(result) = self
            .settings_loader
            .as_mut()
            .and_then(crate::settings::Loader::poll)
        {
            match result {
                Ok(settings) => {
                    self.settings_error = None;
                    if settings != self.settings {
                        self.settings = settings;
                        for doc in &mut self.documents {
                            self.settings.apply(doc);
                        }
                        self.message = format!(
                            "Settings reloaded · {} settings notices (Settings: Compatibility Report)",
                            self.settings.warnings.len()
                        );
                        changed = true;
                    }
                }
                Err(error) if self.settings_error.as_ref() != Some(&error) => {
                    self.message =
                        format!("Settings reload failed; previous settings retained: {error}");
                    self.settings_error = Some(error);
                    changed = true;
                }
                _ => {}
            }
        }
        changed |= self.poll_git();
        changed |= self.poll_files();
        changed |= self.poll_watching();
        changed |= self.poll_debugger();
        changed |= self.poll_extension_management();
        changed |= self.poll_extensions();
        changed |= self.poll_snippet();
        changed |= self.poll_snippet_catalog();
        changed |= self.poll_theme();
        changed |= self.poll_navigation();
        let visible: Vec<_> = self
            .documents
            .iter()
            .filter(|d| self.panes.iter().any(|p| p.document == d.id))
            .collect();
        let (highlighted, error) = self.syntax.poll(&visible);
        changed |= highlighted;
        if let Some(error) = error {
            self.message = error;
            changed = true;
        }
        for terminal in &mut self.terminals {
            changed |= terminal.poll();
        }
        changed
    }
    pub fn active_document(&self) -> Option<&Document> {
        self.documents.get(self.active)
    }
    pub fn doc(&self) -> &Document {
        &self.documents[self.active]
    }
    pub fn configure_settings(&mut self, user: Option<PathBuf>) -> Result<()> {
        self.settings_user = user.clone();
        let mut paths: Vec<_> = user.into_iter().collect();
        paths.push(self.workspace.root.join(".vscode/settings.json"));
        self.settings_loader = Some(crate::settings::Loader::new(paths.clone()));
        self.settings = match crate::settings::Settings::load(&paths) {
            Ok(settings) => settings,
            Err(error) => {
                self.settings_error = Some(format!("{error:#}"));
                return Err(error);
            }
        };
        for doc in &mut self.documents {
            self.settings.apply(doc);
        }
        if !self.settings.warnings.is_empty() {
            self.message = format!(
                "{} settings notices · F1 → Settings: Compatibility Report",
                self.settings.warnings.len()
            );
        }
        Ok(())
    }
    pub fn doc_mut(&mut self) -> &mut Document {
        &mut self.documents[self.active]
    }
    pub fn open(&mut self, path: &Path) -> Result<()> {
        self.cancel_navigation();
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        if let Some(index) = self
            .documents
            .iter()
            .position(|doc| doc.path.as_ref() == Some(&path))
        {
            self.active = index;
            self.focus = Focus::Editor;
            self.sync_pane();
            self.remember_active_file();
            return Ok(());
        }
        let mut d = Document::open(&path)?;
        self.settings.apply(&mut d);
        self.install_open_document(d);
        Ok(())
    }
    fn install_open_document(&mut self, d: Document) {
        if let Some(index) = self.documents.iter().position(|old| old.path == d.path) {
            self.active = index;
        } else if self.documents.len() == 1
            && self.doc().path.is_none()
            && self.doc().is_empty()
            && !self.doc().dirty()
        {
            self.documents[0] = d;
            self.active = 0;
        } else {
            self.documents.push(d);
            self.active = self.documents.len() - 1;
        }
        self.focus = Focus::Editor;
        self.message = format!("Opened {}", self.doc().name());
        self.sync_pane();
        self.remember_active_file();
    }
    pub fn start_prompt(&mut self, kind: PromptKind, text: String) {
        self.cancel_navigation();
        if let Some(doc) = self.documents.get_mut(self.active) {
            doc.break_group();
        }
        self.chord = None;
        self.prompt = Some(Prompt::new(kind, text));
    }
    pub fn palette_items(&self, query: &str) -> Vec<(&str, &str)> {
        let mut items: Vec<_> = COMMANDS
            .iter()
            .filter_map(|(label, id)| {
                score(label, query.trim_start_matches('>')).map(|s| (s, *label, *id))
            })
            .collect();
        if let Some(host) = &self.extension_host {
            items.extend(host.commands.iter().filter_map(|(label, id)| {
                score(label, query.trim_start_matches('>'))
                    .map(|s| (s, label.as_str(), id.as_str()))
            }));
        }
        items.sort_by_key(|a| std::cmp::Reverse(a.0));
        items
            .into_iter()
            .map(|(_, label, id)| (label, id))
            .collect()
    }
    pub fn context(&self) -> HashMap<String, Value> {
        HashMap::from([
            (
                "editorTextFocus".into(),
                json!(
                    self.active_document().is_some()
                        && self.focus == Focus::Editor
                        && self.prompt.is_none()
                        && self.modal.is_none()
                ),
            ),
            (
                "textInputFocus".into(),
                json!(
                    self.active_document().is_some()
                        && self.focus == Focus::Editor
                        && self.prompt.is_none()
                        && self.modal.is_none()
                ),
            ),
            (
                "inSnippetMode".into(),
                json!(self.active_document().is_some_and(Document::in_snippet)),
            ),
            (
                "hasNextTabstop".into(),
                json!(
                    self.active_document()
                        .is_some_and(|d| d.has_snippet_step(false))
                ),
            ),
            (
                "hasPrevTabstop".into(),
                json!(
                    self.active_document()
                        .is_some_and(|d| d.has_snippet_step(true))
                ),
            ),
            (
                "editorFocus".into(),
                json!(self.active_document().is_some() && self.focus == Focus::Editor),
            ),
            (
                "viewContainer.workbench.view.extensions.enabled".into(),
                json!(self.extensions_directory.is_some()),
            ),
            (
                "editorHasSelection".into(),
                json!(
                    self.active_document()
                        .is_some_and(|d| d.selection().is_some())
                ),
            ),
            ("inputFocus".into(), json!(self.prompt.is_some())),
            ("terminalFocus".into(), json!(self.focus == Focus::Terminal)),
            (
                "filesExplorerFocus".into(),
                json!(self.focus == Focus::Explorer),
            ),
            ("editorLangId".into(), json!(self.language())),
            (
                "isLinux".into(),
                json!(self.keymap.profile == Profile::Linux),
            ),
            ("isMac".into(), json!(self.keymap.profile == Profile::Macos)),
            (
                "isWindows".into(),
                json!(self.keymap.profile == Profile::Windows),
            ),
        ])
    }
    pub fn language(&self) -> &str {
        self.active_document()
            .and_then(|d| d.path.as_deref())
            .map_or("plaintext", crate::lsp::language)
    }

    pub fn event(&mut self, event: Event) {
        self.event_inner(event);
        self.sync_pane();
    }
    fn event_inner(&mut self, event: Event) {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
            Event::Paste(text) => {
                if let Some(prompt) = &mut self.prompt {
                    prompt.insert(&text.replace(['\r', '\n'], ""));
                } else if self.modal.is_none() && self.focus == Focus::Terminal {
                    if let Some(terminal) = self.terminals.get_mut(self.active_terminal)
                        && let Err(e) = terminal.paste(&text)
                    {
                        self.message = e.to_string();
                    }
                } else if self.modal.is_none()
                    && self.focus == Focus::Editor
                    && self.active_document().is_some()
                {
                    let normalized = text
                        .replace("\r\n", "\n")
                        .replace('\r', "\n")
                        .replace('\n', &self.doc().eol);
                    self.doc_mut().insert(&normalized, false);
                }
            }
            Event::Mouse(mouse) if self.modal.is_none() && self.prompt.is_none() => {
                let p = ratatui::layout::Position::new(mouse.column, mouse.row);
                if self.terminal_visible && self.terminal_area.contains(p) {
                    self.focus = Focus::Terminal;
                    if let Some(terminal) = self.terminals.get_mut(self.active_terminal) {
                        let offset = terminal.parser.screen().scrollback();
                        match mouse.kind {
                            MouseEventKind::ScrollUp => terminal
                                .parser
                                .screen_mut()
                                .set_scrollback(offset.saturating_add(3)),
                            MouseEventKind::ScrollDown => terminal
                                .parser
                                .screen_mut()
                                .set_scrollback(offset.saturating_sub(3)),
                            _ => {}
                        }
                    }
                    return;
                }
                if let Some(index) = self.pane_areas.iter().position(|area| area.contains(p)) {
                    self.focus_pane(index);
                    self.editor_area = self.pane_areas[index];
                }
                if self.active_document().is_some() && self.editor_area.contains(p) {
                    match mouse.kind {
                        MouseEventKind::Down(MouseButton::Left)
                        | MouseEventKind::Drag(MouseButton::Left) => {
                            self.focus = Focus::Editor;
                            let row = self.doc().top + (mouse.row - self.editor_area.y) as usize;
                            let col =
                                self.doc().left + (mouse.column - self.editor_area.x) as usize;
                            let pos = self.doc().position_at(row, col);
                            if mouse.modifiers.contains(KeyModifiers::ALT)
                                && matches!(mouse.kind, MouseEventKind::Down(_))
                            {
                                self.doc_mut().add_cursor(pos);
                                return;
                            }
                            self.doc_mut().secondary.clear();
                            self.doc_mut().move_to(
                                pos,
                                matches!(mouse.kind, MouseEventKind::Drag(_))
                                    || mouse.modifiers.contains(KeyModifiers::SHIFT),
                            );
                        }
                        MouseEventKind::ScrollDown => {
                            self.doc_mut().vertical(3, false);
                        }
                        MouseEventKind::ScrollUp => {
                            self.doc_mut().vertical(-3, false);
                        }
                        _ => {}
                    }
                } else if self.explorer_area.contains(p)
                    && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                {
                    self.focus = Focus::Explorer;
                    let offset = self
                        .explorer_selected
                        .saturating_sub(self.explorer_area.height.saturating_sub(1) as usize);
                    self.explorer_selected = (offset + (mouse.row - self.explorer_area.y) as usize)
                        .min(self.entries.len().saturating_sub(1));
                    self.explorer_open();
                }
            }
            _ => {}
        }
    }
    fn key(&mut self, key: KeyEvent) {
        let token = keys::token(key);
        self.last_key = format!("{token}  {:?}", key);
        if self.modal.is_some() {
            self.modal_key(key);
            return;
        }
        if self.prompt.is_some() {
            self.prompt_key(key);
            return;
        }
        if self.focus == Focus::Terminal && self.chord.is_none() {
            if let Resolution::Command(command, args) = self.keymap.resolve(&token, &self.context())
                && matches!(
                    command.as_str(),
                    "workbench.action.terminal.toggleTerminal"
                        | "workbench.action.togglePanel"
                        | "workbench.action.showCommands"
                        | "workbench.action.quickOpen"
                        | "workbench.action.focusActiveEditorGroup"
                        | "workbench.action.focusFirstEditorGroup"
                        | "workbench.action.focusSecondEditorGroup"
                        | "workbench.action.focusThirdEditorGroup"
                        | "workbench.action.focusFourthEditorGroup"
                        | "workbench.action.terminal.new"
                        | "workbench.action.terminal.focusNext"
                        | "workbench.action.terminal.focusPrevious"
                        | "workbench.action.closeWindow"
                        | "workbench.view.extensions"
                        | "workbench.extensions.action.showInstalledExtensions"
                )
            {
                self.execute_with_args(&command, args);
            } else if let Some(terminal) = self.terminals.get_mut(self.active_terminal)
                && let Err(e) = terminal.key(key)
            {
                self.message = e.to_string();
            }
            return;
        }
        if key.code == KeyCode::Esc && self.chord.is_some() {
            self.chord = None;
            self.message = "Chord canceled".into();
            return;
        }
        let in_chord = self.chord.is_some();
        let sequence = self
            .chord
            .take()
            .map(|prefix| format!("{prefix} {token}"))
            .unwrap_or(token.clone());
        match self.keymap.resolve(&sequence, &self.context()) {
            Resolution::Command(command, args) => {
                self.execute_with_args(&command, args);
                return;
            }
            Resolution::Chord => {
                self.chord = Some(sequence.clone());
                self.message = format!("({sequence}) waiting for second key…");
                return;
            }
            Resolution::None if in_chord => {
                self.message = format!("No command bound to {sequence}");
                return;
            }
            _ => {}
        }
        if self.focus == Focus::Explorer {
            self.explorer_key(key);
            return;
        }
        if let KeyCode::Char(c) = key.code
            && self.active_document().is_some()
            && !key.modifiers.intersects(
                KeyModifiers::CONTROL
                    | KeyModifiers::ALT
                    | KeyModifiers::SUPER
                    | KeyModifiers::META,
            )
        {
            self.doc_mut().insert(&c.to_string(), true);
            return;
        }
        if !token.is_empty() {
            self.message =
                format!("Unbound or unavailable: {token} · F1 lists implemented commands");
        }
    }
    pub fn execute(&mut self, command: &str, args: Value) {
        self.execute_with_args(command, (!args.is_null()).then_some(args));
    }
    fn execute_with_args(&mut self, command: &str, args: Option<Value>) {
        self.execute_inner(command, args);
        self.sync_pane();
    }
    fn execute_inner(&mut self, command: &str, command_args: Option<Value>) {
        // A duplicate reopen keeps the existing bounded request alive.
        if command != "workbench.action.reopenClosedEditor" {
            self.cancel_navigation();
        }
        let args = command_args.clone().unwrap_or(Value::Null);
        if command.is_empty() {
            return;
        }
        if self.active_document().is_none() && Self::requires_editor(command) {
            self.message = "Open a file or create a new file first".into();
            return;
        }
        if !matches!(
            command,
            "type"
                | "jumpToNextSnippetPlaceholder"
                | "jumpToPrevSnippetPlaceholder"
                | "leaveSnippet"
        ) && let Some(doc) = self.documents.get_mut(self.active)
        {
            doc.break_group();
        }
        if command == "cursorUndo" {
            self.doc_mut().undo_cursor();
            return;
        }
        if command.starts_with("cursor") {
            let select = command.ends_with("Select");
            let name = command.strip_suffix("Select").unwrap_or(command);
            let page = self.editor_area.height.saturating_sub(1).max(1) as isize;
            if !self.doc_mut().navigate_cursors(name, select, page) {
                self.message = format!("Command not implemented: {command}");
            }
            return;
        }
        match command {
            "workbench.action.debug.start" | "workbench.action.debug.continue" => {
                self.start_or_continue_debug()
            }
            "editor.debug.action.toggleBreakpoint" => self.toggle_breakpoint(),
            "workbench.action.debug.stop" => self.debug_control("disconnect"),
            "workbench.action.debug.pause" => self.debug_control("pause"),
            "workbench.action.debug.stepOver" => self.debug_control("next"),
            "workbench.action.debug.stepInto" => self.debug_control("stepIn"),
            "workbench.action.debug.stepOut" => self.debug_control("stepOut"),
            "workbench.view.debug" => {
                self.modal = Some(Modal::Debug {
                    section: 0,
                    selected: 0,
                })
            }
            "workbench.debug.action.toggleRepl" => {
                self.modal = Some(Modal::Debug {
                    section: 3,
                    selected: 0,
                })
            }
            "workbench.action.selectTheme" => self.open_theme_picker(),
            "vscli.theme.report" => self.modal = Some(Modal::Text {
                title: "Color Theme Compatibility".into(),
                text: format!("{}\n\nNative palette: editor background/foreground/selection/current line, shared panels, line numbers and accent.\nSyntax colors map to 14 native categories; this is not TextMate scope parity.\n\n{}", self.theme.name, self.theme.warnings.join("\n")),
                scroll: 0,
            }),
            "vscli.theme.load" => self.start_prompt(PromptKind::ThemeFile, String::new()),
            "vscli.debug.evaluate" => self.start_prompt(PromptKind::DebugEvaluate, String::new()),
            "explorer.newFile" => self.start_prompt(
                PromptKind::CreateFile,
                self.explorer_dir
                    .join("new-file")
                    .to_string_lossy()
                    .into_owned(),
            ),
            "explorer.newFolder" => self.start_prompt(
                PromptKind::CreateFolder,
                self.explorer_dir
                    .join("new-folder")
                    .to_string_lossy()
                    .into_owned(),
            ),
            "renameFile" => {
                if let Some(entry) = self.entries.get(self.explorer_selected).cloned() {
                    self.start_prompt(
                        PromptKind::RenameFile(entry.path.clone()),
                        entry.path.to_string_lossy().into_owned(),
                    );
                }
            }
            "deleteFile" => {
                if let Some(entry) = self.entries.get(self.explorer_selected) {
                    self.modal = Some(Modal::Trash(entry.path.clone()));
                }
            }
            "workbench.files.action.refreshFilesExplorer" => self.refresh_files(),
            "workbench.action.splitEditor" | "workbench.action.splitEditorRight" => {
                self.split_editor(false)
            }
            "workbench.action.splitEditorDown" => self.split_editor(true),
            "workbench.action.focusFirstEditorGroup" => self.focus_pane(0),
            "workbench.action.focusSecondEditorGroup" => self.focus_pane(1),
            "workbench.action.focusThirdEditorGroup" => self.focus_pane(2),
            "workbench.action.focusFourthEditorGroup" => self.focus_pane(3),
            "workbench.action.focusNextGroup" => {
                if !self.panes.is_empty() { self.focus_pane((self.active_pane + 1) % self.panes.len()); }
            }
            "workbench.action.closeEditorsInGroup" => self.close_pane(),
            "workbench.view.scm" | "git.refresh" => {
                self.modal = Some(Modal::Git);
                self.start_git(crate::git::Action::Status);
            }
            "git.stage" => self.git_file_action("stage"),
            "git.unstage" => self.git_file_action("unstage"),
            "git.openChange" => self.git_file_action("diff"),
            "git.commit" => self.start_prompt(PromptKind::GitCommit, String::new()),
            "git.viewHistory" => self.start_git(crate::git::Action::Log),
            "workbench.action.tasks.runTask" => self.show_tasks(false, args.as_str()),
            "workbench.action.tasks.build" => self.show_tasks(true, None),
            "workbench.action.tasks.terminate" => {
                if self
                    .terminals
                    .get(self.active_terminal)
                    .is_some_and(|t| t.title.starts_with("Task: "))
                {
                    self.kill_terminal();
                } else {
                    self.message = "The active terminal is not a task".into();
                }
            }
            "workbench.action.terminal.toggleTerminal" | "workbench.action.togglePanel" => {
                if self.terminals.is_empty() {
                    self.new_terminal();
                } else {
                    self.terminal_visible = !self.terminal_visible;
                    self.focus = if self.terminal_visible {
                        Focus::Terminal
                    } else {
                        Focus::Editor
                    };
                }
            }
            "workbench.action.terminal.new" => self.new_terminal(),
            "workbench.action.terminal.kill" => self.kill_terminal(),
            "workbench.action.focusActiveEditorGroup" => self.focus = Focus::Editor,
            "workbench.action.terminal.focus" => {
                if self.terminals.is_empty() {
                    self.new_terminal();
                } else {
                    self.terminal_visible = true;
                    self.focus = Focus::Terminal;
                }
            }
            "workbench.action.terminal.focusNext" => {
                if !self.terminals.is_empty() {
                    self.active_terminal = (self.active_terminal + 1) % self.terminals.len();
                }
            }
            "workbench.action.terminal.focusPrevious" => {
                if !self.terminals.is_empty() {
                    self.active_terminal =
                        (self.active_terminal + self.terminals.len() - 1) % self.terminals.len();
                }
            }
            "editor.action.showHover" => self.language_request("textDocument/hover", Value::Null),
            "editor.action.triggerSuggest" => self.language_request(
                "textDocument/completion",
                json!({"context":{"triggerKind":1}}),
            ),
            "editor.action.revealDefinition" => {
                self.language_request("textDocument/definition", Value::Null)
            }
            "editor.action.goToReferences" => self.language_request(
                "textDocument/references",
                json!({"context":{"includeDeclaration":true}}),
            ),
            "editor.action.formatDocument" => self.language_request(
                "textDocument/formatting",
                json!({"options":{"tabSize":self.doc().tab_size,"insertSpaces":self.doc().insert_spaces}}),
            ),
            "editor.action.rename" => self.start_prompt(
                PromptKind::Rename,
                self.doc().selected_text().unwrap_or_default(),
            ),
            "workbench.actions.view.problems" => self.show_problems(),
            "editor.action.insertSnippet" => self.insert_snippet(&args),
            "jumpToNextSnippetPlaceholder" | "jumpToPrevSnippetPlaceholder" => {
                if let Err(error) = self.doc_mut().step_snippet(command == "jumpToPrevSnippetPlaceholder") {
                    self.message = format!("Snippet navigation failed: {error}");
                }
            }
            "leaveSnippet" => self.doc_mut().leave_snippet(),
            "type" => {
                if let Some(text) = args.get("text").and_then(Value::as_str) {
                    self.doc_mut().insert(text, false);
                }
            }
            "workbench.action.files.newUntitledFile" => {
                self.cancel_navigation();
                let mut doc = Document::default();
                self.settings.apply(&mut doc);
                self.documents.push(doc);
                self.active = self.documents.len() - 1;
                self.focus = Focus::Editor;
            }
            "workbench.action.files.openFile" => self.start_prompt(
                PromptKind::Open,
                self.workspace.root.to_string_lossy().into_owned() + "/",
            ),
            "workbench.action.files.save" => {
                self.save(None);
            }
            "workbench.action.files.saveAs" => self.save_as(),
            "workbench.action.closeActiveEditor" => {
                if self
                    .panes
                    .iter()
                    .filter(|p| p.document == self.doc().id)
                    .count()
                    > 1
                {
                    self.close_pane();
                } else {
                    self.request_close(AfterSave::Close);
                }
            }
            "workbench.action.closeWindow" | "workbench.action.quit" => {
                self.request_close(AfterSave::Quit)
            }
            "workbench.action.closeAllEditors" => self.request_close(AfterSave::CloseAll),
            "workbench.action.files.revert" => self.modal = Some(Modal::Revert),
            "workbench.action.openRecent" => self.start_prompt(PromptKind::RecentFiles, String::new()),
            "workbench.action.reopenClosedEditor" => self.reopen_closed(),
            "workbench.action.quickOpen" => self.start_prompt(PromptKind::QuickOpen, String::new()),
            "workbench.action.showCommands" => {
                self.start_prompt(PromptKind::Palette, String::new())
            }
            "workbench.action.toggleSidebarVisibility" => {
                self.sidebar = !self.sidebar;
                if !self.sidebar {
                    self.focus = Focus::Editor;
                }
            }
            "workbench.view.explorer" => {
                self.sidebar = true;
                self.focus = Focus::Explorer;
                self.entries = directory_entries(&self.explorer_dir);
            }
            "workbench.action.nextEditor" => {
                self.active = (self.active + 1) % self.documents.len();
                self.focus = Focus::Editor;
            }
            "workbench.action.previousEditor" => {
                self.active = (self.active + self.documents.len() - 1) % self.documents.len();
                self.focus = Focus::Editor;
            }
            "workbench.action.openGlobalKeybindings" => self.modal = Some(Modal::Keys),
            "vscli.keyboardInspector" => self.modal = Some(Modal::Inspector),
            "vscli.help" => self.modal = Some(Modal::Help),
            "undo" => self.doc_mut().undo(),
            "vscli.settings.report" => {
                self.modal = Some(Modal::Text {
                    title: "Settings Compatibility".into(),
                    text: {
                        let mut report = if self.settings.warnings.is_empty() {
                            "No unsupported setting entries detected.\nNative editing settings: editor.tabSize (1–16), editor.insertSpaces, editor.lineNumbers.\nColor-theme selection is resolved at startup; use Preferences: Color Theme Report for appearance limits.".into()
                        } else { self.settings.warnings.join("\n") };
                        if !self.imported_keybinding_notices.is_empty() {
                            report = format!("{report}\n\nImported keybindings:\n{}", self.imported_keybinding_notices.join("\n"));
                        }
                        report
                    },
                    scroll: 0,
                });
            }
            "workbench.action.openSettings" | "workbench.action.openSettingsJson" => {
                if let Some(path) = self.settings_user.clone().or_else(|| crate::recovery::config_path().map(|p| p.with_file_name("settings.json"))) {
                    let result = (|| -> Result<()> {
                        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
                        self.open(&path)?;
                        if self.doc().is_empty() && self.doc().disk_content.is_none() { self.doc_mut().insert("{\n}\n", false); }
                        Ok(())
                    })();
                    if let Err(error) = result { self.message = format!("Cannot open settings: {error:#}"); }
                } else { self.message = "Settings location unavailable; use --settings PATH".into(); }
            }
            "redo" => self.doc_mut().redo(),
            "editor.action.selectAll" => self.doc_mut().select_all(),
            "expandLineSelection" => self.doc_mut().select_line(),
            "deleteLeft" => self.doc_mut().backspace(false),
            "deleteRight" => self.doc_mut().delete(false),
            "deleteWordLeft" => self.doc_mut().backspace(true),
            "deleteWordRight" => self.doc_mut().delete(true),
            "lineBreakInsert" => self.doc_mut().newline(),
            "tab" => {
                if self
                    .doc()
                    .selections()
                    .iter()
                    .any(|s| !s.range().is_empty())
                {
                    self.doc_mut().transform_lines(false, None);
                } else {
                    self.doc_mut().replace_cursors(|doc, selection| {
                        let row = doc.text.char_to_line(selection.cursor);
                        let prefix = doc
                            .text
                            .slice(doc.line_start(row)..selection.cursor)
                            .to_string();
                        if doc.insert_spaces { " ".repeat(doc.tab_size - doc.display_width(&prefix) % doc.tab_size) }
                        else { "\t".into() }
                    });
                }
            }
            "editor.action.indentLines" => self.doc_mut().transform_lines(false, None),
            "outdent" | "editor.action.outdentLines" => self.doc_mut().transform_lines(true, None),
            "cancelSelection" => self.doc_mut().clear_secondary(),
            "editor.action.addSelectionToNextFindMatch" => {
                self.doc_mut().select_next_occurrence(false);
            }
            "editor.action.selectHighlights" | "editor.action.changeAll" => {
                if !self.doc_mut().select_next_occurrence(true) {
                    self.message =
                        "No matching word, or selection exceeds the 10,000-cursor limit".into();
                }
            }
            "editor.action.insertCursorAbove" => self.doc_mut().add_cursor_vertical(false),
            "editor.action.insertCursorBelow" => self.doc_mut().add_cursor_vertical(true),
            "editor.action.insertCursorAtEndOfEachLineSelected" => {
                self.doc_mut().cursors_at_line_ends()
            }
            "editor.action.insertLineBefore" => self.doc_mut().insert_line(true),
            "editor.action.insertLineAfter" => self.doc_mut().insert_line(false),
            "editor.action.copyLinesUpAction" => self.doc_mut().copy_lines(false),
            "editor.action.copyLinesDownAction" => self.doc_mut().copy_lines(true),
            "editor.action.moveLinesUpAction" => self.doc_mut().move_lines(false),
            "editor.action.moveLinesDownAction" => self.doc_mut().move_lines(true),
            "editor.action.jumpToBracket" => {
                self.doc_mut().jump_bracket();
            }
            "editor.action.deleteLines" => self.doc_mut().delete_line(),
            "editor.action.commentLine" => self.doc_mut().transform_lines(false, Some(false)),
            "editor.action.addCommentLine" => {
                self.add_comments();
            }
            "editor.action.removeCommentLine" => self.doc_mut().transform_lines(false, Some(true)),
            "editor.action.clipboardCopyAction" => self.copy(false),
            "editor.action.clipboardCutAction" => self.copy(true),
            "editor.action.clipboardPasteAction" => self.paste(),
            "workbench.action.findInFiles" => self.start_prompt(
                PromptKind::WorkspaceSearch,
                self.active_document().and_then(Document::selected_text).unwrap_or_else(|| {
                    self.search
                        .as_ref()
                        .map_or(String::new(), |s| s.query.clone())
                }),
            ),
            "actions.find" => self.start_prompt(
                PromptKind::Find,
                self.doc()
                    .selected_text()
                    .unwrap_or(self.find_query.clone()),
            ),
            "editor.action.startFindReplaceAction" => self.start_prompt(
                PromptKind::ReplaceQuery,
                self.doc()
                    .selected_text()
                    .unwrap_or(self.find_query.clone()),
            ),
            "editor.action.nextMatchFindAction" => self.find(false),
            "editor.action.previousMatchFindAction" => self.find(true),
            "workbench.action.gotoLine" => self.start_prompt(PromptKind::Goto, String::new()),
            "workbench.extensions.action.installVSIX" => self.start_prompt(PromptKind::InstallExtension, String::new()),
            "workbench.view.extensions" | "workbench.extensions.action.showInstalledExtensions" => self.manage_extension(extension_management::Action::List),
            "vscli.extensions.stop" => self.stop_extension_host(),
            "vscli.extensions.restart" => self.restart_extensions(),
            "vscli.extensions.stopSelected" => {
                if let Some(id) = args.as_str().or_else(|| args["id"].as_str()) { self.stop_selected_extension(id); }
                else { self.start_prompt(PromptKind::StopExtension, String::new()); }
            },
            _ => self.execute_extension(command, command_args),
        }
    }
    fn requires_editor(command: &str) -> bool {
        command.starts_with("editor.")
            || command.starts_with("cursor")
            || matches!(
                command,
                "undo"
                    | "redo"
                    | "type"
                    | "deleteLeft"
                    | "deleteRight"
                    | "deleteWordLeft"
                    | "deleteWordRight"
                    | "lineBreakInsert"
                    | "tab"
                    | "outdent"
                    | "cancelSelection"
                    | "expandLineSelection"
                    | "jumpToNextSnippetPlaceholder"
                    | "jumpToPrevSnippetPlaceholder"
                    | "leaveSnippet"
                    | "actions.find"
                    | "workbench.action.gotoLine"
                    | "workbench.action.files.save"
                    | "workbench.action.files.saveAs"
                    | "workbench.action.files.revert"
                    | "workbench.action.closeActiveEditor"
                    | "workbench.action.nextEditor"
                    | "workbench.action.previousEditor"
            )
    }
    fn add_comments(&mut self) {
        // Force adding only where absent; toggle alone would remove existing comments.
        let prefix = if matches!(self.language(), "python" | "toml" | "shellscript") {
            "#"
        } else {
            "//"
        };
        let mut changes = Vec::new();
        for row in self.doc().all_selected_rows() {
            let line = self.doc().line(row);
            let indent = line.chars().take_while(|c| matches!(c, ' ' | '\t')).count();
            if !line.trim_start().starts_with(prefix) {
                let pos = self.doc().line_start(row) + indent;
                changes.push((pos..pos, format!("{prefix} ")));
            }
        }
        self.doc_mut().apply_changes(changes);
    }
    fn save_as(&mut self) {
        let path = self
            .doc()
            .path
            .clone()
            .unwrap_or_else(|| self.workspace.root.join("untitled.txt"));
        self.start_prompt(PromptKind::SaveAs, path.to_string_lossy().into_owned());
    }
    fn save(&mut self, after: Option<AfterSave>) {
        if self.file_job.is_some() {
            self.message = "Wait for the file operation to finish before saving".into();
            return;
        }
        if self.doc().path.is_none() {
            self.pending = after;
            self.save_as();
            return;
        }
        match self.doc_mut().save() {
            Ok(()) => {
                self.message = format!("Saved {}", self.doc().name());
                self.language_saved();
                self.remember_active_file();
                if let Some(action) = after {
                    self.complete_close(action);
                }
            }
            Err(e) => self.message = format!("Save failed: {e:#}"),
        }
    }
    fn request_close(&mut self, action: AfterSave) {
        if matches!(action, AfterSave::Quit | AfterSave::CloseAll) {
            if let Some(index) = self.documents.iter().position(Document::dirty) {
                self.active = index;
            } else {
                self.complete_close(action);
                return;
            }
        }
        if self.doc().dirty() {
            self.modal = Some(Modal::Confirm(action));
        } else {
            self.complete_close(action);
        }
    }
    fn remove_active(&mut self) {
        self.record_closed();
        self.documents.remove(self.active);
        self.active = self.active.min(self.documents.len().saturating_sub(1));
        self.sync_pane();
    }
    fn complete_close(&mut self, action: AfterSave) {
        match action {
            AfterSave::Close => self.remove_active(),
            AfterSave::Quit => {
                if self.documents.iter().any(Document::dirty) {
                    self.request_close(AfterSave::Quit);
                } else {
                    self.running = false;
                }
            }
            AfterSave::CloseAll => {
                if self.documents.iter().any(Document::dirty) {
                    self.request_close(AfterSave::CloseAll);
                } else {
                    while !self.documents.is_empty() {
                        self.remove_active();
                    }
                    self.sync_pane();
                    self.active = 0;
                }
            }
        }
    }
    fn modal_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            if matches!(self.modal, Some(Modal::Search))
                && self.search.as_ref().is_some_and(|s| s.running)
            {
                self.search = None;
            }
            self.modal = None;
            return;
        }
        let modal = self.modal.take().unwrap();
        match modal {
            Modal::ExtensionsLoading(id) => self.modal = Some(Modal::ExtensionsLoading(id)),
            Modal::Debug { section, selected } => self.debug_modal_key(key, section, selected),
            Modal::Trash(path) => {
                if matches!(key.code, KeyCode::Enter | KeyCode::Char('y' | 'Y')) {
                    self.start_file_job(crate::files::Action::Trash(path));
                } else if !matches!(key.code, KeyCode::Char('n' | 'N')) {
                    self.modal = Some(Modal::Trash(path));
                }
            }
            Modal::Text {
                title,
                text,
                mut scroll,
            } => {
                match key.code {
                    KeyCode::Up => scroll = scroll.saturating_sub(1),
                    KeyCode::Down => scroll = scroll.saturating_add(1),
                    KeyCode::PageUp => scroll = scroll.saturating_sub(15),
                    KeyCode::PageDown => scroll = scroll.saturating_add(15),
                    KeyCode::Home => scroll = 0,
                    KeyCode::End => scroll = text.lines().count().saturating_sub(1),
                    _ => {}
                }
                scroll = scroll.min(text.lines().count().saturating_sub(1));
                self.modal = Some(Modal::Text {
                    title,
                    text,
                    scroll,
                });
            }
            Modal::Git => {
                self.modal = Some(Modal::Git);
                let count = self.git_status.as_ref().map_or(0, |s| s.entries.len());
                match key.code {
                    KeyCode::Up => self.git_selected = self.git_selected.saturating_sub(1),
                    KeyCode::Down => {
                        self.git_selected = (self.git_selected + 1).min(count.saturating_sub(1))
                    }
                    KeyCode::Enter => self.git_file_action("open"),
                    KeyCode::Char('s') => self.git_file_action("stage"),
                    KeyCode::Char('u') => self.git_file_action("unstage"),
                    KeyCode::Char('d') => self.git_file_action("diff"),
                    KeyCode::Char('D') => self.git_file_action("staged_diff"),
                    KeyCode::Char('r') => self.start_git(crate::git::Action::Status),
                    KeyCode::Char('c') => {
                        self.modal = None;
                        self.start_prompt(PromptKind::GitCommit, String::new());
                    }
                    _ => {}
                }
            }
            Modal::Tasks {
                tasks,
                mut selected,
            } => {
                match key.code {
                    KeyCode::Up => selected = selected.saturating_sub(1),
                    KeyCode::Down => selected = (selected + 1).min(tasks.len().saturating_sub(1)),
                    KeyCode::Enter => {
                        if let Some(task) = tasks.get(selected) {
                            self.prepare_task(task);
                        }
                        return;
                    }
                    _ => {}
                }
                self.modal = Some(Modal::Tasks { tasks, selected });
            }
            Modal::ConfirmTask(task) => {
                if matches!(key.code, KeyCode::Enter | KeyCode::Char('y' | 'Y')) {
                    self.trusted_tasks = true;
                    self.run_task(task);
                } else if !matches!(key.code, KeyCode::Char('n' | 'N')) {
                    self.modal = Some(Modal::ConfirmTask(task));
                }
            }
            Modal::Extensions {
                items,
                mut selected,
            } => {
                match key.code {
                    KeyCode::Up => selected = selected.saturating_sub(1),
                    KeyCode::Down => selected = (selected + 1).min(items.len().saturating_sub(1)),
                    KeyCode::Enter => {
                        if let Some(item) = items.get(selected) {
                            if item.manifest["main"].is_string() {
                                self.modal = Some(Modal::RunExtension(item.clone()));
                            } else {
                                self.message = format!("{}: {}", item.id, item.compatibility);
                            }
                        }
                        return;
                    }
                    KeyCode::Char('s' | 'S') => {
                        if let Some(item) = items.get(selected) {
                            self.stop_selected_extension(&item.id);
                        }
                        return;
                    }
                    KeyCode::Char('h' | 'H') => {
                        self.restart_extensions();
                        return;
                    }
                    KeyCode::Delete => {
                        if let Some(item) = items.get(selected) {
                            self.manage_extension(extension_management::Action::Uninstall(
                                item.id.clone(),
                            ));
                        }
                        return;
                    }
                    KeyCode::Char('r' | 'R') => {
                        if let Some(item) = items.get(selected) {
                            self.manage_extension(extension_management::Action::Rollback(
                                item.id.clone(),
                            ));
                        }
                        return;
                    }
                    _ => {}
                }
                self.modal = Some(Modal::Extensions { items, selected });
            }
            Modal::RunExtension(item) => {
                if key.code == KeyCode::Enter {
                    self.run_installed_extension(item);
                } else {
                    self.modal = Some(Modal::RunExtension(item));
                }
            }
            Modal::Language {
                title,
                items,
                mut selected,
            } => {
                match key.code {
                    KeyCode::Up => selected = selected.saturating_sub(1),
                    KeyCode::Down => selected = (selected + 1).min(items.len().saturating_sub(1)),
                    KeyCode::PageUp => selected = selected.saturating_sub(10),
                    KeyCode::PageDown => {
                        selected = (selected + 10).min(items.len().saturating_sub(1))
                    }
                    KeyCode::Enter | KeyCode::Tab => {
                        if let Some(item) = items.get(selected)
                            && let Err(e) = self.language_action(&item.action)
                        {
                            self.message = format!("Language action failed: {e:#}");
                        }
                        return;
                    }
                    _ => {}
                }
                self.modal = Some(Modal::Language {
                    title,
                    items,
                    selected,
                });
            }
            Modal::Search => {
                let Some(search) = self.search.as_mut() else {
                    return;
                };
                match key.code {
                    KeyCode::Up => search.selected = search.selected.saturating_sub(1),
                    KeyCode::Down => {
                        search.selected =
                            (search.selected + 1).min(search.hits.len().saturating_sub(1))
                    }
                    KeyCode::PageUp => search.selected = search.selected.saturating_sub(10),
                    KeyCode::PageDown => {
                        search.selected =
                            (search.selected + 10).min(search.hits.len().saturating_sub(1))
                    }
                    KeyCode::Enter => {
                        if let Some(hit) = search.hits.get(search.selected).cloned() {
                            match self.open(&hit.path) {
                                Ok(()) => {
                                    self.doc_mut().clear_secondary();
                                    let row = hit.row.min(self.doc().line_count() - 1);
                                    let start = self.doc().line_start(row);
                                    if crate::search::line_hash(&self.doc().line(row))
                                        == hit.line_hash
                                    {
                                        self.doc_mut().move_to(start + hit.column, false);
                                        self.doc_mut()
                                            .move_to(start + hit.column + hit.length, true);
                                    } else {
                                        self.doc_mut().move_to(start, false);
                                        self.message = "Search result changed; rerun Find in Files for current matches".into();
                                    }
                                    return;
                                }
                                Err(e) => self.message = format!("Open failed: {e:#}"),
                            }
                        }
                    }
                    _ => {}
                }
                self.modal = Some(Modal::Search);
            }
            Modal::Confirm(action) => match key.code {
                KeyCode::Enter | KeyCode::Char('s' | 'S') => self.save(Some(action)),
                KeyCode::Char('d' | 'D') => {
                    self.remove_active();
                    if !matches!(action, AfterSave::Close) {
                        self.complete_close(action);
                    }
                }
                KeyCode::Char('c' | 'C') => {}
                _ => self.modal = Some(Modal::Confirm(action)),
            },
            Modal::Revert => match key.code {
                KeyCode::Char('y' | 'Y') => {
                    if let Some(path) = self.doc().path.clone() {
                        match crate::document::read_disk(&path) {
                            Ok(Some(content)) => {
                                self.doc_mut().reload_content(content);
                                self.message = "Reloaded from disk".into();
                            }
                            Ok(None) => {
                                self.message = "File no longer exists; buffer retained".into()
                            }
                            Err(e) => self.message = format!("Reload failed: {e:#}"),
                        }
                    } else {
                        self.message = "Untitled file has no disk version".into();
                    }
                }
                KeyCode::Char('n' | 'N') => {}
                _ => self.modal = Some(Modal::Revert),
            },
            other => self.modal = Some(other),
        }
    }
    fn prompt_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            self.prompt = None;
            self.pending = None;
            return;
        }
        if key.code == KeyCode::Enter {
            self.accept_prompt();
            return;
        }
        let p = self.prompt.as_mut().unwrap();
        if matches!(p.kind, PromptKind::WorkspaceSearch)
            && key.modifiers.contains(KeyModifiers::ALT)
        {
            match key.code {
                KeyCode::Char('c') => {
                    self.search_options.case_sensitive = !self.search_options.case_sensitive
                }
                KeyCode::Char('r') => self.search_options.regex = !self.search_options.regex,
                KeyCode::Char('w') => {
                    self.search_options.whole_word = !self.search_options.whole_word
                }
                _ => {}
            }
            return;
        }
        let primary = if self.keymap.profile == Profile::Macos {
            KeyModifiers::SUPER
        } else {
            KeyModifiers::CONTROL
        };
        if key.code == KeyCode::Char('a') && key.modifiers.contains(primary) {
            p.select_all = true;
            return;
        }
        match key.code {
            KeyCode::Up => p.selected = p.selected.saturating_sub(1),
            KeyCode::Down => p.selected = (p.selected + 1).min(99),
            KeyCode::Home => {
                p.cursor = 0;
                p.select_all = false;
            }
            KeyCode::End => {
                p.cursor = p.text.len();
                p.select_all = false;
            }
            KeyCode::Left => {
                p.cursor = p.text[..p.cursor]
                    .grapheme_indices(true)
                    .next_back()
                    .map_or(0, |(b, _)| b);
                p.select_all = false;
            }
            KeyCode::Right => {
                p.cursor += p.text[p.cursor..]
                    .graphemes(true)
                    .next()
                    .map_or(0, str::len);
                p.select_all = false;
            }
            KeyCode::Backspace => {
                if p.select_all {
                    p.text.clear();
                    p.cursor = 0;
                } else if p.cursor > 0 {
                    let prev = p.text[..p.cursor]
                        .grapheme_indices(true)
                        .next_back()
                        .unwrap()
                        .0;
                    p.text.replace_range(prev..p.cursor, "");
                    p.cursor = prev;
                }
                p.select_all = false;
                p.selected = 0;
            }
            KeyCode::Delete => {
                if p.select_all {
                    p.text.clear();
                    p.cursor = 0;
                } else {
                    let end = p.cursor
                        + p.text[p.cursor..]
                            .graphemes(true)
                            .next()
                            .map_or(0, str::len);
                    p.text.replace_range(p.cursor..end, "");
                }
                p.select_all = false;
                p.selected = 0;
            }
            KeyCode::Char(c)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL
                        | KeyModifiers::SUPER
                        | KeyModifiers::ALT
                        | KeyModifiers::META,
                ) =>
            {
                p.insert(&c.to_string())
            }
            _ => {}
        }
    }
    fn accept_prompt(&mut self) {
        let p = self.prompt.take().unwrap();
        match p.kind {
            PromptKind::RecentFiles => self.accept_recent(&p.text, p.selected),
            PromptKind::Snippet => self.accept_snippet(&p.text, p.selected),
            PromptKind::InstallExtension => self.manage_extension(
                extension_management::Action::Install(self.resolve_path(&p.text)),
            ),
            PromptKind::Theme => self.accept_theme(&p.text, p.selected),
            PromptKind::StopExtension => self.stop_selected_extension(p.text.trim()),
            PromptKind::ThemeFile => self.load_theme(self.resolve_path(&p.text)),
            PromptKind::DebugEvaluate => {
                if let Some(client) = self.debugger.as_mut() {
                    if let Err(e) = client.evaluate(&p.text) {
                        self.message = e.to_string();
                    }
                    self.modal = Some(Modal::Debug {
                        section: 3,
                        selected: 0,
                    });
                } else {
                    self.message = "Start a debug session first".into();
                }
            }
            PromptKind::CreateFile => {
                self.start_file_job(crate::files::Action::CreateFile(self.resolve_path(&p.text)))
            }
            PromptKind::CreateFolder => self.start_file_job(crate::files::Action::CreateFolder(
                self.resolve_path(&p.text),
            )),
            PromptKind::RenameFile(from) => self.start_file_job(crate::files::Action::Rename {
                from,
                to: self.resolve_path(&p.text),
            }),
            PromptKind::GitCommit => {
                if !p.text.trim().is_empty() {
                    self.start_git(crate::git::Action::Commit(p.text));
                }
            }
            PromptKind::Rename => {
                if !p.text.trim().is_empty() {
                    self.language_request("textDocument/rename", json!({"newName":p.text}));
                }
            }
            PromptKind::WorkspaceSearch => {
                if p.text.is_empty() {
                    return;
                }
                let overlays = self
                    .documents
                    .iter()
                    .filter_map(|d| d.path.clone().map(|p| (p, d.text.to_string())))
                    .collect();
                match crate::search::Search::start(
                    self.workspace.root.clone(),
                    p.text.clone(),
                    self.search_options.clone(),
                    overlays,
                ) {
                    Ok(search) => {
                        self.search = Some(search);
                        self.modal = Some(Modal::Search);
                    }
                    Err(e) => {
                        self.message = format!("Search failed: {e}");
                        self.start_prompt(PromptKind::WorkspaceSearch, p.text);
                    }
                }
            }
            PromptKind::Palette => {
                let items = self.palette_items(&p.text);
                if let Some((_, id)) = items.get(p.selected.min(items.len().saturating_sub(1))) {
                    let id = id.to_string();
                    self.execute(&id, Value::Null);
                }
            }
            PromptKind::QuickOpen => {
                if p.text.starts_with('>') {
                    self.start_prompt(PromptKind::Palette, p.text[1..].into());
                    return;
                }
                let paths = self.workspace.matches(&p.text);
                if let Some(path) = paths.get(p.selected.min(paths.len().saturating_sub(1))) {
                    if let Err(e) = self.open(path) {
                        self.message = format!("Open failed: {e:#}");
                    }
                } else {
                    self.message = "No matching files. Use Open File to enter a new path.".into();
                }
            }
            PromptKind::Open => {
                let path = self.resolve_path(&p.text);
                if let Err(e) = self.open(&path) {
                    self.message = format!("Open failed: {e:#}");
                }
            }
            PromptKind::SaveAs => {
                if self.file_job.is_some() {
                    self.message = "Wait for the file operation to finish before saving".into();
                    self.pending = None;
                    return;
                }
                let path = self.resolve_path(&p.text);
                if self
                    .documents
                    .iter()
                    .enumerate()
                    .any(|(i, d)| i != self.active && d.path.as_ref() == Some(&path))
                {
                    self.message = "That file is already open in another tab".into();
                    self.pending = None;
                    return;
                }
                match self.doc_mut().save_to(&path, false) {
                    Ok(()) => {
                        self.message = format!("Saved {}", self.doc().name());
                        self.settings.apply(&mut self.documents[self.active]);
                        self.remember_active_file();
                        if let Some(after) = self.pending.take() {
                            self.complete_close(after);
                        }
                    }
                    Err(e) => {
                        self.message = format!("Save failed: {e:#}");
                        self.pending = None;
                    }
                }
            }
            PromptKind::Find => {
                self.find_query = p.text;
                self.find(false);
            }
            PromptKind::ReplaceQuery => {
                if !p.text.is_empty() {
                    self.start_prompt(PromptKind::ReplaceWith(p.text), String::new());
                }
            }
            PromptKind::ReplaceWith(query) => {
                let count = self.doc_mut().replace_all(&query, &p.text);
                self.find_query = query;
                self.message = format!("Replaced {count} matches · Undo to restore");
            }
            PromptKind::Goto => {
                let mut parts = p.text.trim().split(':');
                if let Ok(line) = parts.next().unwrap_or("").parse::<usize>() {
                    let col = parts
                        .next()
                        .and_then(|s| s.parse::<usize>().ok())
                        .unwrap_or(1);
                    let pos = self
                        .doc()
                        .position_at(line.saturating_sub(1), col.saturating_sub(1));
                    self.doc_mut().move_to(pos, false);
                } else {
                    self.message = "Enter a line number, optionally line:column".into();
                }
            }
        }
    }
    fn resolve_path(&self, text: &str) -> PathBuf {
        if let Some(rest) = text.strip_prefix("~/")
            && let Some(home) = directories::BaseDirs::new()
        {
            return home.home_dir().join(rest);
        }
        let path = PathBuf::from(text);
        if path.is_absolute() {
            path
        } else {
            self.workspace.root.join(path)
        }
    }
    fn find(&mut self, backwards: bool) {
        let query = self.find_query.clone();
        if query.is_empty() {
            self.start_prompt(PromptKind::Find, String::new());
        } else if self.doc_mut().find(&query, backwards) {
            self.message = format!(
                "Found {query:?} · next: {}",
                self.keymap.shortcut("editor.action.nextMatchFindAction")
            );
        } else {
            self.message = format!("No matches for {query:?}");
        }
    }
    fn explorer_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Down => {
                self.explorer_selected =
                    (self.explorer_selected + 1).min(self.entries.len().saturating_sub(1))
            }
            KeyCode::Up => self.explorer_selected = self.explorer_selected.saturating_sub(1),
            KeyCode::Enter | KeyCode::Right => self.explorer_open(),
            KeyCode::Left | KeyCode::Backspace => {
                if let Some(parent) = self.explorer_dir.parent() {
                    self.explorer_dir = parent.to_path_buf();
                    self.entries = directory_entries(&self.explorer_dir);
                    self.explorer_selected = 0;
                }
            }
            KeyCode::Esc => self.focus = Focus::Editor,
            _ => {}
        }
    }
    fn explorer_open(&mut self) {
        if let Some(entry) = self.entries.get(self.explorer_selected).cloned() {
            if entry.directory {
                self.explorer_dir = entry.path;
                self.entries = directory_entries(&self.explorer_dir);
                self.explorer_selected = 0;
            } else if let Err(e) = self.open(&entry.path) {
                self.message = format!("Open failed: {e:#}");
            }
        }
    }
    fn copy(&mut self, cut: bool) {
        let mut selections = self.doc().selections();
        selections.sort_by_key(|s| s.range().start);
        self.clipboard_line = selections.iter().all(|s| s.range().is_empty());
        self.clipboard = if self.clipboard_line {
            self.doc()
                .all_selected_rows()
                .into_iter()
                .map(|row| format!("{}{}", self.doc().line(row), self.doc().eol))
                .collect::<String>()
        } else {
            selections
                .iter()
                .map(|s| self.doc().text.slice(s.range()).to_string())
                .collect::<Vec<_>>()
                .join(&self.doc().eol)
        };
        let system = clipboard_write(&self.clipboard);
        if cut {
            if self.clipboard_line {
                self.doc_mut().delete_line();
            } else {
                self.doc_mut().replace_cursors(|_, _| String::new());
            }
        }
        self.message = if system {
            "Copied to system clipboard"
        } else {
            "Copied internally; install wl-clipboard/xclip for system clipboard integration"
        }
        .into();
    }
    fn paste(&mut self) {
        let external = clipboard_read();
        let text = external.clone().unwrap_or_else(|| self.clipboard.clone());
        if text.is_empty() {
            self.message =
                "Clipboard is empty or unavailable; terminal paste is also supported".into();
            return;
        }
        if self.clipboard_line && text == self.clipboard && self.doc().selection().is_none() {
            let selections = self
                .doc()
                .selections()
                .iter()
                .map(|s| {
                    crate::document::Selection::caret(
                        self.doc()
                            .line_start(self.doc().text.char_to_line(s.cursor)),
                    )
                })
                .collect();
            self.doc_mut().set_selections(selections);
        }
        if !self.doc().secondary.is_empty() {
            let lines: Vec<_> = text.lines().collect();
            let mut selections = self.doc().selections();
            selections.sort_by_key(|s| s.range().start);
            if !self.clipboard_line && lines.len() == selections.len() {
                self.doc_mut().replace_cursors(|_, selection| {
                    let index = selections.iter().position(|s| s == selection).unwrap();
                    lines[index].to_owned()
                });
                return;
            }
        }
        self.doc_mut().insert(&text, false);
    }
}

fn clipboard_write(text: &str) -> bool {
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard", "-in"]),
        ]
    };
    for (name, args) in candidates {
        if clipboard_command(name, args, Some(text)).is_some() {
            return true;
        }
    }
    false
}
fn clipboard_read() -> Option<String> {
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbpaste", &[])]
    } else {
        &[
            ("wl-paste", &["--no-newline"]),
            ("xclip", &["-selection", "clipboard", "-out"]),
        ]
    };
    for (name, args) in candidates {
        if let Some(text) = clipboard_command(name, args, None) {
            return Some(text);
        }
    }
    None
}

fn clipboard_command(name: &str, args: &[&str], input: Option<&str>) -> Option<String> {
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    // A stalled display server must not hang the editor indefinitely. Pipe I/O
    // happens in a worker so the deadline covers large or blocked transfers too.
    let mut child = Command::new(name)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(if input.is_none() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let (tx, rx) = mpsc::channel();
    let payload = input.map(str::to_owned);
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    std::thread::spawn(move || {
        let result = if let Some(text) = payload {
            stdin
                .and_then(|mut pipe| pipe.write_all(text.as_bytes()).ok())
                .map(|_| String::new())
        } else {
            stdout.and_then(|pipe| {
                let mut bytes = Vec::new();
                pipe.take(crate::document::MAX_FILE_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .ok()?;
                if bytes.len() as u64 > crate::document::MAX_FILE_BYTES {
                    return None;
                }
                String::from_utf8(bytes).ok()
            })
        };
        let _ = tx.send(result);
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    rx.recv_timeout(Duration::from_millis(50)).ok().flatten()
                } else {
                    None
                };
            }
            Ok(None) if started.elapsed() < Duration::from_millis(300) => {
                std::thread::sleep(Duration::from_millis(5))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
        app.event(Event::Key(KeyEvent::new(code, modifiers)));
    }
    #[test]
    fn editing_shortcuts_save_and_close_guard() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.txt");
        let mut a = App::new(dir.path().into(), Profile::Linux);
        a.execute("workbench.action.files.newUntitledFile", Value::Null);
        a.open(&p).unwrap();
        for c in "hello".chars() {
            key(&mut a, KeyCode::Char(c), KeyModifiers::NONE);
        }
        key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "hello");
        key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
        key(&mut a, KeyCode::Char('x'), KeyModifiers::NONE);
        key(&mut a, KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert!(matches!(a.modal, Some(Modal::Confirm(_))));
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert_eq!(a.doc().text.to_string(), "hello");
        assert!(!a.doc().dirty());
    }
    #[test]
    fn empty_workbench_stays_empty_until_an_editor_is_opened() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(dir.path().into(), Profile::Linux);
        assert!(app.documents.is_empty());
        assert!(app.panes.is_empty());
        assert_eq!(app.context()["editorTextFocus"], json!(false));
        app.poll();
        app.event(Event::Resize(80, 24));
        app.event(Event::Paste("ignored".into()));
        key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
        for command in COMMANDS
            .iter()
            .map(|(_, command)| *command)
            .filter(|command| App::requires_editor(command))
        {
            app.execute(command, Value::Null);
            assert!(app.documents.is_empty(), "{command}");
            assert!(app.prompt.is_none(), "{command}");
        }
        for command in [
            "workbench.action.splitEditor",
            "workbench.action.closeEditorsInGroup",
            "workbench.action.focusNextGroup",
            "workbench.action.focusFirstEditorGroup",
            "workbench.action.closeAllEditors",
        ] {
            app.execute(command, Value::Null);
            assert!(app.documents.is_empty(), "{command}");
        }
        app.execute("workbench.action.showCommands", Value::Null);
        assert!(matches!(
            app.prompt.as_ref().map(|p| &p.kind),
            Some(PromptKind::Palette)
        ));
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        app.execute("workbench.action.findInFiles", Value::Null);
        assert!(matches!(
            app.prompt.as_ref().map(|p| &p.kind),
            Some(PromptKind::WorkspaceSearch)
        ));
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(app.documents.is_empty());
        key(&mut app, KeyCode::Char('n'), KeyModifiers::CONTROL);
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.panes.len(), 1);
        assert_eq!(app.context()["editorTextFocus"], json!(true));
        app.execute("workbench.action.closeActiveEditor", Value::Null);
        assert!(app.documents.is_empty());
        assert!(app.panes.is_empty());
        let path = dir.path().join("file.txt");
        std::fs::write(&path, "saved").unwrap();
        app.open(&path).unwrap();
        app.doc_mut().insert("unsaved", false);
        app.execute("workbench.action.closeActiveEditor", Value::Null);
        assert!(matches!(app.modal, Some(Modal::Confirm(_))));
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(app.doc().dirty());
        app.execute("workbench.action.files.save", Value::Null);
        app.execute("workbench.action.closeActiveEditor", Value::Null);
        assert!(app.documents.is_empty());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "unsavedsaved");
        app.execute("workbench.action.quit", Value::Null);
        assert!(!app.running);
    }

    #[test]
    fn multicursor_commands_use_platform_shortcuts() {
        for profile in [Profile::Linux, Profile::Windows, Profile::Macos] {
            let dir = tempfile::tempdir().unwrap();
            let mut app = App::new(dir.path().into(), profile);
            app.execute("workbench.action.files.newUntitledFile", Value::Null);
            app.documents[0] = Document::from_text("cat cat\ncat");
            let primary = if profile == Profile::Macos {
                KeyModifiers::SUPER
            } else {
                KeyModifiers::CONTROL
            };
            key(&mut app, KeyCode::Char('d'), primary);
            key(&mut app, KeyCode::Char('d'), primary);
            key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
            assert_eq!(app.doc().text.to_string(), "x x\ncat");
            key(&mut app, KeyCode::Char('z'), primary);
            assert_eq!(app.doc().selections().len(), 2);
            key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
            assert!(app.doc().secondary.is_empty());
            let modifiers = match profile {
                Profile::Linux => KeyModifiers::SHIFT | KeyModifiers::ALT,
                Profile::Windows => KeyModifiers::CONTROL | KeyModifiers::ALT,
                Profile::Macos => KeyModifiers::SUPER | KeyModifiers::ALT,
            };
            key(&mut app, KeyCode::Down, modifiers);
            assert_eq!(app.doc().selections().len(), 2);
        }
    }
    #[test]
    fn palette_and_chord_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::new(dir.path().into(), Profile::Linux);
        a.execute("workbench.action.files.newUntitledFile", Value::Null);
        key(
            &mut a,
            KeyCode::Char('p'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert!(matches!(
            a.prompt.as_ref().map(|p| &p.kind),
            Some(PromptKind::Palette)
        ));
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        a.doc_mut().insert("hello", false);
        key(&mut a, KeyCode::Char('k'), KeyModifiers::CONTROL);
        key(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(a.doc().text.to_string(), "// hello");
        key(&mut a, KeyCode::Char('k'), KeyModifiers::CONTROL);
        key(&mut a, KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(a.doc().text.to_string(), "hello");
    }
}
