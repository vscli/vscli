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
    /// Start this language server over stdio (explicit executable, no shell)
    #[arg(long)]
    lsp: Option<String>,
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
    /// Run a trusted unpacked extension directory or installed publisher.name (executes code)
    #[arg(long)]
    extension: Option<PathBuf>,
    /// Install a local VSIX without executing its code
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
    /// Override native extension storage
    #[arg(long)]
    extensions_dir: Option<PathBuf>,
    /// Node executable for the optional extension host
    #[arg(long, default_value = "node")]
    extension_node: String,
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
    let extensions_directory = args
        .extensions_dir
        .clone()
        .or_else(vscli::extension_store::default_directory);
    if args.install_extension.is_some()
        || args.list_extensions
        || args.uninstall_extension.is_some()
        || args.rollback_extension.is_some()
    {
        let store = vscli::extension_store::Store::new(
            extensions_directory
                .clone()
                .context("No extension storage directory; use --extensions-dir")?,
        );
        if let Some(path) = &args.install_extension {
            let installed = store.install(path)?;
            println!(
                "Installed {}@{}\n{}\nInstallation does not activate code. Run: vscli --extension {}",
                installed.id, installed.version, installed.compatibility, installed.id
            );
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
    app.extensions_directory = extensions_directory.clone();
    app.extension_node = args.extension_node.clone();
    let settings_path = args
        .settings
        .clone()
        .or_else(|| recovery::config_path().map(|p| p.with_file_name("settings.json")));
    let settings_error = app.configure_settings(settings_path).err();
    for path in args.paths.iter().filter(|p| !p.is_dir()) {
        app.open(path)?;
    }
    let custom = args
        .keybindings
        .clone()
        .or_else(|| recovery::config_path().filter(|p| p.exists()));
    if let Some(path) = custom {
        let count = app.keymap.load(&path)?;
        app.message = format!("Loaded {count} custom keybinding rules");
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
    if let Some(program) = &args.lsp {
        app.lsp = Some(vscli::lsp::Client::start(
            program,
            &args.lsp_arg,
            &app.workspace.root,
            args.lsp_language,
        )?);
        app.message = format!("Starting language server: {program}");
    }
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
    if let Some(extension) = args.extension {
        let extension = if extension.is_dir() {
            extension
        } else {
            vscli::extension_store::Store::new(
                extensions_directory.context("No extension storage directory")?,
            )
            .get(&extension.to_string_lossy())?
            .path
        };
        app.extension_host = Some(vscli::extensions::Client::start(
            &args.extension_node,
            &extension,
            &app.workspace.root,
            &app.documents,
            app.active,
            &app.settings,
        )?);
    }
    if let Some(error) = settings_error {
        app.message = format!("Settings failed to load; using defaults: {error:#}");
    } else if !app.settings.warnings.is_empty() {
        app.message = format!(
            "{} settings notices · F1 → Settings: Compatibility Report",
            app.settings.warnings.len()
        );
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
                worker.preserve(&app.documents)?;
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
                && let Err(e) = worker.submit(&app.documents)
            {
                app.message = format!("Recovery write failed: {e:#}");
                redraw = true;
            }
            last_recovery = Instant::now();
        }
    }
    drop(terminal);
    drop(guard);
    if let Some(worker) = recovery {
        worker.finish()?;
    }
    Ok(())
}
