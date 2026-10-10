# VS Code parity and user experience priorities

This is the current execution plan, updated 2026-10-10. It supersedes the initial
ordering and staffing estimates in [ROADMAP.md](ROADMAP.md). The full project
objective and [release checklist](RELEASE_CHECKLIST.md) remain intact. A useful
interim release does not complete VS Code parity.

Prioritize the features that make the editor useful, familiar and pleasant for a
broad audience. C++ with clangd is the first complete language workflow to
qualify, not the boundary of supported languages. This is an impact-based
engineering judgment, not a measured ranking of VS Code feature popularity.

## Evidence and current work

Merged source extends through [PR #59](https://github.com/vscli/vscli/pull/59),
which passed all six required platform checks.
Source inclusion is separate from a CI result or complete workflow qualification. Behavioral comparisons target
VS Code **1.95.0**. Current upstream documentation helps inventory additional
capabilities; it does not extend our tested baseline to newer releases.

| Work | Source and recorded evidence | Remaining scope |
| --- | --- | --- |
| Welcome and recent files | [PR #25](https://github.com/vscli/vscli/pull/25) and [PR #34](https://github.com/vscli/vscli/pull/34): true empty workbench, recent-file history and reopening without resurrecting discarded text | Recent workspaces and richer group/tab behavior |
| Snippets, installation, import and themes | [PR #27](https://github.com/vscli/vscli/pull/27), [#28](https://github.com/vscli/vscli/pull/28), [#30](https://github.com/vscli/vscli/pull/30), [#32](https://github.com/vscli/vscli/pull/32): native catalogs including installed VSIX data, immutable local packages/rollback, copied profiles and persistent theme selection | Extension insertion API, choices/nesting, full settings/profile semantics and theme fidelity |
| C/C++ colors while typing | [PR #35](https://github.com/vscli/vscli/pull/35), [#44](https://github.com/vscli/vscli/pull/44): bundled grammars and mapped unchanged-text colors during background refresh | Incremental parse reuse, semantic tokens, TextMate fidelity and more grammars |
| Code actions and parameter hints | [PR #37](https://github.com/vscli/vscli/pull/37), [#39](https://github.com/vscli/vscli/pull/39): native Quick Fix/Refactor and explicit hints; [#55](https://github.com/vscli/vscli/pull/55) combines bounded native/extension actions across retained buffers; [#56](https://github.com/vscli/vscli/pull/56) adds automatic hints and overload navigation; named real clangd workflows | Closed/resource edits, combined edit-and-command actions, full action contexts, multi-provider signature fallback and widget fidelity |
| Document/workspace symbols | [PR #42](https://github.com/vscli/vscli/pull/42): bounded searchable symbols, shared-buffer navigation and asynchronous existing-file opens | Outline qualification, breadcrumbs and range-less workspace-symbol resolution |
| Clean-session restoration | [PR #43](https://github.com/vscli/vscli/pull/43): opt-in clean file-backed tabs, active tab, selections and up to four visible groups; dirty/untitled recovery stays authoritative | Full hot exit, historical tab/group matrix, workspace transitions and terminals |
| Installed language servers | [PR #45](https://github.com/vscli/vscli/pull/45): automatic installed clangd C/C++ and rust-analyzer Rust selection without launch flags; explicit override and disable/configuration controls | Concurrent language pools, downloads/provisioning and broader real-project qualification |
| Registry and activation | [PR #46](https://github.com/vscli/vscli/pull/46), [#47](https://github.com/vscli/vscli/pull/47), [#48](https://github.com/vscli/vscli/pull/48): native stable Open VSX browsing/explicit updates, prerelease filtering, remembered scoped enablement, supported lazy events and selected dependencies | Authenticated registries, dependency downloading/version resolution, broader activation events and engine/ABI qualification |
| Shared host and native prompts | [PR #36](https://github.com/vscli/vscli/pull/36), [#41](https://github.com/vscli/vscli/pull/41): optional shared session, owned callbacks, Unix group teardown and bounded Quick Pick/Input Box | Windows descendant cleanup, richer Quick Input, safe individual hot unload and full lifecycle compatibility |
| Native extension documents/state/providers | [PR #49](https://github.com/vscli/vscli/pull/49): hidden/shared open/show, curated built-in commands, global/workspace Mementos and seven providers; unchanged NPM Intellisense/SQL Formatter workflows | Full completion commands/semantics, filesystem/resource APIs, secrets and broader package qualification; later provider/diagnostic slices are described below |
| Native extension surfaces | [PR #50](https://github.com/vscli/vscli/pull/50): bounded read-only output, status items and declared lazy trees with owner/generation guards and retained callback capacity | Named real-package surface corpus, advanced tree/status/output behavior, progress/menus and graphical compatibility |
| Graphical welcome and keyboard layout | [PR #51](https://github.com/vscli/vscli/pull/51): native mark with Kitty/cell fallback, clickable actions/recent files, settings path and themed keycap inspector | Broader terminal/multiplexer graphics and physical-keyboard/layout qualification; no physical held-key inference |
| Automatic suggestions | [PR #52](https://github.com/vscli/vscli/pull/52): debounced typing/trigger-character completion, nonmodal caret popup and Tab/Enter acceptance from native LSP or active extension provider | Full completion commands/semantics, richer fuzzy ranking, settings and multi-provider aggregation; resolved completion and automatic hints follow in later slices below |

The merged [PR #53](https://github.com/vscli/vscli/pull/53)
[resolved-completion slice](COMPLETIONS.md) adds lazy resolution,
linked snippet/import transactions, inert documentation and bounded abbreviation
ranking. Its checks and recorded workflows remain separate from full IntelliSense
parity. The merged [PR #54 diagnostics slice](DIAGNOSTICS.md) adds independent collections,
native text-epoch/server guards and successful-save events. Extension code-action
providers follow it within priority 2 through the [bounded action slice](CODE_ACTIONS.md), with official unchanged sample and real clangd qualification. Command/resource/full-context parity remains outstanding.

The first priority-3 slice is merged in [PR #56](https://github.com/vscli/vscli/pull/56):
native C/C++ and JSON/JSONC [smart typing](SMART_TYPING.md) and
[automatic parameter hints](PARAMETER_HINTS.md). Its reviewed source passes all six
required CI checks, including fresh pinned-editor comparisons on Linux/macOS/Windows.
Local evidence records 482 ordinary Rust tests, 123 host tests, 18 observed typing
traces, 35 terminal smoke workflows and named release native/extension/clangd PTYs.
The [two published benchmark runs](PERFORMANCE.md#smart-typing-and-idle-hint-work-2026-10-10)
have mixed results and establish no overall speed ranking.

The follow-up slice merged in [PR #57](https://github.com/vscli/vscli/pull/57) adds language-specific advanced/full indentation and closing
alignment. Its [prepared-token contract](ADVANCED_INDENTATION.md) matches 132 actual
Linux reference cases and 530 snapshots; 496 native tests, strict quality checks,
ten new terminal sessions and the existing 35-workflow terminal suite pass.
Fresh pinned-editor comparisons also pass on Linux, macOS and Windows in all six
required checks. The [30-target token-readiness capture](reference/2026-10-10-cpp-token-readiness/README.md)
records an actual pinned-editor startup distinction independently of native
comparison; prepared and natural behavior remain separate qualification scopes.
Additional native profiles and broader signature-extension qualification remain
follow-up scope. The merged [PR #58](https://github.com/vscli/vscli/pull/58) adds
[installed native language configurations](NATIVE_LANGUAGE_CONFIGURATIONS.md),
pair/comment commands and disable/update ownership retirement. Local comparisons
match 50 recorded C++ workflows / 200 snapshots plus 21 workflows / 84 snapshots
with the unchanged bundled C++ declaration. Local qualification passes 533 ordinary Rust tests, five new terminal sessions,
formatting and strict all-target Clippy. All six required checks passed at the
reviewed head, including fresh Linux/macOS/Windows comparisons in
[CI run 38018064477](https://github.com/vscli/vscli/actions/runs/38018064477).
This does not finish installed language configuration or extension parity.

Priority 4 now implements [native Back/Forward history](NAVIGATION_HISTORY.md)
merged in [PR #59](https://github.com/vscli/vscli/pull/59). Local qualification
passes 555 ordinary Rust tests,
five public history journeys, four original-key terminal workflows in both debug
and optimized builds, the existing 35 terminal workflows and seven smart-typing
workflows. The separate pinned observer/native comparison matches ten cases and
85 visible snapshots; five reference integrity tests pass. All six required
platform checks passed before merge. Published ordinary/single-line benchmark
observations show higher candidate latency in these runs.

The next priority-4 slice implements [native Outline](OUTLINE.md) on its feature
branch: hierarchical current-document symbols, arrows/collapse/expand, enclosing
highlights and guarded collapsed identifier-start reveal, with native/optional
extension ownership lanes. Local qualification passed the full 583-test Rust
run across 37 suites (19 opt-in tests ignored), formatting, strict all-target
Clippy and the optional host's 133 Node tests. Focused native/extension publication
checks, one actual clangd nested-C++ workflow and three debug native-only terminal
journeys passed. The pinned comparison passes two geometry cases and two
confirmed collapsed reveals; four integrity tests pass. The complete synthetic
reference retains 20 snapshots, including early no-ops, so this remains narrow
API/observed-reveal evidence rather than whole-sidebar or unchanged-package
qualification. Existing baseline terminal requalification, optimized terminal
workflows, benchmarks and fresh platform CI are pending. Breadcrumbs and full
navigation parity remain outstanding.

Folding is not shipped. The committed `feat/native-folding-foundation` contains
bounded scanning/row mapping; a preserved old dirty prototype contains unfinished
App/Document/UI integration. Neither is a qualified user feature. The bounded diagnostics/action subsets do not finish priority 2 or full extension compatibility; further API/context/resource and production-package qualification remains.

Linked PRs and feature documents record each implemented subset and its named
review/test evidence.
See [usage](USAGE.md), [extension evidence](EXTENSIONS.md),
[provider qualification](EXTENSION_PROVIDERS.md), [state integrity](EXTENSION_STATE.md)
and [registry limits](EXTENSION_REGISTRY.md). Unchanged Sort Lines, Lorem Ipsum, NPM Intellisense and SQL Formatter evidence is workflow-specific,
not whole-package or ecosystem compatibility. Synthetic fixtures establish their
named contracts, not VS Code differential parity. Counts from an older build are
historical evidence and are not a total for today's application.

Linux/macOS CI exercises PTYs. Windows native/reference checks do not qualify every
physical terminal or ConPTY interaction. Original imported files remain separate
from native copied profiles. Branch checks do not prove a combined application
works; every integration still needs combined review and CI.

## What is still missing

“Partial” means a usable subset exists; it does not imply VS Code equivalence.
The [usage guide](USAGE.md), [extension report](EXTENSIONS.md) and
[compatibility contract](COMPATIBILITY.md) carry implementation details.

| Area | Existing foundation | Remaining work |
| --- | --- | --- |
| Welcome and navigation | Explorer, quick open, palette, graphical empty welcome/actions, persistent recent files/reopen, bounded Back/Forward history, document/workspace symbol pickers and implemented Outline awaiting qualification | Recent workspaces, Outline qualification, breadcrumbs, discoverable settings and consistent focus |
| Tabs and layout | Shared-document split views, four equal groups and opt-in clean-session visible-layout restore | Per-group tabs, resizing/nested splits, preview/pinned tabs, move editors between groups and complete layout restoration |
| Session continuity | Dirty-buffer crash recovery plus opt-in clean-file tabs, active tab, cursor/selection and visible-group restoration | Full hot exit, hidden historical tab/view state, workspace history/transitions, terminal restoration and broader failure/durability qualification |
| Editing | Multi-cursor, selections, undo, line commands, literal find/replace and native C/C++/JSON smart-typing subset | Wrapping, folding, broader/extension language rules, richer regex replacement, complete command semantics, encoding/BOM choices and large-file mode |
| Snippets | Literal insertion, linked fields, variables, native catalogs and completion snippet/import transactions | Choices, nested insertion, extension insertion API and full transform semantics |
| Settings and migration | Small settings subset, language overrides, copied-profile import and binding diagnostics | Autosave, format-on-save, indentation detection, excludes, EOL settings, editable settings UI, full profiles/workspace migration, extension inventory reconciliation |
| Themes and highlighting | Tree-sitter C/C++ and other families, mapped stable colors during typing, native JSONC/installed themes and persistent picker | TextMate grammar/scope semantics, font styles, semantic themes, more workbench colors, icon themes, embedded languages and grammars |
| Extension packages | Native local VSIX/rollback, stable Open VSX search/download/explicit updates, scoped enablement and immutable selected generations | Dependency downloading and version solving, engine/ABI qualification, authenticated registries, profiles, package cleanup and automatic update policy |
| Extension host | Optional shared CommonJS cohort, supported lazy activation/selected dependencies, hidden/shared document handles, curated commands and bounded Mementos | Broader activation/dependency APIs, filesystem/resource edits, full editor handles, secrets/sync/storage URIs, Windows descendant cleanup and engine/ABI qualification |
| Extension UI/providers | Commands/messages, bounded Quick Pick/Input Box, output/status/lazy trees, eight language-provider routes and diagnostic collections | Completion commands, full code-action semantics, full diagnostic aggregation, progress/menus, richer tree/UI and task/debug/test/SCM providers |
| Language intelligence | One automatically selected installed native server or manual override; diagnostics, automatic/explicit completion popup, hover/navigation/format/rename, actions, automatic/explicit parameter hints with overloads and symbols; eight extension provider routes | Completion commands/full completion semantics, multi-provider signature fallback, concurrent servers, closed-file refactors, unopened-file extension diagnostics, semantic tokens/inlay hints/code lenses |
| Search and projects | Workspace regex search over unsaved buffers; file operations/watchers | Replace across files with preview, include/exclude controls, live results, multiline regex, multi-root `.code-workspace`, workspace trust and settings layers |
| Git | Status, file diffs, stage/unstage, commits/history | Inline changes, hunk staging, branch/remotes/stash/worktrees, blame/history navigation, merge conflict UI, richer provider support |
| Tasks | Basic process/POSIX-shell tasks and variables | Dependencies, background readiness, problem matchers, auto-detection/providers, inputs, cancellation UX and Windows shell tasks |
| Debugging and testing | Basic explicit DAP launch/step/stack/variables/evaluate | `launch.json`, attach, watches, breakpoint persistence/edit mapping/conditions/logpoints, threads, reverse terminal requests, test discovery/run/debug/results/coverage |
| Terminal | Multiple PTY sessions, VT rendering and scrollback | Shell integration, profiles, splits, links/search, full focus/shortcut routing, reconnect/persistence, Windows ConPTY/clipboard qualification |
| Keyboard | Platform subset, chords/imports, bounded context parser, themed keycap inspector and pinned inventory | All contexts/operators/default commands, live rule editing, physical-key/layout/IME qualification including tmux/SSH and macOS Command |
| Remote work | Can run executable in an ordinary remote terminal | Dedicated SSH/container/WSL setup, remote filesystem and host services, forwarding, reconnect and remote configuration scopes |
| Rich content | No general implementation | Markdown preview, notebooks/kernels/outputs, images/custom editors, webview compatibility route and integrated browser tooling |
| Accounts, collaboration and AI | No general implementation | Authentication providers, sync, collaboration, native chat/inline completions/agent tools and compatible provider/extension integrations |
| Reliability, performance and release | Native core, checks, recovery tests and specific benchmarks | Full IDE contention benchmarks, larger files, async open/save, durability/failure qualification, accessibility, bidi/IME, platform matrix, release packaging/upgrades/notices |

This inventory covers feature families. A complete command/API inventory remains
necessary to prove exact parity; ticking one family off because one demonstration
works would hide missing behavior.

## User-approved implementation order

The following queue follows the order approved in the project conversation.
Finish and qualify each usable slice before advancing; independent review and
integrity/performance work run alongside it. C++/clangd is the first real-project
qualification, while features remain useful across languages.

| Priority | Addition | Next scope |
| --- | --- | --- |
| 1 | Complete IntelliSense | Lazy completion resolution, linked snippet completions/import edits, documentation and better ranking |
| 2 | Extension diagnostics and quick fixes | Owner-scoped diagnostic collections and extension code-action providers |
| 3 | Smart typing and automatic parameter hints | Context-aware indentation, bracket/quote pairing, triggered signatures and overload navigation |
| 4 | Navigation history and outline | Back/forward locations, outline and breadcrumbs |
| 5 | Settings and save automation | Discoverable settings, format-on-save, autosave, indentation detection and excludes |
| 6 | Tabs and workspace continuity | Per-group tabs, resizable groups, preview/pinned tabs and fuller restoration |
| 7 | Word wrap and folding | Consistent display-row mapping and safe per-view folds |
| 8 | Multi-language tooling and setup | Concurrent servers, setup guidance, restart feedback and project qualification |
| 9 | Workspace replace with preview | Reviewed dirty/closed-file replacement, include/exclude and recovery |
| 10 | Git hunk staging and conflicts | Inline changes, partial staging and conflict editing |
| 11 | Build tasks and problem matchers | Dependencies and compiler-error navigation |
| 12 | Project debugging and tests | launch.json, attach, watches, persistent breakpoints and test discovery/results |

Exact shortcuts, named unchanged extension workflows, visual quality, performance,
platform qualification and the full remaining parity objective apply throughout.
This is an impact-based engineering judgment, not a measured popularity ranking.

## Acceptance criteria by feature family

### 1. Finish everyday completion and extension diagnostics

Automatic typing suggestions and the native caret popup are implemented, with
provider/context guards and snippet-placeholder Tab precedence. The resolved-completion
slice adds linked snippet/import insertion and item documentation with exact source
ownership and context guards. Qualify these before adding diagnostic collections
and extension code-action providers; follow-up completion commands remain a separate
required compatibility contract.
These unlock useful coding workflows beyond a list of suggestions. Keep native
LSP fallback and the native core independent of JavaScript.

Acceptance: type in a dirty shared Unicode/CRLF document, choose a completion that
requires lazy resolution, additional edits and linked snippet fields, then undo,
redo and save exact expected bytes. Cursor/view/configuration/provider changes
reject old responses. Uncooperative callbacks retain bounded capacity. Diagnostics
publish/clear by owner and version without deleting another owner's results; a
selected diagnostic action validates all targets before any mutation. Repeat a
named unchanged extension workflow and the native LSP path. Do not count declaring
SnippetString or registering a provider as implementing insertion semantics.

### 2. Qualify the C++ project workflow end to end

Installed clangd startup, C/C++ highlighting, automatic completion, automatic signature help,
definitions/references, symbols and a real quick fix are integrated. Qualify them
together in a representative repository using `compile_commands.json`, with useful
missing-configuration and server-restart feedback. Add completion resolve/snippets,
automatic parameter hints and safe cross-file operations in coherent slices.
Reuse the service architecture for a second language; real clangd working does not
prove the VS Code clangd extension works.

Acceptance: start with no LSP flags, open a translation unit, navigate to a symbol,
complete a call, inspect its active parameter, apply a diagnostic fix, rename across
files, format, build and jump to an error. Delayed replies cannot change newer edits.
Closed-file operations require affected-file review and a tested undo/recovery
policy before the whole workflow is claimed. Test missing clangd, bad project
configuration, crash/restart and explicit disable without blocking editing.

### 3. Navigation, layout and comfortable editing

Use the integrated recent files, symbols and opt-in clean-session restore as the
base for back/forward navigation, outline/breadcrumbs and transactional workspace
switching. Then add independent resizable tab groups, wrapping, safe per-view
folding, snippet choices/nesting and smart indentation/brackets. Add workspace
replace with a reviewable preview over dirty and closed files. Keep autosave,
format-on-save, EOL and indentation settings tied to implemented behavior.

Acceptance: edit shared Unicode/CRLF documents, navigate across locations/groups,
resize/wrap/fold, preview replacements across dirty/closed files, undo and restart
without losing content or redirecting edits.
Display-row mapping must agree for keyboard, mouse and language-service actions.
Folding must preserve selections and invalidate/rebuild mapping through edits and
undo. Failed/partial restore retains the previous session metadata and dirty recovery
remains authoritative. Workspace changes need their own save/cancel transaction;
recent-file support is not workspace support. Qualify bounded large-project and
long-line behavior alongside the new UI.

### 4. Broaden verified extensions and polish onboarding

Retain the graphical welcome and keyboard inspector
empty-state/input guarantees. Build on installed package management, native prompts,
Mementos and surfaces with a pinned, licensed corpus spanning commands, language,
formatting, themes/snippets and a real tree/output/status workflow. Record missing
APIs before implementation; prioritize complete user workflows over registration
counts. Registry installation, execution enablement, successful activation and
verified functionality must remain separately visible.

Acceptance: from a clean native configuration, import a real VS Code user directory
without modifying originals, retain a theme across restart, use an imported binding
and snippet, then install/enable/update/rollback a named package. Repeat beside dirty
shared tabs and at welcome. Several named packages must preserve settings/state and
survive failure/restart without corrupting native text. Publish package version,
source/hash, runtime, exact steps and pass/fail/unsupported results. Malformed input
and failed updates preserve the previous configuration. Unsupported copied settings
must remain attributable notices. Add a terminal/profile/layout matrix for the
inspector; a displayed keycap alone does not qualify physical shortcut delivery.

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
