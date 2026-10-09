# Extension installation and experimental host

VSCLI now has an optional CommonJS extension process. It is an independent, original compatibility shim; the Rust executable remains the owner of native buffers, rendering, input, and persistence. This is an experiment toward the full extension requirement, not completion of that requirement.

## Installing and managing packages

Use **F1 → Extensions: Install from VSIX** and enter a local `.vsix` path. Installation extracts the package without running scripts or extension code. **Ctrl+Shift+X** (**Cmd+Shift+X** on macOS), or **F1 → Extensions: Show Installed Extensions**, lists installed IDs, versions and compatibility descriptions. Enter offers explicit activation of a code extension; **R** restores the previous installation and **Delete** removes the selected package from the installed registry. Starting a code extension adds or replaces its immutable installed snapshot in the explicitly selected session and restarts that entire session. **S** stops the selected package and restarts the remainder; **H** restarts the selected session. The list shows selected and running versions, including selected packages removed from the installed registry. F1 → Extensions: Stop Selected Package accepts an ID even after uninstall. Declarative themes and installed snippet contributions do not need Node.

The same operations work without a terminal UI or JavaScript runtime:

```sh
vscli --install-extension ./publisher.extension.vsix
vscli --list-extensions
vscli --extension publisher.extension --extension another.commands .
vscli --rollback-extension publisher.extension
vscli --uninstall-extension publisher.extension
```

`--extensions-dir /path/to/storage` overrides the platform-local VSCLI extension directory for every operation. `--list-extensions` emits JSON with the manifest, immutable package path, source archive path, SHA-256 digest and a compatibility description. The digest records the exact locally snapshotted archive; it is not a signature or publisher authentication. Obtain packages from authors or registries whose distribution terms permit your use. Native Open VSX search/download and explicit stable updates are available as described in [registry workflows](EXTENSION_REGISTRY.md). Marketplace access is not implemented.

Packages are extracted into private staging directories with limits of **128 MiB per archive**, **256 MiB total extraction**, **32 MiB per file**, **20,000 entries** and **1 MiB for package.json**. Traversal, absolute/nonportable paths, duplicate/case-aliased paths, symlinks and special files are rejected. Declared entry counts are checked before ZIP metadata allocation; the parser is restricted to that validated archive footer so malformed metadata cannot fall back to a different unchecked count. ZIP64, multi-disk and ambiguous-footer archives are unsupported. Installed manifest listings have cumulative limits of 8 MiB of serialized metadata and 50,000 JSON nodes; reaching either limit reports an error, while direct ID lookup and uninstall remain available. A failed extraction or invalid manifest leaves the prior installation selected. Mutations take a nonblocking exclusive storage lock, publish immutable generations and atomically replace the registry. Rollback switches the registry to the previous generation; running hosts keep using their original files until restarted. Uninstall removes the registry entry, while immutable files are retained to protect running hosts; automatic garbage collection remains outstanding. Power-loss durability across filesystems has not been qualified.

**Installed does not mean compatible.** The manifest classification describes runtime shape only. Engine ranges, proposed APIs, native module ABI and broad extension API coverage remain unqualified. Browser-only packages can be stored but cannot run. Dependency-bearing packages require every declared dependency in the selected cohort; missing dependencies and cycles reject before activation. At most eight authorized selected packages share one CommonJS code host in one optional Node process; declarative contribution support depends on native adapters. The editor performs installation and host startup in bounded background jobs; canceled pending starts, replaced hosts, stopped hosts and failed hosts are retired by those workers, and their slots remain occupied until teardown completes; native buffers retain their identity, dirty state and version checks during activation.

Automated evidence includes malicious-path/link and oversized-archive rejection, preservation of an existing install after failures, concurrent-operation rejection, upgrade/rollback/remove, noninteractive CLI operation, and an actual Unix PTY workflow installing a VSIX, explicitly activating it, running its F9 command, saving/undoing and uninstalling while its host is live. The picker is available from an empty welcome screen and from editor, Explorer and terminal focus. Dismissing its loading view prevents a late reply from reopening it over a later prompt; a successfully committed installation is reported even if refreshing another package’s metadata fails. Host lifecycle tests verify canceled and outdated starts cannot become ready, readiness follows worker acknowledgement, and stalled retirement retains its slot while native input continues. A document lifecycle test verifies no fabricated documents at activation, correct mirrors after opening the first file, and rejection of edits to a closed final document. The real-protocol CI additionally packages the unchanged compiled **Sort Lines 1.12.0** entry files into a VSIX, installs it and checks F9 sorting plus native undo. This locally assembled archive tests installation and the named runtime workflow; it is not registry-download or publisher-signature qualification.

## Browsing and updating Open VSX packages

Use **F1 → Extensions: Search Open VSX**, type a query and press Enter. The native
picker shows up to 20 compatible stable packages; Enter installs the displayed
version. **/** in the installed picker opens search, and **U** checks for updates.
**F1 → Extensions: Check for Updates** lists newer stable versions; Enter installs
one. Installed-list refresh displays the operation result. Installation and
updates do not activate code or change running immutable snapshots. Esc dismisses
an operation's view while its bounded worker finishes; a late result cannot replace
a newer prompt.

CLI equivalents work without a terminal UI or Node:

```sh
vscli --search-extensions 'sort lines'
vscli --install-extension Tyriar.sort-lines
vscli --check-extension-updates
vscli --update-extension Tyriar.sort-lines
```

`--extension-registry` selects an Open VSX-compatible server; the default is
`https://open-vsx.org`. Update JSON contains `items` and per-package `notices`.
The native picker shows the notice count and first notice; the CLI preserves all
notices. Unavailable packages do not hide other available updates. Read
[registry bounds, integrity checks and outstanding qualification](EXTENSION_REGISTRY.md).
The Unix PTY fixture exercises browse/install/update/rollback with Node unavailable,
native CRLF save/undo, and a late search reply while a newer Go to File prompt is
active. A local live check on Linux searched public Open VSX and installed
Tyriar Sort Lines 1.12.0 without executing it; that is registry transport and package
installation evidence, not validation of additional published extension workflows.

## Native snippet packages

Installed `contributes.snippets` are available through **F1 → Insert Snippet** and
`editor.action.insertSnippet` named lookup without starting a host. The bounded
catalog worker reads contribution data on each invocation, so upgrade, rollback
and uninstall refresh immediately on the next request. Language contributions
and global `.code-snippets` files are supported; paths must resolve inside the
package. All sources share the native catalog's file, byte and entry budgets.
Bad contributions produce notices, and an unreadable installed registry does
not disable user/workspace snippets. See [snippet scope and evidence](SNIPPETS.md)
for remaining language-registration, completion and extension-API gaps.

Real-VSIX integration tests cover language matching, malformed data, confinement,
combined limits and installation changes. A synthetic PTY package with an
activation marker and unavailable Node path verifies native C++ insertion,
linked Unicode edits, CRLF save/undo and uninstall refresh. This evidence does
not qualify every published snippet package or code extension.

## Running an extension

Build VSCLI normally, install Node 24, and pass an already unpacked, built extension directory:

```sh
vscli --extension /path/to/extension .
```

The flag explicitly runs that extension's code with your user permissions. Process isolation protects editor responsiveness and contains host failures; it is not a filesystem or network sandbox. VSCLI downloads packages only for an explicit Open VSX registry request and never executes package installation scripts; native VSIX installation is described above. Node is unnecessary unless a code extension host is explicitly started. Runtime bridge files are embedded in the native executable and materialized in a temporary directory only for an enabled host.

F1 lists registered commands. Manifest keybindings retain their original combinations, platform overrides, arguments, and supported `when` expressions. Packages activate in canonical extension-ID order. Duplicate extension command registration rejects with its existing owner; native command IDs and the native cursor-dispatch prefix are reserved. Defaults are ordered by package ID and then manifest order; later matching bindings win. These are documented VSCLI precedence rules, not qualified VS Code conflict-resolution parity. They are layered above native defaults and below user bindings; user removal rules are reapplied when the extension activates. Invalid or unsupported expressions reject that package’s contribution with a visible message while other packages retain their defaults. Stopping the host removes its defaults. User keybindings can also target original command IDs. Arguments remain one value, including arrays and explicit `null`; an omitted argument stays absent. Native editing and saving continue after a host crash. F1 → Extensions: Stop Host terminates it; F1 → Extensions: Restart Selected Session restarts the same selected snapshots. Enter in the installed picker replaces only that selected package’s snapshot with the currently installed generation. Adding, stopping or replacing a package restarts the cohort and resets its JavaScript state. There is no safe individual hot unload. No session is automatically restored on editor startup. Ordinary shutdown kills the process and waits up to five seconds for outstanding start/retirement workers; guaranteed `deactivate` completion and persistent extension state are not implemented.

## Native Quick Pick and Input Box

Activated packages can await `vscode.window.showQuickPick` and `showInputBox` through the native prompt layer. Quick Pick accepts an array or promised array of strings, or objects with `label`, `description` and `detail`; a selection returns the original string/object. Filtering uses case-insensitive substring matching on labels, with optional `matchOnDescription` and `matchOnDetail`. Input Box supports `title`, `prompt`, `placeHolder` and `value`. Enter accepts a single selection or input text, including an empty input; Escape returns `undefined`. Native focus is retained when the prompt closes.

Only one extension prompt is visible. Existing native prompts and modals retain priority; extension requests wait in FIFO order. Opening another native prompt cancels the visible extension prompt. Session/owner/request identity checks prevent old answers from reaching a replacement host. Stop, crash and cohort replacement clear visible and queued extension UI; graceful cancellation replies are best effort before process retirement, and callbacks cannot continue after Node is killed. Native buffers remain authoritative and any later extension edit still requires a current document/version.

The session permits eight pending prompt calls and 128 items per pick. Presentation data shares a 64 KiB budget; each label, description, detail, option or input is capped at 4 KiB UTF-8. Native requests expire after five minutes with cancellation. Only the originating awaited command or activation deadline pauses during this bounded human wait, including prompts created during module evaluation; edit, already-issued heartbeat and unrelated-command deadlines remain active. Idle heartbeats are scheduled only when no native request is pending; this does not promise continuous heartbeat dispatch during an awaited human prompt. A completed invocation's delayed callback does not revive its deadline association during another package's activation.

Multi-select, separators, buttons, preselected/always-show items, live selection callbacks, `createQuickPick`/`createInputBox`, cancellation-token arguments, password/validation/value-selection input options and `ignoreFocusOut: true` explicitly reject. Rich matching, accessibility and appearance parity against VS Code 1.95.0 remain unqualified. These adapters implement the named subset, not the complete Quick Input API.

## Implemented behavior

The API reference target for this initial experiment is VS Code 1.95.0; the exposed `vscode.version` identifies that target, not complete compatibility. No complete baseline conformance claim is made. Coordinate clamping and argument dispatch were checked against the pinned [document implementation](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/workbench/api/common/extHostDocumentData.ts) and [keybinding dispatcher](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/platform/keybinding/common/abstractKeybindingService.ts); the first running-reference configuration cases are described in the [differential harness](../tests/vscode-reference/README.md). Document and dispatch differential coverage remains outstanding.

- Native installed snippet contributions, independently of the optional host.
- CommonJS `require('vscode')`, explicit eager activation of up to eight packages, owned command/event registrations, extension path/URI, and extension mode. Each package receives its own API facade over shared document objects and configuration. Bundled modules resolve their package facade; modules outside selected package directories that require `vscode` are unsupported.
- Command registration/disposal/execution, contribution titles in the native palette, single-message notifications without choices, and the bounded native Quick Pick/Input Box subset described above.
- Active editor and selections as read-only mirrors; synchronous document text, lines, UTF-16 position/offset conversion, ranges, selections, URIs, and document lifecycle/change events.
- `TextEditor.edit` replace/insert/delete on open native buffers. Rust validates identity, version, UTF-16 boundaries, overlapping ranges, the 4,096-edit count limit, and size before one native undoable transaction. Stale requests resolve `false`; malformed transactions reject without partial changes.
- A single workspace folder and configuration reads from extension-declared defaults, imported user settings, and workspace `.vscode/settings.json`, including language overrides, `inspect`, and live change events.

Document versions increase across edits and undo/redo observations. State generations reject outdated document notifications. All mirrors are updated before document event callbacks run. Protocol v4 tags command registries and edits with their session and package owner and sends document text only for new or changed document revisions; selection, dirty-state, and path updates reuse cached text. Ordered state notifications precede edit acknowledgements, and the native baseline advances only after a message is queued successfully. Changed text still uses full snapshots rather than edit deltas; the total mirrored text budget is 4 MiB, transport frames are limited to 16 MiB, and pending requests and concurrent shim command calls are capped at 64 with a 30-second native timeout. An idle host receives a heartbeat every five seconds; a missing reply uses the same timeout. One process failure, stalled callback or timeout stops the entire selected cohort; restart remains explicit. Crossing a host limit stops or rejects extension work while preserving native buffers.

Unsupported service APIs throw explicit errors. Browser-only packages, proposed APIs, providers, webviews, notebooks, custom editors, workspace edits, settings writes, storage/secrets, unsupported activation events, extension menus, and built-in command delegation remain unsupported. Only the active editor is mirrored in `visibleTextEditors`; independent extension editor handles for split panes remain incomplete. Package engine ranges, native module ABI compatibility, and the full URI API are not yet validated. Local VSIX installation and rollback are implemented; native Open VSX registry workflows are described above.

## Session limits and failure behavior

Selected manifests have cumulative limits of 8 MiB and 50,000 JSON nodes, with 1 MiB per manifest. A manifest must be a regular file; FIFO manifests reject before opening. Manifest command and keybinding contributions, and runtime command registrations, each have a group limit of 1,024. Shim-owned registrations, including native event listeners, have a group limit of 4,096. The group shares one 4 MiB text-mirror budget and one bounded transport, rather than multiplying these limits per package. Output queues retain the existing 64-message / 64 MiB byte limits; inbound channels hold four frames. Highly escaped snapshots must also fit the 16 MiB wire-frame limit. Reaching a limit rejects extension work or stops the group while preserving native data.

On Unix, the optional host starts in its own process group. Cohort retirement sends SIGKILL to that group before killing and reaping the direct Node child; non-reaping leader status checks also detect crashes when descendants retain inherited pipes. Tests verify stop, restart, leader crash, activation failure and SIGTERM during pending activation terminate a Node descendant holding inherited stdio, and closure of an inherited transport pipe after leader failure. Descendants may remain exited zombies until their parent or system init reaps them; only the direct Node child is reaped by VSCLI. Processes that escape into another group/session are outside this cleanup boundary. Windows still kills and reaps only the direct child; descendant cleanup and lifetime thread/memory bounds there remain unqualified. LSP/DAP processes retain their existing direct-child lifecycle. The five-second App destruction wait is a deadline rather than an unconditional cleanup guarantee: startup or retirement blocked beyond it remains unqualified. The SIGTERM fixture covers a pending JavaScript activation that is killable within three seconds, not indefinitely stalled filesystem/process teardown.

The process starts with a 256 MiB V8 old-space limit. This does not bound its total RSS, Buffers, native-module allocation or subprocesses; total extension-enabled memory and CPU contention remain unqualified. A shared process contains failures outside native editing but gives no isolation between selected extensions and no filesystem/network sandbox.

Explicit startup publishes commands only after the whole selected group activates. Dependencies activate first, concurrent activation requests share one promise, and `vscode.extensions.getExtension` / `all` expose selected same-host descriptors and cached exports. Failed activation is cached until an explicit cohort restart; failed owners lose their registrations and fresh native requests from their retained API handles reject; dynamic reverse activation waits reject instead of deadlocking. Cross-host and proposed lookup variants remain unsupported. Lazy native activation appends selected immutable descriptors without recreating older active exports; remembered grants and supported events are described below. Valid native transactions requested during activation remain dirty and undoable even if a later package fails; failed activation does not roll back native editing. A stopped or failed host immediately loses its commands/default bindings, and kill/wait teardown runs in a worker. New startup remains blocked until retirement acknowledges. Generation-checked startup tickets and owner/session-tagged registries cannot revive a canceled session. Installed generation paths and hashes remain selected across another package's restart, rollback or uninstall. Arbitrary unpacked directories remain user-managed and can change on disk; manifest identity/version changes reject restart, but this is not a cryptographic snapshot of unpacked source files.

## Extension settings

Use the normal `--settings /path/to/user/settings.json` import and workspace `.vscode/settings.json`. Initial values are sent before extension activation; subsequent valid reloads update the host before firing `workspace.onDidChangeConfiguration`. Pending document mirrors are synchronized first so configuration listeners can read preceding native edits. The native loader checks files in the background every two seconds. Missing files become empty scopes, while malformed JSON retains the entire last valid configuration. Native input polling compares shared snapshot identities and only sends changed settings. Scope resolution is cached inside the optional host.

`workspace.getConfiguration(section, scope)` supports `get`, `has`, `inspect`, and direct section properties. General objects merge recursively, arrays/scalars replace earlier values, and explicit `null` remains a value. Extension property types supply defaults when no explicit default is declared. Application/machine settings ignore workspace overrides for effective reads and inspection. The pinned reference fixture verifies that a machine-scoped setting has no `workspaceValue` even when the workspace file supplies one. Extension-provided defaults do not replace the three built-in native setting definitions.

Reads held across a reload remain snapshots; request a fresh configuration in a change callback. `inspect` reads current scope values. Returned `get`/`inspect` objects can be mutated without altering stored settings. A URI alone does not infer a language; pass a document or `{ uri, languageId }` to apply language overrides. Identical combined-language groups merge in their first-seen position across scopes, with single-language groups applied last. Unscoped `affectsConfiguration` can report a changed setting even when a workspace value hides its effect; scoped queries additionally compare effective values. Retained event objects keep their original before/after states.

Behavior was checked against the pinned [extension configuration API](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/workbench/api/common/extHostConfiguration.ts), [configuration model](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/platform/configuration/common/configurationModels.ts), and [schema registry](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/platform/configuration/common/configurationRegistry.ts). These source checks and focused tests do not establish full configuration conformance. The [differential harness](../tests/vscode-reference/README.md) now compares 25 shared configuration observations with the actual VS Code 1.95.0 extension API. It caught and corrected inspection of ignored machine-scoped workspace values. This is a bounded fixture, not complete configuration qualification.

Only the three supported native defaults and selected extensions' `contributes.configuration` defaults are registered. All selected schemas are collected before the first activation; duplicate extension property definitions keep the first definition in canonical package-ID order, while native definitions retain priority. The complete built-in defaults catalog, `configurationDefaults` contributions, folder/multi-root settings, remote/application/policy layers, profiles, trust/restricted-setting handling, and schema validation of arbitrary extension values remain incomplete. Configuration writes explicitly reject. Importing an unknown native setting makes it readable by extensions; it does not enable its native editor behavior.

## Named workflow evidence

| Item | Evidence |
| --- | --- |
| Package | Tyriar Sort Lines 1.12.0, MIT |
| Source | [Upstream commit eaf02bb](https://github.com/Tyriar/vscode-sort-lines/tree/eaf02bb141f1853d571b1e97574c6857e80a727c) |
| Preparation | `npm ci --ignore-scripts --no-audit --no-fund`, then `npm run compile`; source remains unchanged |
| Tested workflow | Select `zebra\napple\npear`, press its original **F9** shortcut (`sortLines.sortLines`), observe `apple\npear\nzebra`, undo to original |
| Settings workflow | With no selection, workspace `sortLines.sortEntireFile: true` enables original F9 whole-file sorting; removing the override restores the user value `false`; adding it back enables sorting live |
| Native data behavior | Edit stays unsaved; native undo/save remain authoritative |
| Local environment | Linux x86_64, Node 26.10.0; no terminal needed for the direct integration test |
| Automated gate | Real protocol servers CI builds this pinned source with Node 24 and runs both workflows |
| Package status | Experimental; the named sorting/undo and settings workflows pass, other package workflows are unqualified |

Reproduce after building the pinned upstream package:

```sh
VSCLI_TEST_SORT_LINES=/absolute/path/to/vscode-sort-lines \
  cargo test --test extension_host -- --ignored
```

Separate synthetic fixtures test Unicode edits, version changes through undo, stale rejection, atomic rejection of overlaps, explicit unsupported API failures, host crashes, and continued editing. Shared-session fixtures additionally check cross-package commands, shared document identity/configuration defaults, concurrent same-version edits, staged activation failure and reserved command conflicts. The Unix shared-session PTY workflow exercises two installed packages, repeated CLI selectors, deterministic F9 precedence, native save/undo/stale rejection, immutable selected generations across upgrade/rollback/uninstall, stop-selected/restart, whole-host crashes, Unix inherited-stdio descendant termination on stop/restart/leader crash, and editing with a missing Node executable. Normal exit and failed activation reap the shared process; SIGTERM during pending activation preserves the latest recovery text, restores the terminal and reaps Node and terminates its inherited-stdio descendant within the fixture’s three-second deadline. These fixtures establish those named behaviors; multi-extension differential qualification against VS Code 1.95.0 remains outstanding. The Unix PTY suite also exercises activation-time settings, live configuration events, palette/F9 dispatch, save, undo, and host stop through the actual executable. Node API tests cover callback lifetime and out-of-order mirror updates.

The upstream-host extraction comparison, broader reference-editor differential tests, broader extension corpus, API capability reports, broader registry distribution, garbage collection, and native adapters for richer contributions remain required work. A single successful command extension does not establish compatibility with language services, Git providers, debuggers, or graphical extensions.

## Named Quick Pick workflow evidence

The unchanged MIT **Tyriar Lorem Ipsum 1.3.1** package at [commit 99a5f039](https://github.com/Tyriar/vscode-lorem-ipsum/tree/99a5f039f26b1b657ac18e79f81a834a4df64a3d) was prepared with `npm ci --ignore-scripts --omit=dev --no-audit --no-fund`; its source remains unchanged. On Linux with Node 26.10.0, `lorem-ipsum.multipleParagraphs` displays its original string Quick Pick, cancel leaves native text unchanged, and selecting `2` inserts two generated paragraphs into the native buffer. The test checks dirty state, stable document identity, save and native undo. The shim omits semantic no-op empty-range deletes before strict native overlap validation; invalid overlapping transactions still reject atomically. Other package commands, multi-cursor selections and broader extension compatibility remain unqualified.

```sh
VSCLI_TEST_LOREM_IPSUM=/absolute/path/to/vscode-lorem-ipsum \
  cargo test --locked --test extension_prompts -- --ignored
```

Deterministic fixtures cover item identity, Unicode input/edit/undo, both cancellation paths, native prompt priority, FIFO order, stale session answers, stop/restart/crash cleanup, module-evaluation prompts and exact-origin deadline suspension. Unix PTY fixtures also cover module-evaluation input acceptance/cancellation before activation completes and refusal of oversized prompt input. Completed-A background prompts during B activation are independent of B's live activation deadline. Cross-package nested commands retain the prompt API owner separately from the originating command owner. Opening a host-requested prompt dismisses parameter hints without changing shared native text. Full Quick Input differential qualification against VS Code 1.95.0 remains outstanding.

## Mirror performance measurement

`node scripts/bench_extension_mirrors.cjs` measures selection-only Node mirror updates for a 1.7 MB, 100,001-line document over 30 iterations. On Linux x86_64 with Node 26.10.0, a comparison against commit `3c7a698` measured 18.28 ms mean / 30.74 ms p95 before and 0.0125 ms mean / 0.0397 ms p95 after. The serialized state payload fell from 1,800,223 bytes to 213 bytes. These are component measurements, excluding Rust serialization, pipe transport, rendering, and end-to-end input latency; they are not a whole-editor speed claim. Timing is reported rather than used as a flaky CI threshold. A Rust regression test independently bounds selection-only wire payloads below 1 KiB for two open documents.

For a previous runtime checkout, set `VSCLI_BENCH_HOST=/path/to/extension-host` and use `--full-text` to reproduce the former update shape. Initial synchronization and actual content edits still have size-dependent costs; incremental content synchronization remains required performance work.

## Remembered execution enablement and supported events

Installation alone never authorizes code execution. Explicit installed Enter / `--extension` remains an eager run-once action and does not write remembered grants. Native commands `vscli.extensions.enableGlobal`, `enableWorkspace`, `disableGlobal`, and `disableWorkspace` accept an installed ID string or `{ "id": "publisher.name" }`. For example, a user keybinding can explicitly enable one package for the workspace:

```json
{ "key": "ctrl+alt+e", "command": "vscli.extensions.enableWorkspace", "args": { "id": "tyriar.sort-lines" } }
```

Both scopes live under the actual native user configuration root: `extensions-enabled.json` globally and `extension-workspaces/<SHA-256-of-canonical-workspace-URI>.json` for a workspace. Imported profile copies and repository `.vscli` files do not grant execution. Workspace values override global values; absent IDs are disabled. These are execution grants: installed native theme/snippet contribution behavior remains separate. Grants require schema 1, at most 128 IDs and a regular nonsymlink file at most 1 MiB. A bounded cross-process lease merges one requested ID against the current file and atomically publishes the result, preserving unrelated updates. Permission changes take effect after that write succeeds; a malformed file or unavailable native configuration rejects new automatic activation while an already active host remains usable. The editor keeps one retained disk/planning worker and one queued enable/disable intent with its original configuration/workspace/store context. A later stop cannot be undone by an older completion.

Supported events are `*`, `onStartupFinished`, `onCommand:<id>`, `onLanguage:<language-id>`, and a limited `workspaceContains:<glob>` scan. Contributed commands/languages infer their corresponding events; explicit non-contributed command events also work through user bindings. Event types outside that subset produce notices. `workspaceContains` supports root-relative, case-sensitive `*`, `?`, and `**` patterns on regular files; bracket/brace/backslash/parent traversal and absolute patterns are unsupported. The worker checks at most 128 patterns and 10,000 entries, depth 16, 4 MiB of path text and 500 ms between filesystem operations; it skips `.git` and symlinks/special files. Unmatched patterns after a limit are unqualified. A blocking filesystem operation can exceed that clock budget; it cannot block input/rendering.

Enabled dormant commands/default bindings are available before Node starts. Native/user key precedence is retained; package default bindings use existing ascending package-ID order, with later package IDs winning conflicts. Duplicate dormant command claims choose the first package ID; runtime registration collisions still fail explicitly. The native dormant command catalog is bounded to 1,024 IDs / 256 KiB of presentation and owner text; excess commands are unavailable with a notice. Native activation holds at most eight queued commands, 64 KiB of arguments each and 256 KiB combined. Planning runs outside input/rendering and keeps selected generation manifests through registry upgrades/removal.

Every declared dependency must be installed and separately enabled, or explicitly selected for the current run-once cohort. Dependencies activate first; missing/disabled dependencies and cycles reject before module evaluation. The eight-package cohort is cumulative: another event cannot silently evict an active package. `vscode.extensions` exposes only selected same-host packages; cached activation coalesces calls and keeps exports by reference inside Node. Native code starts no Node process until an enabled eligible closure contains code. A declarative-only event needs no runtime. Package engine compatibility and broad extension APIs remain unqualified.

Disable retires the entire selected host if the effective disabled ID is selected. Stop/crash/start failure pauses automatic retry until explicit enable or restart; failed same-host packages remain failed until a cohort restart, while older active owners remain usable. A stop clears queued command dispatch and native prompts. This does not provide dependency installation, safe hot unloading, cross-host exports or full VS Code activation parity. Current evidence includes five native activation integrations, four scheduler integrity/bounds regressions, Node dependency/export/failure tests, and two PTY workflows covering explicit remembered grants, missing dependency consent, exact Unicode edits/save/undo, disable/reaping/reopen, and non-contributed command activation with dependency Input Box cancel/accept. Named-package automatic activation is still awaiting separate qualification.
