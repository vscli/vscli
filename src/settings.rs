//! A validated, explicit subset of VS Code settings and scope precedence.
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
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
#[derive(Clone, Default, PartialEq)]
pub struct Settings {
    layers: Vec<Map<String, Value>>,
    pub warnings: Vec<String>,
}
const SUPPORTED: &[&str] = &[
    "editor.tabSize",
    "editor.insertSpaces",
    "editor.lineNumbers",
];
fn read(path: &Path) -> Result<Map<String, Value>> {
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
    let value: Value = json5::from_str(&text)
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
            result.layers.push(layer);
        }
        Ok(result)
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
                self.warnings
                    .push(format!("{source}: unsupported setting {key}"));
            } else if !valid(key, value) {
                self.warnings
                    .push(format!("{source}: invalid value for {key}"));
            }
        }
    }
    fn value(&self, key: &str, language: &str) -> Option<&Value> {
        let mut result = None;
        for layer in &self.layers {
            if let Some(value) = layer.get(key).filter(|v| valid(key, v)) {
                result = Some(value);
            }
        }
        // Multi-language blocks are less specific than a single-language block,
        // even if the multi-language block belongs to a later settings scope.
        for single in [false, true] {
            for layer in &self.layers {
                for (selector, value) in layer {
                    if let Some(inner) =
                        selector.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
                    {
                        let languages: Vec<_> = inner.split("][").collect();
                        if (languages.len() == 1) == single
                            && languages.contains(&language)
                            && let Some(value) = value.get(key).filter(|v| valid(key, v))
                        {
                            result = Some(value);
                        }
                    }
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
