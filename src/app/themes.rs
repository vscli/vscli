use super::*;
use crate::theme::{Choice, Preference, Theme};
use std::sync::mpsc::{self, Receiver, TryRecvError};
struct Loaded {
    theme: Option<Theme>,
    choices: Option<Vec<Choice>>,
    notices: Vec<String>,
}
#[derive(Default)]
pub(super) struct State {
    pending: Option<Receiver<Result<Loaded, String>>>,
    choices: Vec<Choice>,
    preferences: Option<PathBuf>,
    directory: Option<PathBuf>,
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
                        Ok(theme) => Some(theme),
                        Err(error) => {
                            notices.push(format!("Theme unchanged: {error:#}"));
                            None
                        }
                    }
                } else {
                    None
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
    fn select_theme(&mut self, selected: Preference) {
        if self.theme_state.pending.is_some() {
            self.message = "A color theme is still loading".into();
            return;
        }
        let choices = self.theme_state.choices.clone();
        let preferences = self.theme_state.preferences.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        self.theme_state.pending = Some(receiver);
        std::thread::spawn(move || {
            let result = (|| -> Result<Loaded> {
                let theme = resolve(&selected, &choices)?;
                if let Some(path) = preferences {
                    selected.save(&path)?;
                }
                Ok(Loaded {
                    theme: Some(theme),
                    choices: None,
                    notices: Vec::new(),
                })
            })()
            .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
        self.message = "Loading color theme…".into();
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
                if let Some(theme) = loaded.theme {
                    loaded.notices.extend(theme.warnings.clone());
                    self.message = format!("Color theme: {}", theme.name);
                    self.theme = theme;
                }
                if !loaded.notices.is_empty() {
                    self.message = format!("{} · {}", self.message, loaded.notices.join(" · "));
                }
            }
            Err(error) => self.message = format!("Theme unchanged: {error}"),
        }
        true
    }
    pub fn theme_items(&self, query: &str) -> Vec<&str> {
        ["VSCLI Dark", "VSCLI Light"]
            .into_iter()
            .chain(
                self.theme_state
                    .choices
                    .iter()
                    .map(|choice| choice.label.as_str()),
            )
            .filter(|name| score(name, query).is_some())
            .take(100)
            .collect()
    }
    pub(super) fn accept_theme(&mut self, query: &str, selected: usize) {
        let items = self.theme_items(query);
        if let Some(name) = items.get(selected.min(items.len().saturating_sub(1))) {
            self.select_theme(Preference {
                name: (*name).into(),
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
        }
        return Ok(theme);
    }
    match selected.name.as_str() {
        "VSCLI Dark" => Ok(Theme::default()),
        "VSCLI Light" => Ok(Theme::light()),
        name => {
            let choice = choices
                .iter()
                .find(|choice| choice.id == name || choice.label == name)
                .ok_or_else(|| anyhow::anyhow!("Color theme {name:?} is not installed"))?;
            let mut theme = Theme::load_confined(&choice.path, &choice.root)?;
            theme.name = choice.label.clone();
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
