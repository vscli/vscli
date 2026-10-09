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
    pub warnings: Vec<String>,
}
impl PartialEq for Settings {
    fn eq(&self, other: &Self) -> bool {
        self.serialized == other.serialized && self.warnings == other.warnings
    }
}
const SUPPORTED: &[&str] = &[
    "editor.tabSize",
    "editor.insertSpaces",
    "editor.lineNumbers",
    "workbench.colorTheme",
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
        let mut result = Self::default();
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
            }
        }
    }
    fn value(&self, key: &str, language: &str) -> Option<&Value> {
        let mut result = None;
        for layer in self.layers.iter() {
            if let Some(value) = layer.get(key).filter(|v| valid(key, v)) {
                result = Some(value);
            }
        }
        // Merge equal identifier groups in their first-seen position, then apply
        // single-language groups last, as in the pinned configuration model.
        let mut groups: Vec<(Vec<&str>, Option<&Value>)> = Vec::new();
        for layer in self.layers.iter() {
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
fn valid(key: &str, value: &Value) -> bool {
    match key {
        "workbench.colorTheme" => value.is_string(),
        "editor.tabSize" => value.as_u64().is_some_and(|n| (1..=16).contains(&n)),
        "editor.insertSpaces" => value.is_boolean(),
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
                let _ = sender.send(Settings::load(&paths).map_err(|e| format!("{e:#}")));
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
