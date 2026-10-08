# VSCLI

[![CI](https://github.com/vscli/vscli/actions/workflows/ci.yml/badge.svg)](https://github.com/vscli/vscli/actions/workflows/ci.yml)

VSCLI is a native Rust terminal editor working toward the VS Code workflow. **An evolving editor alpha is implemented.** You can edit UTF-8 files with multiple cursors and split views, search a workspace, use tabs and undo/redo, connect a language server for completion, diagnostics, hover, navigation, and formatting, run shells/tasks, and inspect or stage Git changes. It needs no Node.js, browser, or account.

```sh
cargo build --release --locked
./target/release/vscli .
# Or open files directly:
./target/release/vscli src/main.rs README.md
```

Press **F1** for commands, **Ctrl+P** for quick open, **Ctrl+S** to save, and **Ctrl+Shift+W** to exit. macOS uses the corresponding Command bindings; use `--keymap macos` when your local keyboard is macOS but the editor runs remotely. If a shortcut is intercepted, F1 → Keyboard Inspector helps diagnose the received input.

Read the [usage and testing guide](docs/USAGE.md) for the full implemented feature list, shortcuts, recovery behavior, and limitations. `vscli --doctor` prints local diagnostics and configuration paths.

This alpha implements a **subset** of VS Code commands and bindings. Native debugging supports breakpoints, stepping, stack/variables, and expression evaluation through an explicitly configured stdio adapter; Python/debugpy is tested. An optional experimental Node host runs a narrow command/edit extension API; unchanged Sort Lines 1.12.0 is tested. See [extension compatibility](docs/EXTENSIONS.md) for the tested workflow and missing APIs. Language tooling currently requires an explicitly configured stdio server. Full keybinding behavior and extension compatibility remain project goals. CI builds and tests Linux, macOS, and Windows; Unix PTY workflows run on Linux and macOS. Real terminal/keymap qualification and Windows ConPTY interaction coverage remain incomplete.

## Development

Use the pinned Rust toolchain. See [contributing](CONTRIBUTING.md) for atomic commits, checks, review, and release publishing.

```sh
cargo fmt --check
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

The recommended architecture is a Rust editor and terminal interface, with an optional Node.js process for compatible VS Code extensions. Core editing must work without that process. A completely JavaScript-free application cannot also run arbitrary existing JavaScript extensions unchanged.

The product promise should be **a fast terminal IDE with exact VS Code keybindings on qualified terminal configurations and a growing, verified set of extensions**. Exact platform defaults and unchanged user bindings are a firm design requirement. Broad feature and extension compatibility remains staged; exact keyboard parity cannot be claimed on arbitrary unconfigured terminals.

The longer-term design lives in:

1. [Completion criteria](docs/RELEASE_CHECKLIST.md): the active full-project goal and remaining release requirements.
2. [Architecture](docs/ARCHITECTURE.md): product scope, technical decisions, process boundaries, editing model, terminal experience, and performance budgets.
3. [Compatibility](docs/COMPATIBILITY.md): keybindings, configuration, extension execution, distribution, and a measurable compatibility contract.
4. [Roadmap](docs/ROADMAP.md): feasibility experiments, release gates, initial backlog, validation, staffing assumptions, and open-source stewardship.

The architecture documents describe the intended larger IDE; not every component is implemented in this alpha. Their timelines and performance budgets are planning targets, not measured results.

See [performance evidence](docs/PERFORMANCE.md) for the reproducible PTY benchmark, comparison boundaries, and remaining qualification. No fastest-editor claim is established.

Original project code is available under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option. Dependencies retain their own licenses; exact versions are recorded in Cargo.lock.
