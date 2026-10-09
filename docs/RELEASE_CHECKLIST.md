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

This overview describes the source included through
[PR #52](https://github.com/vscli/vscli/pull/52). Follow the
[parity and user experience plan](PARITY_PLAN.md). Native installed-server startup,
explicit parameter hints, document/workspace symbols, opt-in clean-session restore,
stable syntax colors, registry browsing/updates, lazy enabled extensions, native
prompts/documents/Mementos, seven providers and output/status/tree surfaces are
implemented. [PR #51](https://github.com/vscli/vscli/pull/51) adds graphical welcome
and the keycap inspector; PR #52 adds automatic typing suggestions and the native
caret popup with Tab/Enter acceptance. Source inclusion does not assert a future
CI result or full workflow qualification. These remain bounded subsets, so the
complete-product criteria above stay unchecked.

Folding remains unshipped; preserved source and a committed internal foundation
are not an available editor feature. The subsequent [resolved-completion slice](COMPLETIONS.md) adds bounded resolution,
linked snippet/import insertion, documentation and abbreviation ranking. Follow-up
completion commands and full IntelliSense parity remain incomplete. Extension
diagnostics and automatic parameter hints are next in the approved queue.

Follow the [user-approved twelve priorities](PARITY_PLAN.md#user-approved-implementation-order),
starting with IntelliSense qualification, then extension diagnostics and smart typing.
Qualify a complete C++/clangd repository workflow alongside these changes. C++ is the first end-to-end language qualification, not the product's
language boundary. Keep the full release goals and explicit compatibility limits.

The evidence below identifies implemented behavior and named qualification, not
one cumulative test total. Source-of-truth feature limits are in [usage](USAGE.md),
[extension services/surfaces](EXTENSIONS.md), [providers](EXTENSION_PROVIDERS.md),
[state](EXTENSION_STATE.md) and [registry](EXTENSION_REGISTRY.md).

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

- Background Tree-sitter highlighting covers C/C++, Rust, Python, JavaScript/JSX, TypeScript/TSX, and JSON. Tests cover multiline constructs, Unicode, grammar loading, and stale revisions. Native C/C++ tests cover inherited query categories, templates/preprocessors, multiline raw strings, headers/modules and the existing 2 MiB worker budget; a PTY verifies theme colors and CRLF edit/undo/save. Incremental tree reuse, injections, semantic tokens, folding, and additional grammars remain incomplete.

- User/workspace settings import with language overrides, configurable indentation/line numbers, unsupported-setting reporting, and background reload is implemented. The broader configuration and profile migration contract remains incomplete.

- An optional isolated Node host runs CommonJS command extensions with native palette commands, synchronous document mirrors, version-checked edits, and native undo. Extension keybindings preserve platform overrides and user precedence/removals. Unchanged Tyriar Sort Lines 1.12.0 F9 sorting/undo passes; stale/invalid transactions and process crashes are tested. Local VSIX installation now supports bounded extraction, atomic registry updates, rollback, provenance, CLI/editor management and explicit installed-package activation. A second named Sort Lines test packages unchanged compiled entry files and exercises installation, F9 and undo. Subsequent merged slices add stable Open VSX distribution, scoped lazy activation, bounded Mementos, seven language providers and native UI surfaces as recorded below. Broad API coverage, storage garbage collection, menus, advanced providers and graphical UI remain incomplete. See [extension evidence](EXTENSIONS.md).

- Extension configuration now imports user/workspace settings before activation, preserves ordered language overrides, exposes snapshot reads/current inspection, and delivers live change events. Native/Node tests cover scope precedence, object merging, shadowed changes, malformed-JSON retention, and held event snapshots. Unchanged Sort Lines also passes its whole-file setting and reload workflow. A real VS Code 1.95.0 differential fixture now compares 25 read/event observations and caught an ignored-scope inspection mismatch. Complete defaults, configuration writes, additional scopes, and broader differential qualification remain incomplete.

- The pinned reference harness exports full default keybinding inventories and actual keyboard layout metadata, and reports every rule against VSCLI defaults. These structural comparisons do not qualify the resolver, physical input, or command effects. See the [harness boundaries](../tests/vscode-reference/README.md).

- Opened buffers and saved baselines now share rope storage; file reads, save conflict checks, and writes stream through bounded buffers. Periodic unchanged-file comparisons run in the watcher worker without replacement ropes or input-thread whole-file comparisons. Unicode/CRLF persistence, interrupted reads, same-size changes, undo, and recovery remain tested. Executable measurements show lower resident memory, but the 32 MiB cap and larger-file qualification remain unresolved.

- Periodic recovery uses one outstanding shared-rope snapshot and a dedicated writer, preserving the v1 journal format. Tests cover edits during stalled writes, revision acknowledgements, failure/retry, Unicode/CRLF/JSON escaping, and ordered shutdown. PTY coverage includes abrupt recovery and a final snapshot on SIGTERM. A recovery-enabled executable workload records substantially lower typing tail latency at 10/32 MiB; see the performance report and raw samples. Startup restoration and shutdown can still block; language-service/extension contention and power-loss durability remain unqualified.


- Long-line movement and column lookup traverse rope chunks, with upstream Unicode boundary fixes and Unicode 17 conformance checks. Plain-text/ready-grammar rendering limits copying and scanning to viewport prefixes; PTY tests cover long-line Unicode edits, undo, scrolling, and CRLF saves. Executable and native measurements are recorded in the performance report. Far-right layout, fallback syntax work, larger files, and full IDE contention remain unqualified.

- Literal snippet insertion is connected to native variables and conditional Tab/Shift+Tab/Escape bindings, with linked editing, Unicode/CRLF persistence, cursor-order preservation and undo/redo coverage. User/workspace and installed-VSIX catalogs run in a bounded worker without code activation. Reference traces distinguish API, user-command, variable and cancellation behavior; executable PTYs exercise native insertion, installed C++ snippets without Node, and uninstall discovery. Rust integration tests cover upgrade/rollback discovery. Choices, nested session merging, completion/extension insertion and full regex/input qualification remain incomplete.

- The empty welcome screen, native VSIX installation/rollback, copied VS Code user-profile import and persistent native themes are merged. Combined PTY workflows cover original-source preservation, supported imported bindings/snippets, RGB rendering, explicit code activation, save/undo, final-tab close and restart. All required Linux/macOS/Windows PR checks passed. Imported profiles are editable copies; themes map a native palette and syntax categories approximately. Full profiles/sync, broad extension APIs and TextMate fidelity remain incomplete; see the current [parity plan](PARITY_PLAN.md).

- Persistent recent-file history and reopening closed file-backed editors preserve existing dirty/shared buffers and retryable missing-file history. Native tests cover locked merge contention, retained updates and shutdown retry; PTYs cover close-to-welcome, reopen/edit/undo/save and restart. Workspace history and full session/layout restoration remain incomplete; the bounded opt-in clean-file restore subset below is now merged.

- Up to eight explicitly selected code packages share one optional host, with owned commands, versioned document mirrors, immutable installed generations and explicit cohort restart/stop. Unix process-group cleanup handles inherited-stdio descendants across stop/restart/crash/failure/SIGTERM; Windows descendant cleanup remains unqualified. The shared-host PR passed its required checks; named Sort Lines and synthetic session/PTY tests cover the recorded workflows. Subsequent scoped lazy activation, selected dependency exports, Mementos and provider/UI adapters are recorded below. Automatic dependency download, all activation events, full API coverage and platform cleanup parity remain incomplete.

- Native Quick Fix/Refactor actions, lazy resolution and validated workspace text edits to synchronized open buffers are merged. The merged code-action PR passed its required checks, deterministic protocol/PTY integrity tests and a named real clangd C++ quick-fix/save/undo workflow. Command callbacks require explicit document versions; timeout disables further commands until server restart. Undo is per file. Closed-file/resource edits, combined edit-and-command actions and general command compatibility remain incomplete.

## Subsequent source additions and their remaining boundaries

- **Parameter hints — [PR #39](https://github.com/vscli/vscli/pull/39):** original explicit shortcut, server-selected signature and highlighted active parameter in a nonmodal native view. UTF-16 label, context/cancellation, CRLF integrity and real clangd tests qualify the subset. Automatic triggers/retrigger, overload navigation and rich Markdown remain incomplete.
- **Native prompts — [PR #41](https://github.com/vscli/vscli/pull/41):** bounded Quick Pick/Input Box adapters with owner/session validation, cancellation, native-input priority and lifecycle handling. The named Lorem Ipsum workflow and synthetic tests are detailed in EXTENSIONS.md; full Quick Input behavior is not claimed.
- **Symbols — [PR #42](https://github.com/vscli/vscli/pull/42):** searchable native document/workspace symbols, hierarchical/flat responses, UTF-16 ranges, dirty/shared identity and a retained asynchronous existing-file loader. Real clangd and deterministic/PTY evidence qualify named paths. Outline, breadcrumbs, history and range-less WorkspaceSymbol resolution remain incomplete.
- **Clean sessions — [PR #43](https://github.com/vscli/vscli/pull/43):** opt-in `--restore-session`, explicit restore command and `--no-session`; clean file-backed tabs, active tab, visible groups and selection metadata persist in bounded background work. Dirty/untitled recovery and CLI-opened documents remain authoritative. Tests cover canceled/stale/partial restore, leases, retained prior metadata, isolation and Unicode/CRLF integrity. Limits include 32 clean tabs, four equal groups and 128 MiB total restored file reads. Full hot exit, every historical tab/view selection and terminal restoration remain incomplete.
- **Syntax stability — [PR #44](https://github.com/vscli/vscli/pull/44):** unchanged text retains mapped grammar colors while the next parse runs, avoiding whole-view color loss during typing. The old classification can be provisional after syntax-changing edits. This is not incremental parse-tree reuse, semantic highlighting or folding.
- **Automatic native servers — [PR #45](https://github.com/vscli/vscli/pull/45):** background discovery/start of installed clangd for C/C++ and rust-analyzer for Rust, manual override, settings and disable/retry controls. Real clangd is exercised without `--lsp`, alongside synthetic transition/stale-start tests. One selected native server is supported; downloads, concurrent language pools and broad real Rust-project qualification remain outstanding.
- **Registry — [PR #46](https://github.com/vscli/vscli/pull/46) and [#48](https://github.com/vscli/vscli/pull/48):** explicit native Open VSX stable search, download, update and rollback with bounded HTTP work and identity/version validation. Metadata-only prereleases are skipped while stable results remain usable. Deterministic registry/PTY tests qualify failure retention and unrelated editing. Dependency downloading, authenticated registries, prerelease selection, publisher signatures and engine/ABI compatibility remain outstanding.
- **Activation — [PR #47](https://github.com/vscli/vscli/pull/47):** remembered global/workspace enablement, supported command/language lazy activation, selected installed dependencies and bounded cohort lifecycle. Enabling execution is distinct from installation. Synthetic native/PTY tests cover grants, cancellation, dependency ownership and failures. Arbitrary activation events, dependency resolution/download and individual hot unload remain incomplete.
- **Documents and state — [PR #49](https://github.com/vscli/vscli/pull/49):** bounded existing-file/file-URI open/show, native-owned hidden documents, curated built-in commands and persistent global/workspace Mementos. Dirty hidden buffers share native identity, undo, watching and recovery without a phantom active editor. Atomic locked patches preserve unrelated concurrent updates. Exact owner/session receipt guards and aggregate selection budgets reject stale work. Untitled-content overloads, full editor options, general filesystem/resource edits, secrets, storage URIs and sync remain incomplete; power-loss durability is not fully qualified.
- **Seven language providers — [PR #49](https://github.com/vscli/vscli/pull/49):** completion, hover, definitions, references, document formatting, document symbols and signature help route into native UI with LSP fallback. Bounded retained callback slots and document/text-epoch/context guards protect asynchronous replies. Unchanged NPM Intellisense 1.4.5 completion and SQL Formatter VSCode 4.2.6 document formatting pass their named native and terminal workflows; this does not qualify whole packages. Completion resolve/snippets/commands, diagnostics collections, code-action/workspace-symbol providers, aggregation and richer selectors remain incomplete. PR #52 separately adds automatic typing suggestions; signature help remains explicit.
- **Surfaces — [PR #50](https://github.com/vscli/vscli/pull/50):** bounded read-only output, status items and declared lazy native trees. Opaque actions retain owner/session/generation and hidden-document identity. Shared callback capacity remains occupied until an actual tree callback settles; refresh/disposal cannot create unlimited unresolved work. Synthetic Node/native/PTY tests cover held callbacks, lazy admission, failed-owner cleanup, welcome, Unicode/CRLF save and undo. No published tree/status/output package has been qualified end to end. Advanced tree/status/output semantics, progress, menus and graphical compatibility remain incomplete.

- **Welcome and keyboard layout — [PR #51](https://github.com/vscli/vscli/pull/51):** graphical native mark with conservative Kitty support/cell fallback, clickable actions/recent files, actual settings path and themed keycap inspector. The inspector previews received keys/modifiers, profile and resolved command without executing it. The diagram is a US reference layout, not physical keyboard detection or inferred held state. Rendering/interaction tests and named terminal observations are recorded in WELCOME.md; broad graphics/terminal/layout parity remains outstanding.
- **Automatic suggestions — [PR #52](https://github.com/vscli/vscli/pull/52):** debounced identifier/trigger-character requests from a ready native LSP or matching active extension provider, nonmodal caret popup, arrows/page navigation, Tab/Enter acceptance and Escape cancellation. Native snippet Tab takes precedence; stale cached labels cannot apply old edits. Scoped settings and retained request/callback bounds preserve typing and undo. Source tests cover stale replies, shared contexts, native surfaces and CRLF/Unicode acceptance; real clangd and unchanged NPM Intellisense workflows are recorded in compatibility evidence. Completion resolve/snippets/commands, fuzzy ranking, richer settings, aggregation and inline AI suggestions remain incomplete.

No item above closes the full editing, extension, language, reliability or release
requirements merely by adding one usable subset. Advanced Git/tasks/debug/test,
remote services, multi-root/profiles, rich content, authentication/sync, collaboration
and optional AI integrations remain part of the parity plan. Packaging rehearsal is
not a release; intentional reviewed version/tag and platform/artifact qualification
are still required.
