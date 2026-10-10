//! A validated, explicit subset of VS Code settings and scope precedence.
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum LineNumbers {
    #[default]
    On,
    Off,
    Relative,
    Interval,
}
#[derive(Clone, Default)]
pub struct Settings {
    layers: Arc<Vec<Map<String, Value>>>,
    // Map equality ignores insertion order, but language-block ordering affects
    // precedence. This key also lets reload comparison avoid walking JSON trees.
    serialized: Arc<str>,
    workspace_layer: Option<usize>,
    pub warnings: Vec<String>,
}
impl PartialEq for Settings {
    fn eq(&self, other: &Self) -> bool {
        self.serialized == other.serialized
            && self.warnings == other.warnings
            && self.workspace_layer == other.workspace_layer
    }
}
const SUPPORTED: &[&str] = &[
    "editor.tabSize",
    "editor.insertSpaces",
    "editor.lineNumbers",
    "editor.quickSuggestions",
    "editor.quickSuggestionsDelay",
    "editor.suggestOnTriggerCharacters",
    "editor.acceptSuggestionOnEnter",
    "editor.parameterHints.enabled",
    "editor.parameterHints.cycle",
    "editor.autoClosingBrackets",
    "editor.autoClosingQuotes",
    "editor.autoClosingDelete",
    "editor.autoClosingOvertype",
    "editor.autoSurround",
    "editor.autoIndent",
    "workbench.colorTheme",
    "vscli.languageServer.enabled",
    "vscli.languageServer.program",
    "vscli.languageServer.args",
    "vscli.languageServer.allowWorkspaceConfiguration",
];
fn read(path: &Path) -> Result<Map<String, Value>> {
    match std::fs::metadata(path) {
        Ok(metadata) if !metadata.is_file() => bail!("Settings must be a regular file"),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => return Err(error).with_context(|| format!("Cannot read {}", path.display())),
    }
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(e).with_context(|| format!("Cannot read {}", path.display())),
    };
    let mut text = String::new();
    file.take(1024 * 1024 + 1).read_to_string(&mut text)?;
    if text.len() > 1024 * 1024 {
        bail!("Settings file exceeds 1 MiB");
    }
    let value: Value = crate::jsonc::parse(&text)
        .with_context(|| format!("Invalid settings in {}", path.display()))?;
    value
        .as_object()
        .cloned()
        .context("Settings must be a JSON object")
}
impl Settings {
    pub fn load(paths: &[PathBuf]) -> Result<Self> {
        Self::load_scoped(paths, None)
    }
    /// App supplies the workspace file last, independently of file names used
    /// for an explicitly selected user settings file.
    pub(crate) fn load_editor(paths: &[PathBuf]) -> Result<Self> {
        Self::load_scoped(paths, paths.len().checked_sub(1))
    }
    fn load_scoped(paths: &[PathBuf], workspace_layer: Option<usize>) -> Result<Self> {
        let mut result = Self {
            workspace_layer,
            ..Self::default()
        };
        for path in paths {
            let layer = read(path)?;
            result.validate(&layer, &path.display().to_string());
            Arc::make_mut(&mut result.layers).push(layer);
        }
        result.serialized = serde_json::to_string(result.layers.as_ref())?.into();
        Ok(result)
    }
    pub(crate) fn from_values(values: Map<String, Value>, source: &str) -> Result<Self> {
        let mut result = Self::default();
        result.validate(&values, source);
        result.layers = Arc::new(vec![values]);
        result.serialized = serde_json::to_string(result.layers.as_ref())?.into();
        Ok(result)
    }
    pub fn color_theme(&self) -> Option<&str> {
        self.layers
            .iter()
            .rev()
            .find_map(|layer| layer.get("workbench.colorTheme").and_then(Value::as_str))
    }
    pub fn extension_layers(&self) -> &Arc<Vec<Map<String, Value>>> {
        &self.layers
    }
    fn validate(&mut self, values: &Map<String, Value>, source: &str) {
        for (key, value) in values {
            if key.starts_with('[') && key.ends_with(']') {
                if let Some(object) = value.as_object() {
                    self.validate(object, &format!("{source} {key}"));
                } else {
                    self.warnings
                        .push(format!("{source}: {key} must be an object"));
                }
                continue;
            }
            if !SUPPORTED.contains(&key.as_str()) {
                self.warnings.push(format!(
                    "{source}: {key} is not a native setting; extensions may read it"
                ));
            } else if !valid(key, value) {
                self.warnings
                    .push(format!("{source}: invalid value for {key}"));
            } else if key == "editor.autoIndent"
                && matches!(value.as_str(), Some("advanced" | "full"))
            {
                self.warnings.push(format!(
                    "{source}: {key} supports bracket indentation; advanced/full language rules remain incomplete"
                ));
            }
        }
    }
    fn value(&self, key: &str, language: &str) -> Option<&Value> {
        self.value_scoped(key, language, self.layers.len())
    }
    fn value_scoped(&self, key: &str, language: &str, limit: usize) -> Option<&Value> {
        let mut result = None;
        for layer in self.layers.iter().take(limit) {
            if let Some(value) = layer.get(key).filter(|v| valid(key, v)) {
                result = Some(value);
            }
        }
        // Merge equal identifier groups in their first-seen position, then apply
        // single-language groups last, as in the pinned configuration model.
        let mut groups: Vec<(Vec<&str>, Option<&Value>)> = Vec::new();
        for layer in self.layers.iter().take(limit) {
            for (selector, value) in layer {
                let Some(inner) = selector.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
                else {
                    continue;
                };
                let mut ids = Vec::new();
                let mut valid_selector = true;
                for id in inner.split("][") {
                    if id.is_empty() || id.contains(['[', ']']) {
                        valid_selector = false;
                        break;
                    }
                    let id = id.trim();
                    if !id.is_empty() && !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                if !valid_selector || !ids.contains(&language) {
                    continue;
                }
                let found = value.get(key).filter(|v| valid(key, v));
                if let Some((_, previous)) =
                    groups.iter_mut().find(|(existing, _)| *existing == ids)
                {
                    if found.is_some() {
                        *previous = found;
                    }
                } else {
                    groups.push((ids, found));
                }
            }
        }
        for single in [false, true] {
            for (ids, value) in &groups {
                if (ids.len() == 1) == single && value.is_some() {
                    result = *value;
                }
            }
        }
        result
    }
    pub fn language_server(&self, language: &str) -> LanguageServer {
        let user_layers = self.workspace_layer.unwrap_or(self.layers.len());
        let allow = self
            .value_scoped(
                "vscli.languageServer.allowWorkspaceConfiguration",
                language,
                user_layers,
            )
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let limit = if allow {
            self.layers.len()
        } else {
            user_layers
        };
        let program = self
            .value_scoped("vscli.languageServer.program", language, limit)
            .and_then(Value::as_str)
            .map(str::to_owned);
        let args = self
            .value_scoped("vscli.languageServer.args", language, limit)
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect()
            })
            .unwrap_or_default();
        let blocked_workspace = !allow
            && ["vscli.languageServer.program", "vscli.languageServer.args"]
                .iter()
                .any(|key| {
                    self.value(key, language) != self.value_scoped(key, language, user_layers)
                });
        LanguageServer {
            enabled: self
                .value("vscli.languageServer.enabled", language)
                .and_then(Value::as_bool)
                .unwrap_or(true),
            program,
            args,
            blocked_workspace,
        }
    }

    pub fn suggestions(&self, language: &str) -> Suggestions {
        Suggestions {
            quick: self
                .value("editor.quickSuggestions", language)
                .and_then(Value::as_bool)
                .unwrap_or(true),
            delay_ms: self
                .value("editor.quickSuggestionsDelay", language)
                .and_then(Value::as_u64)
                .unwrap_or(120),
            triggers: self
                .value("editor.suggestOnTriggerCharacters", language)
                .and_then(Value::as_bool)
                .unwrap_or(true),
            enter: self
                .value("editor.acceptSuggestionOnEnter", language)
                .and_then(Value::as_str)
                != Some("off"),
        }
    }

    pub fn parameter_hints(&self, language: &str) -> crate::signature::Options {
        crate::signature::Options {
            enabled: self
                .value("editor.parameterHints.enabled", language)
                .and_then(Value::as_bool)
                .unwrap_or(true),
            cycle: self
                .value("editor.parameterHints.cycle", language)
                .and_then(Value::as_bool)
                .unwrap_or(true),
        }
    }

    pub fn typing(&self, language: &str) -> crate::editing_profile::TypingOptions {
        use crate::editing_profile::{AutoClosing, AutoIndent, PairHandling, Surround};
        let closing = |key| match self.value(key, language).and_then(Value::as_str) {
            Some("always") => AutoClosing::Always,
            Some("beforeWhitespace") => AutoClosing::BeforeWhitespace,
            Some("never") => AutoClosing::Never,
            _ => AutoClosing::LanguageDefined,
        };
        let handling = |key| match self.value(key, language).and_then(Value::as_str) {
            Some("always") => PairHandling::Always,
            Some("never") => PairHandling::Never,
            _ => PairHandling::Auto,
        };
        crate::editing_profile::TypingOptions {
            profile: crate::editing_profile::ProfileId::for_language(language),
            brackets: closing("editor.autoClosingBrackets"),
            quotes: closing("editor.autoClosingQuotes"),
            delete: handling("editor.autoClosingDelete"),
            overtype: handling("editor.autoClosingOvertype"),
            surround: match self
                .value("editor.autoSurround", language)
                .and_then(Value::as_str)
            {
                Some("quotes") => Surround::Quotes,
                Some("brackets") => Surround::Brackets,
                Some("never") => Surround::Never,
                _ => Surround::LanguageDefined,
            },
            indent: match self
                .value("editor.autoIndent", language)
                .and_then(Value::as_str)
            {
                Some("none") => AutoIndent::None,
                Some("keep") => AutoIndent::Keep,
                _ => AutoIndent::Brackets,
            },
        }
    }

    pub fn apply(&self, doc: &mut crate::document::Document) {
        let language = doc
            .path
            .as_deref()
            .map_or("plaintext", crate::languages::language);
        let tab_size = self
            .value("editor.tabSize", language)
            .and_then(Value::as_u64)
            .unwrap_or(4) as usize;
        let insert_spaces = self
            .value("editor.insertSpaces", language)
            .and_then(Value::as_bool)
            .unwrap_or(true);
        doc.set_indentation(tab_size, insert_spaces);
        doc.line_numbers = match self
            .value("editor.lineNumbers", language)
            .and_then(Value::as_str)
        {
            Some("off") => LineNumbers::Off,
            Some("relative") => LineNumbers::Relative,
            Some("interval") => LineNumbers::Interval,
            _ => LineNumbers::On,
        };
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Suggestions {
    pub quick: bool,
    pub delay_ms: u64,
    pub triggers: bool,
    pub enter: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageServer {
    pub enabled: bool,
    pub program: Option<String>,
    pub args: Vec<String>,
    pub blocked_workspace: bool,
}
fn valid(key: &str, value: &Value) -> bool {
    match key {
        "workbench.colorTheme" => value.is_string(),
        "vscli.languageServer.enabled" | "vscli.languageServer.allowWorkspaceConfiguration" => {
            value.is_boolean()
        }
        "vscli.languageServer.program" => value
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.len() <= 4096),
        "vscli.languageServer.args" => value.as_array().is_some_and(|args| {
            args.len() <= 32
                && args
                    .iter()
                    .all(|a| a.as_str().is_some_and(|s| s.len() <= 4096))
                && args
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::len)
                    .sum::<usize>()
                    <= 16384
        }),
        "editor.tabSize" => value.as_u64().is_some_and(|n| (1..=16).contains(&n)),
        "editor.insertSpaces"
        | "editor.quickSuggestions"
        | "editor.suggestOnTriggerCharacters"
        | "editor.parameterHints.enabled"
        | "editor.parameterHints.cycle" => value.is_boolean(),
        "editor.autoClosingBrackets" | "editor.autoClosingQuotes" => matches!(
            value.as_str(),
            Some("always" | "languageDefined" | "beforeWhitespace" | "never")
        ),
        "editor.autoClosingDelete" | "editor.autoClosingOvertype" => {
            matches!(value.as_str(), Some("always" | "auto" | "never"))
        }
        "editor.autoSurround" => {
            matches!(
                value.as_str(),
                Some("languageDefined" | "quotes" | "brackets" | "never")
            )
        }
        "editor.autoIndent" => {
            matches!(
                value.as_str(),
                Some("none" | "keep" | "brackets" | "advanced" | "full")
            )
        }
        "editor.quickSuggestionsDelay" => value.as_u64().is_some_and(|n| n <= 2000),
        "editor.acceptSuggestionOnEnter" => matches!(value.as_str(), Some("on" | "off")),
        "editor.lineNumbers" => {
            matches!(value.as_str(), Some("on" | "off" | "relative" | "interval"))
        }
        _ => false,
    }
}
pub struct Loader {
    paths: Vec<PathBuf>,
    last: Instant,
    pending: Option<Receiver<Result<Settings, String>>>,
}
impl Loader {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self {
            paths,
            last: Instant::now(),
            pending: None,
        }
    }
    pub fn poll(&mut self) -> Option<Result<Settings, String>> {
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(result) => {
                    self.pending = None;
                    return Some(result);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    return Some(Err("Settings worker stopped".into()));
                }
                _ => {}
            }
        } else if self.last.elapsed() >= Duration::from_secs(2) {
            let paths = self.paths.clone();
            let (sender, receiver) = mpsc::sync_channel(1);
            std::thread::spawn(move || {
                let _ = sender.send(Settings::load_editor(&paths).map_err(|e| format!("{e:#}")));
            });
            self.pending = Some(receiver);
            self.last = Instant::now();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn suggestions_use_language_precedence_and_reject_unimplemented_or_unbounded_options() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user.json");
        let workspace = root.path().join("workspace.json");
        std::fs::write(&user, r#"{"editor.quickSuggestionsDelay":50,"[rust]":{"editor.quickSuggestions":false,"editor.acceptSuggestionOnEnter":"off"}}"#).unwrap();
        std::fs::write(&workspace, r#"{"editor.quickSuggestionsDelay":2001,"editor.quickSuggestions":{"comments":"on"},"editor.acceptSuggestionOnEnter":"smart","[rust]":{"editor.suggestOnTriggerCharacters":false}}"#).unwrap();
        let settings = Settings::load(&[user, workspace]).unwrap();
        assert_eq!(settings.warnings.len(), 3);
        assert_eq!(
            settings.suggestions("rust"),
            Suggestions {
                quick: false,
                delay_ms: 50,
                triggers: false,
                enter: false
            }
        );
        assert_eq!(
            settings.suggestions("python"),
            Suggestions {
                quick: true,
                delay_ms: 50,
                triggers: true,
                enter: true
            }
        );
    }
    #[test]
    fn language_programs_use_user_provenance_and_explicit_workspace_opt_in() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("settings.json");
        let workspace = root.path().join("workspace.json");
        std::fs::write(&user, r#"{"[cpp]":{"vscli.languageServer.program":"user-clangd","vscli.languageServer.args":["--user"]}}"#).unwrap();
        std::fs::write(&workspace, r#"{"vscli.languageServer.allowWorkspaceConfiguration":true,"[cpp]":{"vscli.languageServer.program":"repo-clangd","vscli.languageServer.args":["--repo"],"vscli.languageServer.enabled":false}}"#).unwrap();
        let paths = [user.clone(), workspace.clone()];
        let settings = Settings::load_editor(&paths).unwrap();
        let cpp = settings.language_server("cpp");
        assert_eq!(cpp.program.as_deref(), Some("user-clangd"));
        assert_eq!(cpp.args, ["--user"]);
        assert!(!cpp.enabled);
        assert!(cpp.blocked_workspace);
        assert!(settings.language_server("rust").program.is_none());
        std::fs::write(
            &user,
            r#"{"vscli.languageServer.allowWorkspaceConfiguration":true}"#,
        )
        .unwrap();
        let cpp = Settings::load_editor(&paths)
            .unwrap()
            .language_server("cpp");
        assert_eq!(cpp.program.as_deref(), Some("repo-clangd"));
        assert_eq!(cpp.args, ["--repo"]);
        assert!(!cpp.blocked_workspace);
        // The explicitly supplied first file is user scope regardless of name.
        let cpp = Settings::load_editor(&[workspace, root.path().join("missing")])
            .unwrap()
            .language_server("cpp");
        assert_eq!(cpp.program.as_deref(), Some("repo-clangd"));
    }
    #[test]
    fn malformed_or_oversized_language_settings_retain_valid_lower_layers() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user.json");
        let workspace = root.path().join("workspace.json");
        std::fs::write(&user, r#"{"vscli.languageServer.allowWorkspaceConfiguration":true,"vscli.languageServer.program":"clangd","vscli.languageServer.args":["--valid"]}"#).unwrap();
        std::fs::write(&workspace, serde_json::to_vec(&serde_json::json!({"vscli.languageServer.program":"x".repeat(4097),"vscli.languageServer.args":[false],"vscli.languageServer.enabled":"bad"})).unwrap()).unwrap();
        let settings = Settings::load_editor(&[user, workspace]).unwrap();
        let cpp = settings.language_server("cpp");
        assert_eq!(cpp.program.as_deref(), Some("clangd"));
        assert_eq!(cpp.args, ["--valid"]);
        assert!(cpp.enabled);
        assert_eq!(settings.warnings.len(), 3);
    }
    #[test]
    fn combined_language_order_survives_parsing_scope_merging_and_reload_comparison() {
        let directory = tempfile::tempdir().unwrap();
        let user = directory.path().join("user.json");
        let workspace = directory.path().join("workspace.json");
        std::fs::write(
            &user,
            r#"{
            "[javascript][typescript]": {"editor.tabSize": 2},
            "[javascript][python]": {"editor.tabSize": 3}
        }"#,
        )
        .unwrap();
        std::fs::write(
            &workspace,
            r#"{"[javascript][typescript]": {"editor.tabSize": 4}}"#,
        )
        .unwrap();
        let settings = Settings::load(&[user.clone(), workspace.clone()]).unwrap();
        let mut doc = crate::document::Document::default();
        doc.path = Some("file.js".into());
        settings.apply(&mut doc);
        assert_eq!(doc.tab_size, 3);
        assert!(Arc::ptr_eq(
            settings.extension_layers(),
            settings.clone().extension_layers()
        ));
        std::fs::write(
            &user,
            r#"{
            "[javascript][python]": {"editor.tabSize": 3},
            "[javascript][typescript]": {"editor.tabSize": 2}
        }"#,
        )
        .unwrap();
        let reordered = Settings::load(&[user.clone(), workspace.clone()]).unwrap();
        assert!(settings != reordered);
        reordered.apply(&mut doc);
        assert_eq!(doc.tab_size, 4);
        std::fs::write(
            &user,
            r#"{"[ javascript ][javascript]": {"editor.tabSize": 8}}"#,
        )
        .unwrap();
        Settings::load(&[user, workspace]).unwrap().apply(&mut doc);
        assert_eq!(doc.tab_size, 8);
    }
    #[test]
    fn language_scope_precedence_validation_and_comments() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user.json");
        let workspace = dir.path().join("workspace.json");
        std::fs::write(
            &user,
            r#"{ // user
            "editor.tabSize": 2,
            "[rust]": {"editor.tabSize": 8},
            "[python]": {"editor.insertSpaces": false}
        }"#,
        )
        .unwrap();
        std::fs::write(
            &workspace,
            r#"{
            "editor.tabSize": 3,
            "[rust][python]": {"editor.tabSize": 6},
            "[python]": {"editor.insertSpaces": true},
            "editor.lineNumbers": "relative",
            "files.autoSave": "afterDelay",
        }"#,
        )
        .unwrap();
        let settings = Settings::load(&[user, workspace]).unwrap();
        assert_eq!(settings.warnings.len(), 1);
        let mut doc = crate::document::Document::default();
        doc.path = Some("main.rs".into());
        settings.apply(&mut doc);
        assert_eq!(doc.tab_size, 8);
        assert!(doc.line_numbers == LineNumbers::Relative);
        doc.path = Some("main.py".into());
        settings.apply(&mut doc);
        assert_eq!(doc.tab_size, 6);
        assert!(doc.insert_spaces);
        doc.path = Some("main.txt".into());
        settings.apply(&mut doc);
        assert_eq!(doc.tab_size, 3);
    }
    #[test]
    fn typing_and_parameter_hints_keep_language_scope_and_report_partial_indentation() {
        use crate::editing_profile::{AutoClosing, AutoIndent, PairHandling, ProfileId, Surround};
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user.json");
        let workspace = root.path().join("workspace.json");
        std::fs::write(
            &user,
            r#"{
            "editor.autoClosingBrackets":"never",
            "editor.parameterHints.enabled":false,
            "[cpp]":{"editor.autoClosingBrackets":"always","editor.parameterHints.enabled":true}
        }"#,
        )
        .unwrap();
        std::fs::write(
            &workspace,
            r#"{
            "editor.autoClosingBrackets":"beforeWhitespace",
            "editor.autoClosingDelete":"always",
            "editor.autoClosingOvertype":"never",
            "editor.autoSurround":"quotes",
            "editor.autoIndent":"full",
            "editor.parameterHints.cycle":false,
            "[cpp]":{"editor.autoClosingQuotes":"never"}
        }"#,
        )
        .unwrap();
        let settings = Settings::load(&[user, workspace]).unwrap();
        let cpp = settings.typing("cpp");
        assert_eq!(cpp.profile, ProfileId::Cpp);
        assert_eq!(cpp.brackets, AutoClosing::Always);
        assert_eq!(cpp.quotes, AutoClosing::Never);
        assert_eq!(cpp.delete, PairHandling::Always);
        assert_eq!(cpp.overtype, PairHandling::Never);
        assert_eq!(cpp.surround, Surround::Quotes);
        assert_eq!(cpp.indent, AutoIndent::Brackets);
        assert!(settings.parameter_hints("cpp").enabled);
        assert!(!settings.parameter_hints("cpp").cycle);
        assert!(!settings.parameter_hints("json").enabled);
        assert_eq!(
            settings.typing("json").brackets,
            AutoClosing::BeforeWhitespace
        );
        assert_eq!(settings.warnings.len(), 1);
        assert!(settings.warnings[0].contains("advanced/full"));
    }

    #[test]
    fn invalid_typing_values_do_not_override_valid_layers_or_coerce_hint_flags() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user.json");
        let workspace = root.path().join("workspace.json");
        std::fs::write(
            &user,
            r#"{"editor.autoClosingBrackets":"never","editor.parameterHints.enabled":false}"#,
        )
        .unwrap();
        std::fs::write(
            &workspace,
            r#"{
            "editor.autoClosingBrackets":"sometimes",
            "editor.parameterHints.enabled":"true",
            "[cpp]":{"editor.autoIndent":true}
        }"#,
        )
        .unwrap();
        let settings = Settings::load(&[user, workspace]).unwrap();
        assert_eq!(
            settings.typing("cpp").brackets,
            crate::editing_profile::AutoClosing::Never
        );
        assert!(!settings.parameter_hints("cpp").enabled);
        assert_eq!(settings.warnings.len(), 3);
    }

    #[test]
    fn tab_width_agrees_with_cursor_motion_and_indent_transactions() {
        let mut doc = crate::document::Document::from_text("\t界\n0123456789");
        doc.set_indentation(2, true);
        doc.move_to(2, false);
        assert_eq!(doc.visual_column(), 4);
        assert_eq!(doc.position_at(0, 2), 1);
        doc.vertical(1, false);
        assert_eq!(doc.column(), 4);
        doc.set_indentation(8, false);
        doc.move_to(2, false);
        assert_eq!(doc.visual_column(), 10);
        doc.vertical(1, false);
        assert_eq!(doc.column(), 10);
        doc.transform_lines(false, None);
        assert_eq!(doc.line(1), "\t0123456789");
        doc.undo();
        assert_eq!(doc.line(1), "0123456789");
    }
}
