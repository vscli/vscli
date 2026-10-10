//! A validated, explicit subset of VS Code settings and scope precedence.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BreadcrumbPath {
    #[default]
    On,
    Off,
    Last,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootWriteScope {
    User,
    Workspace,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveFormatting {
    Off,
    File,
    Unavailable(&'static str),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveActionReason {
    Explicit,
    AfterDelay,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveActionFamily {
    FixAll,
    OrganizeImports,
}
impl SaveActionFamily {
    pub fn kind(self) -> &'static str {
        match self {
            Self::FixAll => "source.fixAll",
            Self::OrganizeImports => "source.organizeImports",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveActionPolicy {
    Off,
    Unavailable(String),
    Native(SaveActionPlan),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveActionPlan {
    /// At most two discovery families, after ancestor removal and ordering.
    pub families: Vec<SaveActionFamily>,
    /// Original effective enabled kinds; a root query does not enable siblings.
    pub enabled: Vec<String>,
    /// Only string `never` excludes a subtree. Deprecated false is distinct.
    pub excluded: Vec<String>,
    pub notices: Vec<String>,
}
impl SaveActionPlan {
    pub fn allows(&self, family: SaveActionFamily, kind: &str) -> bool {
        action_kind_valid(kind)
            && self.families.contains(&family)
            && action_kind_contains(family.kind(), kind)
            && self
                .enabled
                .iter()
                .any(|enabled| action_kind_contains(enabled, kind))
            && !self
                .excluded
                .iter()
                .any(|excluded| action_kind_contains(excluded, kind))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Breadcrumbs {
    pub enabled: bool,
    pub file_path: BreadcrumbPath,
    pub symbol_path: BreadcrumbPath,
}
impl Default for Breadcrumbs {
    fn default() -> Self {
        Self {
            enabled: true,
            file_path: BreadcrumbPath::On,
            symbol_path: BreadcrumbPath::On,
        }
    }
}
/// Workbench close policy; language overrides cannot change this policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreventPinnedEditorClose {
    Never,
    #[default]
    KeyboardAndMouse,
    Mouse,
    Keyboard,
}
impl PreventPinnedEditorClose {
    fn parse(value: &Value) -> Option<Self> {
        match value.as_str()? {
            "never" => Some(Self::Never),
            "keyboardAndMouse" => Some(Self::KeyboardAndMouse),
            "mouse" => Some(Self::Mouse),
            "keyboard" => Some(Self::Keyboard),
            _ => None,
        }
    }
    pub fn protects_keyboard(self) -> bool {
        matches!(self, Self::KeyboardAndMouse | Self::Keyboard)
    }
    pub fn protects_mouse(self) -> bool {
        matches!(self, Self::KeyboardAndMouse | Self::Mouse)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorPreview {
    pub enabled: bool,
    pub from_quick_open: bool,
}
impl Default for EditorPreview {
    fn default() -> Self {
        Self {
            enabled: true,
            from_quick_open: false,
        }
    }
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
#[derive(Clone, Copy)]
struct ScopedValue<'a> {
    value: &'a Value,
    layer: usize,
    selector: Option<&'a str>,
}
const ACTION_ENTRIES: usize = 32;
const ACTION_KIND_BYTES: usize = 128;
const ACTION_TEXT_BYTES: usize = 8 * 1024;
const ACTION_GROUPS: usize = 64;
const ACTION_SELECTOR_BYTES: usize = 16 * 1024;

fn action_kind_valid(kind: &str) -> bool {
    !kind.is_empty()
        && kind.len() <= ACTION_KIND_BYTES
        && !kind.contains(['\0', '\r', '\n'])
        && kind.split('.').all(|part| !part.is_empty())
}
fn action_kind_contains(parent: &str, child: &str) -> bool {
    parent == child
        || child
            .strip_prefix(parent)
            .is_some_and(|suffix| suffix.starts_with('.'))
}
/// References into immutable Settings; merge never clones arbitrary JSON values.
#[derive(Default)]
enum ActionSetting<'a> {
    #[default]
    Missing,
    Object(Vec<(&'a str, &'a Value)>),
    Array(&'a [Value]),
    Invalid,
}
impl<'a> ActionSetting<'a> {
    fn merge(&mut self, value: &'a Value) -> std::result::Result<(), &'static str> {
        match value {
            Value::Object(object) => {
                if object.len() > ACTION_ENTRIES {
                    return Err("editor.codeActionsOnSave exceeds 32 entries");
                }
                if object.keys().any(|kind| kind.len() > ACTION_KIND_BYTES)
                    || object.keys().map(String::len).sum::<usize>() > ACTION_TEXT_BYTES
                {
                    return Err("editor.codeActionsOnSave kind text exceeds native bounds");
                }
                if !matches!(self, Self::Object(_)) {
                    *self = Self::Object(Vec::new());
                }
                let Self::Object(entries) = self else {
                    unreachable!()
                };
                for (kind, value) in object {
                    if let Some((_, old)) =
                        entries.iter_mut().find(|(key, _)| *key == kind.as_str())
                    {
                        *old = value;
                    } else {
                        if entries.len() >= ACTION_ENTRIES {
                            return Err("Merged editor.codeActionsOnSave exceeds 32 entries");
                        }
                        entries.push((kind, value));
                    }
                }
                if entries.iter().map(|(kind, _)| kind.len()).sum::<usize>() > ACTION_TEXT_BYTES {
                    return Err("Merged editor.codeActionsOnSave kind text exceeds 8 KiB");
                }
            }
            Value::Array(array) => {
                if array.len() > ACTION_ENTRIES {
                    return Err("editor.codeActionsOnSave exceeds 32 array entries");
                }
                let mut bytes = 0;
                for kind in array.iter().filter_map(Value::as_str) {
                    if kind.len() > ACTION_KIND_BYTES {
                        return Err("editor.codeActionsOnSave kind exceeds 128 bytes");
                    }
                    bytes += kind.len();
                }
                if bytes > ACTION_TEXT_BYTES {
                    return Err("editor.codeActionsOnSave kind text exceeds 8 KiB");
                }
                *self = Self::Array(array);
            }
            _ => *self = Self::Invalid,
        }
        Ok(())
    }
    fn merge_setting(&mut self, other: &Self) -> std::result::Result<(), &'static str> {
        match other {
            Self::Missing => {}
            Self::Invalid => *self = Self::Invalid,
            Self::Array(array) => *self = Self::Array(array),
            Self::Object(incoming) => {
                if !matches!(self, Self::Object(_)) {
                    *self = Self::Object(Vec::new());
                }
                let Self::Object(entries) = self else {
                    unreachable!()
                };
                for &(kind, value) in incoming {
                    if let Some((_, old)) = entries.iter_mut().find(|(key, _)| *key == kind) {
                        *old = value;
                    } else {
                        if entries.len() >= ACTION_ENTRIES {
                            return Err("Merged editor.codeActionsOnSave exceeds 32 entries");
                        }
                        entries.push((kind, value));
                    }
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditorSideBySideDirection {
    #[default]
    Right,
    Down,
}
const SUPPORTED: &[&str] = &[
    "editor.formatOnSave",
    "editor.formatOnSaveMode",
    "editor.defaultFormatter",
    "editor.codeActionsOnSave",
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
    "breadcrumbs.enabled",
    "breadcrumbs.filePath",
    "breadcrumbs.symbolPath",
    "files.autoSave",
    "files.autoSaveDelay",
    "workbench.colorTheme",
    "workbench.editor.enablePreview",
    "workbench.editor.enablePreviewFromQuickOpen",
    "workbench.editor.preventPinnedEditorClose",
    "workbench.editor.openSideBySideDirection",
    "workbench.editor.openPositioning",
    "workbench.editor.closeEmptyGroups",
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
    /// Workbench policy is root scoped; language blocks cannot override it.
    pub fn editor_preview(&self) -> EditorPreview {
        let root_bool = |key, default| {
            self.layers
                .iter()
                .rev()
                .find_map(|layer| layer.get(key).and_then(Value::as_bool))
                .unwrap_or(default)
        };
        EditorPreview {
            enabled: root_bool("workbench.editor.enablePreview", true),
            from_quick_open: root_bool("workbench.editor.enablePreviewFromQuickOpen", false),
        }
    }
    /// Highest valid root layer wins; invalid values preserve lower policy.
    pub fn prevent_pinned_editor_close(&self) -> PreventPinnedEditorClose {
        self.layers
            .iter()
            .rev()
            .find_map(|layer| {
                layer
                    .get("workbench.editor.preventPinnedEditorClose")
                    .and_then(PreventPinnedEditorClose::parse)
            })
            .unwrap_or_default()
    }
    /// Root-only native transfer policy. Invalid higher values retain lower
    /// valid layers; unsupported valid policy refuses changed transfers.
    pub fn tab_transfer_direction(&self) -> Result<EditorSideBySideDirection> {
        let string = |key: &str, default: &'static str| {
            self.layers
                .iter()
                .rev()
                .find_map(|layer| {
                    layer
                        .get(key)
                        .filter(|value| valid(key, value))
                        .and_then(Value::as_str)
                })
                .unwrap_or(default)
        };
        ensure!(
            string("workbench.editor.openPositioning", "right") == "right",
            "Native group transfer supports only workbench.editor.openPositioning=right"
        );
        let close_empty = self
            .layers
            .iter()
            .rev()
            .find_map(|layer| {
                layer
                    .get("workbench.editor.closeEmptyGroups")
                    .and_then(Value::as_bool)
            })
            .unwrap_or(true);
        ensure!(
            close_empty,
            "Native group transfer cannot retain empty groups (closeEmptyGroups=false)"
        );
        Ok(
            match string("workbench.editor.openSideBySideDirection", "right") {
                "down" => EditorSideBySideDirection::Down,
                _ => EditorSideBySideDirection::Right,
            },
        )
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
        self.validate_scoped(values, source, false);
    }
    fn validate_scoped(&mut self, values: &Map<String, Value>, source: &str, language_scope: bool) {
        for (key, value) in values {
            if key.starts_with('[') && key.ends_with(']') {
                if let Some(object) = value.as_object() {
                    self.validate_scoped(object, &format!("{source} {key}"), true);
                } else {
                    self.warnings
                        .push(format!("{source}: {key} must be an object"));
                }
                continue;
            }
            if language_scope
                && matches!(
                    key.as_str(),
                    "workbench.editor.enablePreview"
                        | "workbench.editor.enablePreviewFromQuickOpen"
                        | "workbench.editor.preventPinnedEditorClose"
                        | "workbench.editor.openSideBySideDirection"
                        | "workbench.editor.openPositioning"
                        | "workbench.editor.closeEmptyGroups"
                )
            {
                self.warnings.push(format!(
                    "{source}: language override ignored for root-scoped {key}"
                ));
            }
            if !SUPPORTED.contains(&key.as_str()) {
                self.warnings.push(format!(
                    "{source}: {key} is not a native setting; extensions may read it"
                ));
            } else if !valid(key, value) {
                self.warnings.push(if key == "files.autoSaveDelay" {
                    format!("{source}: invalid value for {key}; native autosave disabled (delay must be 0–86400000 ms)")
                } else if key == "files.autoSave" {
                    format!("{source}: invalid value for {key}; native autosave disabled")
                } else {
                    format!("{source}: invalid value for {key}")
                });
            } else if key == "files.autoSave"
                && matches!(value.as_str(), Some("onFocusChange" | "onWindowChange"))
            {
                self.warnings.push(format!(
                    "{source}: {key} mode {} is not implemented; native autosave disabled",
                    value.as_str().unwrap()
                ));
            } else if key == "editor.formatOnSaveMode" && value.as_str() != Some("file") {
                self.warnings.push(format!("{source}: modified-lines format-on-save is not implemented; opted-in saves use an explicit skip notice"));
            } else if key == "editor.defaultFormatter" && !value.is_null() {
                self.warnings.push(format!("{source}: extension formatter selection on save is not implemented; opted-in saves use an explicit skip notice"));
            }
        }
    }
    fn value(&self, key: &str, language: &str) -> Option<&Value> {
        self.value_scoped(key, language, self.layers.len())
    }
    fn value_scoped(&self, key: &str, language: &str, limit: usize) -> Option<&Value> {
        self.value_scoped_source(key, language, limit)
            .map(|source| source.value)
    }
    /// Select an existing valid root setting's scope, defaulting to user scope.
    /// The pinned VS Code Breadcrumbs toggle writes a global root setting. Native
    /// language-scoped reads can shadow that setting, so this bounded root-only
    /// writer refuses such a target rather than silently changing another scope.
    /// Restoring a default writes an explicit scalar; upstream default-removal is
    /// a separate behavior and is not inferred by this helper.
    pub fn root_write_target(&self, key: &str, language: &str) -> Result<RootWriteScope> {
        if !SUPPORTED.contains(&key) {
            bail!("Root writes require a supported native setting");
        }
        let Some(source) = self.value_scoped_source(key, language, self.layers.len()) else {
            return Ok(RootWriteScope::User);
        };
        if let Some(selector) = source.selector {
            bail!(
                "{key} is overridden by {selector}; edit that language block before writing a root setting"
            );
        }
        Ok(if self.workspace_layer == Some(source.layer) {
            RootWriteScope::Workspace
        } else {
            RootWriteScope::User
        })
    }
    fn value_scoped_source(
        &self,
        key: &str,
        language: &str,
        limit: usize,
    ) -> Option<ScopedValue<'_>> {
        self.scoped_source(key, language, limit, true)
    }
    fn scoped_source(
        &self,
        key: &str,
        language: &str,
        limit: usize,
        require_valid: bool,
    ) -> Option<ScopedValue<'_>> {
        let mut result = None;
        for (index, layer) in self.layers.iter().take(limit).enumerate() {
            if let Some(value) = layer.get(key).filter(|v| !require_valid || valid(key, v)) {
                result = Some(ScopedValue {
                    value,
                    layer: index,
                    selector: None,
                });
            }
        }
        // Merge equal identifier groups in their first-seen position, then apply
        // single-language groups last, as in the pinned configuration model.
        let mut groups: Vec<(Vec<&str>, Option<ScopedValue<'_>>)> = Vec::new();
        for (index, layer) in self.layers.iter().take(limit).enumerate() {
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
                let found = value
                    .get(key)
                    .filter(|v| !require_valid || valid(key, v))
                    .map(|value| ScopedValue {
                        value,
                        layer: index,
                        selector: Some(selector),
                    });
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
    pub fn breadcrumbs(&self, language: &str) -> Breadcrumbs {
        let path = |key| match self.value(key, language).and_then(Value::as_str) {
            Some("off") => BreadcrumbPath::Off,
            Some("last") => BreadcrumbPath::Last,
            _ => BreadcrumbPath::On,
        };
        Breadcrumbs {
            enabled: self
                .value("breadcrumbs.enabled", language)
                .and_then(Value::as_bool)
                .unwrap_or(true),
            file_path: path("breadcrumbs.filePath"),
            symbol_path: path("breadcrumbs.symbolPath"),
        }
    }

    /// Autosave resolves the highest present value, including malformed values.
    /// Falling back to a lower valid value could unexpectedly enable disk writes.
    /// Language blocks follow the same pinned precedence as editor settings.
    pub fn save_formatting(&self, language: &str) -> SaveFormatting {
        let raw = |key| {
            self.scoped_source(key, language, self.layers.len(), false)
                .map(|source| source.value)
        };
        match raw("editor.formatOnSave") {
            None | Some(Value::Bool(false)) => return SaveFormatting::Off,
            Some(Value::Bool(true)) => {}
            _ => return SaveFormatting::Unavailable("invalid editor.formatOnSave setting"),
        }
        if raw("editor.formatOnSaveMode").is_some_and(|value| value.as_str() != Some("file")) {
            return SaveFormatting::Unavailable("only formatOnSaveMode file is supported");
        }
        if raw("editor.defaultFormatter").is_some_and(|value| !value.is_null()) {
            return SaveFormatting::Unavailable(
                "configured extension formatter is not supported on save yet",
            );
        }
        SaveFormatting::File
    }
    /// Object-aware, language-scoped save policy. This does not alter the
    /// existing scalar-setting resolver or authorize any asynchronous reply.
    pub fn save_code_actions(&self, language: &str, reason: SaveActionReason) -> SaveActionPolicy {
        if reason == SaveActionReason::AfterDelay {
            return SaveActionPolicy::Off;
        }
        let effective = match self.save_action_setting(language) {
            Ok(effective) => effective,
            Err(notice) => return SaveActionPolicy::Unavailable(notice.into()),
        };
        let mut enabled = Vec::<&str>::new();
        let mut excluded = Vec::<&str>::new();
        let mut notices = Vec::<String>::new();
        let array = matches!(effective, ActionSetting::Array(_));
        match effective {
            ActionSetting::Missing => return SaveActionPolicy::Off,
            ActionSetting::Invalid => return SaveActionPolicy::Unavailable(
                "editor.codeActionsOnSave must be an object or string array; native save actions disabled".into()
            ),
            ActionSetting::Object(entries) => {
                for (kind, value) in entries {
                    if !action_kind_valid(kind) {
                        notices.push("Malformed code action kind ignored; native kind names require 1–128 bytes and nonempty dot components".into());
                        continue;
                    }
                    match value {
                        Value::Bool(true) => enabled.push(kind),
                        Value::Bool(false) => {},
                        Value::String(value) if matches!(value.as_str(), "explicit" | "always") => enabled.push(kind),
                        Value::String(value) if value == "never" => excluded.push(kind),
                        _ => notices.push(format!("Invalid editor.codeActionsOnSave value for {kind}; entry disabled")),
                    }
                }
            }
            ActionSetting::Array(entries) => {
                for value in entries {
                    match value.as_str().filter(|kind| action_kind_valid(kind)) {
                        Some(kind) if !enabled.contains(&kind) => enabled.push(kind),
                        Some(_) => {},
                        None => notices.push("Malformed editor.codeActionsOnSave array entry ignored".into()),
                    }
                }
            }
        }
        // Remove descendants before projecting to families: [child, imports,
        // parent] must retain [imports, parent], not the removed child's order.
        let mut canonical: Vec<_> = enabled
            .iter()
            .copied()
            .filter(|kind| {
                !enabled
                    .iter()
                    .any(|other| other != kind && action_kind_contains(other, kind))
            })
            .collect();
        if !array {
            canonical
                .sort_by_key(|kind| !action_kind_contains(SaveActionFamily::FixAll.kind(), kind));
        }
        let mut families = Vec::new();
        for kind in &canonical {
            let mut supported = false;
            for family in [SaveActionFamily::FixAll, SaveActionFamily::OrganizeImports] {
                let root = family.kind();
                if action_kind_contains(kind, root) || action_kind_contains(root, kind) {
                    supported = true;
                    // The intersection is the narrower of enabled kind/root.
                    let intersection = if action_kind_contains(kind, root) {
                        root
                    } else {
                        kind
                    };
                    if !excluded
                        .iter()
                        .any(|excluded| action_kind_contains(excluded, intersection))
                        && !families.contains(&family)
                    {
                        families.push(family);
                    }
                }
            }
            if !supported {
                notices.push(format!("Save action {kind} is not supported; only source.fixAll and source.organizeImports are native save families"));
            } else if *kind == "source" {
                notices.push("Save action source is limited to native fix-all and organize-imports families; other source actions are not run".into());
            }
        }
        if !array {
            families.sort_by_key(|family| *family != SaveActionFamily::FixAll);
        }
        if families.is_empty() {
            return if notices.is_empty() {
                SaveActionPolicy::Off
            } else {
                SaveActionPolicy::Unavailable(notices.join("; "))
            };
        }
        SaveActionPolicy::Native(SaveActionPlan {
            families,
            enabled: canonical.into_iter().map(str::to_owned).collect(),
            excluded: excluded.into_iter().map(str::to_owned).collect(),
            notices,
        })
    }
    fn save_action_setting(
        &self,
        language: &str,
    ) -> std::result::Result<ActionSetting<'_>, &'static str> {
        if language.is_empty() || language.len() > 128 || language.contains('\0') {
            return Err("Save action language requires 1–128 bytes without NUL");
        }
        if self.layers.len() > 16 || self.layers.iter().any(|layer| layer.len() > 4096) {
            return Err("Save action configuration exceeds 16 layers or 4096 settings per layer");
        }
        let mut result = ActionSetting::Missing;
        for layer in self.layers.iter() {
            if let Some(value) = layer.get("editor.codeActionsOnSave") {
                result.merge(value)?;
            }
        }
        let mut groups: Vec<(Vec<&str>, ActionSetting<'_>)> = Vec::new();
        let mut selector_bytes = 0usize;
        for layer in self.layers.iter() {
            for (selector, value) in layer {
                let Some(inner) = selector.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
                else {
                    continue;
                };
                if selector.len() > 1024 {
                    return Err("Save action language selector exceeds 1024 bytes");
                }
                let mut ids = Vec::new();
                let mut valid_selector = true;
                for id in inner.split("][") {
                    if id.is_empty() || id.contains(['[', ']']) {
                        valid_selector = false;
                        break;
                    }
                    let id = id.trim();
                    if !id.is_empty() && !ids.contains(&id) {
                        if ids.len() >= 32 {
                            return Err("Save action selector exceeds 32 language identifiers");
                        }
                        ids.push(id);
                    }
                }
                if !valid_selector || !ids.contains(&language) {
                    continue;
                }
                let Some(found) = value.get("editor.codeActionsOnSave") else {
                    continue;
                };
                if let Some((_, merged)) = groups.iter_mut().find(|(existing, _)| *existing == ids)
                {
                    merged.merge(found)?;
                } else {
                    if groups.len() >= ACTION_GROUPS {
                        return Err(
                            "Save action configuration exceeds 64 matching language groups",
                        );
                    }
                    selector_bytes += selector.len();
                    if selector_bytes > ACTION_SELECTOR_BYTES {
                        return Err("Save action selectors exceed 16 KiB");
                    }
                    let mut merged = ActionSetting::Missing;
                    merged.merge(found)?;
                    groups.push((ids, merged));
                }
            }
        }
        for single in [false, true] {
            for (ids, merged) in &groups {
                if (ids.len() == 1) == single {
                    result.merge_setting(merged)?;
                }
            }
        }
        Ok(result)
    }
    pub fn auto_save(&self, language: &str) -> crate::autosave::Policy {
        use crate::autosave::{DEFAULT_DELAY_MS, MAX_DELAY_MS, Policy};
        let raw = |key| {
            self.scoped_source(key, language, self.layers.len(), false)
                .map(|source| source.value)
        };
        let delay_ms = match raw("files.autoSaveDelay") {
            None => DEFAULT_DELAY_MS,
            Some(value) => match value.as_u64().filter(|&delay| delay <= MAX_DELAY_MS.into()) {
                Some(delay) => delay as u32,
                None => return Policy::Off,
            },
        };
        match raw("files.autoSave") {
            None => Policy::Off,
            Some(value) if value.as_str() == Some("afterDelay") => Policy::AfterDelay { delay_ms },
            _ => Policy::Off,
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
                Some("brackets") => AutoIndent::Brackets,
                Some("advanced") => AutoIndent::Advanced,
                _ => AutoIndent::Full,
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
        "editor.codeActionsOnSave" => match value {
            Value::Object(object) => {
                object.len() <= ACTION_ENTRIES
                    && object.iter().all(|(kind, value)| {
                        action_kind_valid(kind)
                            && (value.is_boolean()
                                || matches!(value.as_str(), Some("always" | "explicit" | "never")))
                    })
            }
            Value::Array(array) => {
                array.len() <= ACTION_ENTRIES
                    && array
                        .iter()
                        .all(|value| value.as_str().is_some_and(action_kind_valid))
            }
            _ => false,
        },
        "editor.formatOnSave" => value.is_boolean(),
        "editor.formatOnSaveMode" => matches!(
            value.as_str(),
            Some("file" | "modifications" | "modificationsIfAvailable")
        ),
        "editor.defaultFormatter" => {
            value.is_null()
                || value
                    .as_str()
                    .is_some_and(|s| !s.is_empty() && s.len() <= 4096)
        }
        "files.autoSave" => matches!(
            value.as_str(),
            Some("off" | "afterDelay" | "onFocusChange" | "onWindowChange")
        ),
        "files.autoSaveDelay" => value
            .as_u64()
            .is_some_and(|delay| delay <= crate::autosave::MAX_DELAY_MS.into()),
        "workbench.editor.preventPinnedEditorClose" => {
            PreventPinnedEditorClose::parse(value).is_some()
        }
        "workbench.editor.openSideBySideDirection" => {
            matches!(value.as_str(), Some("right" | "down"))
        }
        "workbench.editor.openPositioning" => {
            matches!(value.as_str(), Some("right" | "left" | "first" | "last"))
        }
        "workbench.editor.closeEmptyGroups" => value.is_boolean(),
        "workbench.colorTheme" => value.is_string(),
        "workbench.editor.enablePreview" | "workbench.editor.enablePreviewFromQuickOpen" => {
            value.is_boolean()
        }
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
        | "editor.parameterHints.cycle"
        | "breadcrumbs.enabled" => value.is_boolean(),
        "breadcrumbs.filePath" | "breadcrumbs.symbolPath" => {
            matches!(value.as_str(), Some("on" | "off" | "last"))
        }
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
#[cfg(test)]
mod save_actions_tests {
    use super::*;
    use serde_json::json;

    fn settings(layers: Vec<Value>) -> Settings {
        let layers: Vec<_> = layers
            .into_iter()
            .map(|value| value.as_object().unwrap().clone())
            .collect();
        Settings {
            serialized: serde_json::to_string(&layers).unwrap().into(),
            layers: Arc::new(layers),
            ..Settings::default()
        }
    }
    fn plan(policy: SaveActionPolicy) -> SaveActionPlan {
        match policy {
            SaveActionPolicy::Native(plan) => plan,
            other => panic!("Native plan expected: {other:?}"),
        }
    }
    #[test]
    fn all_fourteen_actual_pinned_save_action_settings_and_supported_filters_match() {
        let cases: Vec<Value> = serde_json::from_str(include_str!(
            "../tests/vscode-reference/save-code-actions-cases.json"
        ))
        .unwrap();
        let observed: Vec<Value> = serde_json::from_str(include_str!(
            "../tests/vscode-reference/baselines/1.95.0/save-code-actions/linux.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 14);
        assert_eq!(observed.len(), 14);
        for (fixture, actual) in cases.iter().zip(&observed) {
            assert_eq!(fixture["name"], actual["name"]);
            let settings = settings(vec![fixture["user"].clone(), fixture["workspace"].clone()]);
            let merged = settings.save_action_setting("plaintext").unwrap();
            let (effective, keys) = match merged {
                ActionSetting::Object(entries) => {
                    let keys: Vec<_> = entries.iter().map(|(kind, _)| (*kind).to_owned()).collect();
                    let object: Map<_, _> = entries
                        .into_iter()
                        .map(|(kind, value)| (kind.to_owned(), value.clone()))
                        .collect();
                    (Value::Object(object), json!(keys))
                }
                ActionSetting::Array(array) => (json!(array), Value::Null),
                _ => panic!("Unexpected effective settings for {}", fixture["name"]),
            };
            assert_eq!(
                effective, actual["effective"]["codeActionsOnSave"],
                "{}",
                fixture["name"]
            );
            assert_eq!(keys, actual["effectiveKeys"], "{}", fixture["name"]);
            let automatic = fixture["autosave"] == "afterDelay";
            let reason = if automatic {
                SaveActionReason::AfterDelay
            } else {
                SaveActionReason::Explicit
            };
            let policy = settings.save_code_actions("plaintext", reason);
            if automatic {
                assert_eq!(policy, SaveActionPolicy::Off);
                assert!(actual["callbacks"].as_array().unwrap().is_empty());
                continue;
            }
            let plan = plan(policy);
            let saved = actual["observations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["action"] == "workbench.action.files.save")
                .unwrap();
            let lines: Vec<_> = saved["text"].as_str().unwrap().lines().collect();
            for (family, kind, row) in [
                (SaveActionFamily::FixAll, "source.fixAll", 0),
                (SaveActionFamily::FixAll, "source.fixAll.child", 2),
                (
                    SaveActionFamily::OrganizeImports,
                    "source.organizeImports",
                    1,
                ),
            ] {
                assert_eq!(
                    plan.allows(family, kind),
                    lines[row].ends_with("=1"),
                    "{} {kind}",
                    fixture["name"]
                );
            }
            assert!(!plan.allows(SaveActionFamily::FixAll, "source.fixAllX"));
            // Broad source queries are an explicitly reported native subset;
            // the actual provider's unrelated kind still remains in evidence.
            if lines[3] == "other=1" {
                assert!(
                    plan.notices
                        .iter()
                        .any(|notice| notice.contains("other source actions are not run"))
                );
            }
            let mut expected = Vec::new();
            for callback in actual["callbacks"].as_array().unwrap() {
                let only = callback["only"].as_str().unwrap();
                for family in [SaveActionFamily::FixAll, SaveActionFamily::OrganizeImports] {
                    if (action_kind_contains(only, family.kind())
                        || action_kind_contains(family.kind(), only))
                        && plan.families.contains(&family)
                        && !expected.contains(&family)
                    {
                        expected.push(family);
                    }
                }
            }
            assert_eq!(plan.families, expected, "{}", fixture["name"]);
        }
    }
    #[test]
    fn scalar_array_and_object_replacement_preserve_raw_winning_policy() {
        assert_eq!(
            Settings::default().save_code_actions("cpp", SaveActionReason::Explicit),
            SaveActionPolicy::Off
        );
        let lower = json!({"editor.codeActionsOnSave":{"source.fixAll":"explicit"}});
        for higher in [Value::Null, json!(true), json!("source.fixAll"), json!(5)] {
            assert!(matches!(
                settings(vec![
                    lower.clone(),
                    json!({"editor.codeActionsOnSave":higher})
                ])
                .save_code_actions("cpp", SaveActionReason::Explicit),
                SaveActionPolicy::Unavailable(_)
            ));
        }
        let object_after_array = settings(vec![
            json!({"editor.codeActionsOnSave":["source.fixAll"]}),
            json!({"editor.codeActionsOnSave":{"source.organizeImports":"explicit"}}),
        ]);
        assert_eq!(
            plan(object_after_array.save_code_actions("cpp", SaveActionReason::Explicit)).families,
            vec![SaveActionFamily::OrganizeImports]
        );
        let empty_after_array = settings(vec![
            json!({"editor.codeActionsOnSave":["source.fixAll"]}),
            json!({"editor.codeActionsOnSave":{}}),
        ]);
        assert_eq!(
            empty_after_array.save_code_actions("cpp", SaveActionReason::Explicit),
            SaveActionPolicy::Off
        );
        // One malformed leaf cannot reactivate its inherited lower value.
        let malformed = settings(vec![
            lower,
            json!({"editor.codeActionsOnSave":{
            "source.fixAll":{},"source.organizeImports":"explicit"}}),
        ]);
        let native = plan(malformed.save_code_actions("cpp", SaveActionReason::Explicit));
        assert_eq!(native.families, vec![SaveActionFamily::OrganizeImports]);
        assert!(!native.notices.is_empty());
    }
    #[test]
    fn equal_composite_groups_merge_before_single_language_groups_without_losing_members() {
        let settings = settings(vec![
            json!({
                "[cpp][rust]":{"editor.codeActionsOnSave":{"source.fixAll":"explicit"}},
                "[cpp]":{"editor.codeActionsOnSave":{"source.fixAll.child":"never"}}
            }),
            json!({
                "[cpp][rust]":{"editor.codeActionsOnSave":{"source.organizeImports":"explicit"}},
                "[cpp]":{"editor.codeActionsOnSave":{"source.organizeImports":false}}
            }),
        ]);
        let cpp = plan(settings.save_code_actions("cpp", SaveActionReason::Explicit));
        assert_eq!(cpp.families, vec![SaveActionFamily::FixAll]);
        assert!(cpp.allows(SaveActionFamily::FixAll, "source.fixAll"));
        assert!(!cpp.allows(SaveActionFamily::FixAll, "source.fixAll.child"));
        let rust = plan(settings.save_code_actions("rust", SaveActionReason::Explicit));
        assert_eq!(
            rust.families,
            vec![SaveActionFamily::FixAll, SaveActionFamily::OrganizeImports]
        );
        assert!(rust.allows(SaveActionFamily::FixAll, "source.fixAll.child"));
    }
    #[test]
    fn ancestor_removal_precedes_array_family_order_and_filters_do_not_enable_siblings() {
        let ordered = settings(vec![json!({"editor.codeActionsOnSave":[
            "source.fixAll.child","source.organizeImports","source.fixAll"]})]);
        assert_eq!(
            plan(ordered.save_code_actions("cpp", SaveActionReason::Explicit)).families,
            vec![SaveActionFamily::OrganizeImports, SaveActionFamily::FixAll]
        );
        let children = settings(vec![
            json!({"editor.codeActionsOnSave":["source.fixAll.a","source.fixAll.b"]}),
        ]);
        let children = plan(children.save_code_actions("cpp", SaveActionReason::Explicit));
        assert_eq!(children.families, vec![SaveActionFamily::FixAll]);
        assert!(children.allows(SaveActionFamily::FixAll, "source.fixAll.a.nested"));
        for kind in [
            "source.fixAll",
            "source.fixAll.ab",
            "source.fixAllX.a",
            "source.fixAll.c",
            "source.fixAll.a..bad",
        ] {
            assert!(!children.allows(SaveActionFamily::FixAll, kind), "{kind}");
        }
        let excluded = settings(vec![json!({"editor.codeActionsOnSave":{
            "source.fixAll.child":"explicit","source.fixAll":"never"}})]);
        assert_eq!(
            excluded.save_code_actions("cpp", SaveActionReason::Explicit),
            SaveActionPolicy::Off
        );
    }
    #[test]
    fn malformed_large_contributions_and_group_budgets_fail_closed_without_cloning_values() {
        let excessive: Map<_, _> = (0..33)
            .map(|i| (format!("source.fixAll.k{i}"), json!(true)))
            .collect();
        let excessive_groups: Map<_, _> = (0..65)
            .map(|i| {
                (
                    format!("[cpp][language{i}]"),
                    json!({"editor.codeActionsOnSave":{"source.fixAll":true}}),
                )
            })
            .collect();
        for values in [
            json!({"editor.codeActionsOnSave":excessive}),
            json!({"editor.codeActionsOnSave":vec!["source.fixAll";33]}),
            json!({"editor.codeActionsOnSave":{ "x".repeat(129):true}}),
            Value::Object(excessive_groups),
            json!({format!("[cpp][{}]","x".repeat(1024)):{"editor.codeActionsOnSave":{"source.fixAll":true}}}),
        ] {
            assert!(matches!(
                settings(vec![values]).save_code_actions("cpp", SaveActionReason::Explicit),
                SaveActionPolicy::Unavailable(_)
            ));
        }
        let unsupported: Map<_, _> = (0..32)
            .map(|i| (format!("source.{}.{i}", "x".repeat(100)), json!(true)))
            .collect();
        let policy = settings(vec![json!({"editor.codeActionsOnSave":unsupported})])
            .save_code_actions("cpp", SaveActionReason::Explicit);
        let SaveActionPolicy::Unavailable(notice) = policy else {
            panic!("Unsupported-only policy should have a notice")
        };
        assert!(notice.len() <= 8192);
        let array = settings(vec![
            json!({"editor.codeActionsOnSave":[{},"source.fixAll"]}),
        ]);
        assert_eq!(
            plan(array.save_code_actions("cpp", SaveActionReason::Explicit)).families,
            vec![SaveActionFamily::FixAll]
        );
        assert_eq!(
            array.save_code_actions("cpp", SaveActionReason::AfterDelay),
            SaveActionPolicy::Off
        );
    }
}
const LOADER_PERIOD: Duration = Duration::from_secs(2);
const LOADER_PATH_BYTES: usize = 4096;

struct PendingLoad {
    generation: u64,
    worker: std::thread::JoinHandle<std::result::Result<Settings, String>>,
}

/// One actual settings read and one coalesced desired reload. Invalidating a read
/// discards its publication authority, but retains its worker until it exits.
pub struct Loader {
    paths: Vec<PathBuf>,
    generation: u64,
    last: Instant,
    wanted: bool,
    pending: Option<PendingLoad>,
    #[cfg(test)]
    load: TestLoad,
}
#[cfg(test)]
type TestLoad = Arc<dyn Fn(&[PathBuf]) -> std::result::Result<Settings, String> + Send + Sync>;
impl Loader {
    pub fn new(paths: Vec<PathBuf>) -> Result<Self> {
        Self::validate_paths(&paths)?;
        Ok(Self {
            paths,
            generation: 1,
            last: Instant::now(),
            wanted: false,
            pending: None,
            #[cfg(test)]
            load: Arc::new(|paths| Settings::load_editor(paths).map_err(|e| format!("{e:#}"))),
        })
    }
    fn validate_paths(paths: &[PathBuf]) -> Result<()> {
        if paths.len() > 2 {
            bail!("Settings loader supports at most two paths");
        }
        if paths.iter().any(|path| {
            path.as_os_str().is_empty()
                || path.as_os_str().as_encoded_bytes().len() > LOADER_PATH_BYTES
        }) {
            bail!("Settings loader paths must contain 1–4096 bytes");
        }
        Ok(())
    }
    /// Fence every earlier result, even when the configured paths are unchanged.
    /// Scheduling and reading remain in `poll`; this call performs no I/O.
    pub fn force_reload(&mut self) -> Result<()> {
        let generation = self
            .generation
            .checked_add(1)
            .context("Settings reload generation exhausted")?;
        self.generation = generation;
        self.wanted = true;
        Ok(())
    }
    /// Retain an in-flight read while changing the latest desired profile paths.
    /// A→B→A still advances the generation and cannot authorize the first A read.
    pub fn reconfigure(&mut self, paths: Vec<PathBuf>) -> Result<()> {
        Self::validate_paths(&paths)?;
        self.force_reload()?;
        self.paths = paths;
        Ok(())
    }
    pub fn poll(&mut self) -> Option<std::result::Result<Settings, String>> {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.worker.is_finished())
        {
            // is_finished proves the actual callback has returned; joining here
            // never waits for filesystem work or a canceled callback to finish.
            let pending = self.pending.take().expect("finished settings worker");
            let result = pending
                .worker
                .join()
                .unwrap_or_else(|_| Err("Settings worker stopped".into()));
            if pending.generation == self.generation {
                return Some(result);
            }
        }
        if self.pending.is_none() && (self.wanted || self.last.elapsed() >= LOADER_PERIOD) {
            let paths = self.paths.clone();
            #[cfg(test)]
            let load = self.load.clone();
            self.last = Instant::now();
            self.wanted = false;
            let worker = std::thread::Builder::new()
                .name("vscli-settings".into())
                .spawn(move || {
                    #[cfg(test)]
                    {
                        load(&paths)
                    }
                    #[cfg(not(test))]
                    {
                        Settings::load_editor(&paths).map_err(|e| format!("{e:#}"))
                    }
                });
            match worker {
                Ok(worker) => {
                    self.pending = Some(PendingLoad {
                        generation: self.generation,
                        worker,
                    });
                }
                Err(error) => {
                    return Some(Err(format!("Could not start settings worker: {error}")));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod loader_tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    };

    const WAIT: Duration = Duration::from_secs(3);

    struct HeldLoader {
        loader: Loader,
        started: Receiver<Vec<PathBuf>>,
        release: Sender<()>,
        active: Arc<AtomicUsize>,
        maximum: Arc<AtomicUsize>,
    }
    impl HeldLoader {
        fn new(paths: Vec<PathBuf>) -> Self {
            let mut loader = Loader::new(paths).unwrap();
            let (started_tx, started) = mpsc::sync_channel(1);
            let (release, release_rx) = mpsc::channel();
            let release_rx = Arc::new(Mutex::new(release_rx));
            let active = Arc::new(AtomicUsize::new(0));
            let maximum = Arc::new(AtomicUsize::new(0));
            let active_worker = active.clone();
            let maximum_worker = maximum.clone();
            loader.load = Arc::new(move |paths| {
                let current = active_worker.fetch_add(1, Ordering::SeqCst) + 1;
                maximum_worker.fetch_max(current, Ordering::SeqCst);
                // Read before the explicit gate: the held result is genuinely
                // stale when the test changes the file and invalidates it.
                let result = Settings::load_editor(paths).map_err(|e| format!("{e:#}"));
                started_tx.send(paths.to_vec()).unwrap();
                let released = release_rx.lock().unwrap().recv_timeout(WAIT);
                active_worker.fetch_sub(1, Ordering::SeqCst);
                released.expect("test must release held settings callback");
                result
            });
            Self {
                loader,
                started,
                release,
                active,
                maximum,
            }
        }
        fn start(&mut self) -> Vec<PathBuf> {
            assert!(self.loader.poll().is_none());
            self.started.recv_timeout(WAIT).unwrap()
        }
        fn next_started_without_publication(&mut self) -> Vec<PathBuf> {
            let deadline = Instant::now() + WAIT;
            loop {
                assert!(self.loader.poll().is_none(), "superseded result published");
                if let Ok(paths) = self.started.try_recv() {
                    return paths;
                }
                assert!(Instant::now() < deadline, "latest reload never started");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        fn finish(&mut self) -> std::result::Result<Settings, String> {
            self.release.send(()).unwrap();
            let deadline = Instant::now() + WAIT;
            loop {
                if let Some(result) = self.loader.poll() {
                    return result;
                }
                assert!(Instant::now() < deadline, "settings result never published");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        fn assert_held_alone(&mut self) {
            for _ in 0..256 {
                assert!(self.loader.poll().is_none());
            }
            assert_eq!(self.active.load(Ordering::SeqCst), 1);
            assert_eq!(self.maximum.load(Ordering::SeqCst), 1);
            assert!(matches!(
                self.started.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
        }
    }

    #[test]
    fn initial_period_is_preserved_and_force_reload_starts_immediately() {
        let mut held = HeldLoader::new(Vec::new());
        assert!(held.loader.poll().is_none());
        assert!(matches!(
            held.started.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        held.loader.force_reload().unwrap();
        assert!(held.start().is_empty());
        held.assert_held_alone();
        assert!(held.finish().is_ok());
        assert!(held.loader.poll().is_none());
    }

    #[test]
    fn forced_reload_discards_held_failure_and_coalesces_latest_disk_read() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        std::fs::write(&path, "malformed").unwrap();
        let mut held = HeldLoader::new(vec![path.clone()]);
        held.loader.force_reload().unwrap();
        assert_eq!(held.start(), vec![path.clone()]);
        std::fs::write(&path, r#"{"breadcrumbs.enabled":false}"#).unwrap();
        for _ in 0..256 {
            held.loader.force_reload().unwrap();
        }
        held.assert_held_alone();
        held.release.send(()).unwrap();
        assert_eq!(held.next_started_without_publication(), vec![path]);
        assert!(!held.finish().unwrap().breadcrumbs("cpp").enabled);
        assert_eq!(held.maximum.load(Ordering::SeqCst), 1);
        assert!(held.loader.poll().is_none());
    }

    #[test]
    fn profile_a_b_a_cannot_revive_first_a_or_start_overlapping_reads() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("a.json");
        let second = directory.path().join("b.json");
        std::fs::write(&first, r#"{"breadcrumbs.enabled":true}"#).unwrap();
        std::fs::write(&second, r#"{"breadcrumbs.enabled":true}"#).unwrap();
        let mut held = HeldLoader::new(vec![first.clone()]);
        held.loader.force_reload().unwrap();
        assert_eq!(held.start(), vec![first.clone()]);
        held.loader.reconfigure(vec![second]).unwrap();
        std::fs::write(&first, r#"{"breadcrumbs.enabled":false}"#).unwrap();
        held.loader.reconfigure(vec![first.clone()]).unwrap();
        held.assert_held_alone();
        held.release.send(()).unwrap();
        assert_eq!(held.next_started_without_publication(), vec![first]);
        assert!(!held.finish().unwrap().breadcrumbs("cpp").enabled);
        assert_eq!(held.maximum.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn invalid_paths_and_generation_exhaustion_preserve_current_profile() {
        assert!(Loader::new(vec![PathBuf::new()]).is_err());
        assert!(Loader::new(vec![PathBuf::from("x"); 3]).is_err());
        assert!(Loader::new(vec![PathBuf::from("x".repeat(4097))]).is_err());
        let mut loader = Loader::new(vec![PathBuf::from("original")]).unwrap();
        let original_generation = loader.generation;
        assert!(loader.reconfigure(vec![PathBuf::new()]).is_err());
        assert_eq!(loader.generation, original_generation);
        assert_eq!(loader.paths, vec![PathBuf::from("original")]);
        assert!(!loader.wanted);
        loader.generation = u64::MAX;
        assert!(loader.force_reload().is_err());
        assert!(
            loader
                .reconfigure(vec![PathBuf::from("replacement")])
                .is_err()
        );
        assert_eq!(loader.generation, u64::MAX);
        assert_eq!(loader.paths, vec![PathBuf::from("original")]);
        assert!(!loader.wanted);
        assert!(loader.pending.is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn save_formatting_is_opt_in_language_scoped_and_fails_closed() {
        assert_eq!(
            Settings::default().save_formatting("cpp"),
            SaveFormatting::Off
        );
        let settings = Settings::from_values(
            serde_json::json!({
                "editor.formatOnSave":true,
                "[cpp]":{"editor.formatOnSave":false},
                "[rust]":{"editor.formatOnSaveMode":"modifications"},
                "[python]":{"editor.defaultFormatter":"example.formatter"},
                "[go]":{"editor.formatOnSave":"true"}
            })
            .as_object()
            .unwrap()
            .clone(),
            "formatting test",
        )
        .unwrap();
        assert_eq!(settings.save_formatting("plaintext"), SaveFormatting::File);
        assert_eq!(settings.save_formatting("cpp"), SaveFormatting::Off);
        for language in ["rust", "python", "go"] {
            assert!(matches!(
                settings.save_formatting(language),
                SaveFormatting::Unavailable(_)
            ));
        }
        for value in [serde_json::json!(false), serde_json::json!(0), Value::Null] {
            let settings = Settings::from_values(
                serde_json::json!({"editor.formatOnSave":value})
                    .as_object()
                    .unwrap()
                    .clone(),
                "formatting test",
            )
            .unwrap();
            assert_ne!(settings.save_formatting("cpp"), SaveFormatting::File);
        }
    }
    #[test]
    fn higher_invalid_save_formatting_values_do_not_enable_a_lower_formatter() {
        let directory = tempfile::tempdir().unwrap();
        let user = directory.path().join("user.json");
        let workspace = directory.path().join("workspace.json");
        std::fs::write(
            &user,
            r#"{"editor.formatOnSave":true,"editor.defaultFormatter":"user.formatter"}"#,
        )
        .unwrap();
        std::fs::write(&workspace, r#"{"editor.formatOnSave":"invalid"}"#).unwrap();
        let settings = Settings::load(&[user.clone(), workspace.clone()]).unwrap();
        assert!(matches!(
            settings.save_formatting("cpp"),
            SaveFormatting::Unavailable(_)
        ));
        std::fs::write(
            &workspace,
            r#"{"editor.formatOnSave":true,"editor.defaultFormatter":null}"#,
        )
        .unwrap();
        assert_eq!(
            Settings::load(&[user, workspace])
                .unwrap()
                .save_formatting("cpp"),
            SaveFormatting::File
        );
    }
    fn autosave_settings(layers: impl IntoIterator<Item = Value>) -> Settings {
        let mut settings = Settings::default();
        for value in layers {
            let values = value.as_object().unwrap().clone();
            settings.validate(&values, "autosave fixture");
            Arc::make_mut(&mut settings.layers).push(values);
        }
        settings.serialized = serde_json::to_string(settings.layers.as_ref())
            .unwrap()
            .into();
        settings
    }
    #[test]
    fn autosave_defaults_and_explicit_native_delay_boundaries() {
        use crate::autosave::{MAX_DELAY_MS, Policy};
        assert_eq!(Settings::default().auto_save("cpp"), Policy::Off);
        for delay_ms in [0, 1000, MAX_DELAY_MS] {
            let settings = autosave_settings([serde_json::json!({
                "files.autoSave":"afterDelay", "files.autoSaveDelay":delay_ms
            })]);
            assert_eq!(settings.auto_save("cpp"), Policy::AfterDelay { delay_ms });
            assert!(settings.warnings.is_empty());
        }
        let settings = autosave_settings([serde_json::json!({"files.autoSave":"afterDelay"})]);
        assert_eq!(
            settings.auto_save("cpp"),
            Policy::AfterDelay { delay_ms: 1000 }
        );
    }
    #[test]
    fn autosave_invalid_winning_delay_disables_instead_of_falling_back() {
        use crate::autosave::{MAX_DELAY_MS, Policy};
        for invalid in [
            serde_json::json!(-1),
            serde_json::json!(1.5),
            serde_json::json!("1"),
            Value::Bool(true),
            Value::Null,
            serde_json::json!(MAX_DELAY_MS as u64 + 1),
            serde_json::json!(u32::MAX),
        ] {
            let settings = autosave_settings([
                serde_json::json!({"files.autoSave":"afterDelay", "files.autoSaveDelay":1}),
                serde_json::json!({"files.autoSaveDelay":invalid}),
            ]);
            assert_eq!(settings.auto_save("cpp"), Policy::Off);
            assert_eq!(settings.warnings.len(), 1);
            assert!(settings.warnings[0].contains("native autosave disabled"));
        }
    }
    #[test]
    fn autosave_unsupported_or_malformed_mode_never_enables_automation() {
        use crate::autosave::Policy;
        for mode in [
            serde_json::json!("onFocusChange"),
            serde_json::json!("onWindowChange"),
            serde_json::json!("invalid"),
            Value::Bool(true),
            Value::Null,
        ] {
            let settings = autosave_settings([
                serde_json::json!({"files.autoSave":"afterDelay"}),
                serde_json::json!({"files.autoSave":mode}),
            ]);
            assert_eq!(settings.auto_save("cpp"), Policy::Off);
            assert_eq!(settings.warnings.len(), 1);
            assert!(settings.warnings[0].contains("native autosave disabled"));
        }
        let settings = autosave_settings([serde_json::json!({
            "files.autoSave":"off", "files.autoSaveDelay":null
        })]);
        assert_eq!(settings.auto_save("cpp"), Policy::Off);
        assert_eq!(settings.warnings.len(), 1);
    }
    #[test]
    fn autosave_language_composite_and_single_groups_preserve_pinned_precedence() {
        use crate::autosave::Policy;
        let settings = autosave_settings([
            serde_json::json!({
                "files.autoSave":"off", "files.autoSaveDelay":100,
                "[cpp][rust]":{"files.autoSave":"afterDelay", "files.autoSaveDelay":50},
                "[cpp]":{"files.autoSaveDelay":25}
            }),
            serde_json::json!({
                "files.autoSave":"afterDelay", "files.autoSaveDelay":500,
                "[cpp][rust]":{"files.autoSaveDelay":75},
                "[rust]":{"files.autoSave":"off"}
            }),
        ]);
        assert_eq!(
            settings.auto_save("cpp"),
            Policy::AfterDelay { delay_ms: 25 }
        );
        assert_eq!(settings.auto_save("rust"), Policy::Off);
        assert_eq!(
            settings.auto_save("plaintext"),
            Policy::AfterDelay { delay_ms: 500 }
        );
        assert!(settings.warnings.is_empty());
    }
    #[test]
    fn autosave_malformed_language_winner_disables_without_changing_other_key_fallback() {
        use crate::autosave::Policy;
        let settings = autosave_settings([
            serde_json::json!({
                "files.autoSave":"afterDelay", "files.autoSaveDelay":10, "editor.tabSize":2,
                "[cpp][rust]":{"files.autoSaveDelay":20, "editor.tabSize":8}
            }),
            serde_json::json!({
                "[cpp][rust]":{"files.autoSaveDelay":null, "editor.tabSize":null}
            }),
        ]);
        assert_eq!(settings.auto_save("cpp"), Policy::Off);
        assert_eq!(settings.auto_save("rust"), Policy::Off);
        assert_eq!(
            settings.auto_save("plaintext"),
            Policy::AfterDelay { delay_ms: 10 }
        );
        assert_eq!(
            settings.value("editor.tabSize", "cpp"),
            Some(&serde_json::json!(8))
        );
        assert_eq!(settings.warnings.len(), 2);
        let single = autosave_settings([
            serde_json::json!({"files.autoSave":"afterDelay", "[cpp][rust]":{"files.autoSaveDelay":20}}),
            serde_json::json!({"[cpp]":{"files.autoSaveDelay":"bad"}}),
        ]);
        assert_eq!(single.auto_save("cpp"), Policy::Off);
        assert_eq!(
            single.auto_save("rust"),
            Policy::AfterDelay { delay_ms: 20 }
        );
    }
    #[test]
    fn root_write_scope_defaults_to_user_and_uses_the_recorded_workspace_index() {
        assert_eq!(
            Settings::default()
                .root_write_target("breadcrumbs.enabled", "cpp")
                .unwrap(),
            RootWriteScope::User
        );
        assert!(
            Settings::default()
                .root_write_target("extension.unknown", "cpp")
                .is_err()
        );
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("workspace-looking-name.json");
        let second = root.path().join("user-looking-name.json");
        std::fs::write(
            &first,
            r#"{"breadcrumbs.enabled":false,"breadcrumbs.filePath":"last"}"#,
        )
        .unwrap();
        std::fs::write(
            &second,
            r#"{"breadcrumbs.enabled":true,"breadcrumbs.filePath":false}"#,
        )
        .unwrap();
        let settings = Settings::load_editor(&[first.clone(), second.clone()]).unwrap();
        assert_eq!(
            settings
                .root_write_target("breadcrumbs.enabled", "cpp")
                .unwrap(),
            RootWriteScope::Workspace
        );
        assert_eq!(
            settings
                .root_write_target("breadcrumbs.filePath", "cpp")
                .unwrap(),
            RootWriteScope::User
        );
        assert_eq!(
            settings
                .root_write_target("breadcrumbs.symbolPath", "cpp")
                .unwrap(),
            RootWriteScope::User
        );
        assert_eq!(
            Settings::load(std::slice::from_ref(&second))
                .unwrap()
                .root_write_target("breadcrumbs.enabled", "cpp")
                .unwrap(),
            RootWriteScope::User
        );
        assert_eq!(
            Settings::load_editor(&[second])
                .unwrap()
                .root_write_target("breadcrumbs.enabled", "cpp")
                .unwrap(),
            RootWriteScope::Workspace
        );
    }
    #[test]
    fn malformed_higher_root_and_language_values_do_not_control_the_write_scope() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user.json");
        let workspace = root.path().join("workspace.json");
        std::fs::write(
            &user,
            r#"{"breadcrumbs.enabled":false,"breadcrumbs.filePath":"last"}"#,
        )
        .unwrap();
        std::fs::write(&workspace,r#"{"breadcrumbs.enabled":"true","breadcrumbs.filePath":null,"[cpp]":{"breadcrumbs.enabled":"false"},"[cpp][rust]":{"breadcrumbs.filePath":false},"[python]":{"breadcrumbs.enabled":true}}"#).unwrap();
        let settings = Settings::load_editor(&[user, workspace]).unwrap();
        for key in ["breadcrumbs.enabled", "breadcrumbs.filePath"] {
            assert_eq!(
                settings.root_write_target(key, "cpp").unwrap(),
                RootWriteScope::User
            );
        }
        assert!(!settings.breadcrumbs("cpp").enabled);
        assert_eq!(settings.breadcrumbs("cpp").file_path, BreadcrumbPath::Last);
        assert!(
            settings
                .root_write_target("breadcrumbs.enabled", "python")
                .is_err()
        );
    }
    #[test]
    fn root_writes_refuse_the_exact_composite_or_single_language_winner_even_when_equal_to_root() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user.json");
        let workspace = root.path().join("workspace.json");
        std::fs::write(&user,r#"{"breadcrumbs.enabled":false,"[cpp][rust]":{"breadcrumbs.enabled":true},"[rust][cpp]":{"breadcrumbs.enabled":false},"[cpp]":{"breadcrumbs.enabled":false}}"#).unwrap();
        std::fs::write(&workspace,r#"{"breadcrumbs.enabled":false,"[cpp][rust]":{"breadcrumbs.enabled":true},"[cpp]":{"breadcrumbs.enabled":"bad"}}"#).unwrap();
        let paths = [user, workspace.clone()];
        let settings = Settings::load_editor(&paths).unwrap();
        assert!(!settings.breadcrumbs("cpp").enabled);
        assert!(!settings.breadcrumbs("rust").enabled);
        let cpp = settings
            .value_scoped_source("breadcrumbs.enabled", "cpp", 2)
            .unwrap();
        assert_eq!(cpp.selector, Some("[cpp]"));
        assert_eq!(cpp.layer, 0);
        let rust = settings
            .value_scoped_source("breadcrumbs.enabled", "rust", 2)
            .unwrap();
        assert_eq!(rust.selector, Some("[rust][cpp]"));
        assert_eq!(rust.layer, 0);
        assert!(
            settings
                .root_write_target("breadcrumbs.enabled", "cpp")
                .unwrap_err()
                .to_string()
                .contains("[cpp]")
        );
        assert!(
            settings
                .root_write_target("breadcrumbs.enabled", "rust")
                .unwrap_err()
                .to_string()
                .contains("[rust][cpp]")
        );
        assert_eq!(
            settings
                .root_write_target("breadcrumbs.enabled", "plaintext")
                .unwrap(),
            RootWriteScope::Workspace
        );
        std::fs::write(
            workspace,
            r#"{"breadcrumbs.enabled":false,"[cpp]":{"breadcrumbs.enabled":false}}"#,
        )
        .unwrap();
        let settings = Settings::load_editor(&paths).unwrap();
        let cpp = settings
            .value_scoped_source("breadcrumbs.enabled", "cpp", 2)
            .unwrap();
        assert_eq!(cpp.layer, 1);
        assert_eq!(cpp.selector, Some("[cpp]"));
        assert!(
            settings
                .root_write_target("breadcrumbs.enabled", "cpp")
                .is_err()
        );
    }
    #[test]
    fn breadcrumb_defaults_are_enabled_with_both_paths() {
        assert_eq!(
            Settings::default().breadcrumbs("cpp"),
            Breadcrumbs::default()
        );
        assert_eq!(
            Settings::default().breadcrumbs("plaintext"),
            Breadcrumbs::default()
        );
    }
    #[test]
    fn breadcrumbs_use_jsonc_workspace_and_language_precedence_without_rewriting_imports() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("vscode-user.json");
        let workspace = root.path().join("workspace.json");
        let user_bytes = br#"{
            // Preserve the user's original comments and trailing commas.
            "breadcrumbs.enabled":false,
            "breadcrumbs.filePath":"last",
            "[cpp][rust]":{"breadcrumbs.enabled":true,"breadcrumbs.symbolPath":"last"},
            "[cpp]":{"breadcrumbs.filePath":"off"},
        }"#;
        let workspace_bytes = br#"{
            "breadcrumbs.enabled":true,
            "breadcrumbs.filePath":"on",
            "breadcrumbs.symbolPath":"off",
            "[cpp][rust]":{"breadcrumbs.symbolPath":"on"},
            "[cpp]":{"breadcrumbs.enabled":false,"breadcrumbs.symbolPath":"last"},
        }"#;
        std::fs::write(&user, user_bytes).unwrap();
        std::fs::write(&workspace, workspace_bytes).unwrap();
        let settings = Settings::load_editor(&[user.clone(), workspace.clone()]).unwrap();
        assert_eq!(
            settings.breadcrumbs("cpp"),
            Breadcrumbs {
                enabled: false,
                file_path: BreadcrumbPath::Off,
                symbol_path: BreadcrumbPath::Last,
            }
        );
        assert_eq!(settings.breadcrumbs("rust"), Breadcrumbs::default());
        assert_eq!(
            settings.breadcrumbs("plaintext"),
            Breadcrumbs {
                enabled: true,
                file_path: BreadcrumbPath::On,
                symbol_path: BreadcrumbPath::Off,
            }
        );
        assert!(settings.warnings.is_empty());
        assert_eq!(std::fs::read(user).unwrap(), user_bytes);
        assert_eq!(std::fs::read(workspace).unwrap(), workspace_bytes);
    }
    #[test]
    fn malformed_breadcrumb_values_preserve_lower_layers_and_remain_visible_to_extensions() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user.json");
        let workspace = root.path().join("workspace.json");
        std::fs::write(&user,r#"{"breadcrumbs.enabled":false,"breadcrumbs.filePath":"last","breadcrumbs.symbolPath":"off","[cpp]":{"breadcrumbs.filePath":"off"}}"#).unwrap();
        std::fs::write(&workspace,r#"{"breadcrumbs.enabled":"true","breadcrumbs.filePath":true,"breadcrumbs.symbolPath":"ALL","[cpp]":{"breadcrumbs.enabled":0,"breadcrumbs.filePath":"LAST","breadcrumbs.symbolPath":null}}"#).unwrap();
        let settings = Settings::load_editor(&[user, workspace]).unwrap();
        assert_eq!(
            settings.breadcrumbs("cpp"),
            Breadcrumbs {
                enabled: false,
                file_path: BreadcrumbPath::Off,
                symbol_path: BreadcrumbPath::Off,
            }
        );
        assert_eq!(
            settings.breadcrumbs("plaintext"),
            Breadcrumbs {
                enabled: false,
                file_path: BreadcrumbPath::Last,
                symbol_path: BreadcrumbPath::Off,
            }
        );
        assert_eq!(settings.warnings.len(), 6);
        assert!(
            settings
                .warnings
                .iter()
                .all(|warning| warning.contains("invalid value for breadcrumbs."))
        );
        assert_eq!(
            settings.extension_layers()[1]["breadcrumbs.enabled"],
            "true"
        );
        assert!(settings.extension_layers()[1]["breadcrumbs.filePath"].is_boolean());
        assert!(settings.extension_layers()[1]["[cpp]"]["breadcrumbs.symbolPath"].is_null());
    }
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
        assert!(settings.warnings.is_empty());
        assert_eq!(
            settings.auto_save("rust"),
            crate::autosave::Policy::AfterDelay { delay_ms: 1000 }
        );
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
        assert_eq!(cpp.indent, AutoIndent::Full);
        assert!(settings.parameter_hints("cpp").enabled);
        assert!(!settings.parameter_hints("cpp").cycle);
        assert!(!settings.parameter_hints("json").enabled);
        assert_eq!(
            settings.typing("json").brackets,
            AutoClosing::BeforeWhitespace
        );
        assert!(settings.warnings.is_empty());
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

#[cfg(test)]
mod editor_preview_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn preview_policy_uses_root_layers_and_preserves_imported_language_values() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user.json");
        let workspace = root.path().join("workspace.json");
        let original = r#"{"workbench.editor.enablePreview":false,"workbench.editor.enablePreviewFromQuickOpen":true,"[cpp]":{"workbench.editor.enablePreview":true}}"#;
        std::fs::write(&user, original).unwrap();
        std::fs::write(&workspace, r#"{"workbench.editor.enablePreview":true,"workbench.editor.enablePreviewFromQuickOpen":false,"[cpp]":{"workbench.editor.enablePreviewFromQuickOpen":true}}"#).unwrap();
        let settings = Settings::load_editor(&[user.clone(), workspace]).unwrap();
        assert_eq!(settings.editor_preview(), EditorPreview::default());
        assert_eq!(
            settings.extension_layers()[0]["[cpp]"]["workbench.editor.enablePreview"],
            json!(true)
        );
        assert_eq!(
            settings
                .warnings
                .iter()
                .filter(|warning| warning.contains("language override ignored"))
                .count(),
            2
        );
        assert_eq!(std::fs::read_to_string(user).unwrap(), original);
    }
    #[test]
    fn invalid_preview_layer_values_preserve_valid_root_policy() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first.json");
        let second = root.path().join("second.json");
        std::fs::write(&first, r#"{"workbench.editor.enablePreview":false,"workbench.editor.enablePreviewFromQuickOpen":true}"#).unwrap();
        std::fs::write(&second, r#"{"workbench.editor.enablePreview":"true","workbench.editor.enablePreviewFromQuickOpen":null}"#).unwrap();
        let settings = Settings::load(&[first, second]).unwrap();
        assert_eq!(
            settings.editor_preview(),
            EditorPreview {
                enabled: false,
                from_quick_open: true
            }
        );
        assert_eq!(settings.warnings.len(), 2);
        assert_eq!(
            settings.extension_layers()[1]["workbench.editor.enablePreview"],
            json!("true")
        );
    }
}

#[cfg(test)]
mod sticky_close_policy_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn original_close_policy_values_have_distinct_keyboard_and_mouse_semantics() {
        assert_eq!(
            Settings::default().prevent_pinned_editor_close(),
            PreventPinnedEditorClose::KeyboardAndMouse
        );
        for (text, policy, keyboard, mouse) in [
            (
                "keyboardAndMouse",
                PreventPinnedEditorClose::KeyboardAndMouse,
                true,
                true,
            ),
            ("never", PreventPinnedEditorClose::Never, false, false),
            ("mouse", PreventPinnedEditorClose::Mouse, false, true),
            ("keyboard", PreventPinnedEditorClose::Keyboard, true, false),
        ] {
            let settings = Settings::from_values(
                json!({"workbench.editor.preventPinnedEditorClose":text})
                    .as_object()
                    .unwrap()
                    .clone(),
                "fixture",
            )
            .unwrap();
            assert_eq!(settings.prevent_pinned_editor_close(), policy);
            assert_eq!(policy.protects_keyboard(), keyboard);
            assert_eq!(policy.protects_mouse(), mouse);
            assert!(settings.warnings.is_empty());
        }
    }
    #[test]
    fn sticky_root_precedence_ignores_language_overrides_without_rewriting_imports() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user.json");
        let workspace = root.path().join("workspace.json");
        let original = "// imported 猫🙂\r\n{\r\n\"workbench.editor.preventPinnedEditorClose\":\"mouse\",\r\n\"[cpp]\":{\"workbench.editor.preventPinnedEditorClose\":\"never\"},\r\n}\r\n";
        std::fs::write(&user, original).unwrap();
        std::fs::write(&workspace, r#"{"workbench.editor.preventPinnedEditorClose":"keyboard","[cpp][rust]":{"workbench.editor.preventPinnedEditorClose":"never"},"[rust]":{"workbench.editor.preventPinnedEditorClose":"mouse"}}"#).unwrap();
        let settings = Settings::load_editor(&[user.clone(), workspace]).unwrap();
        assert_eq!(
            settings.prevent_pinned_editor_close(),
            PreventPinnedEditorClose::Keyboard
        );
        assert_eq!(
            settings.extension_layers()[0]["[cpp]"]["workbench.editor.preventPinnedEditorClose"],
            "never"
        );
        assert_eq!(
            settings.extension_layers()[1]["[cpp][rust]"]["workbench.editor.preventPinnedEditorClose"],
            "never"
        );
        assert_eq!(
            settings
                .warnings
                .iter()
                .filter(|warning| warning.contains("language override ignored"))
                .count(),
            3
        );
        assert_eq!(std::fs::read(user).unwrap(), original.as_bytes());
    }
    #[test]
    fn malformed_higher_close_policy_preserves_valid_lower_value_and_warns() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user.json");
        let workspace = root.path().join("workspace.json");
        std::fs::write(
            &user,
            r#"{"workbench.editor.preventPinnedEditorClose":"never"}"#,
        )
        .unwrap();
        for invalid in [
            Value::Null,
            json!(true),
            json!([]),
            json!(0),
            json!("Keyboard"),
            json!("mouse "),
        ] {
            std::fs::write(
                &workspace,
                serde_json::to_vec(&json!({"workbench.editor.preventPinnedEditorClose":invalid}))
                    .unwrap(),
            )
            .unwrap();
            let settings = Settings::load_editor(&[user.clone(), workspace.clone()]).unwrap();
            assert_eq!(
                settings.prevent_pinned_editor_close(),
                PreventPinnedEditorClose::Never
            );
            assert_eq!(settings.warnings.len(), 1);
            assert!(
                settings.warnings[0]
                    .contains("invalid value for workbench.editor.preventPinnedEditorClose")
            );
            assert_eq!(
                settings.extension_layers()[1]["workbench.editor.preventPinnedEditorClose"],
                invalid
            );
        }
    }
    #[test]
    fn tab_transfer_root_direction_ignores_language_and_invalid_higher_values() {
        let directory = tempfile::tempdir().unwrap();
        let user = directory.path().join("user.json");
        let workspace = directory.path().join("workspace.json");
        let original = "// preserve 猫🙂\r\n{\r\n\"workbench.editor.openSideBySideDirection\":\"down\",\r\n\"[cpp]\":{\"workbench.editor.openSideBySideDirection\":\"right\"},\r\n}\r\n";
        std::fs::write(&user, original).unwrap();
        std::fs::write(&workspace, r#"{"workbench.editor.openSideBySideDirection":17,"[cpp]":{"workbench.editor.closeEmptyGroups":false}}"#).unwrap();
        let settings = Settings::load_editor(&[user.clone(), workspace.clone()]).unwrap();
        assert_eq!(
            settings.tab_transfer_direction().unwrap(),
            EditorSideBySideDirection::Down
        );
        assert!(settings.warnings.iter().any(|warning| {
            warning.contains("invalid value for workbench.editor.openSideBySideDirection")
        }));
        assert_eq!(
            settings
                .warnings
                .iter()
                .filter(|warning| warning.contains("language override ignored"))
                .count(),
            2
        );
        assert_eq!(std::fs::read(&user).unwrap(), original.as_bytes());
        std::fs::write(
            &workspace,
            r#"{"workbench.editor.openSideBySideDirection":"right"}"#,
        )
        .unwrap();
        assert_eq!(
            Settings::load_editor(&[user, workspace])
                .unwrap()
                .tab_transfer_direction()
                .unwrap(),
            EditorSideBySideDirection::Right
        );
    }
    #[test]
    fn tab_transfer_unsupported_valid_policy_refuses_but_malformed_values_fall_back() {
        assert_eq!(
            Settings::default().tab_transfer_direction().unwrap(),
            EditorSideBySideDirection::Right
        );
        for values in [
            json!({"workbench.editor.openPositioning":"left"}),
            json!({"workbench.editor.openPositioning":"first"}),
            json!({"workbench.editor.openPositioning":"last"}),
            json!({"workbench.editor.closeEmptyGroups":false}),
        ] {
            let settings =
                Settings::from_values(values.as_object().unwrap().clone(), "fixture").unwrap();
            assert!(settings.tab_transfer_direction().is_err());
        }
        let settings = Settings::from_values(json!({"workbench.editor.openPositioning":false,"workbench.editor.closeEmptyGroups":"false","workbench.editor.openSideBySideDirection":"left"}).as_object().unwrap().clone(), "fixture").unwrap();
        assert_eq!(
            settings.tab_transfer_direction().unwrap(),
            EditorSideBySideDirection::Right
        );
        assert_eq!(settings.warnings.len(), 3);
    }
}
