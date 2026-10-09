use anyhow::{Context, Result, bail};
use clap::Parser;
use crossterm::{
    cursor::Show,
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use vscli::{
    app::App,
    keys::Profile,
    recovery::{self, Recovery},
    ui,
};

#[derive(Parser)]
#[command(
    version,
    about = "Native terminal editor with VS Code keyboard workflows",
    after_help = "Start: vscli .   or   vscli src/main.rs\nF1: commands/help. Ctrl+S: save. Ctrl+Shift+W: exit (Cmd+Shift+W on macOS).\nSome keys require enhanced terminal reporting; use F1 → Keyboard Inspector."
)]
struct Args {
    /// Files to open, or a directory to use as the workspace
    paths: Vec<PathBuf>,
    /// Workspace directory (defaults to the supplied directory or current directory)
    #[arg(short, long)]
    workspace: Option<PathBuf>,
    /// VS Code keyboard profile; choose your local OS when editing over SSH
    #[arg(long, value_enum)]
    keymap: Option<Profile>,
    /// Import a VS Code keybindings.json (supported when expressions are documented)
    #[arg(long)]
    keybindings: Option<PathBuf>,
    /// Import user settings.json; workspace .vscode/settings.json is layered above it
    #[arg(long)]
    settings: Option<PathBuf>,
    /// Preview importing a VS Code User directory (settings, keybindings, snippets)
    #[arg(long, conflicts_with_all = ["install_extension", "list_extensions", "uninstall_extension", "rollback_extension"])]
    import_vscode: Option<PathBuf>,
    /// VS Code extensions directory from which to copy the selected color theme
    #[arg(long, requires = "import_vscode")]
    vscode_extensions: Option<PathBuf>,
    /// Activate the imported profile copy after a successful preview
    #[arg(long, requires = "import_vscode")]
    apply_import: bool,
    /// Native configuration root for import and startup (default: OS config directory)
    #[arg(long)]
    config_dir: Option<PathBuf>,
    /// Load a VS Code JSON/JSONC color theme without a JavaScript runtime
    #[arg(long)]
    theme: Option<PathBuf>,
    /// Start this language server over stdio (explicit executable, no shell)
    #[arg(long)]
    lsp: Option<String>,
    /// Disable native automatic language services for this window
    #[arg(long, conflicts_with = "lsp")]
    no_lsp: bool,
    /// Language served by --lsp, e.g. rust, python, cpp
    #[arg(long, requires = "lsp", default_value = "rust")]
    lsp_language: String,
    /// Argument to pass to the language server; repeat as needed
    #[arg(long, requires = "lsp", allow_hyphen_values = true)]
    lsp_arg: Vec<String>,
    /// Configure a stdio debug adapter; F5 starts it
    #[arg(long)]
    debug_adapter: Option<String>,
    /// Argument to the debug adapter (repeatable)
    #[arg(long, requires = "debug_adapter", allow_hyphen_values = true)]
    debug_arg: Vec<String>,
    /// Program to debug; defaults to the active saved file
    #[arg(long, requires = "debug_adapter")]
    debug_program: Option<PathBuf>,
    /// Argument to the debugged program (repeatable)
    #[arg(long, requires = "debug_adapter", allow_hyphen_values = true)]
    debug_program_arg: Vec<String>,
    /// Run a trusted unpacked extension or installed publisher.name (repeatable; executes code)
    #[arg(long)]
    extension: Vec<PathBuf>,
    /// Remember an installed package's code-execution grant; eligible code runs on later events
    #[arg(long, group = "extension_state", conflicts_with_all = ["disable_extension", "install_extension", "list_extensions", "uninstall_extension", "rollback_extension", "import_vscode", "extension", "search_extensions", "update_extension", "check_extension_updates"])]
    enable_extension: Option<String>,
    /// Revoke a remembered execution grant without starting Node
    #[arg(long, group = "extension_state", conflicts_with_all = ["install_extension", "list_extensions", "uninstall_extension", "rollback_extension", "import_vscode", "extension", "search_extensions", "update_extension", "check_extension_updates"])]
    disable_extension: Option<String>,
    /// Execution grant scope; workspace values override global values
    #[arg(
        long,
        value_enum,
        default_value = "global",
        requires = "extension_state"
    )]
    extension_scope: vscli::extension_activation::Scope,
    /// Install a local VSIX or an Open VSX publisher.name without executing code
    #[arg(long, conflicts_with_all = ["list_extensions", "uninstall_extension", "rollback_extension"])]
    install_extension: Option<PathBuf>,
    /// List installed packages and experimental compatibility status
    #[arg(long, conflicts_with_all = ["uninstall_extension", "rollback_extension"])]
    list_extensions: bool,
    /// Remove an installed package from the registry (running hosts retain their files)
    #[arg(long, conflicts_with = "rollback_extension")]
    uninstall_extension: Option<String>,
    /// Restore the previous installed generation
    #[arg(long)]
    rollback_extension: Option<String>,
    /// Search Open VSX without opening the UI
    #[arg(long, conflicts_with_all = ["install_extension", "list_extensions", "uninstall_extension", "rollback_extension", "update_extension", "check_extension_updates"])]
    search_extensions: Option<String>,
    /// Check installed packages for stable Open VSX updates
    #[arg(long, conflicts_with_all = ["install_extension", "list_extensions", "uninstall_extension", "rollback_extension", "update_extension"])]
    check_extension_updates: bool,
    /// Download and install the latest compatible stable package (never executes code)
    #[arg(long, conflicts_with_all = ["install_extension", "list_extensions", "uninstall_extension", "rollback_extension"])]
    update_extension: Option<String>,
    /// Open VSX-compatible registry URL (HTTPS or numeric loopback HTTP)
    #[arg(long, default_value = vscli::extension_registry::DEFAULT_URL)]
    extension_registry: String,
    /// Override native extension storage
    #[arg(long)]
    extensions_dir: Option<PathBuf>,
    /// Node executable for the optional extension host
    #[arg(long, default_value = "node")]
    extension_node: String,
    /// Restore the previous clean-file session for this workspace (explicit files take precedence)
    #[arg(long, conflicts_with = "no_session")]
    restore_session: bool,
    /// Disable clean-session metadata reads and writes (dirty recovery is independent)
    #[arg(long)]
    no_session: bool,
    /// Disable periodic recovery snapshots and startup recovery
    #[arg(long)]
    no_recovery: bool,
    /// Override the recovery directory
    #[arg(long)]
    recovery_dir: Option<PathBuf>,
    /// Leave mouse selection to the terminal emulator
    #[arg(long)]
    no_mouse: bool,
    /// Do not negotiate enhanced keyboard reporting
    #[arg(long)]
    legacy_keys: bool,
    /// Print environment and feature diagnostics without opening the UI
    #[arg(long)]
    doctor: bool,
    /// Print the implemented default keybinding rules as JSON
    #[arg(long)]
    list_keybindings: bool,
}

static ENHANCED: AtomicBool = AtomicBool::new(false);
struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}
fn restore_terminal() {
    if ENHANCED.swap(false, Ordering::SeqCst) {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
    }
    let _ = execute!(
        io::stdout(),
        DisableMouseCapture,
        DisableBracketedPaste,
        Show,
        LeaveAlternateScreen
    );
    let _ = terminal::disable_raw_mode();
}

fn main() -> Result<()> {
    let args = Args::parse();
    let profile = args.keymap.unwrap_or_else(Profile::native);
    let config_root = args
        .config_dir
        .clone()
        .or_else(vscli::migration::config_directory);
    let extensions_directory = args
        .extensions_dir
        .clone()
        .or_else(vscli::extension_store::default_directory);
    if let Some(id) = args
        .enable_extension
        .as_ref()
        .or(args.disable_extension.as_ref())
    {
        let store = vscli::extension_store::Store::new(
            extensions_directory
                .clone()
                .context("No extension storage directory; use --extensions-dir")?,
        );
        let installed = store.get(id)?;
        let root = args
            .workspace
            .clone()
            .or_else(|| args.paths.iter().find(|path| path.is_dir()).cloned())
            .unwrap_or(std::env::current_dir()?);
        let paths = vscli::extension_activation::state::Paths::new(config_root.as_deref(), &root)?;
        let enabled = args.enable_extension.is_some();
        vscli::extension_activation::state::change(
            paths.path(args.extension_scope)?,
            &installed.id,
            enabled,
        )?;
        let (global, workspace) = paths
            .read()
            .context("Enablement saved; cannot read effective workspace/global state")?;
        println!(
            "{} {} in {:?} scope; effective execution enabled={}\nCode runs only in a later native session after a supported activation event; dependencies need their own grants.",
            if enabled { "Enabled" } else { "Disabled" },
            installed.id,
            args.extension_scope,
            vscli::extension_activation::Preferences::enabled(&global, &workspace, &installed.id)
        );
        return Ok(());
    }
    let registry = vscli::extension_registry::Registry::new(&args.extension_registry)?;
    let registry_cancel = std::sync::atomic::AtomicBool::new(false);
    if let Some(query) = &args.search_extensions {
        println!(
            "{}",
            serde_json::to_string_pretty(&registry.search(query, &registry_cancel)?)?
        );
        return Ok(());
    }
    if args.install_extension.is_some()
        || args.list_extensions
        || args.uninstall_extension.is_some()
        || args.rollback_extension.is_some()
        || args.update_extension.is_some()
        || args.check_extension_updates
    {
        let store = vscli::extension_store::Store::new(
            extensions_directory
                .clone()
                .context("No extension storage directory; use --extensions-dir")?,
        );
        if let Some(path) = &args.install_extension {
            let installed = if path.is_file() {
                store.install(path)?
            } else {
                registry.install(
                    &registry.latest(&path.to_string_lossy(), &registry_cancel)?,
                    &store,
                    &registry_cancel,
                )?
            };
            println!(
                "Installed {}@{}\n{}\nInstallation does not activate code. Run: vscli --extension {}",
                installed.id, installed.version, installed.compatibility, installed.id
            );
        } else if args.check_extension_updates {
            println!(
                "{}",
                serde_json::to_string_pretty(&registry.updates(&store.list()?, &registry_cancel)?)?
            );
        } else if let Some(id) = &args.update_extension {
            let current = store.get(id)?;
            let entry = registry.latest(id, &registry_cancel)?;
            if semver::Version::parse(&entry.version)? > semver::Version::parse(&current.version)? {
                let installed = registry.install(&entry, &store, &registry_cancel)?;
                println!(
                    "Updated {}@{}; restart its host to use this version",
                    installed.id, installed.version
                );
            } else {
                println!(
                    "{}@{} is current; no downgrade applied",
                    current.id, current.version
                );
            }
        } else if let Some(id) = &args.uninstall_extension {
            store.uninstall(id)?;
            println!("Uninstalled {id}; retained immutable files for running hosts");
        } else if let Some(id) = &args.rollback_extension {
            let installed = store.rollback(id)?;
            println!("Restored {}@{}", installed.id, installed.version);
        } else {
            println!("{}", serde_json::to_string_pretty(&store.list()?)?);
        }
        return Ok(());
    }

    if let Some(source) = &args.import_vscode {
        let preview = vscli::migration::preview_with_extensions(
            source,
            profile,
            args.vscode_extensions.as_deref(),
        )?;
        let report = if args.apply_import {
            preview.apply(
                config_root
                    .as_deref()
                    .context("Native configuration directory unavailable; set --config-dir")?,
            )?
        } else {
            preview.report
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    let mut profile_warning = None;
    let active_config = config_root.as_deref().map(|root| {
        vscli::migration::active_directory(root).unwrap_or_else(|error| {
            profile_warning = Some(format!(
                "Imported profile unavailable; using native configuration: {error:#}"
            ));
            root.into()
        })
    });
    if args.doctor {
        println!(
            "VSCLI {}\nProfile: {profile:?}\nTerminal: {}\nTERM_PROGRAM: {}\nInteractive stdin/stdout: {}/{}\nMultiplexer: {}\nRecovery: {}\nUser keybindings: {}\n\nImplemented: UTF-8 editing, tabs, selections, undo/redo, safe save, find/replace,\nquick open, explorer, multi-cursor editing, split views, workspace search, native stdio LSP,\nterminal sessions, project tasks, Git status/diff/stage/commit/history,\nDAP breakpoints/stepping/variables, command palette, keybinding imports, crash snapshots.\nExtensions: optional experimental command/edit host via --extension (requires Node).\n\nRun F1 → Keyboard Inspector inside the editor to test actual key delivery.\nCtrl+Shift+P and other modified keys may require enhanced keyboard support.",
            env!("CARGO_PKG_VERSION"),
            std::env::var("TERM").unwrap_or_else(|_| "unset".into()),
            std::env::var("TERM_PROGRAM").unwrap_or_else(|_| "unset".into()),
            io::stdin().is_terminal(),
            io::stdout().is_terminal(),
            if std::env::var_os("TMUX").is_some() {
                "tmux"
            } else {
                "none detected"
            },
            args.recovery_dir
                .clone()
                .or_else(recovery::default_directory)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "unavailable".into()),
            args.keybindings
                .clone()
                .or_else(recovery::config_path)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "unavailable".into())
        );
        return Ok(());
    }
    if args.list_keybindings {
        let map = vscli::keys::Keymap::new(profile);
        println!("{}", serde_json::to_string_pretty(&map.bindings)?);
        return Ok(());
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("VSCLI needs an interactive terminal. Try vscli --help or --doctor.");
    }
    let root = args
        .workspace
        .clone()
        .or_else(|| args.paths.iter().find(|p| p.is_dir()).cloned())
        .unwrap_or(std::env::current_dir()?);
    let root = std::fs::canonicalize(root).context("Cannot open workspace directory")?;
    if !root.is_dir() {
        bail!("Workspace must be a directory");
    }
    let mut app = App::new(root, profile);
    app.configure_recents(config_root.as_deref());
    app.configure_extension_storage(config_root.as_deref());
    app.extensions_directory = extensions_directory.clone();
    app.extension_registry = registry;
    app.configure_extension_activation(config_root.as_deref());
    app.extension_node = args.extension_node.clone();
    let settings_path = args
        .settings
        .clone()
        .or_else(|| active_config.as_ref().map(|p| p.join("settings.json")));
    let settings_error = app.configure_settings(settings_path).err();
    app.configure_themes(
        args.extensions_dir.clone(),
        active_config
            .as_ref()
            .map(|path| path.join("theme-selection.json")),
        app.settings.color_theme().map(str::to_owned),
        args.theme.clone(),
    );
    for path in args.paths.iter().filter(|p| !p.is_dir()) {
        app.open(path)?;
    }
    let custom = args.keybindings.clone().or_else(|| {
        active_config
            .as_ref()
            .map(|p| p.join("keybindings.json"))
            .filter(|p| p.exists())
    });
    if let Some(path) = custom {
        if args.keybindings.is_none() && active_config != config_root {
            match app.keymap.load_imported(&path) {
                Ok((count, notices)) => {
                    app.message = format!(
                        "Loaded {count} imported keybinding rules · {} skipped rules · Settings: Compatibility Report",
                        notices.len()
                    );
                    app.imported_keybinding_notices = notices;
                }
                Err(error) => {
                    let notice = format!(
                        "Imported keybindings unavailable; using native defaults: {error:#}"
                    );
                    app.message = notice.clone();
                    app.imported_keybinding_notices.push(notice);
                }
            }
        } else {
            let count = app.keymap.load(&path)?;
            app.message = format!("Loaded {count} custom keybinding rules");
        }
    }
    let recovery = if args.no_recovery {
        None
    } else {
        let dir = args
            .recovery_dir
            .or_else(recovery::default_directory)
            .context("No state directory available; use --recovery-dir or --no-recovery")?;
        Some(Recovery::new(dir).context("Cannot initialize recovery storage")?)
    };
    if let Some(store) = &recovery {
        let (docs, old) = store.restore()?;
        if !docs.is_empty() {
            if app.documents.len() == 1 && app.doc().path.is_none() && app.doc().is_empty() {
                app.documents.clear();
            }
            let count = docs.len();
            app.documents.extend(docs);
            for doc in &mut app.documents {
                app.settings.apply(doc);
            }
            app.active = app.documents.len() - 1;
            store.persist(&app.documents)?;
            Recovery::consume(&old);
            app.message = format!(
                "Recovered {count} unsaved buffer(s). Disk files were not modified. Review and Save or Save As."
            );
        }
    }
    if !args.no_session {
        let explicit_files = args.paths.iter().any(|path| !path.is_dir());
        app.configure_session(
            config_root.as_deref(),
            args.restore_session && !explicit_files,
        )?;
    }
    let manual_language = args.lsp.map(|program| vscli::language_services::Launch {
        workspace: app.workspace.root.clone(),
        language: args.lsp_language,
        program,
        args: args.lsp_arg,
    });
    app.configure_language_services(manual_language, args.no_lsp)?;
    if let Some(adapter) = args.debug_adapter {
        let program = args.debug_program.map(|p| {
            if p.is_absolute() {
                p
            } else {
                app.workspace.root.join(p)
            }
        });
        app.debug_configuration = Some(vscli::debug::Configuration {
            adapter,
            adapter_args: args.debug_arg,
            program,
            program_args: args.debug_program_arg,
        });
    }
    if !args.extension.is_empty() {
        let packages = args
            .extension
            .iter()
            .map(|extension| {
                if extension.is_dir() {
                    vscli::extensions::Package::read(extension)
                } else {
                    let installed = vscli::extension_store::Store::new(
                        extensions_directory
                            .clone()
                            .context("No extension storage directory")?,
                    )
                    .get(&extension.to_string_lossy())?;
                    Ok(vscli::extensions::Package::installed(&installed))
                }
            })
            .collect::<Result<Vec<_>>>()?;
        app.start_extension_packages(packages)?;
    }
    if let Some(error) = settings_error {
        app.message = format!("Settings failed to load; using defaults: {error:#}");
    } else if !app.settings.warnings.is_empty() {
        app.message = format!(
            "{} settings notices · F1 → Settings: Compatibility Report",
            app.settings.warnings.len()
        );
    }
    if let Some(warning) = profile_warning {
        app.message = warning;
    }
    let interrupted = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    {
        signal_hook::flag::register(signal_hook::consts::SIGTERM, interrupted.clone())?;
        signal_hook::flag::register(signal_hook::consts::SIGINT, interrupted.clone())?;
    }
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous_hook(info);
    }));
    terminal::enable_raw_mode()?;
    let guard = TerminalGuard;
    execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
    if !args.no_mouse {
        execute!(io::stdout(), EnableMouseCapture)?;
    }
    if !args.legacy_keys && terminal::supports_keyboard_enhancement().unwrap_or(false) {
        execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
            )
        )?;
        ENHANCED.store(true, Ordering::SeqCst);
        app.enhanced = true;
    }
    let mut recovery = recovery.map(recovery::Worker::start).transpose()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    let mut redraw = true;
    let mut last_recovery = Instant::now();
    while app.running {
        if interrupted.load(Ordering::Relaxed) {
            if let Some(worker) = recovery.take() {
                worker.preserve_refs(&app.recovery_documents())?;
            }
            bail!("Interrupted; unsaved buffers retained in recovery storage when enabled");
        }
        if let Some(worker) = &mut recovery
            && let Err(e) = worker.poll()
        {
            app.message = format!("Recovery write failed: {e:#}");
            redraw = true;
        }
        redraw |= app.poll();
        if redraw {
            terminal.draw(|frame| ui::draw(frame, &mut app))?;
            redraw = false;
        }
        if event::poll(Duration::from_millis(50))? {
            app.event(event::read()?);
            redraw = true;
            // Bound each batch so continuous input cannot starve rendering or recovery.
            for _ in 0..31 {
                if !app.running {
                    break;
                }
                if !event::poll(Duration::ZERO)? {
                    break;
                }
                app.event(event::read()?);
            }
        }
        if last_recovery.elapsed() >= Duration::from_secs(2) {
            if let Some(worker) = &mut recovery
                && let Err(e) = worker.submit_refs(&app.recovery_documents())
            {
                app.message = format!("Recovery write failed: {e:#}");
                redraw = true;
            }
            last_recovery = Instant::now();
        }
    }
    drop(terminal);
    drop(guard);
    if let Err(error) = app.finish_session() {
        eprintln!("Session metadata not saved: {error:#}");
    }
    if let Err(error) = app.finish_recents() {
        eprintln!("{error:#}");
    }
    if let Some(worker) = recovery {
        worker.finish()?;
    }
    Ok(())
}
