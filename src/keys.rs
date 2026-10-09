use anyhow::{Context, Result, bail};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, io::Read, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Profile {
    Linux,
    Windows,
    Macos,
}
impl Profile {
    pub fn native() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
    pub fn primary(self) -> &'static str {
        if self == Self::Macos { "cmd" } else { "ctrl" }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Binding {
    pub key: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    #[serde(
        default,
        deserialize_with = "binding_args",
        skip_serializing_if = "Option::is_none"
    )]
    pub args: Option<Value>,
}

fn binding_args<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

pub struct Keymap {
    pub bindings: Vec<Binding>,
    pub profile: Profile,
    defaults: Vec<Binding>,
    extensions: Vec<Binding>,
    user: Vec<Binding>,
}
#[derive(Debug, PartialEq)]
pub enum Resolution {
    Command(String, Option<Value>),
    Chord,
    None,
}

impl Keymap {
    pub fn new(profile: Profile) -> Self {
        let mut map = Self {
            bindings: Vec::new(),
            profile,
            defaults: Vec::new(),
            extensions: Vec::new(),
            user: Vec::new(),
        };
        let p = profile.primary();
        for (key, command) in [
            ("n", "workbench.action.files.newUntitledFile"),
            ("o", "workbench.action.files.openFile"),
            ("s", "workbench.action.files.save"),
            ("shift+s", "workbench.action.files.saveAs"),
            ("w", "workbench.action.closeActiveEditor"),
            ("shift+w", "workbench.action.closeWindow"),
            ("p", "workbench.action.quickOpen"),
            ("shift+p", "workbench.action.showCommands"),
            ("b", "workbench.action.toggleSidebarVisibility"),
            ("shift+e", "workbench.view.explorer"),
        ] {
            map.add(&format!("{p}+{key}"), command, None);
        }
        map.add("ctrl+r", "workbench.action.openRecent", None);
        map.add(
            &format!("{p}+shift+t"),
            "workbench.action.reopenClosedEditor",
            None,
        );
        map.add("f1", "workbench.action.showCommands", None);
        if profile != Profile::Windows {
            map.add(&format!("{p}+q"), "workbench.action.quit", None);
        }
        for (key, command) in [
            ("z", "undo"),
            ("a", "editor.action.selectAll"),
            ("c", "editor.action.clipboardCopyAction"),
            ("x", "editor.action.clipboardCutAction"),
            ("v", "editor.action.clipboardPasteAction"),
            ("f", "actions.find"),
            ("l", "expandLineSelection"),
            ("/", "editor.action.commentLine"),
            ("shift+k", "editor.action.deleteLines"),
        ] {
            map.add(&format!("{p}+{key}"), command, Some("editorTextFocus"));
        }
        for (key, command) in [
            ("d", "editor.action.addSelectionToNextFindMatch"),
            ("shift+l", "editor.action.selectHighlights"),
            ("f2", "editor.action.changeAll"),
            ("u", "cursorUndo"),
            ("enter", "editor.action.insertLineAfter"),
            ("shift+enter", "editor.action.insertLineBefore"),
            ("shift+\\", "editor.action.jumpToBracket"),
        ] {
            map.add(&format!("{p}+{key}"), command, Some("editorTextFocus"));
        }
        map.add(
            "shift+alt+i",
            "editor.action.insertCursorAtEndOfEachLineSelected",
            Some("editorTextFocus"),
        );
        let copy_mod = if profile == Profile::Linux {
            "ctrl+shift+alt"
        } else {
            "shift+alt"
        };
        let cursor_mod = match profile {
            Profile::Linux => "shift+alt",
            Profile::Windows => "ctrl+alt",
            Profile::Macos => "cmd+alt",
        };
        for (key, movement, copy, cursor) in [
            (
                "up",
                "editor.action.moveLinesUpAction",
                "editor.action.copyLinesUpAction",
                "editor.action.insertCursorAbove",
            ),
            (
                "down",
                "editor.action.moveLinesDownAction",
                "editor.action.copyLinesDownAction",
                "editor.action.insertCursorBelow",
            ),
        ] {
            map.add(&format!("alt+{key}"), movement, Some("editorTextFocus"));
            map.add(&format!("{copy_mod}+{key}"), copy, Some("editorTextFocus"));
            map.add(
                &format!("{cursor_mod}+{key}"),
                cursor,
                Some("editorTextFocus"),
            );
        }
        map.add(
            &format!("{p}+shift+f"),
            "workbench.action.findInFiles",
            None,
        );
        for (key, command) in [
            (format!("{p}+k {p}+i"), "editor.action.showHover"),
            ("ctrl+space".into(), "editor.action.triggerSuggest"),
            ("f12".into(), "editor.action.revealDefinition"),
            ("shift+f12".into(), "editor.action.goToReferences"),
            ("f2".into(), "editor.action.rename"),
            (
                if profile == Profile::Linux {
                    "ctrl+shift+i".into()
                } else {
                    "shift+alt+f".into()
                },
                "editor.action.formatDocument",
            ),
            (format!("{p}+shift+m"), "workbench.actions.view.problems"),
        ] {
            map.add(&key, command, Some("editorTextFocus"));
        }
        map.add(
            &format!("{p}+shift+o"),
            "workbench.action.gotoSymbol",
            Some("!accessibilityHelpIsShown && !accessibleViewIsShown"),
        );
        map.add(&format!("{p}+t"), "workbench.action.showAllSymbols", None);
        let action_context =
            Some("editorHasCodeActionsProvider && textInputFocus && !editorReadonly");
        map.add(&format!("{p}+."), "editor.action.quickFix", action_context);
        map.add("ctrl+shift+r", "editor.action.refactor", action_context);
        map.add("ctrl+`", "workbench.action.terminal.toggleTerminal", None);
        map.add("ctrl+shift+`", "workbench.action.terminal.new", None);
        map.add(&format!("{p}+j"), "workbench.action.togglePanel", None);
        map.add(
            &format!("{p}+1"),
            "workbench.action.focusFirstEditorGroup",
            None,
        );
        map.add(
            &format!("{p}+shift+b"),
            "workbench.action.tasks.build",
            None,
        );
        map.add(
            &format!("{p}+shift+x"),
            "workbench.view.extensions",
            Some("viewContainer.workbench.view.extensions.enabled"),
        );
        map.add("ctrl+shift+g", "workbench.view.scm", None);
        map.add(&format!("{p}+\\"), "workbench.action.splitEditor", None);
        for (key, command) in [
            ("1", "workbench.action.focusFirstEditorGroup"),
            ("2", "workbench.action.focusSecondEditorGroup"),
            ("3", "workbench.action.focusThirdEditorGroup"),
            ("4", "workbench.action.focusFourthEditorGroup"),
        ] {
            map.add(&format!("{p}+{key}"), command, None);
        }
        map.add(
            if profile == Profile::Macos {
                "enter"
            } else {
                "f2"
            },
            "renameFile",
            Some("filesExplorerFocus"),
        );
        map.add(
            if profile == Profile::Macos {
                "cmd+backspace"
            } else {
                "delete"
            },
            "deleteFile",
            Some("filesExplorerFocus"),
        );
        for (key, command) in [
            ("f5", "workbench.action.debug.start"),
            ("shift+f5", "workbench.action.debug.stop"),
            ("f9", "editor.debug.action.toggleBreakpoint"),
            ("f10", "workbench.action.debug.stepOver"),
            ("f11", "workbench.action.debug.stepInto"),
            ("shift+f11", "workbench.action.debug.stepOut"),
        ] {
            map.add(key, command, Some("editorTextFocus"));
        }
        map.add(&format!("{p}+shift+d"), "workbench.view.debug", None);
        map.add(&format!("{p}+,"), "workbench.action.openSettings", None);
        let redo = if profile == Profile::Windows {
            "ctrl+y".into()
        } else {
            format!("{p}+shift+z")
        };
        map.add(&redo, "redo", Some("editorTextFocus"));
        if profile == Profile::Linux {
            map.add("ctrl+y", "redo", Some("editorTextFocus"));
        }
        map.add(
            "ctrl+g",
            "workbench.action.gotoLine",
            Some("editorTextFocus"),
        );
        map.add(
            if profile == Profile::Macos {
                "cmd+alt+f"
            } else {
                "ctrl+h"
            },
            "editor.action.startFindReplaceAction",
            Some("editorTextFocus"),
        );
        map.add(
            if profile == Profile::Macos {
                "cmd+g"
            } else {
                "f3"
            },
            "editor.action.nextMatchFindAction",
            Some("editorTextFocus"),
        );
        map.add(
            if profile == Profile::Macos {
                "cmd+shift+g"
            } else {
                "shift+f3"
            },
            "editor.action.previousMatchFindAction",
            Some("editorTextFocus"),
        );
        map.add(
            if profile == Profile::Macos {
                "cmd+alt+right"
            } else {
                "ctrl+pagedown"
            },
            "workbench.action.nextEditor",
            None,
        );
        map.add(
            if profile == Profile::Macos {
                "cmd+alt+left"
            } else {
                "ctrl+pageup"
            },
            "workbench.action.previousEditor",
            None,
        );
        map.add(
            &format!("{p}+]"),
            "editor.action.indentLines",
            Some("editorTextFocus"),
        );
        map.add(
            &format!("{p}+["),
            "editor.action.outdentLines",
            Some("editorTextFocus"),
        );
        map.add(
            &format!("{p}+k {p}+s"),
            "workbench.action.openGlobalKeybindings",
            None,
        );
        map.add(
            &format!("{p}+k {p}+w"),
            "workbench.action.closeAllEditors",
            None,
        );
        map.add(
            &format!("{p}+k {p}+c"),
            "editor.action.addCommentLine",
            Some("editorTextFocus"),
        );
        map.add(
            &format!("{p}+k {p}+u"),
            "editor.action.removeCommentLine",
            Some("editorTextFocus"),
        );
        for (key, command) in [
            ("left", "cursorLeft"),
            ("right", "cursorRight"),
            ("up", "cursorUp"),
            ("down", "cursorDown"),
            ("home", "cursorHome"),
            ("end", "cursorEnd"),
            ("pageup", "cursorPageUp"),
            ("pagedown", "cursorPageDown"),
        ] {
            map.add(key, command, Some("editorTextFocus"));
            map.add(
                &format!("shift+{key}"),
                &format!("{command}Select"),
                Some("editorTextFocus"),
            );
        }
        let word = if profile == Profile::Macos {
            "alt"
        } else {
            "ctrl"
        };
        for (key, cmd) in [("left", "cursorWordLeft"), ("right", "cursorWordRight")] {
            map.add(&format!("{word}+{key}"), cmd, Some("editorTextFocus"));
            map.add(
                &format!("{word}+shift+{key}"),
                &format!("{cmd}Select"),
                Some("editorTextFocus"),
            );
        }
        for (key, cmd) in [("home", "cursorTop"), ("end", "cursorBottom")] {
            map.add(&format!("ctrl+{key}"), cmd, Some("editorTextFocus"));
            map.add(
                &format!("ctrl+shift+{key}"),
                &format!("{cmd}Select"),
                Some("editorTextFocus"),
            );
        }
        if profile == Profile::Macos {
            for (key, cmd) in [
                ("left", "cursorHome"),
                ("right", "cursorEnd"),
                ("up", "cursorTop"),
                ("down", "cursorBottom"),
            ] {
                map.add(&format!("cmd+{key}"), cmd, Some("editorTextFocus"));
                map.add(
                    &format!("cmd+shift+{key}"),
                    &format!("{cmd}Select"),
                    Some("editorTextFocus"),
                );
            }
        }
        for (key, cmd) in [
            ("backspace", "deleteLeft"),
            ("delete", "deleteRight"),
            ("enter", "lineBreakInsert"),
            ("tab", "tab"),
            ("shift+tab", "outdent"),
            ("escape", "cancelSelection"),
        ] {
            map.add(key, cmd, Some("editorTextFocus"));
        }
        map.add(
            &format!("{word}+backspace"),
            "deleteWordLeft",
            Some("editorTextFocus"),
        );
        map.add(
            &format!("{word}+delete"),
            "deleteWordRight",
            Some("editorTextFocus"),
        );
        map.add(
            "tab",
            "jumpToNextSnippetPlaceholder",
            Some("hasNextTabstop && inSnippetMode && textInputFocus"),
        );
        map.add(
            "shift+tab",
            "jumpToPrevSnippetPlaceholder",
            Some("hasPrevTabstop && inSnippetMode && textInputFocus"),
        );
        for key in ["escape", "shift+escape"] {
            map.add(key, "leaveSnippet", Some("inSnippetMode && textInputFocus"));
        }
        map.add(
            &format!("{p}+shift+space"),
            "editor.action.triggerParameterHints",
            Some("editorHasSignatureHelpProvider && editorTextFocus"),
        );
        for key in ["escape", "shift+escape"] {
            map.add(
                key,
                "closeParameterHints",
                Some("editorFocus && parameterHintsVisible"),
            );
        }
        for (key, command) in [
            ("down", "selectNextSuggestion"),
            ("up", "selectPrevSuggestion"),
            ("pagedown", "selectNextPageSuggestion"),
            ("pageup", "selectPrevPageSuggestion"),
            ("escape", "hideSuggestWidget"),
            ("shift+escape", "hideSuggestWidget"),
        ] {
            map.add(key, command, Some("textInputFocus && suggestWidgetVisible"));
        }
        map.add(
            "tab",
            "acceptSelectedSuggestion",
            Some("textInputFocus && suggestWidgetVisible && !inSnippetMode"),
        );
        map.add(
            "enter",
            "acceptSelectedSuggestion",
            Some("textInputFocus && suggestWidgetVisible && acceptSuggestionOnEnter"),
        );
        map.defaults = map.bindings.clone();
        map
    }
    fn add(&mut self, key: &str, command: &str, when: Option<&str>) {
        self.bindings.push(Binding {
            key: normalize_sequence(key),
            command: command.into(),
            when: when.map(str::to_owned),
            args: None,
        });
    }
    pub fn load(&mut self, path: &Path) -> Result<usize> {
        if !std::fs::metadata(path)
            .with_context(|| format!("Cannot read {}", path.display()))?
            .is_file()
        {
            bail!("Keybindings must be a regular file");
        }
        let file =
            std::fs::File::open(path).with_context(|| format!("Cannot read {}", path.display()))?;
        let mut raw = String::new();
        file.take(1024 * 1024 + 1).read_to_string(&mut raw)?;
        if raw.len() > 1024 * 1024 {
            bail!("Keybindings file exceeds 1 MiB");
        }
        let entries: Vec<Binding> =
            crate::jsonc::parse(&raw).context("Invalid keybindings.json")?;
        validate_bindings(&entries)?;
        let count = entries.len();
        self.user.extend(entries);
        self.rebuild();
        Ok(count)
    }
    /// Preserve imported files while applying only rules this resolver can parse.
    pub fn load_imported(&mut self, path: &Path) -> Result<(usize, Vec<String>)> {
        use std::io::Read;
        if !std::fs::metadata(path)?.is_file() {
            bail!("Imported keybindings must be a regular file");
        }
        let mut raw = String::new();
        std::fs::File::open(path)?
            .take(1024 * 1024 + 1)
            .read_to_string(&mut raw)?;
        if raw.len() > 1024 * 1024 {
            bail!("Imported keybindings exceed 1 MiB");
        }
        let entries: Vec<Binding> =
            crate::jsonc::parse(&raw).context("Invalid keybindings.json")?;
        let mut notices = Vec::new();
        let mut count = 0;
        for entry in entries {
            if let Err(error) = validate_bindings(std::slice::from_ref(&entry)) {
                notices.push(format!("Skipped imported {}: {error:#}", entry.key));
            } else {
                self.user.push(entry);
                count += 1;
            }
        }
        self.rebuild();
        Ok((count, notices))
    }
    pub fn set_extension_bindings(&mut self, value: Value) -> Result<usize> {
        let entries = match value {
            Value::Null => Vec::new(),
            Value::Array(entries) => entries,
            Value::Object(_) => vec![value],
            _ => bail!("Extension keybindings must be an object or array"),
        };
        if entries.len() > 1024 {
            bail!("Extension keybinding limit exceeded");
        }
        let mut bindings = Vec::new();
        let platform = match self.profile {
            Profile::Linux => "linux",
            Profile::Windows => "win",
            Profile::Macos => "mac",
        };
        for mut entry in entries {
            if let Some(key) = entry.get(platform).cloned() {
                entry["key"] = key;
            }
            let binding: Binding = serde_json::from_value(entry)?;
            if binding.command.starts_with('-') {
                bail!("Extension default bindings cannot remove other defaults");
            }
            bindings.push(binding);
        }
        validate_bindings(&bindings)?;
        let count = bindings.len();
        self.extensions = bindings;
        self.rebuild();
        Ok(count)
    }
    pub fn set_extension_binding_sets(
        &mut self,
        mut sets: Vec<(String, Value)>,
    ) -> Result<Vec<String>> {
        sets.sort_by(|a, b| a.0.cmp(&b.0));
        let mut combined = Vec::new();
        let mut errors = Vec::new();
        for (owner, value) in sets {
            let mut parsed = Self::new(self.profile);
            match parsed.set_extension_bindings(value) {
                Ok(_) => combined.extend(parsed.extensions),
                Err(error) => errors.push(format!("{owner}: {error:#}")),
            }
            if combined.len() > 1024 {
                bail!("Extension session keybinding limit exceeded");
            }
        }
        self.extensions = combined;
        self.rebuild();
        Ok(errors)
    }
    pub fn clear_extension_bindings(&mut self) {
        self.extensions.clear();
        self.rebuild();
    }
    fn rebuild(&mut self) {
        self.bindings = self.defaults.clone();
        self.bindings
            .extend(self.extensions.iter().cloned().map(|mut b| {
                b.key = normalize_sequence(&b.key);
                b
            }));
        // Reapply user removals after extension defaults, including extensions
        // that activate after the user's keybindings were imported.
        for mut b in self.user.clone() {
            b.key = normalize_sequence(&b.key);
            if let Some(command) = b.command.strip_prefix('-') {
                self.bindings.retain(|old| {
                    !(old.key == b.key
                        && old.command == command
                        && (b.when.is_none() || b.when == old.when))
                });
            } else {
                self.bindings.push(b);
            }
        }
    }
    pub fn resolve(&self, sequence: &str, context: &HashMap<String, Value>) -> Resolution {
        for b in self.bindings.iter().rev() {
            let exact = b.key == sequence;
            let chord = b
                .key
                .strip_prefix(sequence)
                .is_some_and(|suffix| suffix.starts_with(' '));
            if !exact && !chord {
                continue;
            }
            if b.when
                .as_ref()
                .is_some_and(|w| !evaluate(w, context).unwrap_or(false))
            {
                continue;
            }
            if exact {
                return Resolution::Command(b.command.clone(), b.args.clone());
            }
            if chord {
                return Resolution::Chord;
            }
        }
        Resolution::None
    }
    /// A displayed hint must resolve to this command in the current UI context.
    pub fn shortcut_in_context(&self, command: &str, context: &HashMap<String, Value>) -> String {
        self.bindings.iter().rev()
            .find(|binding| binding.command == command && matches!(self.resolve(&binding.key, context), Resolution::Command(id, _) if id == command))
            .map(|binding| binding.key.clone()).unwrap_or_default()
    }
    pub fn shortcut(&self, command: &str) -> String {
        self.bindings
            .iter()
            .rev()
            .find(|b| b.command == command)
            .map(|b| b.key.clone())
            .unwrap_or_default()
    }
}

pub(crate) fn validate_bindings(entries: &[Binding]) -> Result<()> {
    for b in entries {
        if b.key.trim().is_empty() {
            bail!("Empty keybinding");
        }
        if let Some(when) = &b.when {
            evaluate(when, &HashMap::new())
                .with_context(|| format!("Unsupported when expression: {when}"))?;
        }
    }
    Ok(())
}

pub fn token(event: KeyEvent) -> String {
    let mut mods = event.modifiers;
    let name = match event.code {
        KeyCode::Char(' ') => "space".into(),
        KeyCode::Null => {
            mods.insert(KeyModifiers::CONTROL);
            "space".into()
        }
        KeyCode::Char(c) => {
            if c.is_ascii_uppercase() {
                mods.insert(KeyModifiers::SHIFT);
            }
            c.to_ascii_lowercase().to_string()
        }
        KeyCode::F(n) => format!("f{n}"),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::Home => "home".into(),
        KeyCode::End => "end".into(),
        KeyCode::PageUp => "pageup".into(),
        KeyCode::PageDown => "pagedown".into(),
        KeyCode::Enter => "enter".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::BackTab => {
            mods.insert(KeyModifiers::SHIFT);
            "tab".into()
        }
        KeyCode::Backspace => "backspace".into(),
        KeyCode::Delete => "delete".into(),
        KeyCode::Esc => "escape".into(),
        KeyCode::Insert => "insert".into(),
        _ => return String::new(),
    };
    let mut parts = Vec::new();
    if mods.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl".into());
    }
    if mods.intersects(KeyModifiers::SUPER | KeyModifiers::META) {
        parts.push("cmd".into());
    }
    if mods.contains(KeyModifiers::ALT) {
        parts.push("alt".into());
    }
    if mods.contains(KeyModifiers::SHIFT) {
        parts.push("shift".into());
    }
    parts.push(name);
    parts.join("+")
}
fn normalize_sequence(s: &str) -> String {
    s.split_whitespace()
        .map(|stroke| {
            let parts: Vec<_> = stroke.split('+').map(str::to_lowercase).collect();
            let mut ordered = Vec::new();
            for m in ["ctrl", "cmd", "alt", "shift"] {
                if parts
                    .iter()
                    .any(|p| p == m || (m == "cmd" && (p == "meta" || p == "win")))
                {
                    ordered.push(m.to_string());
                }
            }
            if let Some(key) = parts.last() {
                ordered.push(if key == "esc" {
                    "escape".into()
                } else {
                    key.clone()
                });
            }
            ordered.join("+")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, PartialEq)]
enum Tok {
    Name(String),
    Not,
    And,
    Or,
    Eq,
    Ne,
    L,
    R,
}
const MAX_WHEN_BYTES: usize = 8 * 1024;
const MAX_WHEN_TOKENS: usize = 1024;
const MAX_WHEN_DEPTH: usize = 64;

fn lex(s: &str) -> Result<Vec<Tok>> {
    if s.len() > MAX_WHEN_BYTES {
        bail!("When expression exceeds 8 KiB");
    }
    let chars: Vec<_> = s.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if out.len() == MAX_WHEN_TOKENS {
            bail!("When expression exceeds 1024 tokens");
        }
        match c {
            '(' => {
                out.push(Tok::L);
                i += 1;
            }
            ')' => {
                out.push(Tok::R);
                i += 1;
            }
            '!' if chars.get(i + 1) != Some(&'=') => {
                out.push(Tok::Not);
                i += 1;
            }
            '&' | '|' if chars.get(i + 1) == Some(&c) => {
                out.push(if c == '&' { Tok::And } else { Tok::Or });
                i += 2;
            }
            '=' | '!' if chars.get(i + 1) == Some(&'=') => {
                out.push(if c == '=' { Tok::Eq } else { Tok::Ne });
                i += 2;
                if chars.get(i) == Some(&'=') {
                    i += 1;
                }
            }
            '\'' | '"' => {
                let quote = c;
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != quote {
                    i += 1;
                }
                if i == chars.len() {
                    bail!("Unclosed string");
                }
                out.push(Tok::Name(chars[start..i].iter().collect()));
                i += 1;
            }
            c if c.is_alphanumeric() || c == '_' => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '.' | '-'))
                {
                    i += 1;
                }
                out.push(Tok::Name(chars[start..i].iter().collect()));
            }
            _ => bail!(
                "Only !, &&, ||, ==, != and parentheses are supported in when clauses in this alpha"
            ),
        }
    }
    Ok(out)
}
pub fn evaluate(s: &str, context: &HashMap<String, Value>) -> Result<bool> {
    struct Parser<'a> {
        t: Vec<Tok>,
        i: usize,
        c: &'a HashMap<String, Value>,
        depth: usize,
    }
    impl Parser<'_> {
        fn expr(&mut self) -> Result<bool> {
            let mut v = self.and()?;
            while self.t.get(self.i) == Some(&Tok::Or) {
                self.i += 1;
                let rhs = self.and()?;
                v |= rhs;
            }
            Ok(v)
        }
        fn and(&mut self) -> Result<bool> {
            let mut v = self.atom()?;
            while self.t.get(self.i) == Some(&Tok::And) {
                self.i += 1;
                let rhs = self.atom()?;
                v &= rhs;
            }
            Ok(v)
        }
        fn atom(&mut self) -> Result<bool> {
            if self.depth > MAX_WHEN_DEPTH {
                bail!("When expression exceeds 64 nested groups or negations");
            }
            self.depth += 1;
            let result = self.atom_inner();
            self.depth -= 1;
            result
        }
        fn atom_inner(&mut self) -> Result<bool> {
            if self.t.get(self.i) == Some(&Tok::Not) {
                self.i += 1;
                return Ok(!self.atom()?);
            }
            if self.t.get(self.i) == Some(&Tok::L) {
                self.i += 1;
                let v = self.expr()?;
                if self.t.get(self.i) != Some(&Tok::R) {
                    bail!("Missing )");
                }
                self.i += 1;
                return Ok(v);
            }
            let Some(Tok::Name(name)) = self.t.get(self.i) else {
                bail!("Expected context key");
            };
            let value = if name == "true" {
                Value::Bool(true)
            } else if name == "false" {
                Value::Bool(false)
            } else {
                self.c.get(name).cloned().unwrap_or(Value::Null)
            };
            self.i += 1;
            if matches!(self.t.get(self.i), Some(Tok::Eq | Tok::Ne)) {
                let equal = self.t[self.i] == Tok::Eq;
                self.i += 1;
                let Some(Tok::Name(rhs)) = self.t.get(self.i) else {
                    bail!("Expected comparison value");
                };
                self.i += 1;
                let lhs = match value {
                    Value::String(s) => s,
                    Value::Null => "undefined".into(),
                    v => v.to_string(),
                };
                return Ok((lhs == *rhs) == equal);
            }
            Ok(match value {
                Value::Bool(b) => b,
                Value::Null => false,
                Value::String(s) => !s.is_empty(),
                _ => true,
            })
        }
    }
    let mut p = Parser {
        t: lex(s)?,
        i: 0,
        c: context,
        depth: 0,
    };
    let value = p.expr()?;
    if p.i != p.t.len() {
        bail!("Unexpected expression token");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn suggestion_bindings_are_conditional_and_preserve_snippet_tab_on_every_profile() {
        for profile in [Profile::Linux, Profile::Macos, Profile::Windows] {
            let map = Keymap::new(profile);
            let mut context = HashMap::from([
                ("editorTextFocus".into(), json!(true)),
                ("textInputFocus".into(), json!(true)),
                ("suggestWidgetVisible".into(), json!(false)),
                ("inSnippetMode".into(), json!(false)),
                ("acceptSuggestionOnEnter".into(), json!(true)),
            ]);
            assert!(
                matches!(map.resolve("tab", &context), Resolution::Command(id, _) if id == "tab")
            );
            context.insert("suggestWidgetVisible".into(), json!(true));
            for (key, command) in [
                ("tab", "acceptSelectedSuggestion"),
                ("enter", "acceptSelectedSuggestion"),
                ("up", "selectPrevSuggestion"),
                ("down", "selectNextSuggestion"),
                ("escape", "hideSuggestWidget"),
            ] {
                assert!(
                    matches!(map.resolve(key, &context), Resolution::Command(id, _) if id == command)
                );
            }
            context.insert("inSnippetMode".into(), json!(true));
            context.insert("hasNextTabstop".into(), json!(true));
            assert!(
                matches!(map.resolve("tab", &context), Resolution::Command(id, _) if id == "jumpToNextSnippetPlaceholder")
            );
            context.insert("acceptSuggestionOnEnter".into(), json!(false));
            assert!(
                matches!(map.resolve("enter", &context), Resolution::Command(id, _) if id == "lineBreakInsert")
            );
        }
    }
    #[test]
    fn key_prefilter_preserves_context_priority_and_chord_resolution() {
        // Compare against the original resolver over every shipped rule, chord
        // prefix, platform, and combinations of the contexts used by defaults.
        fn reference(map: &Keymap, sequence: &str, context: &HashMap<String, Value>) -> Resolution {
            for b in map.bindings.iter().rev() {
                if b.when
                    .as_ref()
                    .is_some_and(|w| !evaluate(w, context).unwrap_or(false))
                {
                    continue;
                }
                if b.key == sequence {
                    return Resolution::Command(b.command.clone(), b.args.clone());
                }
                if b.key.starts_with(&format!("{sequence} ")) {
                    return Resolution::Chord;
                }
            }
            Resolution::None
        }
        for profile in [Profile::Linux, Profile::Windows, Profile::Macos] {
            let mut map = Keymap::new(profile);
            for (key, when, args) in [
                ("ctrl+k", "inputFocus", Some(Value::Null)),
                (
                    "ctrl+k ctrl+x",
                    "editorTextFocus",
                    Some(serde_json::json!([1, 2])),
                ),
                ("x", "!editorTextFocus", None),
            ] {
                map.bindings.push(Binding {
                    key: key.into(),
                    command: "fixture.command".into(),
                    when: Some(when.into()),
                    args,
                });
            }
            let mut sequences = vec!["".into(), "x".into(), "unbound".into(), "ctrl".into()];
            for binding in &map.bindings {
                sequences.push(binding.key.clone());
                if let Some((prefix, _)) = binding.key.split_once(' ') {
                    sequences.push(prefix.into());
                }
            }
            for bits in 0..32 {
                let context = [
                    "editorTextFocus",
                    "inputFocus",
                    "filesExplorerFocus",
                    "terminalFocus",
                    "editorHasSelection",
                ]
                .into_iter()
                .enumerate()
                .map(|(i, key)| (key.into(), Value::Bool(bits & (1 << i) != 0)))
                .collect();
                for sequence in &sequences {
                    assert_eq!(
                        map.resolve(sequence, &context),
                        reference(&map, sequence, &context),
                        "{profile:?} {sequence} {bits}"
                    );
                }
            }
        }
    }
    #[test]
    fn defaults_chords_and_platforms() {
        let ctx = HashMap::from([("editorTextFocus".into(), Value::Bool(true))]);
        for profile in [Profile::Linux, Profile::Windows, Profile::Macos] {
            let m = Keymap::new(profile);
            let p = profile.primary();
            assert!(
                matches!(m.resolve(&format!("{p}+s"),&ctx),Resolution::Command(c,_) if c=="workbench.action.files.save")
            );
            assert!(matches!(
                m.resolve(&format!("{p}+k"), &ctx),
                Resolution::Chord
            ));
            assert!(
                matches!(m.resolve(&format!("{p}+k {p}+c"),&ctx),Resolution::Command(c,_) if c=="editor.action.addCommentLine")
            );
        }
    }
    #[test]
    fn contexts_and_import_order() {
        let ctx = HashMap::from([
            ("editorTextFocus".into(), Value::Bool(true)),
            ("editorLangId".into(), Value::String("rust".into())),
        ]);
        assert!(
            evaluate(
                "editorTextFocus && (!inputFocus || editorLangId == 'rust')",
                &ctx
            )
            .unwrap()
        );
        assert!(evaluate("editorLangId =~ /rust/", &ctx).is_err());
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("keys.json");
        std::fs::write(
            &p,
            r#"[ // comments and trailing commas
            {"key":"ctrl+s","command":"-workbench.action.files.save"},
            {"key":"ctrl+s","command":"type","args":{"text":"yes"},"when":"editorTextFocus"},
        ]"#,
        )
        .unwrap();
        let mut map = Keymap::new(Profile::Linux);
        map.load(&p).unwrap();
        assert!(matches!(map.resolve("ctrl+s",&ctx),Resolution::Command(c,_) if c=="type"));
    }
    #[test]
    fn when_expression_limits_bound_recursive_and_flat_parsing() {
        let context = HashMap::new();
        for nested in [
            format!("{}true{}", "(".repeat(64), ")".repeat(64)),
            format!("{}true", "!".repeat(64)),
            format!("{}{}true{}", "!".repeat(32), "(".repeat(32), ")".repeat(32)),
        ] {
            assert!(evaluate(&nested, &context).unwrap());
        }
        for nested in [
            format!("{}true{}", "(".repeat(65), ")".repeat(65)),
            format!("{}true", "!".repeat(65)),
            format!("{}{}true{}", "!".repeat(33), "(".repeat(32), ")".repeat(32)),
        ] {
            assert!(
                evaluate(&nested, &context)
                    .unwrap_err()
                    .to_string()
                    .contains("64 nested")
            );
        }
        assert!(evaluate(&vec!["true"; 512].join(" && "), &context).unwrap());
        assert!(
            evaluate(&vec!["true"; 513].join(" && "), &context)
                .unwrap_err()
                .to_string()
                .contains("1024 tokens")
        );
        assert!(!evaluate(&"x".repeat(MAX_WHEN_BYTES), &context).unwrap());
        assert!(
            evaluate(&"x".repeat(MAX_WHEN_BYTES + 1), &context)
                .unwrap_err()
                .to_string()
                .contains("8 KiB")
        );
    }
    #[test]
    fn deeply_nested_user_and_extension_rules_preserve_the_working_keymap() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("keybindings.json");
        let context = HashMap::new();
        let mut map = Keymap::new(Profile::Linux);
        std::fs::write(&path, r#"[{"key":"f9","command":"user.command"}]"#).unwrap();
        map.load(&path).unwrap();
        map.set_extension_bindings(serde_json::json!([
            {"key":"f8","command":"extension.command"}
        ]))
        .unwrap();
        let before = serde_json::to_value(&map.bindings).unwrap();
        let invalid = serde_json::json!([
            {"key":"f9","command":"-user.command"},
            {"key":"f8","command":"invalid.command","when":format!("{}true", "!".repeat(65))}
        ]);
        std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(format!("{:#}", map.load(&path).unwrap_err()).contains("64 nested"));
        assert_eq!(serde_json::to_value(&map.bindings).unwrap(), before);
        let extension = serde_json::json!([
            {"key":"f8","command":"invalid.command","when":format!("{}true", "!".repeat(65))}
        ]);
        assert!(
            format!("{:#}", map.set_extension_bindings(extension).unwrap_err())
                .contains("64 nested")
        );
        assert_eq!(serde_json::to_value(&map.bindings).unwrap(), before);
        assert!(
            matches!(map.resolve("f9", &context), Resolution::Command(id, _) if id == "user.command")
        );
        assert!(
            matches!(map.resolve("f8", &context), Resolution::Command(id, _) if id == "extension.command")
        );
    }
    #[test]
    fn oversized_or_invalid_imports_preserve_existing_user_rules() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("keybindings.json");
        let context = HashMap::from([("editorTextFocus".into(), Value::Bool(true))]);
        let mut map = Keymap::new(Profile::Linux);
        let rule = r#"[{"key":"f9","command":"user.command"}]"#;
        let mut at_limit = rule.to_owned();
        at_limit.extend(std::iter::repeat_n(' ', 1024 * 1024 - rule.len()));
        std::fs::write(&path, &at_limit).unwrap();
        assert_eq!(map.load(&path).unwrap(), 1);
        let before = serde_json::to_value(&map.bindings).unwrap();
        at_limit.push(' ');
        std::fs::write(&path, at_limit).unwrap();
        assert!(map.load(&path).unwrap_err().to_string().contains("1 MiB"));
        assert_eq!(serde_json::to_value(&map.bindings).unwrap(), before);
        std::fs::write(&path, b"[\xff]").unwrap();
        assert!(map.load(&path).is_err());
        assert_eq!(serde_json::to_value(&map.bindings).unwrap(), before);
        assert!(
            matches!(map.resolve("f9", &context), Resolution::Command(id, _) if id == "user.command")
        );
    }
    #[test]
    fn extension_defaults_preserve_platform_keys_user_overrides_and_removals() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("keybindings.json");
        let context = HashMap::from([("editorTextFocus".into(), Value::Bool(true))]);
        let contribution = serde_json::json!([{"key":"f9", "mac":"cmd+alt+s", "command":"extension.sort", "when":"editorTextFocus"}]);
        let mut map = Keymap::new(Profile::Linux);
        map.set_extension_bindings(contribution.clone()).unwrap();
        assert!(
            matches!(map.resolve("f9", &context), Resolution::Command(id,_) if id == "extension.sort")
        );
        map.clear_extension_bindings();
        assert!(
            matches!(map.resolve("f9", &context), Resolution::Command(id,_) if id == "editor.debug.action.toggleBreakpoint")
        );
        std::fs::write(&path, r#"[{"key":"f9","command":"-extension.sort"},{"key":"ctrl+k ctrl+b","command":"extension.sort","when":"editorTextFocus"}]"#).unwrap();
        map.load(&path).unwrap();
        map.set_extension_bindings(contribution.clone()).unwrap();
        assert!(
            matches!(map.resolve("f9", &context), Resolution::Command(id,_) if id == "editor.debug.action.toggleBreakpoint")
        );
        assert!(
            matches!(map.resolve("ctrl+k ctrl+b", &context), Resolution::Command(id,_) if id == "extension.sort")
        );
        std::fs::write(&path, r#"[{"key":"f9","command":"user.override"}]"#).unwrap();
        map.load(&path).unwrap();
        assert!(
            matches!(map.resolve("f9", &context), Resolution::Command(id,_) if id == "user.override")
        );
        let mut mac = Keymap::new(Profile::Macos);
        mac.set_extension_bindings(contribution).unwrap();
        assert!(
            matches!(mac.resolve("cmd+alt+s", &context), Resolution::Command(id,_) if id == "extension.sort")
        );
        assert!(
            mac.set_extension_bindings(serde_json::json!([{"key":"", "command":"invalid"}]))
                .is_err()
        );
        assert!(
            matches!(mac.resolve("cmd+alt+s", &context), Resolution::Command(id,_) if id == "extension.sort")
        );
    }
    #[test]
    fn binding_export_preserves_argument_presence() {
        for source in [
            serde_json::json!({"key":"f9", "command":"example"}),
            serde_json::json!({"key":"f9", "command":"example", "args":null}),
            serde_json::json!({"key":"f9", "command":"example", "when":"editorTextFocus",
                "args":{"nested":[1, true, "text"]}}),
        ] {
            let binding: Binding = serde_json::from_value(source.clone()).unwrap();
            assert_eq!(serde_json::to_value(binding).unwrap(), source);
        }
    }
    #[cfg(unix)]
    #[test]
    fn imported_nonregular_binding_files_reject_without_blocking_or_mutation() {
        let root = tempfile::tempdir().unwrap();
        let pipe = root.path().join("keys.json");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&pipe)
                .status()
                .unwrap()
                .success()
        );
        for path in [root.path().to_owned(), pipe] {
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            std::thread::spawn(move || {
                let mut map = Keymap::new(Profile::Linux);
                let result = map.load_imported(&path);
                let _ = sender.send((result, map));
            });
            let (result, map) = receiver
                .recv_timeout(std::time::Duration::from_secs(3))
                .expect("Imported bindings must reject nonregular files without blocking");
            assert!(result.unwrap_err().to_string().contains("regular file"));
            let context = HashMap::from([("editorTextFocus".into(), Value::Bool(true))]);
            assert!(
                matches!(map.resolve("ctrl+s", &context), Resolution::Command(id, _) if id == "workbench.action.files.save")
            );
        }
    }
    #[test]
    fn failed_imported_bindings_preserve_defaults_and_source_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("keys.json");
        let context = HashMap::from([("editorTextFocus".into(), Value::Bool(true))]);
        for bytes in [
            b"[{ broken".to_vec(),
            vec![b' '; 1024 * 1024 + 1],
            format!("{}{}", "[".repeat(65), "]".repeat(65)).into_bytes(),
        ] {
            std::fs::write(&path, &bytes).unwrap();
            let mut map = Keymap::new(Profile::Linux);
            assert!(map.load_imported(&path).is_err());
            assert!(
                matches!(map.resolve("ctrl+s", &context), Resolution::Command(id, _) if id == "workbench.action.files.save")
            );
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
    #[test]
    fn displayed_shortcuts_honor_removal_inactive_overrides_and_shadowing() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("keys.json");
        let command = "workbench.action.openRecent";
        let welcome = HashMap::from([("editorTextFocus".into(), Value::Bool(false))]);
        for (rules, expected) in [
            (
                r#"[{"key":"f8","command":"workbench.action.openRecent","when":"editorTextFocus"}]"#,
                "ctrl+r",
            ),
            (
                r#"[{"key":"ctrl+r","command":"workbench.action.files.newUntitledFile"}]"#,
                "",
            ),
            (
                r#"[{"key":"ctrl+r","command":"-workbench.action.openRecent"}]"#,
                "",
            ),
            (
                r#"[{"key":"f8","command":"workbench.action.openRecent"}]"#,
                "f8",
            ),
        ] {
            std::fs::write(&path, rules).unwrap();
            let mut map = Keymap::new(Profile::Linux);
            map.load(&path).unwrap();
            assert_eq!(map.shortcut_in_context(command, &welcome), expected);
        }
    }
    #[test]
    fn recent_and_reopen_shortcuts_match_all_platform_profiles_without_editor_context() {
        for profile in [Profile::Linux, Profile::Windows, Profile::Macos] {
            let map = Keymap::new(profile);
            let platform = match profile {
                Profile::Linux => "linux",
                Profile::Windows => "win32",
                Profile::Macos => "darwin",
            };
            let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/vscode-reference/baselines/1.95.0/{platform}/inventory.json"
            ));
            let inventory: Value =
                serde_json::from_slice(&std::fs::read(fixture).unwrap()).unwrap();
            for command in [
                "workbench.action.openRecent",
                "workbench.action.reopenClosedEditor",
            ] {
                let reference = inventory["bindings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|binding| binding["command"] == command)
                    .unwrap();
                assert!(reference.get("when").is_none());
                assert_eq!(
                    map.shortcut(command),
                    normalize_sequence(reference["key"].as_str().unwrap())
                );
            }
            let context = HashMap::new();
            assert!(
                matches!(map.resolve("ctrl+r", &context), Resolution::Command(id, _) if id == "workbench.action.openRecent")
            );
            let shortcut = format!("{}+shift+t", profile.primary());
            assert!(
                matches!(map.resolve(&shortcut, &context), Resolution::Command(id, _) if id == "workbench.action.reopenClosedEditor")
            );
            for id in [
                "workbench.action.openRecent",
                "workbench.action.reopenClosedEditor",
            ] {
                assert!(
                    map.bindings
                        .iter()
                        .filter(|binding| binding.command == id)
                        .all(|binding| binding.when.is_none())
                );
            }
        }
    }
    #[test]
    fn symbol_shortcuts_match_pinned_platform_bindings_and_contexts() {
        for (profile, platform) in [
            (Profile::Linux, "linux"),
            (Profile::Windows, "win32"),
            (Profile::Macos, "darwin"),
        ] {
            let map = Keymap::new(profile);
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/vscode-reference/baselines/1.95.0/{platform}/inventory.json"
            ));
            let inventory: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            for command in [
                "workbench.action.gotoSymbol",
                "workbench.action.showAllSymbols",
            ] {
                let reference = inventory["bindings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|binding| binding["command"] == command)
                    .unwrap();
                let binding = map
                    .bindings
                    .iter()
                    .find(|binding| binding.command == command)
                    .unwrap();
                assert_eq!(
                    binding.key,
                    normalize_sequence(reference["key"].as_str().unwrap())
                );
                assert_eq!(binding.when.as_deref(), reference["when"].as_str());
                assert!(
                    matches!(map.resolve(&binding.key, &HashMap::new()), Resolution::Command(id, _) if id == command)
                );
            }
        }
    }
    #[test]
    fn code_action_shortcuts_match_pinned_platform_keys_and_provider_contexts() {
        for (profile, platform) in [
            (Profile::Linux, "linux"),
            (Profile::Windows, "win32"),
            (Profile::Macos, "darwin"),
        ] {
            let map = Keymap::new(profile);
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/vscode-reference/baselines/1.95.0/{platform}/inventory.json"
            ));
            let inventory: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            let enabled = HashMap::from([
                ("editorHasCodeActionsProvider".into(), Value::Bool(true)),
                ("textInputFocus".into(), Value::Bool(true)),
                ("editorReadonly".into(), Value::Bool(false)),
            ]);
            for command in ["editor.action.quickFix", "editor.action.refactor"] {
                let reference = inventory["bindings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|binding| binding["command"] == command)
                    .unwrap();
                let binding = map
                    .bindings
                    .iter()
                    .find(|binding| binding.command == command)
                    .unwrap();
                assert_eq!(
                    binding.key,
                    normalize_sequence(reference["key"].as_str().unwrap())
                );
                assert_eq!(binding.when.as_deref(), reference["when"].as_str());
                assert!(
                    matches!(map.resolve(&binding.key,&enabled),Resolution::Command(id,_) if id==command)
                );
                let mut disabled = enabled.clone();
                disabled.insert("editorHasCodeActionsProvider".into(), Value::Bool(false));
                assert!(
                    !matches!(map.resolve(&binding.key,&disabled),Resolution::Command(id,_) if id==command)
                );
            }
        }
    }
    #[test]
    fn signature_shortcuts_match_pinned_platform_bindings_and_contexts() {
        for (profile, platform) in [
            (Profile::Linux, "linux"),
            (Profile::Windows, "win32"),
            (Profile::Macos, "darwin"),
        ] {
            let map = Keymap::new(profile);
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/vscode-reference/baselines/1.95.0/{platform}/inventory.json"
            ));
            let reference: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            for binding in reference["bindings"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|b| {
                    matches!(
                        b["command"].as_str(),
                        Some("editor.action.triggerParameterHints" | "closeParameterHints")
                    )
                })
            {
                assert!(map.bindings.iter().any(|native| native.command
                    == binding["command"].as_str().unwrap()
                    && native.key == normalize_sequence(binding["key"].as_str().unwrap())
                    && native.when.as_deref() == binding["when"].as_str()));
            }
            let key = format!("{}+shift+space", profile.primary());
            let context = HashMap::from([
                ("editorTextFocus".into(), Value::Bool(true)),
                ("editorHasSignatureHelpProvider".into(), Value::Bool(true)),
            ]);
            assert!(
                matches!(map.resolve(&key,&context),Resolution::Command(id,_) if id=="editor.action.triggerParameterHints")
            );
            assert!(!matches!(
                map.resolve(&key, &HashMap::new()),
                Resolution::Command(_, _)
            ));
        }
    }
    #[test]
    fn terminal_key_normalization() {
        assert_eq!(
            token(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::CONTROL)),
            "ctrl+shift+p"
        );
        assert_eq!(normalize_sequence("shift+ctrl+p"), "ctrl+shift+p");
    }
    #[test]
    fn multiple_extension_bindings_have_stable_precedence_and_independent_validation() {
        let mut map = Keymap::new(Profile::Linux);
        let context = HashMap::from([("editorTextFocus".into(), Value::Bool(true))]);
        let binding = |command| serde_json::json!([{ "key": "f9", "mac":"cmd+alt+s", "command": command, "when": "editorTextFocus" }]);
        let errors = map
            .set_extension_binding_sets(vec![
                ("test.z".into(), binding("z.run")),
                (
                    "test.invalid".into(),
                    serde_json::json!([{ "key":"", "command":"broken" }]),
                ),
                ("test.a".into(), binding("a.run")),
            ])
            .unwrap();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("test.invalid"));
        assert_eq!(
            map.resolve("f9", &context),
            Resolution::Command("z.run".into(), None)
        );
        map.set_extension_binding_sets(vec![("test.a".into(), binding("a.run"))])
            .unwrap();
        assert_eq!(
            map.resolve("f9", &context),
            Resolution::Command("a.run".into(), None)
        );
        let directory = tempfile::tempdir().unwrap();
        let user = directory.path().join("keybindings.json");
        std::fs::write(&user, r#"[{"key":"f9","command":"-z.run"}]"#).unwrap();
        map.load(&user).unwrap();
        map.set_extension_binding_sets(vec![
            ("test.z".into(), binding("z.run")),
            ("test.a".into(), binding("a.run")),
        ])
        .unwrap();
        assert_eq!(
            map.resolve("f9", &context),
            Resolution::Command("a.run".into(), None)
        );
        let mut mac = Keymap::new(Profile::Macos);
        mac.set_extension_binding_sets(vec![
            ("test.a".into(), binding("a.run")),
            ("test.z".into(), binding("z.run")),
        ])
        .unwrap();
        assert_eq!(
            mac.resolve("cmd+alt+s", &context),
            Resolution::Command("z.run".into(), None)
        );
    }
}
