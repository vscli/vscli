# VSCLI

**A native terminal editor with familiar VS Code workflows.**

[![CI](https://github.com/vscli/vscli/actions/workflows/ci.yml/badge.svg)](https://github.com/vscli/vscli/actions/workflows/ci.yml)

Open a project, find files, edit across split panes, and run commands without
leaving your terminal. VSCLI is written in Rust. Core editing needs no Node.js,
browser, or account.

**Currently in alpha.** Everyday editing is available, while VS Code feature and
extension compatibility are still being built. See [current limits](#current-limits)
before making it your main editor.

## What you can do

- **Edit with familiar controls:** multiple cursors, selections, Undo/Redo,
  comments, snippets, and find/replace.
- **Move around your project:** file Explorer, quick open, workspace search,
  symbol navigation, Outline, and Breadcrumbs.
- **Work across files:** independent tabs and split panes sharing the same
  document, with Save/Discard/Cancel when closing unsaved work.
- **Get language help:** completion, diagnostics, navigation, and formatting
  through installed language servers.
- **Keep tools close:** an integrated terminal, shell tasks from
  `.vscode/tasks.json`, and Git status, diffs, staging, and commits.
- **Bring your preferences:** preview and import supported VS Code settings,
  keybindings, snippets, and color themes.

The [user guide](docs/USAGE.md) covers these workflows in detail.

## Install from source

You'll need Git, Rust installed through rustup, and your platform's native build
tools. The repository selects its pinned Rust toolchain automatically
(currently 1.99.0).

```sh
git clone https://github.com/vscli/vscli.git
cd vscli
cargo install --path . --locked
vscli .
```

Make sure Cargo's binary directory is on your `PATH`. To try VSCLI without
installing it:

```sh
cargo build --release --locked
./target/release/vscli .
```

## Your first session

Open a project directory or individual files:

```sh
vscli .
vscli src/main.rs README.md
```

Press **F1** to discover commands, **Ctrl+P** to find a file, and **Ctrl+S** to
save. On macOS, use the Command shortcuts below. Starting without a file opens
the welcome screen; **Ctrl+N** / **Cmd+N** creates a new buffer.

| Action | Linux / Windows | macOS |
| --- | --- | --- |
| Command palette | F1 or Ctrl+Shift+P | F1 or Cmd+Shift+P |
| Quick open | Ctrl+P | Cmd+P |
| Save / Save As | Ctrl+S / Ctrl+Shift+S | Cmd+S / Cmd+Shift+S |
| Undo / Redo | Ctrl+Z / Ctrl+Y | Cmd+Z / Cmd+Shift+Z |
| Find in file | Ctrl+F | Cmd+F |
| Search workspace | Ctrl+Shift+F | Cmd+Shift+F |
| Settings | Ctrl+, | Cmd+, |
| Close tab | Ctrl+W | Cmd+W |
| Exit | Ctrl+Shift+W | Cmd+Shift+W |

If a shortcut doesn't arrive as expected, open **F1 → Keyboard Inspector** to see
what your terminal sends. For a remote session, choose your local keyboard's
profile, for example `vscli --keymap macos .`.

Clean-file session restoration is opt-in: launch with `vscli --restore-session .`.
Recovery of unsaved buffers after a crash is enabled separately by default.

## Coming from VS Code?

Choose **Import from VS Code** on the welcome screen, or run
**F1 → Preferences: Import from VS Code**. Pick a local profile, review its
configuration and extensions, then apply a separate native copy. Open documents
and unsaved edits stay in place. Code activation is a separate choice in review.

CLI equivalents:

```sh
vscli --list-vscode-profiles
vscli --import-vscode
vscli --import-vscode --import-extensions --apply-import
```

Use `--vscode-profile NAME` to choose a named profile, or pass a User directory
after `--import-vscode`. Add `--activate-imported-extensions` only when you want
source-enabled code extensions to activate. To restore the previous profile,
use **Preferences: Roll Back VS Code Import** or `vscli --rollback-import`.

Discovery also looks for the source application so its built-in extensions can
come across. If it is missing or ambiguous, press **B** on the profile picker or
pass `--vscode-application /path/to/Code` (an executable, installation directory or
`resources/app` root). Source-disabled native data stays disabled; copying packages
without code activation still makes enabled themes, snippets and typing data available.

Your original VS Code files stay unchanged. Unsupported behavior is reported;
copying an extension does not establish that all its features work. See the
[migration and theme guide](docs/IMPORT_AND_THEMES.md) for setup and current limits.

Use **F1 → Preferences: Color Theme** to choose a built-in or installed theme.
Imported `workbench.colorCustomizations` applies to supported native colors,
including theme-specific overrides. Themes and snippet packages work without Node.js.
See the [migration release plan](docs/MIGRATION_RELEASE_PLAN.md) for the complete
setup migration goal.

## Language support and extensions

Install `clangd` for C/C++ or `rust-analyzer` for Rust and put it on your `PATH`.
VSCLI starts the appropriate installed server when you open a saved file. Other
servers can be configured with `--lsp`; see [language setup](docs/USAGE.md#language-servers).
Language servers are not downloaded automatically.

Browse packages with **F1 → Extensions: Search Open VSX**, or install a local
`.vsix` with **Extensions: Install from VSIX**. Installation does not execute
extension code. Open the installed list with **Ctrl+Shift+X** / **Cmd+Shift+X**
to review a package and explicitly activate it.

Code extensions use an optional Node.js process and currently support a subset
of the VS Code API. An installed extension may have features that don't work yet;
Microsoft Marketplace access is not implemented. Read the
[extension compatibility guide](docs/EXTENSIONS.md) before relying on a package.
The entire stable extension API remains the implementation goal, tracked in
[API progress](docs/extension-api/PROGRESS.md).

## Current limits

- Files must be UTF-8, contain no NUL bytes, and be at most 32 MiB when opened.
- Some shortcuts depend on enhanced terminal keyboard reporting. Keyboard
  layouts, multiplexers, and Windows terminal workflows still need broader testing.
- Native language support runs one language server per window at a time.
- Rich webviews, notebooks, and complete VS Code settings and extension
  compatibility are still in progress.
- Windows system clipboard integration is not implemented. Linux uses
  `wl-copy`/`wl-paste` or `xclip` when available; an internal clipboard remains
  available without them.

VSCLI checks for external file changes before saving and periodically snapshots
unsaved buffers for crash recovery. The latest edits may not reach a recovery
snapshot; save regularly. See [saving and recovery](docs/USAGE.md#saving-and-recovery)
for the behavior and remaining limits.

## Help and feedback

Run `vscli --help` for launch options or `vscli --doctor` for environment
diagnostics and configuration paths. For bugs, [open an issue](https://github.com/vscli/vscli/issues)
with your OS, terminal, VSCLI version, and steps to reproduce.

Want to contribute? Start with [CONTRIBUTING.md](CONTRIBUTING.md). The
[release checklist](docs/RELEASE_CHECKLIST.md), [architecture](docs/ARCHITECTURE.md),
and [feature backlog](docs/FEATURE_TODO.md) describe the wider project.

## License

Original project code is available under [MIT](LICENSE-MIT) or
[Apache 2.0](LICENSE-APACHE), at your option. Dependencies retain their own licenses.
