use anyhow::{Context, Result, bail};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, path::Path};

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

#[derive(Debug, Clone, Deserialize)]
pub struct Binding {
    pub key: String,
    pub command: String,
    #[serde(default)]
    pub when: Option<String>,
    #[serde(default)]
    pub args: Value,
}

pub struct Keymap {
    pub bindings: Vec<Binding>,
    pub profile: Profile,
}
pub enum Resolution {
    Command(String, Value),
    Chord,
    None,
}

impl Keymap {
    pub fn new(profile: Profile) -> Self {
        let mut map = Self {
            bindings: Vec::new(),
            profile,
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
        map
    }
    fn add(&mut self, key: &str, command: &str, when: Option<&str>) {
        self.bindings.push(Binding {
            key: normalize_sequence(key),
            command: command.into(),
            when: when.map(str::to_owned),
            args: Value::Null,
        });
    }
    pub fn load(&mut self, path: &Path) -> Result<usize> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("Cannot read {}", path.display()))?;
        let entries: Vec<Binding> = json5::from_str(&raw).context("Invalid keybindings.json")?;
        // Validate the complete import before changing the working map.
        for b in &entries {
            if b.key.trim().is_empty() {
                bail!("Empty keybinding");
            }
            if let Some(when) = &b.when {
                evaluate(when, &HashMap::new())
                    .with_context(|| format!("Unsupported when expression: {when}"))?;
            }
        }
        let count = entries.len();
        for mut b in entries {
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
        Ok(count)
    }
    pub fn resolve(&self, sequence: &str, context: &HashMap<String, Value>) -> Resolution {
        for b in self.bindings.iter().rev() {
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
    pub fn shortcut(&self, command: &str) -> String {
        self.bindings
            .iter()
            .rev()
            .find(|b| b.command == command)
            .map(|b| b.key.clone())
            .unwrap_or_default()
    }
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
fn lex(s: &str) -> Result<Vec<Tok>> {
    let chars: Vec<_> = s.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
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
    fn terminal_key_normalization() {
        assert_eq!(
            token(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::CONTROL)),
            "ctrl+shift+p"
        );
        assert_eq!(normalize_sequence("shift+ctrl+p"), "ctrl+shift+p");
    }
}
