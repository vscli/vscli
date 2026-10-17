//! Read-only migration preview and atomic activation of versioned imported profile copies.
pub mod application;
pub mod discovery;
pub mod packages;
#[cfg(test)]
mod profile_tests;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<discovery::Source>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<packages::Inventory>,
    pub files: Vec<PathBuf>,
    pub bytes: usize,
    pub notices: Vec<String>,
    pub activated_profile: Option<PathBuf>,
    pub extensions_imported: usize,
    pub extensions_execution_enabled: usize,
}
pub struct Preview {
    pub report: Report,
    files: Vec<(PathBuf, Vec<u8>)>,
    theme: Option<(String, PathBuf)>,
    source_roots: Vec<PathBuf>,
}
#[derive(Serialize, Deserialize, Clone)]
struct Active {
    schema: u32,
    profile: String,
}

pub fn config_directory() -> Option<PathBuf> {
    crate::recovery::config_path().and_then(|path| path.parent().map(Path::to_path_buf))
}
/// A pointer may select only an immediate copied profile under imports/.
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
    preview_layout(source, profile, extensions, None, None)
}
pub fn preview_profile(
    source: &discovery::Source,
    profile: crate::keys::Profile,
) -> Result<Preview> {
    let inventory = packages::inventory(source)?;
    let mut preview = preview_layout(
        &source.user,
        profile,
        Some(&source.extensions),
        Some(source.clone()),
        Some(&inventory),
    )?;
    preview.report.extensions = Some(inventory);
    if let Some(application) = &source.application {
        preview.source_roots.push(application.root.clone());
    }
    Ok(preview)
}
fn preview_layout(
    source: &Path,
    profile: crate::keys::Profile,
    extensions: Option<&Path>,
    layout: Option<discovery::Source>,
    inventory: Option<&packages::Inventory>,
) -> Result<Preview> {
    let source = fs::canonicalize(source).context("Cannot open VS Code User directory")?;
    if !source.is_dir() {
        bail!("VS Code User path must be a directory");
    }
    let directory = |kind: &str| {
        layout
            .as_ref()
            .map_or(source.as_path(), |layout| layout.resource_directory(kind))
    };
    let mut paths = Vec::new();
    for name in ["settings.json", "keybindings.json", "tasks.json"] {
        if directory(name.strip_suffix(".json").unwrap())
            .join(name)
            .is_file()
        {
            paths.push(PathBuf::from(name));
        }
    }
    let snippets = directory("snippets").join("snippets");
    let mut snippet_count = 0;
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
                snippet_count += 1;
                if snippet_count > 128 {
                    bail!("Import exceeds 128 snippet files");
                }
                paths.push(PathBuf::from("snippets").join(entry.file_name()));
            }
        }
    }
    if paths.is_empty() && layout.is_none() {
        bail!(
            "No settings.json, keybindings.json or snippet files in {}",
            source.display()
        );
    }
    paths.sort();
    let mut result = Preview {
        report: Report {
            source: source.clone(),
            profile: layout.clone(),
            extensions: None,
            files: paths.clone(),
            bytes: 0,
            notices: Vec::new(),
            activated_profile: None,
            extensions_imported: 0,
            extensions_execution_enabled: 0,
        },
        files: Vec::new(),
        theme: None,
        source_roots: vec![
            layout
                .as_ref()
                .map_or(source.clone(), |layout| layout.default_user.clone()),
        ],
    };
    let mut selected_theme = None;
    let _profile = profile;
    let native_commands = crate::app::native_command_ids();
    for path in paths {
        let kind = if path.starts_with("snippets") {
            "snippets"
        } else {
            path.file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("")
        };
        let bytes = read(
            &directory(kind).join(&path),
            MAX_FILE.min(MAX_TOTAL.saturating_sub(result.report.bytes)),
        )?;
        result.report.bytes += bytes.len();
        let text = std::str::from_utf8(&bytes).context("Imported config is not UTF-8")?;
        let value: Value = crate::jsonc::parse(text)
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
                if !native_commands.iter().any(|id| id == command) {
                    result.report.notices.push(format!(
                        "Keybinding command {command:?} has no native implementation; preserved"
                    ));
                }
            }
            result.report.notices.push("Imported shortcut contexts and physical terminal delivery require qualification; preserving a rule does not implement its command".into());
        } else if path == Path::new("tasks.json") {
            if !value.is_object() {
                bail!("tasks.json must contain an object");
            }
            result.report.notices.push("User tasks.json is preserved; native task execution currently reads workspace .vscode/tasks.json".into());
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
            let snapshot = if let Some(inventory) = inventory {
                snapshot_package_themes(
                    inventory
                        .selected
                        .iter()
                        .filter(|package| package.enabled)
                        .map(|package| package.path.clone())
                        .collect(),
                    &name,
                )
                .and_then(|found| {
                    // An explicit unpacked theme directory remains a supported
                    // data-only source when no authoritative user index exists.
                    // This fallback never registers or executes package code.
                    if found.is_none() && !inventory.index.exists() {
                        snapshot_theme(directory, &name)
                    } else {
                        Ok(found)
                    }
                })
            } else {
                snapshot_theme(directory, &name)
            };
            match snapshot {
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
    result.report.notices.push("Original files and unknown settings are retained byte-for-byte. Registered extension inventory is separate from package import and explicit code activation; accounts and sync are not imported.".into());
    Ok(result)
}
#[derive(Clone, Copy, Default)]
pub struct ApplyOptions {
    pub import_extensions: bool,
    pub activate_extensions: bool,
}
#[derive(Serialize, Deserialize)]
struct Previous {
    schema: u32,
    pointer: Option<Active>,
}
struct ImportLease(fs::File);
impl Drop for ImportLease {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
fn import_lease(root: &Path) -> Result<ImportLease> {
    fs::create_dir_all(root)?;
    let path = root.join(".migration.lock");
    if fs::symlink_metadata(&path).is_ok_and(|metadata| !metadata.is_file()) {
        bail!("Migration lease must be a regular file");
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.try_lock()
        .context("Another migration or rollback is running")?;
    Ok(ImportLease(file))
}
impl Preview {
    /// Prepare every file before activating the profile with a single atomic pointer replacement.
    pub fn apply(self, root: &Path) -> Result<Report> {
        self.apply_controlled(
            root,
            ApplyOptions::default(),
            &std::sync::atomic::AtomicBool::new(false),
            &mut |_, _, _| {},
        )
    }
    pub fn apply_controlled(
        mut self,
        root: &Path,
        options: ApplyOptions,
        cancel: &std::sync::atomic::AtomicBool,
        progress: &mut dyn FnMut(usize, usize, &str),
    ) -> Result<Report> {
        if options.activate_extensions && !options.import_extensions {
            bail!("Code activation requires extension import");
        }
        let root = destination_path(root)?;
        if self
            .source_roots
            .iter()
            .any(|source| root.starts_with(source))
        {
            bail!("Import destination must be outside the original VS Code User directory");
        }
        let _lease = import_lease(&root)?;
        let previous = read_active_pointer(&root)?;
        // Validate the current profile before creating a new activation chain.
        active_directory(&root)?;
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
        if options.import_extensions {
            let inventory = self
                .report
                .extensions
                .as_ref()
                .context("Preview has no registered extension inventory")?;
            // Refuse an unsupported explicit execution cohort before copying
            // any package payload; the active profile remains untouched.
            let mut preferences = crate::extension_activation::Preferences::default();
            for package in &inventory.selected {
                if package.code && options.activate_extensions && package.enabled {
                    preferences.set(&package.id, true)?;
                    self.report.extensions_execution_enabled += 1;
                }
            }
            let grants = serde_json::to_vec_pretty(&preferences)?;
            let sources: Vec<_> = inventory
                .selected
                .iter()
                .map(|package| crate::extension_store::DirectorySource {
                    enabled: package.enabled,
                    path: package.path.clone(),
                    id: package.id.clone(),
                    version: package.version.clone(),
                    manifest_sha256: package.manifest_sha256.clone(),
                })
                .collect();
            let store = crate::extension_store::Store::new(staging.path().join("extensions"));
            let installed = store.import_directories(&sources, cancel, progress)?;
            self.report.extensions_imported = installed.len();
            write_new(&staging.path().join("extensions-enabled.json"), &grants)?;
            write_new(
                &staging.path().join("extension-migration.json"),
                &serde_json::to_vec_pretty(inventory)?,
            )?;
        }
        write_new(
            &staging.path().join("previous-profile.json"),
            &serde_json::to_vec(&Previous {
                schema: 1,
                pointer: previous,
            })?,
        )?;
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            bail!("Migration canceled before profile activation");
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
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn read_active_pointer(root: &Path) -> Result<Option<Active>> {
    let path = root.join("active-profile.json");
    if !path.try_exists()? {
        return Ok(None);
    }
    let active: Active = serde_json::from_slice(&read(&path, 4096)?)?;
    if active.schema != 1
        || active.profile.len() != 36
        || uuid::Uuid::parse_str(&active.profile).is_err()
    {
        bail!("Invalid imported-profile pointer");
    }
    Ok(Some(active))
}
pub fn rollback(root: &Path) -> Result<PathBuf> {
    let root = destination_path(root)?;
    let _lease = import_lease(&root)?;
    let current = read_active_pointer(&root)?.context("No imported profile is active")?;
    let directory = active_directory(&root)?;
    let previous: Previous =
        serde_json::from_slice(&read(&directory.join("previous-profile.json"), 4096)?)?;
    if previous.schema != 1 {
        bail!("Unsupported previous-profile schema");
    }
    if let Some(pointer) = previous.pointer {
        if pointer.schema != 1
            || pointer.profile.len() != 36
            || uuid::Uuid::parse_str(&pointer.profile).is_err()
            || pointer.profile == current.profile
        {
            bail!("Invalid previous profile");
        }
        let imports = root.join("imports");
        managed_directory(&imports)?;
        let previous = imports.join(&pointer.profile);
        managed_directory(&previous)?;
        let mut temporary = tempfile::NamedTempFile::new_in(&root)?;
        temporary.write_all(&serde_json::to_vec_pretty(&pointer)?)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(root.join("active-profile.json"))
            .map_err(|error| error.error)?;
        Ok(previous)
    } else {
        fs::remove_file(root.join("active-profile.json"))?;
        Ok(root)
    }
}
pub fn imported_extensions_directory(profile: &Path) -> Option<PathBuf> {
    let path = profile.join("extensions");
    path.join("registry.json").is_file().then_some(path)
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
    snapshot_package_themes(packages, selected)
}
fn snapshot_package_themes(
    mut packages: Vec<PathBuf>,
    selected: &str,
) -> Result<Option<ThemeSnapshot>> {
    packages.sort();
    let mut manifest_bytes = 0;
    let mut defaults = crate::extension_manifest::Defaults::default();
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
        let manifest = defaults.localize(&package, &manifest);
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
                let value: Value = crate::jsonc::parse(std::str::from_utf8(&bytes)?)?;
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
    if !fs::metadata(path)?.is_file() {
        bail!("Import source must be a regular file: {}", path.display());
    }
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
    #[cfg(unix)]
    #[test]
    fn named_pipe_theme_include_is_reported_without_opening_or_activating_it() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("User");
        let extensions = root.path().join("extensions");
        let package = extensions.join("theme");
        fs::create_dir(&source).unwrap();
        fs::create_dir_all(&package).unwrap();
        let settings = br#"{"workbench.colorTheme":"Named Pipe"}"#;
        fs::write(source.join("settings.json"), settings).unwrap();
        fs::write(
            package.join("package.json"),
            r#"{"contributes":{"themes":[{"label":"Named Pipe","path":"theme.json"}]}}"#,
        )
        .unwrap();
        fs::write(package.join("theme.json"), r#"{"include":"pipe.json"}"#).unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(package.join("pipe.json"))
                .status()
                .unwrap()
                .success()
        );
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let worker_source = source.clone();
        std::thread::spawn(move || {
            let _ = sender.send(preview_with_extensions(
                &worker_source,
                crate::keys::Profile::Linux,
                Some(&extensions),
            ));
        });
        let preview = receiver
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("Import must reject named pipes without blocking")
            .unwrap();
        assert!(
            preview
                .report
                .notices
                .iter()
                .any(|notice| notice.contains("regular file"))
        );
        assert_eq!(preview.report.files, vec![PathBuf::from("settings.json")]);
        assert!(preview.theme.is_none());
        assert_eq!(fs::read(source.join("settings.json")).unwrap(), settings);
        assert!(!root.path().join("active-profile.json").exists());
    }
    #[test]
    fn snippet_file_limit_does_not_depend_on_optional_settings_and_keybindings() {
        let root = tempfile::tempdir().unwrap();
        let snippets = root.path().join("snippets");
        fs::create_dir(&snippets).unwrap();
        for index in 0..128 {
            fs::write(snippets.join(format!("{index}.json")), "{}").unwrap();
        }
        assert_eq!(
            preview(root.path(), crate::keys::Profile::Linux)
                .unwrap()
                .report
                .files
                .len(),
            128
        );
        fs::write(snippets.join("overflow.json"), "{}").unwrap();
        assert!(
            preview(root.path(), crate::keys::Profile::Linux)
                .err()
                .unwrap()
                .to_string()
                .contains("128 snippet")
        );
    }
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
        for entry in fs::read_dir(target.join("imports")).into_iter().flatten() {
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
