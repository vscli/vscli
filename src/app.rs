mod actions;
mod breadcrumbs;
mod code_actions;
mod completion_edits;
mod debugger;
mod editor_layout;
mod extension_activation;
mod extension_management;
mod extension_prompts;
mod extension_providers;
mod extension_services;
pub mod extension_surfaces;
mod extensions;
mod files;
pub mod keyboard;
mod language;
mod language_services;
mod navigation;
mod navigation_history;
mod outline;
mod panes;
mod preview_tabs;
mod save_code_actions;
mod save_formatting;
mod saving;
mod settings_persistence;
mod signature_help;
mod sticky_tabs;
mod suggestions;
mod tab_reordering;
mod tab_transfer;
mod workspace_edits;

mod session;
mod snippet_catalogs;
mod snippets;
mod source_control;
mod spatial_focus;
mod symbols;
mod tasks;
mod terminals;
mod themes;
mod typing;
mod watching;
use crate::{
    document::Document,
    keys::{self, Keymap, Profile, Resolution},
    workspace::{Entry, Workspace, directory_entries, score},
};
use anyhow::Result;
pub use breadcrumbs::{BreadcrumbPickerView, BreadcrumbsView};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
pub use language::{Diagnostics, LanguageAction, LanguageItem};
pub use outline::{OutlineStatus, OutlineView};
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
            "vscli.extensions.enableGlobal",
            "vscli.extensions.disableGlobal",
            "vscli.extensions.enableWorkspace",
            "vscli.extensions.disableWorkspace",
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
    ("View: Keep Editor", "workbench.action.keepEditor"),
    ("View: Pin Editor", "workbench.action.pinEditor"),
    ("View: Unpin Editor", "workbench.action.unpinEditor"),
    (
        "View: Close Pinned Editor",
        "workbench.action.closeActivePinnedEditor",
    ),
    ("Language: Restart Server", "vscli.languageServer.restart"),
    ("Language: Disable Services", "vscli.languageServer.disable"),
    ("Language: Enable Services", "vscli.languageServer.enable"),
    ("Language: Server Status", "vscli.languageServer.status"),
    ("Extensions: Output Channels", "vscli.extensions.output"),
    ("Extensions: Status Items", "vscli.extensions.status"),
    ("Extensions: Tree Views", "vscli.extensions.trees"),
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
    ("Extensions: Search Open VSX", "vscli.extensions.search"),
    ("Extensions: Check for Updates", "vscli.extensions.updates"),
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
    (
        "File: Restore Previous Clean Session",
        "vscli.session.restore",
    ),
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
        "View: Increase Current View Width",
        "workbench.action.increaseViewWidth",
    ),
    (
        "View: Decrease Current View Width",
        "workbench.action.decreaseViewWidth",
    ),
    (
        "View: Increase Current View Height",
        "workbench.action.increaseViewHeight",
    ),
    (
        "View: Decrease Current View Height",
        "workbench.action.decreaseViewHeight",
    ),
    (
        "View: Reset Editor Group Sizes",
        "workbench.action.evenEditorWidths",
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
    (
        "View: Move Editor Left",
        "workbench.action.moveEditorLeftInGroup",
    ),
    (
        "View: Move Editor Right",
        "workbench.action.moveEditorRightInGroup",
    ),
    (
        "View: Move Editor into Previous Group",
        "workbench.action.moveEditorToPreviousGroup",
    ),
    (
        "View: Move Editor into Next Group",
        "workbench.action.moveEditorToNextGroup",
    ),
    (
        "View: Move Editor into First Group",
        "workbench.action.moveEditorToFirstGroup",
    ),
    (
        "View: Move Editor into Last Group",
        "workbench.action.moveEditorToLastGroup",
    ),
    (
        "View: Focus Left Editor Group",
        "workbench.action.focusLeftGroup",
    ),
    (
        "View: Focus Right Editor Group",
        "workbench.action.focusRightGroup",
    ),
    (
        "View: Focus Editor Group Above",
        "workbench.action.focusAboveGroup",
    ),
    (
        "View: Focus Editor Group Below",
        "workbench.action.focusBelowGroup",
    ),
    ("View: Next Editor", "workbench.action.nextEditor"),
    ("View: Previous Editor", "workbench.action.previousEditor"),
    (
        "View: Next Editor in Group",
        "workbench.action.nextEditorInGroup",
    ),
    (
        "View: Previous Editor in Group",
        "workbench.action.previousEditorInGroup",
    ),
    ("Edit: Undo", "undo"),
    ("Edit: Redo", "redo"),
    ("Edit: Copy", "editor.action.clipboardCopyAction"),
    ("Edit: Cut", "editor.action.clipboardCutAction"),
    ("Edit: Paste", "editor.action.clipboardPasteAction"),
    ("Edit: Select All", "editor.action.selectAll"),
    ("Edit: Toggle Line Comment", "editor.action.commentLine"),
    ("Edit: Toggle Block Comment", "editor.action.blockComment"),
    ("Edit: Add Line Comment", "editor.action.addCommentLine"),
    (
        "Edit: Remove Line Comment",
        "editor.action.removeCommentLine",
    ),
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
    ("Go: Back", "workbench.action.navigateBack"),
    ("Go: Forward", "workbench.action.navigateForward"),
    ("Outline: Focus", "outline.focus"),
    ("Outline: Collapse All", "outline.collapse"),
    ("Outline: Expand All", "outline.expand"),
    ("Outline: Toggle Follow Cursor", "outline.followCursor"),
    ("View: Toggle Breadcrumbs", "breadcrumbs.toggle"),
    ("Focus Breadcrumbs", "breadcrumbs.focus"),
    ("Focus Breadcrumbs and Select", "breadcrumbs.focusAndSelect"),
    (
        "Language: Parameter Hints",
        "editor.action.triggerParameterHints",
    ),
    ("Language: Previous Parameter Hint", "showPrevParameterHint"),
    ("Language: Next Parameter Hint", "showNextParameterHint"),
    ("Language: Close Parameter Hints", "closeParameterHints"),
    ("Language: Hover", "editor.action.showHover"),
    ("Language: Complete", "editor.action.triggerSuggest"),
    (
        "Language: Go to Definition",
        "editor.action.revealDefinition",
    ),
    ("Language: Find References", "editor.action.goToReferences"),
    ("Language: Format Document", "editor.action.formatDocument"),
    ("Language: Rename Symbol", "editor.action.rename"),
    ("Language: Quick Fix", "editor.action.quickFix"),
    ("Go to Symbol in Editor", "workbench.action.gotoSymbol"),
    (
        "Go to Symbol in Workspace",
        "workbench.action.showAllSymbols",
    ),
    ("Language: Refactor", "editor.action.refactor"),
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
    Outline,
    Breadcrumbs,
    Terminal,
    Output,
}
#[derive(Clone)]
pub enum PromptKind {
    Extension(Box<crate::extensions::NativePrompt>),
    Symbols,
    InstallExtension,
    SearchExtensions,
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
    pub fn insert(&mut self, text: &str) -> bool {
        let limit = match self.kind {
            PromptKind::Extension(_) => crate::extensions::MAX_PROMPT_TEXT,
            PromptKind::Symbols | PromptKind::SearchExtensions => 1024,
            _ => usize::MAX,
        };
        if (if self.select_all { 0 } else { self.text.len() }) + text.len() > limit {
            return false;
        }
        if self.select_all {
            self.text.clear();
            self.cursor = 0;
            self.select_all = false;
        }
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.selected = 0;
        true
    }
}
#[derive(Clone)]
pub enum AfterSave {
    Close,
    Quit,
    CloseAll,
}
pub enum Modal {
    ExtensionTree,
    ExtensionSurfaces(extension_surfaces::Picker),
    ExtensionsLoading(u64),
    ExtensionRegistry {
        items: Vec<crate::extension_registry::Entry>,
        selected: usize,
    },
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

#[derive(Clone)]
pub struct TabHit {
    pub area: Rect,
    pub membership: crate::editor_groups::Membership,
    pub proof: crate::editor_groups::UiProof,
}

struct ClosingGroup {
    proof: crate::editor_groups::GroupProof,
    others: Vec<crate::editor_groups::GroupProof>,
    remaining: Vec<crate::editor_groups::Membership>,
    settings: std::sync::Arc<Vec<serde_json::Map<String, Value>>>,
    workspace: PathBuf,
    profile: u64,
}

pub struct App {
    pub panes: Vec<Pane>,
    pub active_pane: usize,
    pub pane_areas: Vec<Rect>,
    pub horizontal_split: bool,
    next_pane_id: u64,
    editor_groups: crate::editor_groups::Groups,
    editor_layout: crate::editor_layout::Layout,
    pub(crate) editor_presentation: crate::editor_presentation::Presentation,
    editor_geometry_inputs: Option<editor_layout::GeometryInputs>,
    pending_editor_resize: Option<editor_layout::PendingResize>,
    group_fallback: bool,
    preview_tabs: preview_tabs::State,
    preview_admission_failed: bool,
    closing_group: Option<ClosingGroup>,
    close_membership: Option<crate::editor_groups::Membership>,
    close_eligibility: sticky_tabs::CloseEligibility,
    pub tab_hits: Vec<TabHit>,
    pub documents: Vec<Document>,
    pub(crate) hidden_documents: Vec<Document>,
    extension_services: extension_services::State,
    extension_providers: extension_providers::State,
    actions: actions::State,
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
    pub last_key: Option<KeyEvent>,
    pub keyboard: keyboard::State,
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
    pub extension_registry: crate::extension_registry::Registry,
    pub extension_node: String,
    activation: extension_activation::State,
    extension_job: Option<extension_management::Job>,
    extension_retirement: Option<std::sync::mpsc::Receiver<()>>,
    extension_epoch: u64,
    pub extension_packages: Vec<crate::extensions::Package>,
    pub extension_host: Option<crate::extensions::Client>,
    pub extension_surfaces: extension_surfaces::State,
    pub lsp: Option<crate::lsp::Client>,
    language_services: language_services::State,
    signature: signature_help::State,
    suggestions: suggestions::State,
    pub syntax: crate::syntax::Engine,
    pub theme: crate::theme::Theme,
    pub recent_files: crate::recent::State,
    pub welcome_brand: crate::brand::State,
    pub(crate) welcome_actions: Vec<(Rect, crate::ui::welcome::Action)>,
    session: session::State,
    navigation: navigation::State,
    navigation_history: navigation_history::State,
    outline: outline::State,
    breadcrumbs: breadcrumbs::State,
    symbols: symbols::State,
    theme_state: themes::State,
    pub settings: crate::settings::Settings,
    pub imported_keybinding_notices: Vec<String>,
    settings_loader: Option<crate::settings::Loader>,
    settings_error: Option<String>,
    settings_user: Option<PathBuf>,
    settings_writes: settings_persistence::State,
    saving: saving::State,
    pub diagnostics: HashMap<PathBuf, crate::lsp::DiagnosticPublication>,
    pub editor_area: Rect,
    pub explorer_area: Rect,
    pub outline_area: Rect,
    pub breadcrumbs_area: Rect,
    pub breadcrumbs_picker_area: Rect,
    pub(crate) breadcrumbs_hits: Vec<(Rect, usize)>,
    pub(crate) breadcrumbs_presented: Option<u64>,
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
        self.shutdown_language_services();
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
            editor_groups: crate::editor_groups::Groups::default(),
            editor_layout: crate::editor_layout::Layout::default(),
            editor_presentation: crate::editor_presentation::Presentation::default(),
            editor_geometry_inputs: None,
            pending_editor_resize: None,
            group_fallback: false,
            preview_tabs: preview_tabs::State::default(),
            preview_admission_failed: false,
            closing_group: None,
            close_membership: None,
            close_eligibility: sticky_tabs::CloseEligibility::AnyMode,
            tab_hits: Vec::new(),
            documents: Vec::new(),
            hidden_documents: Vec::new(),
            extension_services: extension_services::State::default(),
            extension_providers: extension_providers::State::default(),
            actions: actions::State::default(),
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
            last_key: None,
            keyboard: keyboard::State::default(),
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
            extension_surfaces: extension_surfaces::State::default(),
            extensions_directory: crate::extension_store::default_directory(),
            extension_registry: crate::extension_registry::Registry::default(),
            extension_node: "node".into(),
            activation: extension_activation::State::default(),
            extension_job: None,
            extension_retirement: None,
            extension_epoch: 0,
            extension_packages: Vec::new(),
            lsp: None,
            language_services: language_services::State::default(),
            signature: signature_help::State::default(),
            suggestions: suggestions::State::default(),
            syntax: crate::syntax::Engine::default(),
            theme: crate::theme::Theme::default(),
            recent_files: crate::recent::State::default(),
            welcome_brand: crate::brand::State::default(),
            welcome_actions: Vec::new(),
            session: session::State::default(),
            navigation: navigation::State::default(),
            navigation_history: navigation_history::State::default(),
            outline: outline::State::default(),
            breadcrumbs: breadcrumbs::State::default(),
            symbols: symbols::State::default(),
            theme_state: themes::State::default(),
            settings: crate::settings::Settings::default(),
            imported_keybinding_notices: Vec::new(),
            settings_loader: None,
            settings_error: None,
            settings_user: None,
            settings_writes: settings_persistence::State::default(),
            saving: saving::State::default(),
            diagnostics: HashMap::new(),
            editor_area: Rect::default(),
            explorer_area: Rect::default(),
            outline_area: Rect::default(),
            breadcrumbs_area: Rect::default(),
            breadcrumbs_picker_area: Rect::default(),
            breadcrumbs_hits: Vec::new(),
            breadcrumbs_presented: None,
            tab_area: Rect::default(),
            pending: None,
            snippet_pending: None,
            snippet_catalog: snippet_catalogs::State::default(),
            clipboard: String::new(),
            clipboard_line: false,
        }
    }
    pub fn poll(&mut self) -> bool {
        self.observe_editor_geometry();
        let actions_changed = self.poll_code_actions();
        let brand_changed = self.welcome_brand.poll();
        let invalidated = self.poll_signature();
        let invalidated = self.poll_suggestions() || invalidated;
        let changed = self.poll_symbols() || invalidated;
        let changed = self.workspace.poll() || changed;
        let changed = self.search.as_mut().is_some_and(|s| s.poll()) || changed;
        let mut changed = actions_changed | brand_changed | changed;
        changed |= self.poll_settings_writes();
        if let Some(result) = self
            .settings_loader
            .as_mut()
            .and_then(crate::settings::Loader::poll)
        {
            match result {
                Ok(settings) => {
                    self.settings_error = None;
                    self.settings_profile_loaded();
                    if settings != self.settings {
                        // Even a later reload back to the original settings
                        // cannot revive a completion from an earlier context.
                        self.cancel_suggestions();
                        self.cancel_code_actions();
                        self.clear_signature();
                        self.retire_save_code_actions("settings changed during code actions");
                        self.retire_save_formatting("settings changed during formatting");
                        let previous = std::mem::replace(&mut self.settings, settings);
                        if !self.settings.editor_preview().enabled
                            && let Err(error) = self.keep_preview_tabs()
                        {
                            self.message = format!(
                                "Preview policy update rejected; buffers retained: {error:#}"
                            );
                        }
                        for doc in self.documents.iter_mut().chain(&mut self.hidden_documents) {
                            let language = doc
                                .path
                                .as_deref()
                                .map_or("plaintext", crate::languages::language);
                            if previous.typing(language) != self.settings.typing(language) {
                                doc.retire_typing_pairs();
                            }
                            self.settings.apply(doc);
                        }
                        self.message = format!(
                            "Settings reloaded · {} settings notices (Settings: Compatibility Report)",
                            self.settings.warnings.len()
                        );
                        changed = true;
                    }
                    changed |= self.settings_reload_settled();
                }
                Err(error) => {
                    self.settings_profile_load_failed();
                    if self.settings_error.as_ref() != Some(&error) {
                        self.message =
                            format!("Settings reload failed; previous settings retained: {error}");
                        self.settings_error = Some(error);
                        changed = true;
                    }
                    changed |= self.settings_reload_settled();
                }
            }
        }
        changed |= self.poll_language_services();
        changed |= self.poll_language();
        changed |= self.poll_save_code_actions(std::time::Instant::now());
        changed |= self.poll_save_formatting(std::time::Instant::now());
        changed |= self.poll_native_saves();
        changed |= self.poll_git();
        changed |= self.poll_files();
        changed |= self.poll_watching();
        changed |= self.poll_debugger();
        changed |= self.poll_extension_management();
        changed |= self.poll_extensions();
        changed |= self.poll_extension_activation();
        changed |= self.poll_extension_providers();
        changed |= self.poll_extension_surfaces();
        changed |= self.poll_snippet();
        changed |= self.poll_snippet_catalog();
        changed |= self.poll_theme();
        changed |= self.poll_navigation();
        changed |= self.poll_session();
        changed |= self.poll_autosave(self.autosave_now());
        changed |= self.dispatch_queued_suggestions();
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
        self.observe_navigation(navigation_history::Reason::Ordinary);
        changed |= self.poll_outline();
        changed |= self.poll_breadcrumbs();
        changed |= self.observe_editor_geometry();
        changed |= self.poll_editor_resize();
        changed
    }
    pub fn recovery_documents(&self) -> Vec<&Document> {
        self.documents
            .iter()
            .chain(self.hidden_documents.iter().filter(|doc| doc.dirty()))
            .collect()
    }
    pub fn active_document(&self) -> Option<&Document> {
        self.documents.get(self.active)
    }
    pub fn doc(&self) -> &Document {
        &self.documents[self.active]
    }
    pub fn user_settings_path(&self) -> Option<PathBuf> {
        self.settings_user
            .clone()
            .or_else(|| crate::recovery::config_path().map(|p| p.with_file_name("settings.json")))
    }
    pub fn configure_settings(&mut self, user: Option<PathBuf>) -> Result<()> {
        self.invalidate_settings_profile()?;
        self.settings_user = user.clone();
        let mut paths: Vec<_> = user.into_iter().collect();
        paths.push(self.workspace.root.join(".vscode/settings.json"));
        if let Some(loader) = &mut self.settings_loader {
            loader.reconfigure(paths.clone())?;
        } else {
            self.settings_loader = Some(crate::settings::Loader::new(paths.clone())?);
        }
        self.settings = match crate::settings::Settings::load_editor(&paths) {
            Ok(settings) => settings,
            Err(error) => {
                self.settings_error = Some(format!("{error:#}"));
                return Err(error);
            }
        };
        self.settings_profile_loaded();
        if !self.settings.editor_preview().enabled {
            self.keep_preview_tabs()?;
        }
        for doc in self.documents.iter_mut().chain(&mut self.hidden_documents) {
            self.settings.apply(doc);
        }
        self.refresh_document_language_configurations();
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
    /// Open or focus a file. Hidden-model alias resolution can finish in poll;
    /// follow-up cursor/edit actions must travel with open_with_intent instead
    /// of assuming this call synchronously selected the requested document.
    pub fn open(&mut self, path: &Path) -> Result<()> {
        self.open_with_intent(path, navigation::OpenIntent::Focus)
    }
    fn open_with_intent(&mut self, path: &Path, intent: navigation::OpenIntent) -> Result<()> {
        // Accepted opens advance the loader fence in the inner path. An open
        // rejected by an occupied slot must leave its original request valid.
        self.observe_navigation(navigation_history::Reason::Ordinary);
        let reason = if matches!(
            intent,
            navigation::OpenIntent::Focus
                | navigation::OpenIntent::Preview
                | navigation::OpenIntent::Settings
        ) {
            navigation_history::Reason::EditorChange
        } else {
            navigation_history::Reason::Jump
        };
        let previous = self.suspend_navigation_observation();
        let result = self.open_with_intent_inner(path, intent);
        self.resume_navigation_observation(
            previous,
            if result.is_ok() {
                reason
            } else {
                navigation_history::Reason::Ordinary
            },
        );
        self.observe_outline();
        self.observe_breadcrumbs();
        result
    }
    fn open_with_intent_inner(
        &mut self,
        path: &Path,
        intent: navigation::OpenIntent,
    ) -> Result<()> {
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
            intent.validate(&self.documents[index])?;
            self.can_admit_editor(self.documents[index].id)?;
            if !self.group_fallback {
                let change = self.open_editor_group(
                    self.documents[index].id,
                    crate::editor_groups::OpenMode::Committed,
                    None,
                )?;
                self.apply_group_change(change);
            }
            self.cancel_navigation();
            self.active = index;
            self.focus = Focus::Editor;
            self.sync_pane();
            self.remember_active_file();
            intent.apply(self);
            self.observe_navigation(navigation_history::Reason::EditorChange);
            return Ok(());
        }
        if let Some(index) = self
            .hidden_documents
            .iter()
            .position(|doc| doc.path.as_ref() == Some(&path))
        {
            intent.validate(&self.hidden_documents[index])?;
            self.can_admit_editor(self.hidden_documents[index].id)?;
            self.cancel_navigation();
            if self.group_fallback {
                let doc = self.hidden_documents.remove(index);
                self.install_open_document(doc)?;
            } else {
                self.open_preview_model(
                    self.hidden_documents[index].id,
                    crate::editor_groups::OpenMode::Committed,
                )?;
            }
            intent.apply(self);
            return Ok(());
        }
        if !self.hidden_documents.is_empty() {
            self.open_hidden_aware(path, intent);
            return Ok(());
        }
        self.cancel_navigation();
        let mut d = Document::open(&path)?;
        let target = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .find(|doc| doc.path == d.path)
            .unwrap_or(&d);
        intent.validate(target)?;
        self.settings.apply(&mut d);
        self.install_open_document(d)?;
        intent.apply(self);
        Ok(())
    }
    fn install_open_document(&mut self, d: Document) -> Result<()> {
        if !self.group_fallback {
            self.preview_edit_barrier();
            return self.install_preview_document(d, crate::editor_groups::OpenMode::Committed);
        }
        let id = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .find(|old| old.path == d.path)
            .map_or(d.id, |old| old.id);
        self.can_admit_editor(id)?;
        let mut d = if let Some(index) = self
            .hidden_documents
            .iter()
            .position(|doc| doc.path == d.path)
        {
            self.hidden_documents.remove(index)
        } else {
            d
        };
        let configuration_error = self.configure_document_language(&mut d).err();
        if let Some(index) = self.documents.iter().position(|old| old.path == d.path) {
            self.active = index;
        } else {
            self.documents.push(d);
            self.active = self.documents.len() - 1;
        }
        self.focus = Focus::Editor;
        self.message = format!("Opened {}", self.doc().name());
        if let Some(error) = configuration_error {
            self.message.push_str(&format!(
                " · Native language configuration rejected: {error:#}"
            ));
        }
        self.sync_pane();
        self.remember_active_file();
        self.observe_navigation(navigation_history::Reason::EditorChange);
        Ok(())
    }
    pub fn start_prompt(&mut self, kind: PromptKind, text: String) {
        self.invalidate_editor_presentation();
        if self
            .prompt
            .as_ref()
            .is_some_and(|prompt| matches!(prompt.kind, PromptKind::SaveAs))
            && !matches!(kind, PromptKind::SaveAs)
        {
            self.cancel_save_continuations();
        }
        self.cancel_suggestions();
        self.session_interaction();
        self.clear_signature();
        self.cancel_extension_prompt();
        if !matches!(kind, PromptKind::Symbols) {
            self.cancel_symbols();
        }
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
                if *id == "workbench.action.keepEditor" && !self.settings.editor_preview().enabled {
                    return None;
                }
                score(label, query.trim_start_matches('>')).map(|s| (s, *label, *id))
            })
            .collect();
        items.extend(
            self.extension_dormant_commands()
                .iter()
                .filter_map(|(label, id)| {
                    score(label, query.trim_start_matches('>'))
                        .map(|s| (s, label.as_str(), id.as_str()))
                }),
        );
        if let Some(host) = &self.extension_host {
            items.extend(
                host.commands
                    .iter()
                    .filter(|(_, id)| {
                        !self
                            .extension_dormant_commands()
                            .iter()
                            .any(|(_, dormant)| dormant == id)
                    })
                    .filter_map(|(label, id)| {
                        score(label, query.trim_start_matches('>'))
                            .map(|s| (s, label.as_str(), id.as_str()))
                    }),
            );
        }
        items.sort_by_key(|a| std::cmp::Reverse(a.0));
        items
            .into_iter()
            .map(|(_, label, id)| (label, id))
            .collect()
    }
    pub fn context(&self) -> HashMap<String, Value> {
        let breadcrumbs = self.breadcrumbs_view();
        HashMap::from([
            ("breadcrumbsPossible".into(), json!(breadcrumbs.possible)),
            ("breadcrumbsVisible".into(), json!(breadcrumbs.visible)),
            (
                "breadcrumbsActive".into(),
                json!(self.focus == Focus::Breadcrumbs),
            ),
            (
                "config.breadcrumbs.enabled".into(),
                json!(self.breadcrumbs_enabled()),
            ),
            (
                "listFocus".into(),
                json!(self.focus == Focus::Breadcrumbs && breadcrumbs.picker.is_some()),
            ),
            ("treestickyScrollFocused".into(), json!(false)),
            ("canNavigateBack".into(), json!(self.can_navigate_back())),
            (
                "canNavigateForward".into(),
                json!(self.can_navigate_forward()),
            ),
            (
                "suggestWidgetVisible".into(),
                json!(self.suggestion_model().is_some()),
            ),
            (
                "acceptSuggestionOnEnter".into(),
                json!(
                    self.settings.suggestions(self.language()).enter
                        && self.suggestion_acceptable()
                ),
            ),
            (
                "editorHasSignatureHelpProvider".into(),
                json!(self.has_signature_provider()),
            ),
            (
                "parameterHintsVisible".into(),
                json!(self.signature_help().is_some()),
            ),
            (
                "parameterHintsMultipleSignatures".into(),
                json!(self.signature_help().is_some_and(|hint| hint.count > 1)),
            ),
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
                "editorHasCodeActionsProvider".into(),
                json!(
                    self.active_document().is_some()
                        && (self.lsp.as_ref().is_some_and(|c| c.ready
                            && (c.capabilities["codeActionProvider"].is_object()
                                || c.capabilities["codeActionProvider"] == true))
                            || self.has_extension_provider(
                                crate::extension_providers::Kind::CodeAction
                            ))
                ),
            ),
            (
                "editorHasSelection".into(),
                json!(
                    self.active_document()
                        .is_some_and(|d| d.selection().is_some())
                ),
            ),
            (
                "activeEditorIsNotPreview".into(),
                json!(self.active_tab_membership().is_some() && !self.active_editor_is_preview()),
            ),
            (
                "activeEditorIsPinned".into(),
                json!(self.active_editor_is_sticky()),
            ),
            (
                "config.workbench.editor.enablePreview".into(),
                json!(self.settings.editor_preview().enabled),
            ),
            ("inputFocus".into(), json!(self.prompt.is_some())),
            ("terminalFocus".into(), json!(self.focus == Focus::Terminal)),
            (
                "filesExplorerFocus".into(),
                json!(self.focus == Focus::Explorer),
            ),
            ("outlineFocused".into(), json!(self.focus == Focus::Outline)),
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
        self.observe_editor_geometry();
        self.breadcrumbs_ui_event(&event);
        self.observe_navigation(navigation_history::Reason::Ordinary);
        if matches!(&event, Event::Paste(_))
            || matches!(&event, Event::Mouse(mouse) if mouse.kind != MouseEventKind::Moved)
        {
            self.navigation_input_interaction();
        }
        let suggestion_edit = self.suggestion_edit_event(&event);
        let signature_edit = self.signature_edit_event(&event);
        self.signature_ui_event(&event);
        self.suggestion_ui_event(&event);
        self.provider_ui_event(&event);
        self.code_action_ui_event(&event);
        if matches!(&event, Event::Key(key) if key.kind != KeyEventKind::Release)
            || matches!(&event, Event::Paste(_))
            || matches!(&event, Event::Mouse(mouse) if mouse.kind != MouseEventKind::Moved)
        {
            self.session_interaction();
        }
        self.event_inner(event);
        self.preview_edit_barrier();
        self.sync_pane();
        self.observe_navigation(navigation_history::Reason::Ordinary);
        self.invalidate_symbol_context();
        self.observe_outline();
        self.observe_breadcrumbs();
        self.invalidate_pending_extension_commands();
        self.observe_suggestion_edit(suggestion_edit);
        self.observe_signature_edit(signature_edit);
        self.refresh_signature();
        self.observe_editor_geometry();
    }
    fn event_inner(&mut self, event: Event) {
        match event {
            Event::Resize(_, _) => {
                self.invalidate_editor_presentation();
                self.tab_hits.clear();
                self.welcome_actions.clear();
                self.welcome_brand.resize();
                self.breadcrumbs_area = Rect::default();
                self.breadcrumbs_picker_area = Rect::default();
                self.breadcrumbs_hits.clear();
                self.breadcrumbs_presented = None;
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
            Event::Key(key) if matches!(self.modal, Some(Modal::Inspector)) => {
                self.inspect_key(key)
            }
            Event::Paste(text) => {
                if let Some(prompt) = &mut self.prompt {
                    if !prompt.insert(&text.replace(['\r', '\n'], "")) {
                        self.message = match prompt.kind {
                            PromptKind::Symbols => "Symbol query exceeds 1 KiB",
                            PromptKind::SearchExtensions => "Extension search exceeds 1 KiB",
                            _ => "Extension prompt text exceeds 4 KiB",
                        }
                        .into();
                    }
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
                if self.surface_mouse(mouse) {
                    return;
                }
                let p = ratatui::layout::Position::new(mouse.column, mouse.row);
                let geometry_current = !self.group_fallback && self.editor_geometry_current();
                if !self.group_fallback
                    && geometry_current
                    && self.editor_presentation.divider_at(p).is_some()
                {
                    self.message =
                        "Divider dragging is not implemented; use editor resize commands".into();
                    return;
                }
                if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                    && let Some(hit) = self
                        .tab_hits
                        .iter()
                        .find(|hit| hit.area.contains(p))
                        .cloned()
                {
                    if !geometry_current
                        || !self.editor_groups.proof_current(&hit.proof)
                        || !self.editor_groups.membership_current(hit.membership)
                    {
                        self.message = "Editor tabs changed; choose the current tab".into();
                    } else if let Err(error) = self.focus_tab(hit.membership) {
                        self.message = format!("Tab focus rejected: {error:#}");
                    }
                    return;
                }
                if geometry_current
                    && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                    && (self.breadcrumbs_area.contains(p)
                        || self.breadcrumbs_picker_area.contains(p))
                {
                    if self.breadcrumbs_presented != Some(self.breadcrumbs_view().generation) {
                        self.message = "Breadcrumbs changed; choose the current item".into();
                        return;
                    }
                    if self.breadcrumbs_picker_area.contains(p) {
                        let picker = self.breadcrumbs_view().picker;
                        if let Some(picker) = picker {
                            let offset = picker.selected.saturating_sub(
                                self.breadcrumbs_picker_area.height.saturating_sub(1) as usize,
                            );
                            self.breadcrumbs_picker_click(
                                offset + (mouse.row - self.breadcrumbs_picker_area.y) as usize,
                            );
                        }
                    } else if let Some((_, index)) = self
                        .breadcrumbs_hits
                        .iter()
                        .find(|(area, _)| area.contains(p))
                    {
                        self.breadcrumbs_click(*index);
                    }
                    return;
                }
                if self.documents.is_empty()
                    && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                    && let Some((_, action)) = self
                        .welcome_actions
                        .iter()
                        .find(|(rect, _)| rect.contains(p))
                {
                    match action.clone() {
                        crate::ui::welcome::Action::Command(command) => {
                            self.execute(command, Value::Null)
                        }
                        crate::ui::welcome::Action::Recent(path) => self.open_welcome_recent(path),
                    }
                    return;
                }

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
                if matches!(mouse.kind, MouseEventKind::Moved | MouseEventKind::Up(_)) {
                    return;
                }
                // A header/divider/gutter is never interpreted as source text.
                // Capture the original drawn text rectangle before our own
                // accepted focus transition retires the presentation proof.
                let pane = geometry_current
                    .then(|| self.editor_presentation.pane_at(p))
                    .flatten();
                let text_area = if self.group_fallback {
                    // Recovery overflow has no native membership/geometry seal.
                    // Its displayed active buffer remains keyboard editable;
                    // no ordinal or raw editor rectangle authorizes pointers.
                    None
                } else if let Some(pane) = pane {
                    if !pane.text.contains(p) {
                        return;
                    }
                    if let Err(error) = self.focus_tab(pane.membership) {
                        self.message = format!("Editor focus rejected: {error:#}");
                        return;
                    }
                    Some(pane.text)
                } else {
                    None
                };
                if let Some(text_area) = text_area.filter(|_| self.active_document().is_some()) {
                    match mouse.kind {
                        MouseEventKind::Down(MouseButton::Left)
                        | MouseEventKind::Drag(MouseButton::Left) => {
                            self.focus = Focus::Editor;
                            let row = self.doc().top + (mouse.row - text_area.y) as usize;
                            let col = self.doc().left + (mouse.column - text_area.x) as usize;
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
                        MouseEventKind::ScrollDown => self.doc_mut().vertical(3, false),
                        MouseEventKind::ScrollUp => self.doc_mut().vertical(-3, false),
                        _ => {}
                    }
                } else if self.outline_area.contains(p)
                    && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                {
                    self.outline_click((mouse.row - self.outline_area.y) as usize);
                } else if self.explorer_area.contains(p)
                    && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                {
                    self.focus = Focus::Explorer;
                    let offset = self
                        .explorer_selected
                        .saturating_sub(self.explorer_area.height.saturating_sub(1) as usize);
                    self.explorer_selected = (offset + (mouse.row - self.explorer_area.y) as usize)
                        .min(self.entries.len().saturating_sub(1));
                    self.explorer_open_mode(true);
                }
            }
            _ => {}
        }
    }
    fn key(&mut self, key: KeyEvent) {
        let token = keys::token(key);
        self.last_key = Some(key);
        if key.code == KeyCode::Esc && self.escape_save_code_actions() {
            return;
        }
        if key.code == KeyCode::Esc && self.escape_save_formatting() {
            return;
        }
        if key.code == KeyCode::Esc && self.cancel_deferred_save_close() {
            return;
        }
        if matches!(self.modal, Some(Modal::Inspector)) {
            self.inspect_key(key);
        }
        if self.modal.is_some() {
            self.navigation_input_interaction();
            self.modal_key(key);
            return;
        }
        if self.prompt.is_some() {
            self.navigation_input_interaction();
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
                        | "workbench.action.navigateBack"
                        | "workbench.action.navigateForward"
                        | "workbench.view.extensions"
                        | "workbench.extensions.action.showInstalledExtensions"
                )
            {
                self.execute_with_args(&command, args);
            } else {
                self.navigation_input_interaction();
                if let Some(terminal) = self.terminals.get_mut(self.active_terminal)
                    && let Err(e) = terminal.key(key)
                {
                    self.message = e.to_string();
                }
            }
            return;
        }
        if key.code == KeyCode::Esc && self.chord.is_some() {
            self.navigation_input_interaction();
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
                if command == "acceptSelectedSuggestion" && !self.suggestion_acceptable() {
                    match key.code {
                        KeyCode::Tab => {
                            self.execute_with_args("tab", None);
                            return;
                        }
                        KeyCode::Enter => {
                            self.execute_with_args("lineBreakInsert", None);
                            return;
                        }
                        _ => {}
                    }
                }
                self.execute_with_args(&command, args);
                return;
            }
            Resolution::Chord => {
                self.navigation_input_interaction();
                self.chord = Some(sequence.clone());
                self.message = format!("({sequence}) waiting for second key…");
                return;
            }
            Resolution::None if in_chord => {
                self.navigation_input_interaction();
                self.message = format!("No command bound to {sequence}");
                return;
            }
            _ => {}
        }
        self.navigation_input_interaction();
        if self.focus == Focus::Output {
            self.output_key(key);
            return;
        }
        if self.focus == Focus::Outline {
            self.outline_key(key);
            return;
        }
        if self.focus == Focus::Breadcrumbs {
            self.breadcrumbs_key(key);
            return;
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
            self.type_editor_text(&c.to_string(), true);
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
        self.observe_editor_geometry();
        if self.execute_editor_layout_command(command) {
            return;
        }
        if self.execute_tab_reorder_command(command) {
            return;
        }
        if self.execute_tab_transfer_command(command) {
            return;
        }
        if self.execute_spatial_focus_command(command) {
            return;
        }
        self.breadcrumbs_ui_command(command);
        let history_travel = matches!(
            command,
            "workbench.action.navigateBack" | "workbench.action.navigateForward"
        );
        let duplicate_reopen =
            command == "workbench.action.reopenClosedEditor" && self.duplicate_reopen_pending();
        if !history_travel && !duplicate_reopen {
            self.navigation_input_interaction();
            self.observe_navigation(navigation_history::Reason::Ordinary);
        }
        let suggestion_edit = self.suggestion_edit_command(command, args.as_ref());
        let signature_edit = self.signature_edit_command(command, args.as_ref());
        self.execute_inner(command, args);
        self.preview_edit_barrier();
        self.sync_pane();
        self.observe_outline();
        self.observe_breadcrumbs();
        if !history_travel {
            self.observe_navigation(navigation_history::Reason::Ordinary);
        }
        self.invalidate_pending_extension_commands();
        self.observe_suggestion_edit(suggestion_edit);
        self.observe_signature_edit(signature_edit);
        self.observe_editor_geometry();
    }
    fn execute_inner(&mut self, command: &str, command_args: Option<Value>) {
        self.advance_signature_interaction(command);
        if suggestions::command(command) || signature_help::command(command) {
            self.advance_suggestion_interaction();
        } else if !suggestions::typing_command(command) {
            self.cancel_suggestions();
        }
        self.cancel_symbols();

        if command != "vscli.session.restore" {
            self.session_interaction();
        }
        // A duplicate reopen keeps the existing bounded request alive.
        if !matches!(
            command,
            "workbench.action.reopenClosedEditor"
                | "workbench.action.navigateBack"
                | "workbench.action.navigateForward"
        ) {
            self.cancel_navigation();
        }
        if suggestions::command(command) {
            self.execute_suggestion_command(command);
            return;
        }
        let args = command_args.clone().unwrap_or(Value::Null);
        if command.is_empty() {
            return;
        }
        if self.execute_breadcrumbs_command(command) {
            return;
        }
        if self.focus == Focus::Output && Self::requires_editor(command) {
            self.message = "Output is read-only; focus an editor to edit".into();
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
            "vscli.extensions.output" => self.surface_picker(extension_surfaces::PickerKind::Output),
            "vscli.extensions.status" => self.surface_picker(extension_surfaces::PickerKind::Status),
            "vscli.extensions.trees" => self.surface_picker(extension_surfaces::PickerKind::Trees),
            "vscli.session.restore" => self.restore_session(),
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
            "workbench.action.keepEditor" => self.keep_active_editor(),
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
            "workbench.action.pinEditor" => self.set_active_editor_sticky(true),
            "workbench.action.unpinEditor" => self.set_active_editor_sticky(false),
            "workbench.action.closeActivePinnedEditor" => self.close_active_editor(true),
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
            "vscli.languageServer.restart" | "vscli.languageServer.disable" | "vscli.languageServer.enable" | "vscli.languageServer.status" => self.language_service_command(command),
            "editor.action.triggerParameterHints" => self.request_signature(),
            "closeParameterHints" => self.clear_signature(),
            "showPrevParameterHint" => self.cycle_signature(false),
            "showNextParameterHint" => self.cycle_signature(true),
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
            "workbench.action.gotoSymbol" => self.start_symbols(false),
            "workbench.action.showAllSymbols" => self.start_symbols(true),
            "editor.action.quickFix" => self.request_code_actions(None),
            "editor.action.refactor" => self.request_code_actions(Some("refactor")),
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
                    self.type_editor_text(text, false);
                }
            }
            "workbench.action.files.newUntitledFile" => {
                self.cancel_navigation();
                let mut doc = Document::default();
                if let Err(error) = self.can_admit_editor(doc.id) {
                    self.message = format!("New tab rejected; existing buffers retained: {error:#}");
                    return;
                }
                self.settings.apply(&mut doc);
                if let Err(error) = self.configure_document_language(&mut doc) {
                    self.message = format!("Native language configuration rejected: {error:#}");
                }
                if self.group_fallback {
                    if let Err(error) = self.documents.try_reserve(1) {
                        self.message = format!("New tab rejected; existing buffers retained: {error}");
                        return;
                    }
                    self.documents.push(doc);
                    self.active = self.documents.len() - 1;
                    self.focus = Focus::Editor;
                } else if let Err(error) = self.install_preview_document(
                    doc, crate::editor_groups::OpenMode::Committed,
                ) {
                    self.message = format!("New tab rejected; existing buffers retained: {error:#}");
                }
            }
            "workbench.action.files.openFile" => self.start_prompt(
                PromptKind::Open,
                self.workspace.root.to_string_lossy().into_owned() + "/",
            ),
            "workbench.action.files.save" => {
                self.save(None);
            }
            "workbench.action.files.saveAs" => {
                self.pending = None;
                self.save_as();
            }
            "workbench.action.closeActiveEditor" => self.close_active_editor(false),
            "workbench.action.closeWindow" | "workbench.action.quit" => {
                self.request_close(AfterSave::Quit)
            }
            "workbench.action.closeAllEditors" => self.begin_close_editor_batch(true),
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
                self.navigate_editor_tabs(crate::editor_groups::Navigate::Next);
            }
            "workbench.action.previousEditor" => {
                self.navigate_editor_tabs(crate::editor_groups::Navigate::Previous);
            }
            "workbench.action.nextEditorInGroup" => self.navigate_editor_tabs(crate::editor_groups::Navigate::NextInGroup),
            "workbench.action.previousEditorInGroup" => self.navigate_editor_tabs(crate::editor_groups::Navigate::PreviousInGroup),
            "workbench.action.openGlobalKeybindings" => self.modal = Some(Modal::Keys),
            "vscli.keyboardInspector" => self.open_keyboard_inspector(),
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
                if let Some(path) = self.user_settings_path() {
                    let result = (|| -> Result<()> {
                        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
                        self.open_with_intent(&path, navigation::OpenIntent::Settings)?;
                        Ok(())
                    })();
                    if let Err(error) = result { self.message = format!("Cannot open settings: {error:#}"); }
                } else { self.message = "Settings location unavailable; use --settings PATH".into(); }
            }
            "redo" => self.doc_mut().redo(),
            "editor.action.selectAll" => self.doc_mut().select_all(),
            "expandLineSelection" => self.doc_mut().select_line(),
            "deleteLeft" => self.backspace_editor(false),
            "deleteRight" => self.doc_mut().delete(false),
            "deleteWordLeft" => self.backspace_editor(true),
            "deleteWordRight" => self.doc_mut().delete(true),
            "lineBreakInsert" => {
                let options = self.settings.typing(self.language());
                if let Err(error) = self.doc_mut().line_break_with_options(options) {
                    self.message = format!("Line break failed: {error:#}");
                }
            }
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
            "editor.action.commentLine" | "editor.action.addCommentLine" | "editor.action.removeCommentLine" => {
                use crate::document::CommentOperation;
                let operation = match command {
                    "editor.action.addCommentLine" => CommentOperation::Add,
                    "editor.action.removeCommentLine" => CommentOperation::Remove,
                    _ => CommentOperation::Toggle,
                };
                if let Err(error) = self.doc_mut().comment_lines(operation) {
                    self.message = format!("Line comment failed: {error:#}");
                }
            }
            "editor.action.blockComment" => {
                if let Err(error) = self.doc_mut().toggle_block_comment() {
                    self.message = format!("Block comment failed: {error:#}");
                }
            }
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
            "workbench.action.navigateBack" => self.navigate_history(navigation_history::Direction::Back),
            "workbench.action.navigateForward" => self.navigate_history(navigation_history::Direction::Forward),
            "outline.focus" => self.focus_outline(),
            "outline.collapse" => self.outline_expand_all(false),
            "outline.expand" => self.outline_expand_all(true),
            "outline.followCursor" => self.outline_follow_cursor(),
            "vscli.extensions.search" => self.start_prompt(PromptKind::SearchExtensions, String::new()),
            "vscli.extensions.updates" => self.manage_extension(extension_management::Action::CheckUpdates),
            "workbench.extensions.action.installVSIX" => self.start_prompt(PromptKind::InstallExtension, String::new()),
            "workbench.view.extensions" | "workbench.extensions.action.showInstalledExtensions" => self.manage_extension(extension_management::Action::List),
            "vscli.extensions.enableGlobal" | "vscli.extensions.enableWorkspace"
                | "vscli.extensions.disableGlobal" | "vscli.extensions.disableWorkspace" => {
                if let Some(id) = args.as_str().or_else(|| args["id"].as_str()) {
                    let scope = if command.ends_with("Workspace") { crate::extension_activation::Scope::Workspace } else { crate::extension_activation::Scope::Global };
                    if let Err(error) = self.set_extension_enabled(id, command.contains(".enable"), scope) {
                        self.message = format!("Cannot change extension enablement: {error:#}");
                    }
                } else { self.message = "Choose an installed extension ID to change remembered enablement".into(); }
            }
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
                    | "workbench.action.nextEditorInGroup"
                    | "workbench.action.previousEditorInGroup"
            )
    }
    fn save_as(&mut self) {
        self.capture_save_as_origin();
        let path = self
            .doc()
            .path
            .clone()
            .unwrap_or_else(|| self.workspace.root.join("untitled.txt"));
        self.start_prompt(PromptKind::SaveAs, path.to_string_lossy().into_owned());
    }
    fn save(&mut self, after: Option<AfterSave>) {
        if let Err(error) = self.request_native_save(after) {
            self.message = format!("Save failed; unsaved work retained: {error:#}");
        }
    }
    fn request_close(&mut self, action: AfterSave) {
        self.sync_pane();
        if matches!(action, AfterSave::Close) && !self.group_fallback {
            let Some(member) = self.active_tab_membership() else {
                return;
            };
            if self.close_membership != Some(member) {
                self.close_eligibility = sticky_tabs::CloseEligibility::AnyMode;
            }
            self.close_membership = Some(member);
            // Shared text belongs to its remaining tabs. Closing this view
            // requires neither discarding nor persisting the shared model.
            if self.editor_groups.memberships(member.document).count() > 1 {
                self.complete_close(action);
                return;
            }
        }
        if self.defer_close_for_persistence(action.clone()) {
            return;
        }
        if matches!(action, AfterSave::Quit | AfterSave::CloseAll) {
            // Hidden models remain authoritative native buffers. Bring dirty
            // models into the existing Save/Discard/Cancel flow before exit.
            let mut index = 0;
            while index < self.hidden_documents.len() {
                if self.hidden_documents[index].dirty() {
                    self.documents.push(self.hidden_documents.remove(index));
                } else {
                    index += 1;
                }
            }
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
        if let Some(document) = self.active_document().map(|doc| doc.id) {
            if self.group_fallback {
                self.record_closed();
                self.documents.remove(self.active);
                self.active = self.active.min(self.documents.len().saturating_sub(1));
                self.sync_pane();
            } else {
                let members: Vec<_> = self.editor_groups.memberships(document).collect();
                for member in members {
                    if let Err(error) = self.close_tab_membership(member) {
                        self.message = format!("Close rejected; buffers retained: {error:#}");
                        return;
                    }
                }
                // Public fixtures and newly promoted hidden models may not
                // have an installed membership yet.
                self.documents.retain(|doc| doc.id != document);
                self.project_editor_groups();
            }
        }
        if self.documents.is_empty() {
            self.session_closed_all();
        }
    }
    pub(super) fn finish_tab_close(&mut self, member: crate::editor_groups::Membership) {
        self.finish_tab_close_eligible(member, self.captured_close_eligibility(Some(member)));
    }
    fn finish_tab_close_eligible(
        &mut self,
        member: crate::editor_groups::Membership,
        eligibility: sticky_tabs::CloseEligibility,
    ) {
        if !self.editor_groups.membership_current(member) {
            self.closing_group = None;
            if self.close_membership == Some(member) {
                self.close_membership = None;
            }
            self.message =
                "Close retired: the original editor was closed; no other tab removed".into();
            return;
        }
        if !self.close_target_eligible(member, eligibility) {
            self.reject_protected_close(member, false);
            return;
        }
        let owned_group = self.close_batch_owns(member);
        if self.closing_group.is_some() && !owned_group {
            self.closing_group = None;
            self.message = "Close group retired: its tabs changed; buffers retained".into();
        }
        self.close_membership = None;
        match self.close_tab_membership(member) {
            Ok(()) => {
                if owned_group {
                    self.refresh_closing_group();
                }
            }
            Err(error) => {
                self.closing_group = None;
                self.message = format!("Close retired; buffers retained: {error:#}");
            }
        }
    }
    fn complete_close(&mut self, action: AfterSave) {
        match action {
            AfterSave::Close => {
                if self.group_fallback {
                    self.remove_active();
                } else if let Some(member) = self
                    .close_membership
                    .or_else(|| self.active_tab_membership())
                {
                    self.finish_tab_close(member);
                }
            }
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
                    self.session_closed_all();
                }
            }
        }
    }
    fn modal_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            if matches!(self.modal, Some(Modal::Confirm(_))) {
                self.cancel_save_continuations();
            }
            if matches!(self.modal, Some(Modal::ExtensionTree)) {
                self.close_surface_tree();
            }
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
            Modal::ExtensionSurfaces(picker) => self.surface_picker_key(key, picker),
            Modal::ExtensionTree => self.surface_tree_key(key),
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
            Modal::ExtensionRegistry {
                items,
                mut selected,
            } => {
                match key.code {
                    KeyCode::Up => selected = selected.saturating_sub(1),
                    KeyCode::Down => selected = (selected + 1).min(items.len().saturating_sub(1)),
                    KeyCode::Enter => {
                        if let Some(item) = items.get(selected) {
                            self.manage_extension(extension_management::Action::RegistryInstall(
                                item.clone(),
                            ));
                        }
                        return;
                    }
                    KeyCode::Char('/') => {
                        self.start_prompt(PromptKind::SearchExtensions, String::new());
                        return;
                    }
                    _ => {}
                }
                self.modal = Some(Modal::ExtensionRegistry { items, selected });
            }
            Modal::Extensions {
                items,
                mut selected,
            } => {
                match key.code {
                    KeyCode::Up => selected = selected.saturating_sub(1),
                    KeyCode::Down => selected = (selected + 1).min(items.len().saturating_sub(1)),
                    KeyCode::Char(character @ ('e' | 'E' | 'd' | 'D'))
                        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                    {
                        if let Some(item) = items.get(selected) {
                            let scope = if character.is_ascii_uppercase()
                                || key.modifiers.contains(KeyModifiers::SHIFT)
                            {
                                crate::extension_activation::Scope::Workspace
                            } else {
                                crate::extension_activation::Scope::Global
                            };
                            if let Err(error) = self.set_extension_enabled(
                                &item.id,
                                matches!(character, 'e' | 'E'),
                                scope,
                            ) {
                                self.message =
                                    format!("Cannot change extension enablement: {error:#}");
                            }
                        }
                    }
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
                    KeyCode::Char('/') => {
                        self.start_prompt(PromptKind::SearchExtensions, String::new());
                        return;
                    }
                    KeyCode::Char('u' | 'U') => {
                        self.manage_extension(extension_management::Action::CheckUpdates);
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
                            let path = hit.path.clone();
                            match self.open_with_intent(&path, navigation::OpenIntent::Search(hit))
                            {
                                Ok(()) => return,
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
                    if matches!(action, AfterSave::Close) {
                        self.complete_close(action);
                    } else {
                        self.remove_active();
                        self.complete_close(action);
                    }
                }
                KeyCode::Char('c' | 'C') => self.cancel_save_continuations(),
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
            if self
                .prompt
                .as_ref()
                .is_some_and(|prompt| matches!(prompt.kind, PromptKind::SaveAs))
            {
                self.cancel_save_continuations();
            }
            self.cancel_extension_prompt();
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
            KeyCode::Down => {
                p.selected = (p.selected + 1).min(if matches!(p.kind, PromptKind::Extension(_)) {
                    127
                } else if matches!(p.kind, PromptKind::Symbols) {
                    511
                } else {
                    99
                })
            }
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
                let accepted = p.insert(&c.to_string());
                if !accepted {
                    self.message = match p.kind {
                        PromptKind::Symbols => "Symbol query exceeds 1 KiB",
                        PromptKind::SearchExtensions => "Extension search exceeds 1 KiB",
                        _ => "Extension prompt text exceeds 4 KiB",
                    }
                    .into();
                }
            }
            _ => {}
        }
    }
    fn accept_prompt(&mut self) {
        let p = self.prompt.take().unwrap();
        match p.kind {
            PromptKind::Extension(request) => {
                self.accept_extension_prompt(request, p.text, p.selected)
            }
            PromptKind::Symbols => self.accept_symbol(&p.text, p.selected),
            PromptKind::RecentFiles => self.accept_recent(&p.text, p.selected),
            PromptKind::Snippet => self.accept_snippet(&p.text, p.selected),
            PromptKind::SearchExtensions => {
                self.manage_extension(extension_management::Action::Search(p.text))
            }
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
                    let preview = self.settings.editor_preview();
                    self.open_editor_navigation(
                        path.clone(),
                        preview.enabled && preview.from_quick_open,
                    );
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
                let path = self.resolve_path(&p.text);
                if let Err(error) = self.request_native_save_as(path) {
                    self.message = format!("Save failed; unsaved work retained: {error:#}");
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
                    self.observe_navigation(navigation_history::Reason::Jump);
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
            KeyCode::Tab if self.outline_view().status != OutlineStatus::Hidden => {
                self.focus_outline();
            }
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
        self.explorer_open_mode(false);
    }
    fn explorer_open_mode(&mut self, preview: bool) {
        if let Some(entry) = self.entries.get(self.explorer_selected).cloned() {
            if entry.directory {
                self.explorer_dir = entry.path;
                self.entries = directory_entries(&self.explorer_dir);
                self.explorer_selected = 0;
            } else {
                self.open_editor_navigation(
                    entry.path,
                    preview && self.settings.editor_preview().enabled,
                );
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
    #[test]
    fn actual_mru_reservation_refusal_does_not_publish_new_untitled_or_touch_original_redo() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.documents.push(Document::from_text("猫🙂 original\r\n"));
        app.sync_pane();
        app.doc_mut().move_to(1, false);
        app.doc_mut().insert(" dirty λ", false);
        let dirty = app.doc().text.clone();
        app.doc_mut().undo();
        let original = (
            app.doc().id,
            app.doc().cursor,
            app.doc().anchor,
            app.doc().revision,
            app.doc().text_epoch(),
            app.doc().text.clone(),
        );
        let groups = app.editor_groups.clone();
        let panes = app
            .panes
            .iter()
            .map(|pane| (pane.id, pane.document))
            .collect::<Vec<_>>();
        let active = (app.active, app.active_pane, app.focus.clone());
        app.editor_groups.fail_next_recent_reservation();
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        assert!(
            app.message.contains("injected allocation refusal"),
            "{}",
            app.message
        );
        assert_eq!(app.documents.len(), 1);
        assert!(app.hidden_documents.is_empty());
        assert_eq!(app.editor_groups, groups);
        assert_eq!(
            app.panes
                .iter()
                .map(|pane| (pane.id, pane.document))
                .collect::<Vec<_>>(),
            panes
        );
        assert!((app.active, app.active_pane, app.focus.clone()) == active);
        assert_eq!(
            (
                app.doc().id,
                app.doc().cursor,
                app.doc().anchor,
                app.doc().revision,
                app.doc().text_epoch(),
                app.doc().text.clone()
            ),
            original
        );
        app.doc_mut().redo();
        assert_eq!(app.doc().text, dirty);
        app.doc_mut().undo();
        assert_eq!(app.doc().text, original.5);
    }
    fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
        app.event(Event::Key(KeyEvent::new(code, modifiers)));
    }
    fn settle_saves(app: &mut App) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.saves_pending() {
            app.poll();
            assert!(std::time::Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    #[test]
    fn hidden_dirty_models_reuse_identity_and_require_close_confirmation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("hidden.txt");
        std::fs::write(&path, "original").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        let mut hidden = Document::open_existing(&path).unwrap();
        let id = hidden.id;
        let canonical = hidden.path.clone().unwrap();
        hidden.insert("unsaved ", false);
        app.hidden_documents.push(hidden);
        assert!(app.active_document().is_none());
        assert!(app.panes.is_empty());
        assert_eq!(app.recovery_documents().len(), 1);
        std::fs::remove_file(&path).unwrap();
        app.open(&canonical).unwrap();
        assert_eq!(app.doc().id, id);
        assert!(app.hidden_documents.is_empty());
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "original");
        app.doc_mut().redo();
        app.hidden_documents.push(app.documents.remove(0));
        app.sync_pane();
        app.request_close(AfterSave::Quit);
        assert_eq!(app.doc().id, id);
        assert!(matches!(app.modal, Some(Modal::Confirm(AfterSave::Quit))));
        app.modal_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(app.running);
        assert_eq!(app.doc().text.to_string(), "unsaved original");
    }
    #[test]
    fn extension_picker_grants_require_plain_or_shift_keys() {
        let dir = tempfile::tempdir().unwrap();
        for modifiers in [
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::SUPER,
            KeyModifiers::HYPER,
            KeyModifiers::META,
            KeyModifiers::META | KeyModifiers::SHIFT,
            KeyModifiers::HYPER | KeyModifiers::SHIFT,
            KeyModifiers::NONE,
            KeyModifiers::SHIFT,
        ] {
            for character in ['e', 'd'] {
                let mut app = App::new(dir.path().into(), Profile::Linux);
                app.extensions_directory = Some(dir.path().join("extensions"));
                app.configure_extension_activation(Some(&dir.path().join("config")));
                app.modal = Some(Modal::Extensions {
                    items: vec![crate::extension_store::Installed {
                        id: "test.modifiers".into(),
                        version: "1".into(),
                        path: dir.path().join("package"),
                        source: "fixture".into(),
                        sha256: String::new(),
                        manifest: json!({"main":"index.cjs"}),
                        compatibility: "fixture".into(),
                    }],
                    selected: 0,
                });
                app.message = "No grant requested".into();
                key(&mut app, KeyCode::Char(character), modifiers);
                if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT {
                    assert!(app.message.starts_with("Saving "), "{modifiers:?}");
                } else {
                    assert_eq!(app.message, "No grant requested", "{modifiers:?}");
                }
                assert!(matches!(app.modal, Some(Modal::Extensions { .. })));
                assert!(app.documents.is_empty());
            }
        }
        assert!(!dir.path().join("config/extensions-enabled.json").exists());
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
        settle_saves(&mut a);
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
        settle_saves(&mut app);
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
