# VS Code parity and user experience priorities

This is the current execution plan, updated 2026-10-09. It supersedes the initial
ordering and staffing estimates in [ROADMAP.md](ROADMAP.md). The full project
objective and [release checklist](RELEASE_CHECKLIST.md) remain intact. A useful
interim release does not complete VS Code parity.

Prioritize the features that make the editor useful, familiar and pleasant for a
broad audience. C++ with clangd is the first complete language workflow to
qualify, not the boundary of supported languages. This is an impact-based
engineering judgment, not a measured ranking of VS Code feature popularity.

## Evidence and current work

The merged baseline for this update is `cddfc2d`. Behavioral comparisons target
VS Code **1.95.0**. Current upstream documentation helps inventory additional
capabilities; it does not extend our tested baseline to newer releases.

| Work | State at this audit | What remains after integration |
| --- | --- | --- |
| Native empty welcome screen | Merged [PR #25](https://github.com/vscli/vscli/pull/25); no initial or last-close phantom document; recent paths and keymap-aware hints now present | Recent workspaces, onboarding and session restore |
| Snippet catalogs | Merged user/workspace catalogs in [PR #27](https://github.com/vscli/vscli/pull/27) and native installed-VSIX contributions in [PR #32](https://github.com/vscli/vscli/pull/32); code activation and Node are unnecessary for declarative snippets | Choice UI, nested sessions, completion and extension insertion API |
| Extension installation | Merged [PR #28](https://github.com/vscli/vscli/pull/28): native local VSIX install/list/uninstall/rollback and explicit optional code activation | Registry discovery, dependencies, durable enable/disable state and broader API support |
| VS Code import and themes | Merged [PR #30](https://github.com/vscli/vscli/pull/30): read-only preview, copied active profiles, original-byte preservation, native theme picker and persistent selection | Full settings/profile behavior, extension inventory migration, theme fidelity and sync |
| Recent-file navigation | Merged [PR #34](https://github.com/vscli/vscli/pull/34): persistent native file MRU and session-local reopening, retaining dirty/shared identity and retryable history | Workspace transitions, back/forward navigation and layout restoration |
| Native C/C++ highlighting | Merged [PR #35](https://github.com/vscli/vscli/pull/35): bundled grammars, templates/preprocessors, multiline raw strings and Unicode comments through the bounded worker | CUDA, semantic tokens and TextMate scope fidelity |
| Shared code-extension sessions | Merged [PR #36](https://github.com/vscli/vscli/pull/36): up to eight explicitly selected packages in one shared host, generation-preserving restart/stop and Unix group cleanup | Dependencies, automatic activation, Windows descendant cleanup and broader APIs |
| Native code actions | Merged [PR #37](https://github.com/vscli/vscli/pull/37): native Quick Fix/Refactor picker, lazy resolution and staged edits to synchronized open buffers; real clangd C++ quick fix tested | Closed-file/resource operations, combined edit-and-command actions and general command compatibility remain separate |
| Signature help | Implemented and locally tested in `feat/native-signature-help`, including real clangd C++; awaiting integration | Automatic triggers/retrigger and overload navigation remain separate |
| Native extension prompts | Implementation underway in `feat/extension-native-prompts`; not merged | Quick-pick/input cancellation, prompt ownership and lifecycle qualification |
| Native folding | Preserved unfinished work in `feat/native-folding`; not shipped | Resume after the current language/extension slices integrate |

The merged onboarding and code-action changes passed all six required PR checks.
The combined code-action build passed 201 enabled Rust tests, 32 baseline PTY
workflows and five shared-extension session workflows, including import/theme/snippet/VSIX
activation/save/undo/restart and installed C++ snippets with Node unavailable.
Original VS Code configuration hashes remain unchanged in the import journey.
Linux/macOS CI exercises PTYs; Windows native/reference checks do not qualify
every physical terminal or ConPTY interaction. Named real Sort Lines evidence is
separate from deterministic integration fixtures. See the linked usage and
compatibility reports for each feature's limits. Branch checks do not prove a
combined application works; future changes still require combined review and CI.

## What is still missing

“Partial” means a usable subset exists; it does not imply VS Code equivalence.
The [usage guide](USAGE.md), [extension report](EXTENSIONS.md) and
[compatibility contract](COMPATIBILITY.md) carry implementation details.

| Area | Existing foundation | Remaining work |
| --- | --- | --- |
| Welcome and navigation | Explorer, quick open, palette, native welcome with recent paths, persistent recent-file picker and reopen closed editor | Recent workspaces, back/forward history, symbol search, outline, breadcrumbs, discoverable settings and consistent focus |
| Tabs and layout | Shared-document split views, four equal groups | Per-group tabs, resizing/nested splits, preview/pinned tabs, move editors between groups, persisted layout |
| Session continuity | Dirty-buffer crash recovery | Reopen clean files, cursors, selections and groups after restart; recent workspaces; explicit restore controls; independent terminal restoration |
| Editing | Multi-cursor, selections, undo, line commands, literal find/replace | Wrapping, folding, smart indentation/brackets, richer regex replacement, complete command semantics, encoding/BOM choices and large-file mode |
| Snippets | Literal insertion, linked fields, variables, user/workspace and installed-package catalogs | Choices, nested insertion, completion snippets, extension API, full transform semantics |
| Settings and migration | Small settings subset, language overrides, copied-profile import and binding diagnostics | Autosave, format-on-save, indentation detection, excludes, EOL settings, editable settings UI, full profiles/workspace migration, extension inventory reconciliation |
| Themes and highlighting | C/C++ and other language families through Tree-sitter; native JSONC themes, includes, installed contributions and persistent picker | TextMate grammar/scope semantics, token font styles, semantic themes, additional workbench colors, icon themes, embedded languages, more grammars |
| Extension packages | Native local VSIX install/list/uninstall/rollback | Search/download from permitted registries, dependency/version/platform resolution, updates, durable enable/disable state, profiles, package cleanup |
| Extension host | One optional shared CommonJS host for up to eight explicit packages; versioned mirrors, owner-tagged commands, cohort restart/stop and named Sort Lines evidence | Lazy activation, dependency APIs, storage/secrets, full document/editor handles, workspace edits/filesystem, Windows descendant cleanup and engine/ABI qualification |
| Extension UI/providers | Commands and simple messages | Quick picks, input boxes, output/status/tree contributions, menus, progress, language/task/debug/test/SCM providers, built-in command delegation |
| Language intelligence | Explicit single LSP server: diagnostics, basic completion/hover/navigation/formatting/rename and code actions with staged open-buffer edits | Automatic project setup, multiple servers, signature help, symbols, completion resolve/additional edits/snippets, closed-file refactors, semantic tokens/inlay hints/code lenses |
| Search and projects | Workspace regex search over unsaved buffers; file operations/watchers | Replace across files with preview, include/exclude controls, live results, multiline regex, multi-root `.code-workspace`, workspace trust and settings layers |
| Git | Status, file diffs, stage/unstage, commits/history | Inline changes, hunk staging, branch/remotes/stash/worktrees, blame/history navigation, merge conflict UI, richer provider support |
| Tasks | Basic process/POSIX-shell tasks and variables | Dependencies, background readiness, problem matchers, auto-detection/providers, inputs, cancellation UX and Windows shell tasks |
| Debugging and testing | Basic explicit DAP launch/step/stack/variables/evaluate | `launch.json`, attach, watches, breakpoint persistence/edit mapping/conditions/logpoints, threads, reverse terminal requests, test discovery/run/debug/results/coverage |
| Terminal | Multiple PTY sessions, VT rendering and scrollback | Shell integration, profiles, splits, links/search, full focus/shortcut routing, reconnect/persistence, Windows ConPTY/clipboard qualification |
| Keyboard | Platform subset, chords/imports, inspector, pinned inventory | All contexts/operators, complete defaults and commands, live rule editing, physical-key/layout/IME qualification including tmux/SSH and macOS Command |
| Remote work | Can run executable in an ordinary remote terminal | Dedicated SSH/container/WSL setup, remote filesystem and host services, forwarding, reconnect and remote configuration scopes |
| Rich content | No general implementation | Markdown preview, notebooks/kernels/outputs, images/custom editors, webview compatibility route and integrated browser tooling |
| Accounts, collaboration and AI | No general implementation | Authentication providers, sync, collaboration, native chat/inline completions/agent tools and compatible provider/extension integrations |
| Reliability, performance and release | Native core, checks, recovery tests and specific benchmarks | Full IDE contention benchmarks, larger files, async open/save, durability/failure qualification, accessibility, bidi/IME, platform matrix, release packaging/upgrades/notices |

This inventory covers feature families. A complete command/API inventory remains
necessary to prove exact parity; ticking one family off because one demonstration
works would hide missing behavior.

## Execution order and acceptance criteria

### 1. A polished first session

The welcome screen, theme picker, migration preview, compatibility notices,
installed-extension management, native snippet contributions and recent-file/reopen
navigation are integrated. Next add workspace transitions and session continuity.
Continue reviewing combined behavior rather than counting a
single successful demonstration as full feature parity.

Acceptance: from a clean configuration, start with no phantom document; import a
real VS Code user directory without modifying its original bytes; select and
retain a theme across restart; use an imported keybinding and snippet; install,
activate, update and roll back a known supported extension. Repeat with an empty
editor, split views and malformed imported files. A failed step preserves the
previous configuration and unsaved work. Incompatible rules/settings are visible
and individually attributable. Unsupported settings being copied is not success
at reproducing their behavior.

### 2. Useful extensions, together

Explicit concurrent packages, owned commands, shared mirrors and cohort lifecycle
are integrated. Prioritize extension capabilities that unlock complete workflows:
native quick picks/input next, then activation/dependencies, built-in command
execution, document opening, output/status/tree views, persistent state and provider
registration. Extend the broker shared with native services so extension and
native language providers do not spawn duplicate servers accidentally.

Build a pinned, licensed test corpus spanning command/edit, formatting, language,
theme, snippet and tree/output workflows. Start from the existing Sort Lines
case; select additional actual packages by their useful workflows and required
APIs. Record failures and missing APIs before implementation. Native clangd
working does not prove that its VS Code extension works.

Acceptance: several named packages run together, expose their original commands
and shortcuts, preserve settings/state, and pass observable workflows across
restart, crash and upgrade. Publish package version/source/hash, runtime, exact
steps and pass/fail/unsupported results. Extend registry search/download and
updates on top of the local installer. Installation, activation and verified
functionality must remain separate statuses. Inspect engine/dependency/platform
requirements and explain unavailable packages before attempting activation.

### 3. Familiar navigation and productive coding

Make C++/clangd the first full project qualification: discover/configure an
installed server, understand `compile_commands.json`, diagnose missing project
configuration, and support completion, signature help, definitions/references,
document/workspace symbols, quick fixes, rename and formatting. Completion must
handle resolve, snippets and additional edits with revision checks. Provide
restart/logs and useful failures. Reuse the same service architecture for other
languages rather than special-casing all behavior to C++.

Alongside this, implement back/forward history, outline, workspace symbols,
format-on-save and autosave with explicit settings. Code actions are integrated;
qualify signature help next. Add safe closed-file edits and a complete
undo/recovery policy before calling a multi-file refactor complete.

Acceptance: in a representative C++ repository, open a translation unit, navigate
to a symbol, complete a call, apply a diagnostic fix, rename across files, format,
build and inspect errors using the original commands. Delayed replies cannot
change newer edits; cross-file operations show affected files and have a tested
undo/recovery policy. Qualify a second language without changing ownership rules.

### 4. Comfortable long editing sessions

Restore open files/cursors/layout, finish resizable independent tab groups,
wrapping and folding, strengthen snippet choices/completion and smart editing,
and add search/replace across files with a reviewable preview. Tie wrapping,
autosave, indentation, whitespace/EOL and formatting settings to actual behavior.

Acceptance: edit Unicode/CRLF files in multiple views, resize/wrap/fold/navigate,
replace across dirty and closed files, undo and restart without losing content
or moving edits to the wrong document. Folded/wrapped display rows must map to
real document coordinates for keyboard/mouse/LSP actions. Performance remains
bounded on long lines and large projects. Crash recovery and clean-session
restore have separate, tested contracts.

### 5. Complete the development loop

Upgrade Git with hunk staging and conflict resolution; add branch/remotes/stash
workflows. Implement task dependencies/problem matchers, configured debugging
through `launch.json`, attach/watches and a native testing view. Qualify a C++
build/debug/test workflow and a second language with real adapters/providers.

Acceptance: modify code, inspect/stage selected changes, resolve a conflict,
build, jump to a compiler error, debug a breakpoint and run/debug a selected
test from the editor. Cancellation and subprocess failures leave the workbench
usable. Imported project configurations execute their supported behavior and
identify unsupported fields before running the wrong command.

### 6. Broader product parity

Deliver multi-root workspaces/profiles, remote services, richer extension UI,
notebooks/Markdown/custom editors, sync/authentication and optional AI/provider
integrations. These remain required tracks in the complete-product objective;
they are not silently removed by an earlier release milestone.

Graphical extension compatibility needs an explicit architecture experiment.
Compare native adapters, terminal image capabilities and an optional isolated
browser companion against named real workflows. Keep the native editor usable
without JavaScript. Record which interactions require a browser-capable surface;
character-cell output alone cannot promise arbitrary DOM/CSS behavior. Package
availability and service access are separate from API compatibility.

## Work that runs throughout every priority

- **Exact shortcuts:** derive new bindings from the pinned inventories, implement
  their contexts/effects, and test overrides, chords and focus. Expand the context
  parser and command registry in parallel with feature work. Physical delivery,
  resolution and behavior need separate results on named terminal configurations.
- **Performance:** measure startup, input-to-render p50/p95/p99, total editor and
  host memory, idle CPU and background contention on the same machine. Include
  completion, search, Git, recovery, multiple extensions and long lines. Compare
  real VS Code and other editors with equivalent workloads; no fastest-editor
  claim follows from isolated core timings.
- **Data integrity:** preserve shared document identity, unsaved text, async
  versions and bounded queues. Exercise failed imports/updates/saves, corrupted
  journals, stale replies, disk failures and interrupted shutdown.
- **Visual quality:** coherent theme colors, spacing, focus, selection contrast,
  narrow-terminal layouts and helpful empty/loading/error states. Verify text,
  Unicode widths, mouse/keyboard behavior and accessibility with actual terminals.
- **Release quality:** Linux/macOS/Windows behavior, terminal/multiplexer recipes,
  trustworthy packages, upgrade/rollback, notices and reproducible artifacts.
  Workflow packaging rehearsal is distinct from an intentional release tag.

Each PR owns one coherent change with its behavior tests and documentation.
Review combined behavior after rebasing, rather than treating parallel agent
branches as independent finished products. Keep a runnable main and publish
measured evidence with explicit gaps; avoid arbitrary completion percentages or
calendar claims before the work is qualified.

## Upstream references for the inventory

- [Core editor workflows](https://code.visualstudio.com/docs/editing/getting-started/overview)
- [Extension capabilities](https://code.visualstudio.com/api/extension-capabilities/overview) and [host execution models](https://code.visualstudio.com/api/advanced-topics/extension-host)
- [Themes](https://code.visualstudio.com/docs/configure/themes) and [profiles](https://code.visualstudio.com/docs/configure/profiles)
- [Settings Sync](https://code.visualstudio.com/docs/configure/settings-sync)
- [Testing](https://code.visualstudio.com/docs/debugtest/testing) and [webview interfaces](https://code.visualstudio.com/api/extension-guides/webview)
