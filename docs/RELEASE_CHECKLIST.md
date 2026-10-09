# VSCLI completion criteria

The active goal is the complete terminal IDE described by the user, not another basic editor milestone. This checklist records implementation and verification gaps. Passing a subset does not make the overall project complete. Exact compatibility claims require a named VS Code reference and qualified terminal configurations; graphical extensions and restricted services require an explicit compatibility route rather than silent omission.

## Existing foundation

- [x] Native Rust executable with open/edit/save, tabs, explorer, command palette, and quick open.
- [x] Selection, grapheme movement, undo/redo, literal find/replace, comments, and indentation.
- [x] Optimistic external-change protection and process-crash recovery snapshots.
- [x] Platform shortcut profiles and a tested subset of custom keybinding rules.
- [x] Unit tests, CI definition, and end-to-end PTY tests for the first alpha.

## Required development and verification

- [ ] Mature editing: multiple cursors, line operations, bracket matching, snippets, folding, wrapping, and independent split views.
- [ ] Exact keybinding contract: pinned upstream inventory, context/command behavior tests, terminal configuration recipes, and real-terminal qualification.
- [ ] Language intelligence: LSP lifecycle, diagnostics, completion, hover, navigation, references, formatting, rename, code actions, and server provisioning.
- [ ] Workspace workflows: cancellable search, replacement, file create/rename/delete, live file/index updates, and multi-root support.
- [ ] Git workflows: status, diffs, stage/unstage, commits, history, and conflict handling.
- [ ] Task execution: structured process/shell tasks, output, cancellation, variable expansion, and problem matchers.
- [ ] Integrated terminal: PTY, emulation, scrolling, resize, keyboard routing, and platform verification.
- [ ] Debugging and tests: DAP sessions, breakpoints, stepping, stack/variables/watch, console, and test-provider UI.
- [ ] Extensions: actual host strategy experiment, stable API baseline, package install/rollback, lifecycle, and named real-extension workflow tests.
- [ ] Rich extension UI: native contribution adapters plus an evaluated browser compatibility route for webviews/notebooks/custom editors.
- [ ] Configuration: settings/workspaces/profiles migration, full supported context expressions, secrets, trust, and understandable capability reporting.
- [ ] Reliability: versioned edits, recovery/persistence failure injection, encoding handling, performance measurements, accessibility, and independent state ownership.
- [ ] Release: platform qualification, reproducible packaging, dependency notices, upgrade/rollback, contributor tooling, and honest compatibility reports.

## Current implementation pass

Follow the [parity and user experience plan](PARITY_PLAN.md): integrate welcome, extension installation, themes and migration first; then useful concurrent extensions, familiar navigation and language workflows, session continuity, and the full development loop. C++/clangd is the first complete language qualification, while broad user impact determines priorities. Keep main runnable and distinguish branch implementation, merged behavior and qualification evidence.

## Verified additions during the full-project goal

- Multiple cursors and selection transactions; Unicode replacement and grouped undo verified through a real PTY. Disjoint line move/copy and adjacent selection boundaries are tested.
- Bounded, cancellable workspace search with regex/options, ignore handling, unsaved overlays, and result navigation.
- Native stdio LSP transport, diagnostics, hover, completion, definition/references, formatting, and limited rename. Fixture tests cover stale edits; installed clangd passes diagnostics/hover/formatting integration.
- Embedded native PTY terminal with VT rendering, asynchronous I/O, scrollback, resizing, session switching, and shell routing; native and nested-PTY tests pass.
- Explicit tasks.json process/POSIX-shell execution with variable expansion, session trust, build shortcut, and terminal output; dependencies and matchers remain incomplete.
- Git status, file diff, stage/unstage, commit, and history; temporary-repository tests and Source Control PTY workflow pass.
- These additions do not satisfy the complete editing, language-tooling, or workspace criteria above.

- Independent split views share a document and versioned undo history; core tests cover cursor mapping through disjoint edits and grouped undo/redo. Nested/resizable layouts and per-group tab stacks remain incomplete.

- Explorer file/folder creation, rename with open-buffer path updates, system trash, and explicit index refresh are implemented; create/rename collision and unsaved-buffer tests pass. Trash restoration remains an OS workflow; native watching and index updates are implemented as described below.

- Native DAP launch, breakpoints, stepping, stack/scopes/variables, and evaluation pass a real debugpy integration test. A deterministic adapter covers out-of-order variables, failed stepping, and disconnect. Launch/attach configuration, advanced breakpoints, watches, and test providers remain incomplete.

- Native file notifications, background index replacement, version-checked disk reads, undoable clean-buffer reloads, dirty conflict notices, and deleted-buffer retention are implemented. Non-Linux/network watcher behavior and large-tree performance remain unqualified.

- Background Tree-sitter highlighting covers Rust, Python, JavaScript/JSX, TypeScript/TSX, and JSON. Tests cover multiline constructs, Unicode, grammar loading, and stale revisions. Incremental tree reuse, injections, semantic tokens, folding, and additional grammars remain incomplete.

- User/workspace settings import with language overrides, configurable indentation/line numbers, unsupported-setting reporting, and background reload is implemented. The broader configuration and profile migration contract remains incomplete.

- An optional isolated Node host runs CommonJS command extensions with native palette commands, synchronous document mirrors, version-checked edits, and native undo. Extension keybindings preserve platform overrides and user precedence/removals. Unchanged Tyriar Sort Lines 1.12.0 F9 sorting/undo passes; stale/invalid transactions and process crashes are tested. Local VSIX installation now supports bounded extraction, atomic registry updates, rollback, provenance, CLI/editor management and explicit installed-package activation. A second named Sort Lines test packages unchanged compiled entry files and exercises installation, F9 and undo. Broad API coverage, registry distribution, storage garbage collection, automatic activation, menus, providers, and rich UI remain incomplete. See [extension evidence](EXTENSIONS.md).

- Extension configuration now imports user/workspace settings before activation, preserves ordered language overrides, exposes snapshot reads/current inspection, and delivers live change events. Native/Node tests cover scope precedence, object merging, shadowed changes, malformed-JSON retention, and held event snapshots. Unchanged Sort Lines also passes its whole-file setting and reload workflow. A real VS Code 1.95.0 differential fixture now compares 25 read/event observations and caught an ignored-scope inspection mismatch. Complete defaults, configuration writes, additional scopes, and broader differential qualification remain incomplete.

- The pinned reference harness exports full default keybinding inventories and actual keyboard layout metadata, and reports every rule against VSCLI defaults. These structural comparisons do not qualify the resolver, physical input, or command effects. See the [harness boundaries](../tests/vscode-reference/README.md).

- Opened buffers and saved baselines now share rope storage; file reads, save conflict checks, and writes stream through bounded buffers. Periodic unchanged-file comparisons run in the watcher worker without replacement ropes or input-thread whole-file comparisons. Unicode/CRLF persistence, interrupted reads, same-size changes, undo, and recovery remain tested. Executable measurements show lower resident memory, but the 32 MiB cap and larger-file qualification remain unresolved.

- Periodic recovery uses one outstanding shared-rope snapshot and a dedicated writer, preserving the v1 journal format. Tests cover edits during stalled writes, revision acknowledgements, failure/retry, Unicode/CRLF/JSON escaping, and ordered shutdown. PTY coverage includes abrupt recovery and a final snapshot on SIGTERM. A recovery-enabled executable workload records substantially lower typing tail latency at 10/32 MiB; see the performance report and raw samples. Startup restoration and shutdown can still block; language-service/extension contention and power-loss durability remain unqualified.


- Long-line movement and column lookup traverse rope chunks, with upstream Unicode boundary fixes and Unicode 17 conformance checks. Plain-text/ready-grammar rendering limits copying and scanning to viewport prefixes; PTY tests cover long-line Unicode edits, undo, scrolling, and CRLF saves. Executable and native measurements are recorded in the performance report. Far-right layout, fallback syntax work, larger files, and full IDE contention remain unqualified.

- Literal snippet insertion is connected to native variables and conditional Tab/Shift+Tab/Escape bindings, with linked editing, Unicode/CRLF persistence, cursor-order preservation and undo/redo coverage. User/workspace and installed-VSIX catalogs run in a bounded worker without code activation. Reference traces distinguish API, user-command, variable and cancellation behavior; executable PTYs exercise native insertion, installed C++ snippets without Node, and uninstall discovery. Rust integration tests cover upgrade/rollback discovery. Choices, nested session merging, completion/extension insertion and full regex/input qualification remain incomplete.

- The empty welcome screen, native VSIX installation/rollback, copied VS Code user-profile import and persistent native themes are merged. Combined PTY workflows cover original-source preservation, supported imported bindings/snippets, RGB rendering, explicit code activation, save/undo, final-tab close and restart. All required Linux/macOS/Windows PR checks passed. Imported profiles are editable copies; themes map a native palette and syntax categories approximately. Full profiles/sync, broad extension APIs and TextMate fidelity remain incomplete; see the current [parity plan](PARITY_PLAN.md).
