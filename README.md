# VSCLI

[![CI](https://github.com/vscli/vscli/actions/workflows/ci.yml/badge.svg)](https://github.com/vscli/vscli/actions/workflows/ci.yml)

VSCLI is a native Rust terminal editor working toward the VS Code workflow. **An evolving editor alpha is implemented.** Edit UTF-8 files with multiple cursors and shared split views, search workspaces, navigate symbols, run shells/tasks, and inspect or stage Git changes. Installed clangd (C/C++) and rust-analyzer (Rust) start automatically when a saved file is selected; completion appears in a native caret popup as you type, with Tab/Enter acceptance. Core editing needs no Node.js, browser or account.

```sh
cargo build --release --locked
./target/release/vscli .
# Or open files directly:
./target/release/vscli src/main.rs README.md
```

Press **F1** for commands, **Ctrl+P** for quick open, **Ctrl+S** to save, and **Ctrl+Shift+W** to exit. macOS uses the corresponding Command bindings; use `--keymap macos` when your local keyboard is macOS but the editor runs remotely. The empty welcome screen offers recent files, clickable actions and the active settings path, with graphical or cell-based artwork. F1 → Keyboard Inspector shows themed keycaps, received modifiers and the resolved command; it diagnoses delivered input, not physical keys intercepted by the terminal.

Read the [usage and testing guide](docs/USAGE.md) for the full implemented feature list, shortcuts, recovery behavior, and limitations. `vscli --doctor` prints local environment diagnostics and keybinding/recovery paths; the welcome screen shows the active settings path.

This alpha implements a **subset** of VS Code commands and bindings. Native language features include diagnostics, completion, explicit parameter hints, symbols, navigation, formatting and limited rename/refactoring. One native language server is active per window; manual configuration and disable controls remain available. Clean-file session restoration is opt-in and separate from dirty-buffer crash recovery. Native debugging supports breakpoints, stepping, stack/variables and expression evaluation through an explicitly configured stdio adapter; Python/debugpy is tested.

An optional Node.js **CommonJS compatibility host** runs up to eight authorized packages with supported lazy activation. Its bounded subset includes shared documents/edits, configuration/Mementos, seven language-provider routes, native prompts and output/status/tree surfaces. Native Open VSX installation, themes and snippet catalogs work without code activation. See [extension compatibility](docs/EXTENSIONS.md) and [named provider workflows](docs/EXTENSION_PROVIDERS.md): installing or activating an extension does not establish that all its features work.

Full keybinding behavior and extension compatibility remain project goals. CI covers Linux, macOS and Windows, with Unix PTY workflows on Linux/macOS. Physical keyboard/layout, arbitrary terminal graphics and Windows ConPTY qualification remain incomplete; folding and many advanced IDE workflows remain outstanding. Resolved completion and linked snippet/import insertion follow the [bounded IntelliSense contract](docs/COMPLETIONS.md).

## Development

Use the pinned Rust toolchain. See [contributing](CONTRIBUTING.md) for atomic commits, checks, review, and release publishing.

```sh
cargo fmt --all --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
# Optional real-server test, requires clangd:
cargo test --test language_server real_clangd -- --ignored
# Optional real-debugger test, requires debugpy in the chosen Python:
VSCLI_TEST_PYTHON=/path/to/python cargo test --test debug_adapter -- --ignored
cargo build --locked
python3 tests/pty_smoke.py target/debug/vscli
```

The PTY tests require Unix and Python 3 and edit only temporary files. They exercise the actual binary, including a forced crash and recovery. Unit tests also cover randomized edits, Unicode, undo, external modifications, permissions, symlinks, keybinding contexts, and rendering at small sizes.

## Project direction

The implementation uses a Rust editor and terminal interface with an optional isolated Node.js compatibility process. Core editing must work without that process. A completely JavaScript-free application cannot also run arbitrary existing JavaScript extensions unchanged.

The product promise should be **a fast terminal IDE with exact VS Code keybindings on qualified terminal configurations and a growing, verified set of extensions**. Exact platform defaults and unchanged user bindings are a firm design requirement. Broad feature and extension compatibility remains staged; exact keyboard parity cannot be claimed on arbitrary unconfigured terminals.

The longer-term design lives in:

1. [Completion criteria](docs/RELEASE_CHECKLIST.md): the active full-project goal and remaining release requirements.
2. [Architecture](docs/ARCHITECTURE.md): product scope, technical decisions, process boundaries, editing model, terminal experience, and performance budgets.
3. [Compatibility](docs/COMPATIBILITY.md): keybindings, configuration, extension execution, distribution, and a measurable compatibility contract.
4. [Current parity plan](docs/PARITY_PLAN.md): implemented subsets, remaining scope and next priorities.
5. [Original roadmap](docs/ROADMAP.md): feasibility experiments, release gates, initial estimates and open-source stewardship.

The architecture documents describe the intended larger IDE; not every component is implemented in this alpha. Their timelines and performance budgets are planning targets, not measured results.

See [performance evidence](docs/PERFORMANCE.md) for the reproducible PTY benchmark, comparison boundaries, and remaining qualification. No fastest-editor claim is established.

Original project code is available under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option. Dependencies retain their own licenses; exact versions are recorded in Cargo.lock.
