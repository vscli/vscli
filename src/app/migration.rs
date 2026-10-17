//! Joined migration jobs and native review. Editor documents are never replaced.
use super::*;
use crate::migration::{
    self as engine, ApplyOptions, Preview,
    discovery::{self, Source},
};
use anyhow::Context;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver},
};
use std::thread::JoinHandle;

#[derive(Default)]
pub(super) struct State {
    root: Option<PathBuf>,
    base_extensions: Option<PathBuf>,
    sources: Vec<Source>,
    selected: usize,
    preview: Option<Preview>,
    options: ApplyOptions,
    job: Option<Job>,
    generation: u64,
    phase: Phase,
}
#[derive(Default, PartialEq)]
enum Phase {
    #[default]
    Sources,
    Review,
    Running,
    Done,
}
struct Job {
    generation: u64,
    cancel: Arc<AtomicBool>,
    progress: Receiver<String>,
    thread: JoinHandle<Result<Output, String>>,
}
enum Work {
    Discover(Option<PathBuf>),
    Preview(Source),
    Application(Source, PathBuf),
    Apply(Box<Preview>, ApplyOptions),
    Rollback,
}
enum Output {
    Sources(discovery::Discovery),
    Preview(Preview),
    Applied(engine::Report, Result<Runtime, String>),
    RolledBack(Result<Runtime, String>),
}
struct Runtime {
    directory: PathBuf,
    paths: Vec<PathBuf>,
    settings: crate::settings::Settings,
    keymap: crate::keys::Keymap,
    notices: Vec<String>,
    extensions: Option<PathBuf>,
    extension_config: PathBuf,
}
impl Runtime {
    fn prepare(
        directory: PathBuf,
        workspace: &Path,
        profile: Profile,
        native_root: &Path,
        base_extensions: Option<&Path>,
    ) -> Result<Self> {
        let paths = vec![
            directory.join("settings.json"),
            workspace.join(".vscode/settings.json"),
        ];
        let settings = crate::settings::Settings::load_editor(&paths)?;
        let mut keymap = crate::keys::Keymap::new(profile);
        let keys = directory.join("keybindings.json");
        let notices = if keys.is_file() {
            keymap.load_imported(&keys)?.1
        } else {
            Vec::new()
        };
        let imported = engine::imported_extensions_directory(&directory);
        let extension_config = if imported.is_some() {
            directory.clone()
        } else {
            native_root.to_path_buf()
        };
        let extensions = imported.or_else(|| base_extensions.map(Path::to_path_buf));
        Ok(Self {
            directory,
            paths,
            settings,
            keymap,
            notices,
            extensions,
            extension_config,
        })
    }
}
impl App {
    pub fn configure_migration(&mut self, root: Option<PathBuf>, base_extensions: Option<PathBuf>) {
        self.migration.root = root;
        self.migration.base_extensions = base_extensions;
    }
    pub(super) fn open_vscode_import(&mut self) {
        self.migration.options = ApplyOptions {
            import_extensions: true,
            activate_extensions: false,
        };
        self.start_migration(Work::Discover(None));
    }
    pub(super) fn import_vscode_path(&mut self, path: PathBuf) {
        self.start_migration(Work::Discover(Some(path)));
    }
    pub(super) fn set_import_extension_directory(&mut self, path: PathBuf) {
        if let Some(source) = self.migration.sources.get_mut(self.migration.selected) {
            source.extensions = path;
            let source = source.clone();
            self.start_migration(Work::Preview(source));
        }
    }
    pub(super) fn set_import_application_path(&mut self, path: PathBuf) {
        if let Some(source) = self.migration.sources.get(self.migration.selected).cloned() {
            self.start_migration(Work::Application(source, path));
        }
    }
    pub(super) fn rollback_vscode_import(&mut self) {
        self.start_migration(Work::Rollback);
    }
    fn start_migration(&mut self, work: Work) {
        if self.migration.job.is_some() {
            self.message = "A migration is still running; wait for its actual completion".into();
            return;
        }
        let Some(root) = self.migration.root.clone() else {
            self.message = "No native configuration directory; use --config-dir".into();
            return;
        };
        let Some(next) = self.migration.generation.checked_add(1) else {
            self.message = "Migration identity exhausted; restart the editor".into();
            return;
        };
        self.migration.generation = next;
        let generation = self.migration.generation;
        let workspace = self.workspace.root.clone();
        let base_extensions = self.migration.base_extensions.clone();
        let profile = self.keymap.profile;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, progress) = mpsc::sync_channel(1);
        let thread = std::thread::spawn(move || {
            let result = (|| -> Result<Output> {
                if worker_cancel.load(Ordering::Acquire) {
                    anyhow::bail!("Migration canceled");
                }
                match work {
                    Work::Discover(path) => {
                        let sources = if let Some(path) = path {
                            let extensions = directories::BaseDirs::new()
                                .context("No home directory")?
                                .home_dir()
                                .join(".vscode/extensions");
                            discovery::Discovery {
                                profiles: discovery::installation(
                                    "Custom VS Code",
                                    &path,
                                    &extensions,
                                )?,
                                notices: Vec::new(),
                            }
                        } else {
                            discovery::discover()?
                        };
                        Ok(Output::Sources(sources))
                    }
                    Work::Application(mut source, path) => {
                        source.application = Some(engine::application::Application::open(&path)?);
                        Ok(Output::Preview(engine::preview_profile(&source, profile)?))
                    }
                    Work::Preview(source) => {
                        Ok(Output::Preview(engine::preview_profile(&source, profile)?))
                    }
                    Work::Apply(preview, options) => {
                        let report = preview.apply_controlled(
                            &root,
                            options,
                            &worker_cancel,
                            &mut |done, total, id| {
                                let _ = sender.try_send(format!(
                                    "Copying local extensions {done}/{total}: {id}"
                                ));
                            },
                        )?;
                        let directory = report
                            .activated_profile
                            .clone()
                            .context("Import has no activated profile")?;
                        let runtime = Runtime::prepare(
                            directory,
                            &workspace,
                            profile,
                            &root,
                            base_extensions.as_deref(),
                        )
                        .map_err(|error| format!("{error:#}"));
                        Ok(Output::Applied(report, runtime))
                    }
                    Work::Rollback => {
                        let directory = engine::rollback(&root)?;
                        Ok(Output::RolledBack(
                            Runtime::prepare(
                                directory,
                                &workspace,
                                profile,
                                &root,
                                base_extensions.as_deref(),
                            )
                            .map_err(|error| format!("{error:#}")),
                        ))
                    }
                }
            })();
            result.map_err(|error| format!("{error:#}"))
        });
        self.migration.job = Some(Job {
            generation,
            cancel,
            progress,
            thread,
        });
        self.migration.phase = Phase::Running;
        self.migration_modal(
            "VS Code Migration · Esc cancels",
            "Preparing migration…".into(),
        );
    }
    fn migration_modal(&mut self, title: &str, text: String) {
        self.prompt = None;
        self.focus = Focus::Editor;
        self.modal = Some(Modal::Migration {
            generation: self.migration.generation,
            title: title.into(),
            text,
            scroll: 0,
        });
    }
    fn show_migration_sources(&mut self, notices: &[String]) {
        let mut lines = vec![
            "Choose the VS Code profile to bring across.".into(),
            "↑/↓ select · Enter reviews · P custom User directory · E extensions directory · B application/built-ins · Esc cancels".into(),
            String::new(),
        ];
        for (index, source) in self.migration.sources.iter().enumerate() {
            lines.push(format!(
                "{} {} / {}",
                if index == self.migration.selected {
                    ">"
                } else {
                    " "
                },
                source.product,
                source.name
            ));
        }
        if self.migration.sources.is_empty() {
            lines.push("No local profiles found. Press P to choose a User directory.".into());
        }
        lines.extend(notices.iter().take(16).cloned());
        self.migration_modal("Import from VS Code", lines.join("\n"));
    }
    fn show_migration_review(&mut self) {
        let Some(preview) = &self.migration.preview else {
            return;
        };
        let report = &preview.report;
        let inventory = report.extensions.as_ref();
        let packages = inventory.map_or(0, |value| value.selected.len());
        let mut lines =
            vec![
            format!("Source: {}", report.source.display()),
            report.profile.as_ref().and_then(|source| source.application.as_ref()).map_or_else(
                || "Application: not selected; choose B on the profile picker for built-ins".into(),
                |app| format!("Application: {} {} · {}", app.name, app.version, app.root.display()),
            ),
            format!(
                "Configuration: {} files · {} bytes",
                report.files.len(),
                report.bytes
            ),
            format!(
                "Extensions: {packages} selected IDs · {} source built-ins",
                inventory.map_or(0, |inventory| inventory.built_in_packages)
            ),
            String::new(),
            format!(
                "[{}] Copy registered local extension packages · I toggles",
                if self.migration.options.import_extensions {
                    "x"
                } else {
                    " "
                }
            ),
            format!(
                "[{}] Allow source-enabled code extensions to activate · A toggles",
                if self.migration.options.activate_extensions {
                    "x"
                } else {
                    " "
                }
            ),
            "Enter applies this separate native copy · B returns to profiles · Esc cancels".into(),
            "Original VS Code files stay unchanged. Installation does not establish compatibility."
                .into(),
            String::new(),
        ];
        lines.push("Configuration and compatibility notices:".into());
        lines.extend(
            report
                .notices
                .iter()
                .take(80)
                .map(|notice| notice.chars().take(512).collect()),
        );
        if let Some(inventory) = inventory {
            lines.extend(inventory.notices.iter().take(32).cloned());
            lines.push(String::new());
            lines.push("Extensions (source state retained; workflows remain unqualified):".into());
            lines.extend(inventory.selected.iter().map(|package| {
                format!(
                    "{} {}@{}{}",
                    if package.enabled {
                        "enabled "
                    } else {
                        "disabled"
                    },
                    package.id,
                    package.version,
                    if package.code { " · code" } else { " · data" }
                )
            }));
        }
        self.migration_modal(
            "Review VS Code Import · arrows/PageUp/PageDown scroll",
            lines.join("\n"),
        );
    }
    pub(super) fn migration_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            if let Some(job) = &self.migration.job {
                job.cancel.store(true, Ordering::Release);
            }
            self.modal = None;
            return;
        }
        match self.migration.phase {
            Phase::Sources => match key.code {
                KeyCode::Up => {
                    self.migration.selected = self.migration.selected.saturating_sub(1);
                    self.show_migration_sources(&[]);
                }
                KeyCode::Down => {
                    self.migration.selected = (self.migration.selected + 1)
                        .min(self.migration.sources.len().saturating_sub(1));
                    self.show_migration_sources(&[]);
                }
                KeyCode::Enter => {
                    if let Some(source) =
                        self.migration.sources.get(self.migration.selected).cloned()
                    {
                        self.start_migration(Work::Preview(source));
                    }
                }
                KeyCode::Char('b' | 'B') => {
                    if let Some(source) = self.migration.sources.get(self.migration.selected) {
                        let path = source.application.as_ref().map_or_else(String::new, |app| {
                            app.root.to_string_lossy().into_owned()
                        });
                        self.modal = None;
                        self.start_prompt(PromptKind::VscodeApplicationPath, path);
                    }
                }
                KeyCode::Char('e' | 'E') => {
                    if let Some(source) = self.migration.sources.get(self.migration.selected) {
                        let path = source.extensions.to_string_lossy().into_owned();
                        self.modal = None;
                        self.start_prompt(PromptKind::VscodeExtensionsPath, path);
                    }
                }
                KeyCode::Char('p' | 'P') => {
                    self.modal = None;
                    self.start_prompt(PromptKind::VscodeImportPath, String::new());
                }
                _ => {}
            },
            Phase::Review => match key.code {
                KeyCode::Char('i' | 'I') => {
                    self.migration.options.import_extensions =
                        !self.migration.options.import_extensions;
                    if !self.migration.options.import_extensions {
                        self.migration.options.activate_extensions = false;
                    }
                    self.show_migration_review();
                }
                KeyCode::Char('a' | 'A') => {
                    self.migration.options.activate_extensions =
                        !self.migration.options.activate_extensions;
                    if self.migration.options.activate_extensions {
                        self.migration.options.import_extensions = true;
                    }
                    self.show_migration_review();
                }
                KeyCode::Char('b' | 'B') => {
                    self.migration.phase = Phase::Sources;
                    self.show_migration_sources(&[]);
                }
                KeyCode::Enter => {
                    if let Some(preview) = self.migration.preview.take() {
                        self.start_migration(Work::Apply(
                            Box::new(preview),
                            self.migration.options,
                        ));
                    }
                }
                _ => self.scroll_migration(key.code),
            },
            _ => self.scroll_migration(key.code),
        }
    }
    fn scroll_migration(&mut self, key: KeyCode) {
        if let Some(Modal::Migration { scroll, text, .. }) = &mut self.modal {
            *scroll = match key {
                KeyCode::Up => scroll.saturating_sub(1),
                KeyCode::Down => scroll.saturating_add(1),
                KeyCode::PageUp => scroll.saturating_sub(12),
                KeyCode::PageDown => scroll.saturating_add(12),
                KeyCode::Home => 0,
                KeyCode::End => text.lines().count().saturating_sub(1),
                _ => *scroll,
            };
            *scroll = (*scroll).min(text.lines().count().saturating_sub(1));
        }
    }
    pub(super) fn poll_migration(&mut self) -> bool {
        let Some(job) = self.migration.job.as_ref() else {
            return false;
        };
        let visible = matches!(self.modal,Some(Modal::Migration {generation,..}) if generation==job.generation);
        let mut changed = false;
        if let Ok(progress) = job.progress.try_recv()
            && visible
        {
            self.migration_modal("Importing VS Code · Esc cancels", progress);
            changed = true;
        }
        if !self.migration.job.as_ref().unwrap().thread.is_finished() {
            return changed;
        }
        let job = self.migration.job.take().unwrap();
        let result = job
            .thread
            .join()
            .unwrap_or_else(|_| Err("Migration worker failed".into()));
        let committed = matches!(&result, Ok(Output::Applied(..) | Output::RolledBack(..)));
        if !visible && !committed {
            return true;
        }
        match result {
            Ok(Output::Sources(sources)) => {
                self.migration.sources = sources.profiles;
                self.migration.selected = 0;
                self.migration.phase = Phase::Sources;
                self.show_migration_sources(&sources.notices);
            }
            Ok(Output::Preview(preview)) => {
                if let Some(source) = &preview.report.profile
                    && let Some(previous) = self.migration.sources.get_mut(self.migration.selected)
                {
                    *previous = source.clone();
                }
                self.migration.preview = Some(preview);
                self.migration.phase = Phase::Review;
                self.show_migration_review();
            }
            Ok(Output::Applied(report, runtime)) => {
                let message = format!(
                    "Imported {} configuration files and {} local extensions; {} code execution grants. Original files retained. Extension workflows remain unqualified.",
                    report.files.len(),
                    report.extensions_imported,
                    report.extensions_execution_enabled
                );
                self.finish_migration_runtime(runtime, &message, visible);
            }
            Ok(Output::RolledBack(runtime)) => self.finish_migration_runtime(
                runtime,
                "Previous profile restored; imported copies and open documents are retained.",
                visible,
            ),
            Err(error) => {
                self.migration.phase = Phase::Done;
                self.message = format!("Migration failed: {error}");
                if visible {
                    self.migration_modal("VS Code Migration · Esc closes",format!("Migration failed: {error}\n\nOriginal VS Code files and open documents were retained. If profile activation completed before a later reload failure, restart to load the committed copy."));
                }
            }
        }
        true
    }
    fn finish_migration_runtime(
        &mut self,
        runtime: Result<Runtime, String>,
        message: &str,
        visible: bool,
    ) {
        let result = runtime
            .map_err(anyhow::Error::msg)
            .and_then(|runtime| self.install_migration_runtime(runtime));
        self.migration.phase = Phase::Done;
        self.message = match result {
            Ok(()) => message.into(),
            Err(error) => format!(
                "Profile committed; editor reload needs restart: {error:#}. Open buffers retained."
            ),
        };
        if visible {
            self.migration_modal(
                "VS Code Migration Complete · Esc returns to editing",
                self.message.clone(),
            );
        }
    }
    fn install_migration_runtime(&mut self, runtime: Runtime) -> Result<()> {
        self.invalidate_settings_profile()?;
        if let Some(loader) = &mut self.settings_loader {
            loader.reconfigure(runtime.paths.clone())?;
        } else {
            self.settings_loader = Some(crate::settings::Loader::new(runtime.paths.clone())?);
        }
        self.cancel_suggestions();
        self.cancel_code_actions();
        self.clear_signature();
        self.settings_user = runtime.paths.first().cloned();
        self.settings = runtime.settings;
        self.settings_error = None;
        self.settings_profile_loaded();
        self.keymap = runtime.keymap;
        self.imported_keybinding_notices = runtime.notices;
        for document in self.documents.iter_mut().chain(&mut self.hidden_documents) {
            document.retire_typing_pairs();
            self.settings.apply(document);
        }
        if !self.settings.editor_preview().enabled {
            self.keep_preview_tabs()?;
        }
        if runtime.extensions != self.extensions_directory {
            self.stop_extension_host();
            self.extension_packages.clear();
            self.extensions_directory = runtime.extensions;
            self.configure_extension_storage(Some(&runtime.extension_config));
            self.configure_extension_activation(Some(&runtime.extension_config));
            self.resume_automatic_extensions();
        }
        self.refresh_document_language_configurations();
        self.configure_themes(
            self.extensions_directory.clone(),
            Some(runtime.directory.join("theme-selection.json")),
            Some(self.settings.color_theme().unwrap_or("VSCLI Dark").into()),
            None,
        );
        Ok(())
    }
    pub(super) fn settle_migration(&mut self) {
        if let Some(job) = self.migration.job.take() {
            job.cancel.store(true, Ordering::Release);
            let _ = job.thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        fs,
        time::{Duration, Instant},
    };
    fn source(root: &Path) -> Source {
        let user = root.join("User");
        let extensions = root.join("source-extensions");
        fs::create_dir_all(&user).unwrap();
        fs::create_dir_all(extensions.join("fixture.native")).unwrap();
        fs::write(
            user.join("settings.json"),
            r##"{// original
            "editor.tabSize":2,"workbench.colorCustomizations":{"editor.foreground":"#123456"},
        }"##,
        )
        .unwrap();
        fs::write(
            user.join("keybindings.json"),
            r#"[{"key":"f6","command":"type","args":{"text":"imported"}}]"#,
        )
        .unwrap();
        fs::write(extensions.join("fixture.native/package.json"),r#"{"publisher":"fixture","name":"native","version":"1.0.0","main":"main.cjs","engines":{"vscode":"^1.95.0"}}"#).unwrap();
        fs::write(
            extensions.join("fixture.native/main.cjs"),
            "throw new Error('migration must not execute code')",
        )
        .unwrap();
        fs::write(extensions.join("extensions.json"),serde_json::to_vec(&json!([{"identifier":{"id":"fixture.native"},"version":"1.0.0","relativeLocation":"fixture.native","location":{"scheme":"file","path":extensions.join("fixture.native")}}])).unwrap()).unwrap();
        discovery::default_source("Fixture", &user, &extensions).unwrap()
    }
    fn settle(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(8);
        while app.migration.job.is_some() {
            app.poll_migration();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    #[test]
    fn actual_review_apply_and_rollback_preserve_dirty_shared_documents_and_replace_bindings() {
        let root = tempfile::tempdir().unwrap();
        let source = source(root.path());
        let native = root.path().join("native");
        fs::create_dir(&native).unwrap();
        fs::write(native.join("settings.json"), r#"{"editor.tabSize":8}"#).unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.configure_migration(Some(native.clone()), None);
        app.configure_settings(Some(native.join("settings.json")))
            .unwrap();
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.doc_mut().insert("unsaved λ🙂\r\n", false);
        app.execute("workbench.action.splitEditor", Value::Null);
        let before = (
            app.doc().id,
            app.doc().revision,
            app.doc().text.to_string(),
            app.doc().selections(),
        );
        let original = fs::read(source.user.join("settings.json")).unwrap();
        app.migration.options = ApplyOptions {
            import_extensions: true,
            activate_extensions: false,
        };
        app.start_migration(Work::Preview(source.clone()));
        settle(&mut app);
        assert!(app.migration.phase == Phase::Review);
        app.migration_key(key(KeyCode::Enter));
        settle(&mut app);
        assert!(app.migration.phase == Phase::Done, "{}", app.message);
        assert_eq!(
            (
                app.doc().id,
                app.doc().revision,
                app.doc().text.to_string(),
                app.doc().selections()
            ),
            before
        );
        assert_eq!(app.doc().tab_size, 2);
        app.migration_key(key(KeyCode::Esc));
        app.event(Event::Key(key(KeyCode::F(6))));
        assert!(app.doc().text.to_string().contains("imported"));
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), before.2);
        assert_eq!(
            fs::read(source.user.join("settings.json")).unwrap(),
            original
        );
        app.rollback_vscode_import();
        settle(&mut app);
        assert_eq!(app.doc().tab_size, 8);
        assert_eq!(app.doc().id, before.0);
        assert_eq!(app.doc().text.to_string(), before.2);
        assert!(app.doc().dirty());
        assert_eq!(engine::active_directory(&native).unwrap(), native);
    }
    #[test]
    fn dismissed_jobs_keep_their_actual_slot_and_do_not_reopen_the_review() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.configure_migration(Some(root.path().join("native")), None);
        let cancel = Arc::new(AtomicBool::new(false));
        let (_progress, receiver) = mpsc::sync_channel(1);
        let (release, held) = mpsc::sync_channel(1);
        let thread = std::thread::spawn(move || {
            let _ = held.recv_timeout(Duration::from_secs(5));
            Err("canceled fixture".into())
        });
        app.migration.job = Some(Job {
            generation: 1,
            cancel: cancel.clone(),
            progress: receiver,
            thread,
        });
        app.migration.generation = 1;
        app.migration_modal("held", String::new());
        app.migration_key(key(KeyCode::Esc));
        assert!(cancel.load(Ordering::Acquire));
        app.open_vscode_import();
        assert_eq!(app.migration.generation, 1);
        assert!(app.migration.job.is_some());
        release.send(()).unwrap();
        settle(&mut app);
        assert!(app.modal.is_none());
    }
    #[test]
    fn a_committed_receipt_survives_dismissal_even_when_later_runtime_reload_fails() {
        let root = tempfile::tempdir().unwrap();
        let source = source(root.path());
        let native = root.path().join("native");
        let report = engine::preview_profile(&source, Profile::Linux)
            .unwrap()
            .apply(&native)
            .unwrap();
        let committed = report.activated_profile.clone().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.configure_migration(Some(native.clone()), None);
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.doc_mut().insert("retained dirty text", false);
        let cancel = Arc::new(AtomicBool::new(false));
        let (_progress, receiver) = mpsc::sync_channel(1);
        let (release, held) = mpsc::sync_channel(1);
        let thread = std::thread::spawn(move || {
            let _ = held.recv_timeout(Duration::from_secs(5));
            Ok(Output::Applied(
                report,
                Err("workspace settings became malformed".into()),
            ))
        });
        app.migration.job = Some(Job {
            generation: 1,
            cancel,
            progress: receiver,
            thread,
        });
        app.migration.generation = 1;
        app.migration_modal("held", String::new());
        app.migration_key(key(KeyCode::Esc));
        release.send(()).unwrap();
        settle(&mut app);
        assert!(app.message.contains("Profile committed"), "{}", app.message);
        assert!(app.message.contains("restart"));
        assert!(app.modal.is_none());
        assert_eq!(app.doc().text.to_string(), "retained dirty text");
        assert_eq!(engine::active_directory(&native).unwrap(), committed);
    }
}
