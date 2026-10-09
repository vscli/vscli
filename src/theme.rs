//! Native VS Code color-theme reader. Scope colors map onto native syntax categories.
use anyhow::{Context, Result, bail};
use ratatui::style::Color;
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug)]
pub struct Colors {
    pub background: Color,
    pub panel: Color,
    pub foreground: Color,
    pub muted: Color,
    pub accent: Color,
    pub selection: Color,
    pub current_line: Color,
}
#[derive(Clone)]
pub struct Theme {
    pub name: String,
    pub colors: Colors,
    pub tokens: [Color; 14],
    pub warnings: Vec<String>,
}
impl Default for Theme {
    fn default() -> Self {
        let colors = Colors {
            background: Color::Rgb(20, 24, 33),
            panel: Color::Rgb(27, 32, 43),
            foreground: Color::Rgb(214, 222, 235),
            muted: Color::Rgb(116, 131, 154),
            accent: Color::Rgb(100, 175, 255),
            selection: Color::Rgb(47, 73, 112),
            current_line: Color::Rgb(26, 31, 42),
        };
        Self {
            name: "VSCLI Dark".into(),
            colors,
            tokens: [
                Color::Rgb(105, 145, 116),
                Color::Rgb(218, 180, 137),
                Color::Rgb(181, 205, 154),
                Color::Rgb(181, 205, 154),
                Color::Rgb(197, 134, 192),
                Color::Rgb(220, 220, 170),
                Color::Rgb(78, 201, 176),
                colors.foreground,
                Color::Rgb(156, 220, 254),
                Color::Rgb(197, 134, 192),
                colors.foreground,
                Color::Rgb(156, 220, 254),
                Color::Rgb(220, 220, 170),
                Color::Rgb(218, 180, 137),
            ],
            warnings: Vec::new(),
        }
    }
}
impl Theme {
    pub fn light() -> Self {
        let mut theme = Self::default();
        theme.name = "VSCLI Light".into();
        theme.colors = Colors {
            background: Color::Rgb(255, 255, 255),
            panel: Color::Rgb(243, 243, 243),
            foreground: Color::Rgb(30, 30, 30),
            muted: Color::Rgb(110, 110, 110),
            accent: Color::Rgb(0, 100, 190),
            selection: Color::Rgb(173, 214, 255),
            current_line: Color::Rgb(245, 245, 245),
        };
        theme.tokens = [
            Color::Rgb(0, 128, 0),
            Color::Rgb(163, 21, 21),
            Color::Rgb(9, 134, 88),
            Color::Rgb(9, 134, 88),
            Color::Rgb(0, 0, 255),
            Color::Rgb(121, 94, 38),
            Color::Rgb(38, 127, 153),
            theme.colors.foreground,
            Color::Rgb(0, 16, 128),
            Color::Rgb(0, 0, 255),
            theme.colors.foreground,
            Color::Rgb(0, 16, 128),
            Color::Rgb(121, 94, 38),
            Color::Rgb(163, 21, 21),
        ];
        theme
    }
    pub fn load(path: &Path) -> Result<Self> {
        let mut visited = HashSet::new();
        let mut bytes = 0;
        load(path, &mut visited, &mut bytes, None)
    }
    pub fn load_confined(path: &Path, root: &Path) -> Result<Self> {
        let root = fs::canonicalize(root)?;
        load(path, &mut HashSet::new(), &mut 0, Some(&root))
    }
    pub fn token(&self, style: usize) -> Color {
        self.tokens
            .get(style)
            .copied()
            .unwrap_or(self.colors.foreground)
    }
}
fn load(
    path: &Path,
    visited: &mut HashSet<PathBuf>,
    bytes: &mut usize,
    root: Option<&Path>,
) -> Result<Theme> {
    if visited.len() >= 8 {
        bail!("Theme includes exceed 8 files");
    }
    let path =
        fs::canonicalize(path).with_context(|| format!("Cannot open theme {}", path.display()))?;
    if root.is_some_and(|root| !path.starts_with(root)) {
        bail!("Theme include escapes its extension package");
    }
    if !path.is_file() {
        bail!("Theme must be a regular file");
    }
    if !visited.insert(path.clone()) {
        bail!("Theme include cycle at {}", path.display());
    }
    let mut raw = Vec::new();
    let remaining = (4 * 1024 * 1024_usize).saturating_sub(*bytes);
    if remaining == 0 {
        bail!("Theme exhausted its 4 MiB read budget");
    }
    fs::File::open(&path)?
        .take((1024 * 1024 + 1).min(remaining) as u64)
        .read_to_end(&mut raw)?;
    *bytes += raw.len();
    if raw.len() > 1024 * 1024 || raw.len() == remaining {
        bail!("Theme exceeds file/read budget (1 MiB/file, 4 MiB total)");
    }
    let value: Value =
        json5::from_str(std::str::from_utf8(&raw)?).context("Invalid theme JSONC")?;
    if !value.is_object() {
        bail!("Theme must contain an object");
    }
    for field in ["name", "type", "include"] {
        if value.get(field).is_some_and(|value| !value.is_string()) {
            bail!("Theme {field} must be a string");
        }
    }
    if value.get("colors").is_some_and(|value| !value.is_object()) {
        bail!("Theme colors must contain an object");
    }
    if value
        .get("tokenColors")
        .is_some_and(|value| !value.is_array() && !value.is_string())
    {
        bail!("Theme tokenColors must contain an array or tmTheme path");
    }
    let mut theme = if let Some(include) = value.get("include").and_then(Value::as_str) {
        load(&path.parent().unwrap().join(include), visited, bytes, root)?
    } else if value.get("type").and_then(Value::as_str) == Some("light") {
        Theme::light()
    } else {
        Theme::default()
    };
    theme.name = value
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        });
    if let Some(colors) = value.get("colors").and_then(Value::as_object) {
        let previous_foreground = theme.colors.foreground;
        // Resolve background first so transparent overlays blend against the right base.
        if let Some(color) = colors.get("editor.background").and_then(Value::as_str) {
            theme.colors.background =
                color_value(color, theme.colors.background).context("Invalid editor.background")?;
        }
        let mut unsupported = Vec::new();
        for (key, value) in colors {
            let target = match key.as_str() {
                "editor.background" => continue,
                "editor.foreground" => &mut theme.colors.foreground,
                "sideBar.background" | "editorWidget.background" => &mut theme.colors.panel,
                "editorLineNumber.foreground" => &mut theme.colors.muted,
                "focusBorder" | "textLink.foreground" => &mut theme.colors.accent,
                "editor.selectionBackground" => &mut theme.colors.selection,
                "editor.lineHighlightBackground" => &mut theme.colors.current_line,
                _ => {
                    unsupported.push(key.as_str());
                    continue;
                }
            };
            *target = color_value(
                value.as_str().context("Theme color must be a string")?,
                theme.colors.background,
            )
            .with_context(|| format!("Invalid color for {key}"))?;
        }
        if !unsupported.is_empty() {
            theme.warnings.push(format!(
                "{} workbench color keys are not mapped: {}{}",
                unsupported.len(),
                unsupported
                    .iter()
                    .take(6)
                    .copied()
                    .collect::<Vec<_>>()
                    .join(", "),
                if unsupported.len() > 6 { ", …" } else { "" }
            ));
        }
        for index in [7, 10] {
            if theme.tokens[index] == previous_foreground {
                theme.tokens[index] = theme.colors.foreground;
            }
        }
    }
    if let Some(rules) = value.get("tokenColors").and_then(Value::as_array) {
        theme.warnings.push("Token scopes use an approximate mapping to 14 native syntax categories; full TextMate scope matching is not implemented".into());
        if rules
            .iter()
            .any(|rule| rule.pointer("/settings/fontStyle").is_some())
        {
            theme
                .warnings
                .push("Token fontStyle rules are preserved in source but are not applied".into());
        }
        for rule in rules {
            let Some(settings) = rule.get("settings") else {
                continue;
            };
            let Some(foreground) = settings.get("foreground").and_then(Value::as_str) else {
                continue;
            };
            let color = color_value(foreground, theme.colors.background)
                .context("Invalid token foreground")?;
            let scopes: Vec<_> = match rule.get("scope") {
                Some(Value::String(scope)) => scope.split(',').map(str::trim).collect(),
                Some(Value::Array(scopes)) => scopes.iter().filter_map(Value::as_str).collect(),
                _ => Vec::new(),
            };
            if scopes.is_empty() {
                theme.colors.foreground = color;
                theme.tokens.fill(color);
            }
            for scope in scopes {
                for (index, candidates) in TOKEN_SCOPES.iter().enumerate() {
                    if candidates.iter().any(|candidate| {
                        *candidate == scope
                            || candidate
                                .strip_prefix(scope)
                                .is_some_and(|s| s.starts_with('.'))
                    }) {
                        theme.tokens[index] = color;
                    }
                }
            }
        }
    }
    if value.get("semanticTokenColors").is_some() {
        theme.warnings.push(
            "Semantic token colors require semantic highlighting, which is not implemented".into(),
        );
    }
    if value.get("tokenColors").is_some_and(Value::is_string) {
        theme
            .warnings
            .push("External tmTheme token files are not supported".into());
    }
    Ok(theme)
}
const TOKEN_SCOPES: &[&[&str]] = &[
    &["comment"],
    &["string"],
    &["constant.numeric"],
    &["constant"],
    &["keyword", "storage"],
    &["entity.name.function", "support.function"],
    &["entity.name.type", "support.type", "support.class"],
    &["variable"],
    &["variable.other.property", "support.variable.property"],
    &["keyword.operator"],
    &["punctuation"],
    &["entity.other.attribute-name"],
    &["entity.name.function.constructor"],
    &["constant.character.escape"],
];
pub fn color_value(text: &str, background: Color) -> Result<Color> {
    let raw = text
        .strip_prefix('#')
        .context("Expected #RGB, #RGBA, #RRGGBB or #RRGGBBAA")?;
    let expanded = match raw.len() {
        3 | 4 => raw.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 | 8 => raw.into(),
        _ => bail!("Invalid hex color length"),
    };
    if !expanded.is_ascii() {
        bail!("Invalid hex color");
    }
    let byte = |i| u8::from_str_radix(&expanded[i..i + 2], 16).context("Invalid hex color");
    let (red, green, blue) = (byte(0)?, byte(2)?, byte(4)?);
    if expanded.len() == 6 {
        return Ok(Color::Rgb(red, green, blue));
    }
    let alpha = byte(6)? as u32;
    let Color::Rgb(br, bg, bb) = background else {
        return Ok(Color::Rgb(red, green, blue));
    };
    let blend = |a: u8, b: u8| ((a as u32 * alpha + b as u32 * (255 - alpha) + 127) / 255) as u8;
    Ok(Color::Rgb(
        blend(red, br),
        blend(green, bg),
        blend(blue, bb),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn themes_load_includes_overrides_token_scopes_and_alpha() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("base.json"), r##"{"type":"light","colors":{"editor.background":"#fff"},"tokenColors":[{"scope":"keyword,storage","settings":{"foreground":"#123456"}}]}"##).unwrap();
        fs::write(root.path().join("theme.json"), r##"{// comment
          "include":"base.json","name":"Test","colors":{"editor.foreground":"#010203","editor.selectionBackground":"#0008"},"tokenColors":[{"scope":["string"],"settings":{"foreground":"#abcdef"}}],}"##).unwrap();
        let theme = Theme::load(&root.path().join("theme.json")).unwrap();
        assert_eq!(theme.name, "Test");
        assert_eq!(theme.colors.foreground, Color::Rgb(1, 2, 3));
        assert_eq!(theme.colors.selection, Color::Rgb(119, 119, 119));
        assert_eq!(theme.token(4), Color::Rgb(18, 52, 86));
        assert_eq!(theme.token(1), Color::Rgb(171, 205, 239));
        assert_eq!(theme.token(7), Color::Rgb(1, 2, 3));
        fs::write(
            root.path().join("base.json"),
            r##"{"tokenColors":[{"scope":"variable","settings":{"foreground":"#123456"}}]}"##,
        )
        .unwrap();
        let theme = Theme::load(&root.path().join("theme.json")).unwrap();
        assert_eq!(theme.token(7), Color::Rgb(18, 52, 86));
        assert_eq!(theme.token(10), Color::Rgb(1, 2, 3));
    }
    #[test]
    fn compatibility_notices_identify_unmapped_colors_font_styles_and_semantics() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("theme.json");
        fs::write(&path, r##"{"colors":{"activityBar.background":"#ffffff"},"tokenColors":[{"scope":"keyword","settings":{"foreground":"#123456","fontStyle":"italic"}}],"semanticTokenColors":{"class":"#abcdef"}}"##).unwrap();
        let theme = Theme::load(&path).unwrap();
        let report = theme.warnings.join("\n");
        for expected in [
            "activityBar.background",
            "approximate",
            "fontStyle",
            "Semantic",
        ] {
            assert!(report.contains(expected), "{report}");
        }
    }
    #[test]
    fn installed_includes_are_confined_and_preferences_fail_without_replacing_existing_bytes() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("package");
        fs::create_dir(&package).unwrap();
        fs::write(root.path().join("outside.json"), "{}").unwrap();
        let path = package.join("theme.json");
        fs::write(&path, r#"{"include":"../outside.json"}"#).unwrap();
        assert!(
            Theme::load_confined(&path, &package)
                .err()
                .unwrap()
                .to_string()
                .contains("escapes")
        );
        let preference = root.path().join("selection.json");
        fs::write(&preference, "original bytes").unwrap();
        assert!(
            Preference {
                name: "x".repeat(5000),
                path: None
            }
            .save(&preference)
            .is_err()
        );
        assert_eq!(fs::read_to_string(&preference).unwrap(), "original bytes");
        assert!(
            Preference::read(&package)
                .err()
                .unwrap()
                .to_string()
                .contains("regular file")
        );
    }
    #[test]
    fn cyclic_malformed_and_oversized_themes_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("theme.json");
        for text in [
            r#"{"include":"theme.json"}"#.into(),
            r#"{"include":false}"#.into(),
            r#"{"colors":[]}"#.into(),
            r#"{"tokenColors":42}"#.into(),
            r##"{"colors":{"editor.background":"#zzffff"}}"##.into(),
            " ".repeat(1024 * 1024 + 1),
        ] {
            fs::write(&path, text).unwrap();
            assert!(Theme::load(&path).is_err());
        }
        assert!(color_value("#éé", Color::Black).is_err());
    }
}

#[derive(Clone, Debug)]
pub struct Choice {
    pub id: String,
    pub label: String,
    pub path: PathBuf,
    pub root: PathBuf,
}
/// Installed manifests are data; discovering a color contribution never starts its extension.
pub fn contributions(packages: &[(PathBuf, Value)]) -> (Vec<Choice>, Vec<String>) {
    let mut choices = Vec::new();
    let mut warnings = Vec::new();
    for (root, manifest) in packages.iter().take(128) {
        let Some(themes) = manifest
            .pointer("/contributes/themes")
            .and_then(Value::as_array)
        else {
            continue;
        };
        for theme in themes.iter().take(128) {
            let Some(relative) = theme.get("path").and_then(Value::as_str) else {
                continue;
            };
            let label = theme
                .get("label")
                .and_then(Value::as_str)
                .or_else(|| theme.get("id").and_then(Value::as_str))
                .unwrap_or(relative);
            let path = match fs::canonicalize(root.join(relative)).and_then(|path| {
                let root = fs::canonicalize(root)?;
                if path.starts_with(root) {
                    Ok(path)
                } else {
                    Err(std::io::Error::other(
                        "Theme path escapes its installed package",
                    ))
                }
            }) {
                Ok(path) => path,
                Err(error) => {
                    warnings.push(format!("Theme {label:?}: {error}"));
                    continue;
                }
            };
            choices.push(Choice {
                id: theme
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or(label)
                    .into(),
                label: label.into(),
                path,
                root: root.clone(),
            });
            if choices.len() >= 1024 {
                warnings.push("Installed theme catalog limited to 1024 entries".into());
                return (choices, warnings);
            }
        }
    }
    if packages.len() > 128 {
        warnings.push("Theme discovery limited to 128 packages".into());
    }
    (choices, warnings)
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Preference {
    pub name: String,
    pub path: Option<PathBuf>,
}
impl Preference {
    pub fn read(path: &Path) -> Result<Option<Self>> {
        match fs::metadata(path) {
            Ok(metadata) if !metadata.is_file() => bail!("Theme preference must be a regular file"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
            _ => {}
        }
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        file.take(4097).read_to_end(&mut bytes)?;
        if bytes.len() > 4096 {
            bail!("Theme preference exceeds 4 KiB");
        }
        Ok(Some(serde_json::from_slice(&bytes)?))
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        use std::io::Write;
        let parent = path
            .parent()
            .context("Theme preference has no parent directory")?;
        fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        let bytes = serde_json::to_vec_pretty(self)?;
        if bytes.len() > 4096 {
            bail!("Theme preference exceeds 4 KiB");
        }
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|error| error.error)?;
        Ok(())
    }
}
