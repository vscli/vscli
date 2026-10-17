use super::*;
use crate::theme::{Choice, Preference, Theme};
use std::sync::mpsc::{self, Receiver, TryRecvError};
struct Loaded {
    theme: Option<Styled>,
    choices: Option<Vec<Choice>>,
    notices: Vec<String>,
}
struct Styled {
    base: Theme,
    rendered: Theme,
    settings: crate::settings::Settings,
}
impl Styled {
    fn new(base: Theme, settings: crate::settings::Settings) -> Self {
        let rendered = base.customized(&settings);
        Self {
            base,
            rendered,
            settings,
        }
    }
}
#[derive(Default)]
pub(super) struct State {
    pending: Option<Receiver<Result<Loaded, String>>>,
    choices: Vec<Choice>,
    preferences: Option<PathBuf>,
    directory: Option<PathBuf>,
    base: Theme,
    configured: bool,
    refresh_pending: bool,
}
impl App {
    pub fn configure_themes(
        &mut self,
        directory: Option<PathBuf>,
        preferences: Option<PathBuf>,
        requested: Option<String>,
        explicit_file: Option<PathBuf>,
    ) {
        if self.theme_state.pending.is_some() {
            return;
        }
        self.theme_state.preferences = preferences.clone();
        self.theme_state.directory = directory.clone();
        self.theme_state.configured = true;
        let settings = self.settings.clone();
        let fallback = self.theme_state.base.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        self.theme_state.pending = Some(receiver);
        std::thread::spawn(move || {
            let result = {
                let (choices, mut notices) = match installed_packages(directory.as_deref()) {
                    Ok(packages) => crate::theme::contributions(&packages),
                    Err(error) => (
                        Vec::new(),
                        vec![format!("Installed theme discovery failed: {error:#}")],
                    ),
                };
                let saved = if let Some(path) = preferences {
                    match Preference::read(&path) {
                        Ok(preference) => preference,
                        Err(error) => {
                            notices.push(format!("Theme preference ignored: {error:#}"));
                            None
                        }
                    }
                } else {
                    None
                };
                let selection = explicit_file
                    .map(|path| Preference {
                        name: String::new(),
                        path: Some(path),
                    })
                    .or(saved)
                    .or_else(|| requested.map(|name| Preference { name, path: None }));
                let theme = if let Some(selection) = selection {
                    match resolve(&selection, &choices) {
                        Ok(theme) => Some(Styled::new(theme, settings)),
                        Err(error) => {
                            notices.push(format!("Theme unchanged: {error:#}"));
                            None
                        }
                    }
                } else {
                    Some(Styled::new(fallback, settings))
                };
                Loaded {
                    theme,
                    choices: Some(choices),
                    notices,
                }
            };
            let _ = sender.send(Ok(result));
        });
    }
    pub(super) fn open_theme_picker(&mut self) {
        self.start_prompt(PromptKind::Theme, String::new());
        if self.theme_state.pending.is_some() {
            return;
        }
        let directory = self.theme_state.directory.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        self.theme_state.pending = Some(receiver);
        std::thread::spawn(move || {
            let result = installed_packages(directory.as_deref())
                .map(|packages| {
                    let (choices, notices) = crate::theme::contributions(&packages);
                    Loaded {
                        choices: Some(choices),
                        theme: None,
                        notices,
                    }
                })
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
    }
    pub fn load_theme(&mut self, path: PathBuf) {
        self.select_theme(Preference {
            name: String::new(),
            path: Some(path),
        });
    }
    fn select_theme(&mut self, mut selected: Preference) {
        if self.theme_state.pending.is_some() {
            self.message = "A color theme is still loading".into();
            return;
        }
        let choices = self.theme_state.choices.clone();
        let preferences = self.theme_state.preferences.clone();
        let settings = self.settings.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        self.theme_state.pending = Some(receiver);
        std::thread::spawn(move || {
            let result = (|| -> Result<Loaded> {
                let theme = resolve(&selected, &choices)?;
                if let Some(path) = selected.path.as_mut() {
                    *path = std::fs::canonicalize(&*path)?;
                }
                if let Some(path) = preferences {
                    selected.save(&path)?;
                }
                Ok(Loaded {
                    theme: Some(Styled::new(theme, settings)),
                    choices: None,
                    notices: Vec::new(),
                })
            })()
            .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
        self.message = "Loading color theme…".into();
    }
    pub(super) fn refresh_theme_customizations(&mut self) {
        if !self.theme_state.configured {
            return;
        }
        if self.theme_state.pending.is_some() {
            self.theme_state.refresh_pending = true;
            return;
        }
        self.theme_state.refresh_pending = false;
        let base = self.theme_state.base.clone();
        let settings = self.settings.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        self.theme_state.pending = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(Ok(Loaded {
                theme: Some(Styled::new(base, settings)),
                choices: None,
                notices: Vec::new(),
            }));
        });
    }
    pub(super) fn poll_theme(&mut self) -> bool {
        let Some(receiver) = &self.theme_state.pending else {
            return false;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err("Theme loader stopped".into()),
        };
        self.theme_state.pending = None;
        match result {
            Ok(mut loaded) => {
                if let Some(choices) = loaded.choices {
                    self.theme_state.choices = choices;
                }
                if let Some(styled) = loaded.theme {
                    self.theme_state.base = styled.base;
                    if styled.settings == self.settings {
                        self.theme_state.refresh_pending = false;
                        loaded.notices.extend(styled.rendered.warnings.clone());
                        self.message = format!("Color theme: {}", styled.rendered.name);
                        self.theme = styled.rendered;
                    } else {
                        // A settings reply cannot authorize an older appearance.
                        self.refresh_theme_customizations();
                    }
                }
                if !loaded.notices.is_empty() {
                    self.message = format!("{} · {}", self.message, loaded.notices.join(" · "));
                }
            }
            Err(error) => self.message = format!("Theme unchanged: {error}"),
        }
        if self.theme_state.refresh_pending && self.theme_state.pending.is_none() {
            self.refresh_theme_customizations();
        }
        true
    }
    fn theme_options(&self, query: &str) -> Vec<(&str, &str)> {
        [("VSCLI Dark", "VSCLI Dark"), ("VSCLI Light", "VSCLI Light")]
            .into_iter()
            .chain(
                self.theme_state
                    .choices
                    .iter()
                    .map(|choice| (choice.picker_label.as_str(), choice.key.as_str())),
            )
            .filter(|(label, _)| score(label, query).is_some())
            .take(100)
            .collect()
    }
    pub fn theme_items(&self, query: &str) -> Vec<&str> {
        self.theme_options(query)
            .into_iter()
            .map(|(label, _)| label)
            .collect()
    }
    pub(super) fn accept_theme(&mut self, query: &str, selected: usize) {
        let items = self.theme_options(query);
        if let Some((_, key)) = items.get(selected.min(items.len().saturating_sub(1))) {
            self.select_theme(Preference {
                name: (*key).into(),
                path: None,
            });
        }
    }
}
fn resolve(selected: &Preference, choices: &[Choice]) -> Result<Theme> {
    if let Some(path) = &selected.path {
        let mut theme = Theme::load(path)?;
        if !selected.name.is_empty() {
            theme.name = selected.name.clone();
            theme.settings_id = selected.name.clone();
        }
        return Ok(theme);
    }
    match selected.name.as_str() {
        "VSCLI Dark" => Ok(Theme::default()),
        "VSCLI Light" => Ok(Theme::light()),
        name => {
            let choice = choices
                .iter()
                .find(|choice| choice.key == name)
                .or_else(|| {
                    choices
                        .iter()
                        .find(|choice| choice.id == name || choice.label == name)
                })
                .ok_or_else(|| anyhow::anyhow!("Color theme {name:?} is not installed"))?;
            let mut theme = Theme::load_confined(&choice.path, &choice.root)?;
            theme.name = choice.label.clone();
            theme.settings_id = choice.id.clone();
            Ok(theme)
        }
    }
}

fn installed_packages(directory: Option<&Path>) -> Result<Vec<(PathBuf, Value)>> {
    let directory = directory
        .map(Path::to_path_buf)
        .or_else(crate::extension_store::default_directory);
    let Some(directory) = directory else {
        return Ok(Vec::new());
    };
    crate::extension_store::Store::new(directory)
        .list()
        .map(|packages| {
            packages
                .into_iter()
                .map(|package| (package.path, package.manifest))
                .collect()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn poll(app: &mut App) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !app.poll_theme() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    #[test]
    fn selections_persist_explicit_paths_win_and_failed_loads_preserve_editor() {
        let root = tempfile::tempdir().unwrap();
        let preference = root.path().join("config/theme-selection.json");
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.doc_mut().insert("unsaved text", false);
        app.configure_themes(
            Some(root.path().join("extensions")),
            Some(preference.clone()),
            Some("VSCLI Light".into()),
            None,
        );
        poll(&mut app);
        assert_eq!(app.theme.name, "VSCLI Light");
        app.accept_theme("VSCLI Dark", 0);
        poll(&mut app);
        assert_eq!(
            Preference::read(&preference).unwrap().unwrap().name,
            "VSCLI Dark"
        );
        let custom = root.path().join("theme.json");
        std::fs::write(
            &custom,
            r##"{"name":"Custom","colors":{"editor.background":"#112233"}}"##,
        )
        .unwrap();
        app.configure_themes(
            Some(root.path().join("extensions")),
            Some(preference.clone()),
            Some("VSCLI Dark".into()),
            Some(custom.clone()),
        );
        poll(&mut app);
        assert_eq!(app.theme.name, "Custom");
        let bytes = std::fs::read(&preference).unwrap();
        app.load_theme(root.path().join("missing.json"));
        poll(&mut app);
        assert_eq!(app.theme.name, "Custom");
        assert_eq!(app.doc().text.to_string(), "unsaved text");
        assert_eq!(std::fs::read(&preference).unwrap(), bytes);
        assert!(app.message.contains("Theme unchanged"));
        app.load_theme(custom);
        poll(&mut app);
        assert!(
            Preference::read(&preference)
                .unwrap()
                .unwrap()
                .path
                .is_some()
        );
    }
    #[test]
    fn relative_custom_theme_paths_are_persisted_as_absolute_paths() {
        let cwd = std::env::current_dir().unwrap();
        let root = tempfile::tempdir_in(&cwd).unwrap();
        let path = root.path().join("theme.json");
        std::fs::write(&path, r#"{"name":"Relative"}"#).unwrap();
        let preference = root.path().join("selection.json");
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.configure_themes(
            Some(root.path().join("extensions")),
            Some(preference.clone()),
            None,
            None,
        );
        poll(&mut app);
        app.load_theme(path.strip_prefix(&cwd).unwrap().into());
        poll(&mut app);
        let saved = Preference::read(&preference).unwrap().unwrap();
        assert!(saved.path.as_ref().unwrap().is_absolute());
        assert_eq!(
            saved.path.as_ref().unwrap(),
            &std::fs::canonicalize(&path).unwrap()
        );
        assert_eq!(resolve(&saved, &[]).unwrap().name, "Relative");
    }
    #[test]
    fn broken_extension_registry_does_not_block_explicit_or_builtin_theme_selection() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("extensions");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("registry.json"), "invalid").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.configure_themes(
            Some(directory.clone()),
            None,
            Some("VSCLI Light".into()),
            None,
        );
        poll(&mut app);
        assert_eq!(app.theme.name, "VSCLI Light");
        assert!(app.message.contains("discovery failed"));
        let path = root.path().join("theme.json");
        std::fs::write(&path, r#"{"name":"Explicit"}"#).unwrap();
        app.configure_themes(Some(directory), None, None, Some(path));
        poll(&mut app);
        assert_eq!(app.theme.name, "Explicit");
    }
    #[test]
    fn duplicate_theme_labels_select_exact_rows_and_survive_package_directory_changes() {
        let root = tempfile::tempdir().unwrap();
        let mut packages = Vec::new();
        for (publisher, color) in [("first", "#112233"), ("second", "#445566")] {
            let directory = root.path().join(publisher);
            std::fs::create_dir(&directory).unwrap();
            std::fs::write(
                directory.join("theme.json"),
                format!(r#"{{"colors":{{"editor.background":"{color}"}}}}"#),
            )
            .unwrap();
            packages.push((directory, json!({"publisher": publisher, "name":"theme", "contributes":{"themes":[{"id":"duplicate","label":"VSCLI Dark","path":"theme.json"}]}})));
        }
        let (choices, warnings) = crate::theme::contributions(&packages);
        assert!(warnings.is_empty());
        let mut app = App::new(root.path().into(), Profile::Linux);
        let preference = root.path().join("preference.json");
        app.theme_state.preferences = Some(preference.clone());
        app.theme_state.choices = choices;
        let labels = app.theme_items("VSCLI Dark");
        assert_eq!(labels.len(), 3);
        assert!(labels[1].contains("first.theme"));
        assert!(labels[2].contains("second.theme"));
        app.accept_theme("VSCLI Dark", 2);
        poll(&mut app);
        assert_eq!(
            app.theme.colors.background,
            ratatui::style::Color::Rgb(68, 85, 102)
        );
        let selected = Preference::read(&preference).unwrap().unwrap();
        assert_eq!(selected.name, "extension:second.theme/theme.json");
        let moved = root.path().join("second-new-version");
        std::fs::rename(&packages[1].0, &moved).unwrap();
        packages[1].0 = moved;
        let (choices, _) = crate::theme::contributions(&packages);
        assert_eq!(
            resolve(&selected, &choices).unwrap().colors.background,
            ratatui::style::Color::Rgb(68, 85, 102)
        );
        app.accept_theme("VSCLI Dark", 0);
        poll(&mut app);
        assert_eq!(
            app.theme.colors.background,
            Theme::default().colors.background
        );
    }
    #[test]
    fn imported_binding_notices_are_readable_without_creating_an_editor() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.imported_keybinding_notices
            .push("Skipped rule 3: unsupported expression".into());
        app.execute("vscli.settings.report", serde_json::Value::Null);
        assert!(app.active_document().is_none());
        assert!(
            matches!(&app.modal, Some(Modal::Text { text, .. }) if text.contains("Skipped rule 3"))
        );
    }
    #[test]
    fn installed_contributions_resolve_by_id_without_executing_package() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("colors.json");
        std::fs::write(&path, r##"{"colors":{"editor.background":"#010203"}}"##).unwrap();
        let manifest = json!({"main":"never-run.js", "contributes":{"themes":[{"id":"theme.id","label":"Installed Theme","path":"colors.json"},{"label":"Escape","path":"../missing.json"}]}});
        let (choices, warnings) = crate::theme::contributions(&[(root.path().into(), manifest)]);
        assert_eq!(choices.len(), 1);
        assert_eq!(warnings.len(), 1);
        let theme = resolve(
            &Preference {
                name: "theme.id".into(),
                path: None,
            },
            &choices,
        )
        .unwrap();
        assert_eq!(theme.name, "Installed Theme");
        assert_eq!(theme.colors.background, ratatui::style::Color::Rgb(1, 2, 3));
    }
}

#[cfg(test)]
mod customization_tests {
    use super::*;
    use ratatui::style::Color;
    use serde_json::json;

    fn settings(color: &str) -> crate::settings::Settings {
        crate::settings::Settings::from_values(
            json!({"workbench.colorCustomizations":{
                "editor.foreground":color
            }})
            .as_object()
            .unwrap()
            .clone(),
            "fixture",
        )
        .unwrap()
    }
    fn settle(app: &mut App) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.theme_state.pending.is_some() {
            app.poll_theme();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn imported_theme_uses_original_settings_id_and_reload_removal_restores_base() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("User");
        let package = root.path().join("extensions/theme");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&package).unwrap();
        let original = r##"{// keep original\n
            "workbench.colorTheme":"theme.id",
            "workbench.colorCustomizations":{"[theme.id]":{"editor.foreground":"#123456"}},
        }"##;
        std::fs::write(user.join("settings.json"), original).unwrap();
        std::fs::write(package.join("package.json"), r#"{"contributes":{"themes":[{"id":"theme.id","label":"Display Label","path":"theme.json"}]}}"#).unwrap();
        std::fs::write(
            package.join("theme.json"),
            r##"{"name":"File Name","colors":{"editor.foreground":"#abcdef"}}"##,
        )
        .unwrap();
        let native = root.path().join("native");
        let report = crate::migration::preview_with_extensions(
            &user,
            Profile::Linux,
            Some(&root.path().join("extensions")),
        )
        .unwrap()
        .apply(&native)
        .unwrap();
        let active = report.activated_profile.unwrap();
        let path = active.join("settings.json");
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.doc_mut().insert("unsaved λ🙂\r\n", false);
        let version = app.doc().revision;
        app.configure_settings(Some(path.clone())).unwrap();
        app.configure_themes(
            Some(root.path().join("empty-store")),
            Some(active.join("theme-selection.json")),
            None,
            None,
        );
        settle(&mut app);
        assert_eq!(app.theme.settings_id, "theme.id");
        assert_eq!(app.theme.colors.foreground, Color::Rgb(18, 52, 86));
        std::fs::write(
            &path,
            r##"{"workbench.colorCustomizations":{"editor.foreground":"#654321"}}"##,
        )
        .unwrap();
        app.configure_settings(Some(path.clone())).unwrap();
        settle(&mut app);
        assert_eq!(app.theme.colors.foreground, Color::Rgb(101, 67, 33));
        std::fs::write(&path, "{}").unwrap();
        app.configure_settings(Some(path)).unwrap();
        settle(&mut app);
        assert_eq!(app.theme.colors.foreground, Color::Rgb(171, 205, 239));
        app.configure_themes(None, None, None, None);
        settle(&mut app);
        assert_eq!(app.theme.colors.foreground, Color::Rgb(171, 205, 239));
        assert_eq!(app.doc().revision, version);
        assert_eq!(app.doc().text.to_string(), "unsaved λ🙂\r\n");
        assert!(app.doc().dirty());
        assert_eq!(
            std::fs::read_to_string(user.join("settings.json")).unwrap(),
            original
        );
        app.doc_mut().undo();
        assert_eq!(app.doc().text.len_chars(), 0);
    }

    #[test]
    fn stale_style_never_publishes_and_latest_settings_are_reapplied_to_selected_base() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        let original_foreground = app.theme.colors.foreground;
        app.theme_state.configured = true;
        let (sender, receiver) = mpsc::sync_channel(1);
        app.theme_state.pending = Some(receiver);
        sender
            .send(Ok(Loaded {
                theme: Some(Styled::new(Theme::light(), settings("#ff0000"))),
                choices: None,
                notices: Vec::new(),
            }))
            .unwrap();
        app.settings = settings("#00ff00");
        app.refresh_theme_customizations();
        assert!(app.poll_theme());
        assert_eq!(app.theme.colors.foreground, original_foreground);
        assert!(app.theme_state.pending.is_some());
        settle(&mut app);
        assert_eq!(app.theme.name, "VSCLI Light");
        assert_eq!(app.theme.colors.foreground, Color::Rgb(0, 255, 0));
    }

    #[test]
    fn failed_theme_or_picker_reply_does_not_drop_a_pending_settings_refresh() {
        for failed in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let mut app = App::new(root.path().into(), Profile::Linux);
            app.theme_state.configured = true;
            let (sender, receiver) = mpsc::sync_channel(1);
            app.theme_state.pending = Some(receiver);
            let result = if failed {
                Err("failed fixture".into())
            } else {
                Ok(Loaded {
                    theme: None,
                    choices: Some(Vec::new()),
                    notices: Vec::new(),
                })
            };
            sender.send(result).unwrap();
            app.settings = settings("#123456");
            app.refresh_theme_customizations();
            settle(&mut app);
            assert_eq!(app.theme.colors.foreground, Color::Rgb(18, 52, 86));
        }
    }
}
