//! Read-only migration preview and atomic activation of immutable imported profiles.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
const MAX_FILE: usize = 1024 * 1024;
const MAX_TOTAL: usize = 16 * MAX_FILE;

#[derive(Serialize)]
pub struct Report {
    pub source: PathBuf,
    pub files: Vec<PathBuf>,
    pub bytes: usize,
    pub notices: Vec<String>,
    pub activated_profile: Option<PathBuf>,
}
pub struct Preview {
    pub report: Report,
    files: Vec<(PathBuf, Vec<u8>)>,
    theme: Option<(String, PathBuf)>,
}
#[derive(Serialize, Deserialize)]
struct Active {
    schema: u32,
    profile: String,
}

pub fn config_directory() -> Option<PathBuf> {
    crate::recovery::config_path().and_then(|path| path.parent().map(Path::to_path_buf))
}
/// A pointer may select only an immediate immutable profile under imports/.
pub fn active_directory(root: &Path) -> Result<PathBuf> {
    let pointer = root.join("active-profile.json");
    match fs::symlink_metadata(&pointer) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(root.into()),
        Err(error) => return Err(error.into()),
        Ok(metadata) if !metadata.file_type().is_file() => {
            bail!("Imported-profile pointer must be a regular file")
        }
        _ => {}
    }
    let bytes = read(&pointer, 4096)?;
    let active: Active =
        serde_json::from_slice(&bytes).context("Invalid imported-profile pointer")?;
    if active.schema != 1
        || active.profile.len() != 36
        || uuid::Uuid::parse_str(&active.profile).is_err()
    {
        bail!("Invalid imported-profile identifier");
    }
    let imports = root.join("imports");
    managed_directory(&imports)?;
    let directory = imports.join(active.profile);
    if !fs::symlink_metadata(&directory)?.file_type().is_dir() {
        bail!(
            "Active imported profile is missing: {}",
            directory.display()
        );
    }
    Ok(fs::canonicalize(directory)?)
}

pub fn preview(source: &Path, profile: crate::keys::Profile) -> Result<Preview> {
    preview_with_extensions(source, profile, None)
}
pub fn preview_with_extensions(
    source: &Path,
    profile: crate::keys::Profile,
    extensions: Option<&Path>,
) -> Result<Preview> {
    let source = fs::canonicalize(source).context("Cannot open VS Code User directory")?;
    if !source.is_dir() {
        bail!("VS Code User path must be a directory");
    }
    let mut paths = Vec::new();
    for name in ["settings.json", "keybindings.json"] {
        if source.join(name).is_file() {
            paths.push(PathBuf::from(name));
        }
    }
    let snippets = source.join("snippets");
    if snippets.is_dir() {
        for (index, entry) in fs::read_dir(&snippets)?.enumerate() {
            if index >= 4096 {
                bail!("Snippet directory exceeds 4096 entries");
            }
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_file()
                && matches!(
                    path.extension().and_then(|v| v.to_str()),
                    Some("json" | "code-snippets")
                )
            {
                paths.push(PathBuf::from("snippets").join(entry.file_name()));
            }
        }
    }
    if paths.is_empty() {
        bail!(
            "No settings.json, keybindings.json or snippet files in {}",
            source.display()
        );
    }
    if paths.len() > 130 {
        bail!("Import exceeds 128 snippet files");
    }
    paths.sort();
    let mut result = Preview {
        report: Report {
            source: source.clone(),
            files: paths.clone(),
            bytes: 0,
            notices: Vec::new(),
            activated_profile: None,
        },
        files: Vec::new(),
        theme: None,
    };
    let mut selected_theme = None;
    let native = crate::keys::Keymap::new(profile);
    for path in paths {
        let bytes = read(
            &source.join(&path),
            MAX_FILE.min(MAX_TOTAL.saturating_sub(result.report.bytes)),
        )?;
        result.report.bytes += bytes.len();
        let text = std::str::from_utf8(&bytes).context("Imported config is not UTF-8")?;
        let value: Value = json5::from_str(text)
            .with_context(|| format!("Invalid JSONC in {}", path.display()))?;
        if path == Path::new("settings.json") {
            if !value.is_object() {
                bail!("settings.json must contain an object");
            }
            let settings = crate::settings::Settings::from_values(
                value.as_object().unwrap().clone(),
                &path.display().to_string(),
            )?;
            result.report.notices.extend(settings.warnings);
            if let Some(theme) = value.get("workbench.colorTheme").and_then(Value::as_str) {
                selected_theme = Some(theme.to_owned());
            }
        } else if path == Path::new("keybindings.json") {
            let bindings: Vec<crate::keys::Binding> =
                serde_json::from_value(value).context("Invalid keybinding entries")?;
            for binding in bindings {
                if let Err(error) = crate::keys::validate_bindings(std::slice::from_ref(&binding)) {
                    result.report.notices.push(format!("Keybinding {:?} is preserved but skipped by the native resolver: {error:#}", binding.key));
                }
                let command = binding.command.trim_start_matches('-');
                if !native.bindings.iter().any(|rule| rule.command == command)
                    && !crate::app::COMMANDS.iter().any(|(_, id)| *id == command)
                {
                    result.report.notices.push(format!(
                        "Keybinding command {command:?} has no native implementation; preserved"
                    ));
                }
            }
            result.report.notices.push("Imported shortcut contexts and physical terminal delivery require qualification; preserving a rule does not implement its command".into());
        } else if !value.is_object() {
            bail!("Snippet file {} must contain an object", path.display());
        }
        result.files.push((path, bytes));
    }
    if let Some(name) = selected_theme {
        let default_extensions =
            directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".vscode/extensions"));
        let directory = extensions.or(default_extensions.as_deref());
        if let Some(directory) = directory {
            match snapshot_theme(directory, &name) {
                Ok(Some((relative, files, warnings))) => {
                    result.report.notices.extend(warnings);
                    for (path, bytes) in files {
                        result.report.bytes += bytes.len();
                        if result.report.bytes > MAX_TOTAL { bail!("Import exceeds 16 MiB including selected theme"); }
                        result.report.files.push(path.clone());
                        result.files.push((path, bytes));
                    }
                    result.theme = Some((name.clone(), relative));
                    result.report.notices.push(format!("Selected theme {name:?} will be copied as native theme data; extension code is not executed"));
                }
                Ok(None) => result.report.notices.push(format!("Selected theme {name:?} was not found; install its theme extension or load a color-theme JSON")),
                Err(error) => result.report.notices.push(format!("Selected theme {name:?} could not be imported: {error:#}")),
            }
        }
    }
    result.report.notices.push("Original files and unknown settings are retained byte-for-byte. Extension code, profiles, accounts and sync are not executed or imported by this operation.".into());
    Ok(result)
}
impl Preview {
    /// Prepare every file before activating the profile with a single atomic pointer replacement.
    pub fn apply(mut self, root: &Path) -> Result<Report> {
        let root = destination_path(root)?;
        if root.starts_with(&self.report.source) {
            bail!("Import destination must be outside the original VS Code User directory");
        }
        fs::create_dir_all(&root)?;
        let imports = root.join("imports");
        if imports.exists() || fs::symlink_metadata(&imports).is_ok() {
            managed_directory(&imports)?;
        } else {
            fs::create_dir(&imports)?;
        }
        let staging = tempfile::Builder::new()
            .prefix(".staging-")
            .tempdir_in(&imports)?;
        for (relative, bytes) in &self.files {
            let path = staging.path().join(relative);
            fs::create_dir_all(path.parent().unwrap())?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let destination = imports.join(&id);
        if let Some((name, relative)) = &self.theme {
            crate::theme::Preference {
                name: name.clone(),
                path: Some(destination.join(relative)),
            }
            .save(&staging.path().join("theme-selection.json"))?;
        }
        fs::rename(staging.path(), &destination)?;
        let pointer = Active {
            schema: 1,
            profile: id,
        };
        let mut file = tempfile::NamedTempFile::new_in(&root)?;
        file.write_all(&serde_json::to_vec_pretty(&pointer)?)?;
        file.as_file().sync_all()?;
        file.persist(root.join("active-profile.json"))
            .map_err(|error| error.error)?;
        self.report.activated_profile = Some(destination);
        Ok(self.report)
    }
}
fn managed_directory(path: &Path) -> Result<()> {
    if !fs::symlink_metadata(path)?.file_type().is_dir() {
        bail!(
            "Managed import path must be a directory, not a symlink: {}",
            path.display()
        );
    }
    Ok(())
}
fn destination_path(path: &Path) -> Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(fs::canonicalize(path)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let name = path.file_name().context("Invalid import destination")?;
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            Ok(destination_path(parent)?.join(name))
        }
        Err(error) => Err(error.into()),
    }
}
/// Keep source include bytes portable by refusing aliases or absolute paths.
fn portable_theme_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in relative.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => normalized.push(part),
            std::path::Component::ParentDir if normalized.pop() => {}
            _ => bail!("Imported theme paths must be relative and stay inside the package"),
        }
    }
    let path = root.join(normalized);
    let canonical = fs::canonicalize(&path)?;
    if path != canonical || !canonical.starts_with(root) {
        bail!("Imported theme includes cannot use symlinks or escape the package");
    }
    Ok(canonical)
}

type ThemeSnapshot = (PathBuf, Vec<(PathBuf, Vec<u8>)>, Vec<String>);
fn snapshot_theme(directory: &Path, selected: &str) -> Result<Option<ThemeSnapshot>> {
    if !directory.exists() {
        return Ok(None);
    }
    let mut packages = Vec::new();
    for (index, entry) in fs::read_dir(directory)?.enumerate() {
        if index >= 4096 {
            bail!("VS Code extension directory exceeds 4096 entries");
        }
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            packages.push(entry.path());
        }
        if packages.len() > 128 {
            bail!("Theme discovery exceeds 128 extension packages");
        }
    }
    packages.sort();
    let mut manifest_bytes = 0;
    for package in packages {
        let manifest_path = package.join("package.json");
        if !manifest_path.is_file() {
            continue;
        }
        if manifest_bytes >= MAX_TOTAL {
            bail!("Theme manifest scan reached its 16 MiB read budget");
        }
        let mut raw = Vec::new();
        fs::File::open(&manifest_path)?
            .take((MAX_FILE + 1).min(MAX_TOTAL - manifest_bytes) as u64)
            .read_to_end(&mut raw)?;
        manifest_bytes += raw.len();
        if raw.len() > MAX_FILE {
            continue;
        }
        let manifest: Value = match serde_json::from_slice(&raw) {
            Ok(manifest) => manifest,
            Err(_) => continue,
        };
        let Some(themes) = manifest
            .pointer("/contributes/themes")
            .and_then(Value::as_array)
        else {
            continue;
        };
        for theme in themes.iter().take(128) {
            if theme.get("id").and_then(Value::as_str) != Some(selected)
                && theme.get("label").and_then(Value::as_str) != Some(selected)
            {
                continue;
            }
            let relative = theme
                .get("path")
                .and_then(Value::as_str)
                .context("Theme contribution lacks path")?;
            let root = fs::canonicalize(&package)?;
            let path = portable_theme_path(&root, Path::new(relative))?;
            if !path.starts_with(&root) {
                bail!("Theme contribution escapes its extension package");
            }
            let selected_path = path.clone();
            let target = PathBuf::from("themes/imported").join(path.strip_prefix(&root)?);
            let mut files = Vec::new();
            let mut current = Some(path);
            let mut seen = std::collections::HashSet::new();
            let mut total = 0;
            while let Some(path) = current.take() {
                if files.len() >= 8 || !seen.insert(path.clone()) {
                    bail!("Theme include cycle or file budget exceeded");
                }
                if !path.starts_with(&root) {
                    bail!("Theme include escapes its extension package");
                }
                let bytes = read(&path, MAX_FILE)?;
                total += bytes.len();
                if total > 4 * MAX_FILE {
                    bail!("Theme snapshot exceeds 4 MiB");
                }
                let value: Value = json5::from_str(std::str::from_utf8(&bytes)?)?;
                if let Some(include) = value.get("include").and_then(Value::as_str) {
                    if Path::new(include).is_absolute() {
                        bail!("Absolute theme includes cannot be imported portably");
                    }
                    let relative = path.parent().unwrap().strip_prefix(&root)?.join(include);
                    current = Some(portable_theme_path(&root, &relative)?);
                }
                files.push((
                    PathBuf::from("themes/imported").join(path.strip_prefix(&root)?),
                    bytes,
                ));
            }
            let theme = crate::theme::Theme::load(&selected_path)?;
            return Ok(Some((target, files, theme.warnings)));
        }
    }
    Ok(None)
}

fn read(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        bail!("Import file/read budget exceeded at {}", path.display());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_is_read_only_and_activation_preserves_original_and_previous_profiles() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("User");
        let target = temp.path().join("native");
        fs::create_dir_all(source.join("snippets")).unwrap();
        let settings = b"{// preserve comment\n\"editor.tabSize\":2,\"unknown.setting\":true,}";
        fs::write(source.join("settings.json"), settings).unwrap();
        fs::write(
            source.join("keybindings.json"),
            r#"[{"key":"f9","command":"missing.command"}]"#,
        )
        .unwrap();
        fs::write(
            source.join("snippets/rust.json"),
            r#"{"Sample":{"body":"$0"}}"#,
        )
        .unwrap();
        let plan = preview(&source, crate::keys::Profile::Linux).unwrap();
        assert!(!target.exists());
        assert!(
            plan.report
                .notices
                .iter()
                .any(|notice| notice.contains("unknown.setting"))
        );
        assert!(
            plan.report
                .notices
                .iter()
                .any(|notice| notice.contains("missing.command"))
        );
        let report = plan.apply(&target).unwrap();
        let first = report.activated_profile.unwrap();
        assert_eq!(active_directory(&target).unwrap(), first);
        assert_eq!(fs::read(first.join("settings.json")).unwrap(), settings);
        fs::write(source.join("settings.json"), "{}").unwrap();
        let second = preview(&source, crate::keys::Profile::Linux)
            .unwrap()
            .apply(&target)
            .unwrap()
            .activated_profile
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(active_directory(&target).unwrap(), second);
        assert_eq!(fs::read(first.join("settings.json")).unwrap(), settings);
        assert_eq!(
            fs::read_to_string(source.join("settings.json")).unwrap(),
            "{}"
        );
        assert!(first.join("snippets/rust.json").exists());
    }
    #[test]
    fn selected_extension_theme_and_includes_are_snapshotted_without_source_changes() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("User");
        let extensions = root.path().join("extensions");
        let package = extensions.join("publisher.theme-1.0.0");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(package.join("themes")).unwrap();
        fs::write(
            source.join("settings.json"),
            r#"{"workbench.colorTheme":"Test Theme"}"#,
        )
        .unwrap();
        fs::write(package.join("package.json"), r#"{"main":"must-not-run.js","contributes":{"themes":[{"id":"test","label":"Test Theme","path":"themes/theme.json"}]}}"#).unwrap();
        let original = r##"{// preserve theme comment
          "include":"../base.json","colors":{"editor.foreground":"#abcdef"},}"##;
        fs::write(package.join("themes/theme.json"), original).unwrap();
        fs::write(
            package.join("base.json"),
            r##"{"colors":{"editor.background":"#010203"}}"##,
        )
        .unwrap();
        let plan = preview_with_extensions(&source, crate::keys::Profile::Linux, Some(&extensions))
            .unwrap();
        assert_eq!(plan.report.files.len(), 3);
        let config = root.path().join("native");
        let report = plan.apply(&config).unwrap();
        let selected = report.activated_profile.unwrap();
        let preference = crate::theme::Preference::read(&selected.join("theme-selection.json"))
            .unwrap()
            .unwrap();
        let path = preference.path.unwrap();
        assert!(path.starts_with(&selected));
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        fs::remove_dir_all(&extensions).unwrap();
        let theme = crate::theme::Theme::load(&path).unwrap();
        assert_eq!(theme.colors.background, ratatui::style::Color::Rgb(1, 2, 3));
        assert_eq!(
            theme.colors.foreground,
            ratatui::style::Color::Rgb(171, 205, 239)
        );
        assert_eq!(preference.name, "Test Theme");
        assert!(!selected.join("must-not-run.js").exists());
    }

    #[test]
    fn relative_destinations_freeze_source_bytes_and_failed_publish_retains_native_files() {
        let temp = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let relative = temp
            .path()
            .strip_prefix(std::env::current_dir().unwrap())
            .unwrap();
        let source = temp.path().join("User");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("settings.json"), "{\"editor.tabSize\":2}").unwrap();
        let plan = preview(&source, crate::keys::Profile::Linux).unwrap();
        fs::write(source.join("settings.json"), "{\"editor.tabSize\":8}").unwrap();
        let report = plan.apply(&relative.join("config")).unwrap();
        let snapshot = report.activated_profile.unwrap();
        assert!(snapshot.is_absolute());
        assert_eq!(
            fs::read_to_string(snapshot.join("settings.json")).unwrap(),
            "{\"editor.tabSize\":2}"
        );
        assert_eq!(
            fs::read_to_string(source.join("settings.json")).unwrap(),
            "{\"editor.tabSize\":8}"
        );
        let target = temp.path().join("failed");
        fs::create_dir_all(target.join("active-profile.json")).unwrap();
        fs::write(target.join("active-profile.json/sentinel"), "untouched").unwrap();
        fs::write(target.join("settings.json"), "native settings").unwrap();
        assert!(
            preview(&source, crate::keys::Profile::Linux)
                .unwrap()
                .apply(&target)
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(target.join("settings.json")).unwrap(),
            "native settings"
        );
        assert_eq!(
            fs::read_to_string(target.join("active-profile.json/sentinel")).unwrap(),
            "untouched"
        );
        for entry in fs::read_dir(target.join("imports")).unwrap() {
            let entry = entry.unwrap();
            assert!(!entry.file_name().to_string_lossy().starts_with(".staging-"));
            assert_eq!(
                fs::read_to_string(entry.path().join("settings.json")).unwrap(),
                "{\"editor.tabSize\":8}"
            );
        }
        assert!(
            preview(&source, crate::keys::Profile::Linux)
                .unwrap()
                .apply(&source.join("native"))
                .is_err()
        );
        assert!(!source.join("native").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_import_controls_and_unportable_theme_aliases_are_rejected() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("User");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("settings.json"), "{}").unwrap();
        let native = temp.path().join("native");
        let snapshot = preview(&source, crate::keys::Profile::Linux)
            .unwrap()
            .apply(&native)
            .unwrap()
            .activated_profile
            .unwrap();
        let moved = temp.path().join("moved");
        fs::rename(&snapshot, &moved).unwrap();
        symlink(&moved, &snapshot).unwrap();
        assert!(active_directory(&native).is_err());
        fs::remove_file(&snapshot).unwrap();
        fs::rename(&moved, &snapshot).unwrap();
        let pointer = native.join("active-profile.json");
        fs::rename(&pointer, native.join("pointer-copy")).unwrap();
        symlink(native.join("pointer-copy"), &pointer).unwrap();
        assert!(active_directory(&native).is_err());
        fs::remove_file(&pointer).unwrap();
        fs::rename(native.join("pointer-copy"), &pointer).unwrap();
        let external = temp.path().join("external-imports");
        fs::rename(native.join("imports"), &external).unwrap();
        symlink(&external, native.join("imports")).unwrap();
        assert!(active_directory(&native).is_err());
        assert!(
            preview(&source, crate::keys::Profile::Linux)
                .unwrap()
                .apply(&native)
                .is_err()
        );
        fs::write(temp.path().join("real-theme.json"), "{}").unwrap();
        symlink(
            temp.path().join("real-theme.json"),
            temp.path().join("alias.json"),
        )
        .unwrap();
        let canonical = fs::canonicalize(temp.path()).unwrap();
        assert!(portable_theme_path(&canonical, Path::new("alias.json")).is_err());
        assert!(portable_theme_path(&canonical, &canonical.join("real-theme.json")).is_err());
    }

    #[test]
    fn invalid_input_or_activation_failure_preserves_the_selected_profile() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("User");
        let target = temp.path().join("native");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("settings.json"), "{}").unwrap();
        preview(&source, crate::keys::Profile::Linux)
            .unwrap()
            .apply(&target)
            .unwrap();
        let original = fs::read(target.join("active-profile.json")).unwrap();
        fs::write(source.join("settings.json"), "{invalid}").unwrap();
        assert!(preview(&source, crate::keys::Profile::Linux).is_err());
        assert_eq!(
            fs::read(target.join("active-profile.json")).unwrap(),
            original
        );
        fs::write(
            target.join("active-profile.json"),
            r#"{"schema":1,"profile":"../../outside"}"#,
        )
        .unwrap();
        assert!(active_directory(&target).is_err());
    }
}
