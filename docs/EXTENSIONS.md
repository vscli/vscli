# Extension installation and experimental host

VSCLI now has an optional CommonJS extension process. It is an independent, original compatibility shim; the Rust executable remains the owner of native buffers, rendering, input, and persistence. This is an experiment toward the full extension requirement, not completion of that requirement.

## Installing and managing packages

Use **F1 → Extensions: Install from VSIX** and enter a local `.vsix` path. Installation extracts the package without running scripts or extension code. **Ctrl+Shift+X** (**Cmd+Shift+X** on macOS), or **F1 → Extensions: Show Installed Extensions**, lists installed IDs, versions and compatibility descriptions. Enter offers explicit activation of a code extension; **R** restores the previous installation and **Delete** removes the selected package from the installed registry. Starting a code extension replaces the single running host. Declarative themes and installed snippet contributions do not need Node.

The same operations work without a terminal UI or JavaScript runtime:

```sh
vscli --install-extension ./publisher.extension.vsix
vscli --list-extensions
vscli --extension publisher.extension .
vscli --rollback-extension publisher.extension
vscli --uninstall-extension publisher.extension
```

`--extensions-dir /path/to/storage` overrides the platform-local VSCLI extension directory for every operation. `--list-extensions` emits JSON with the manifest, immutable package path, source archive path, SHA-256 digest and a compatibility description. The digest records the exact locally snapshotted archive; it is not a signature or publisher authentication. Obtain packages from authors or registries whose distribution terms permit your use. Network registry search/download and Marketplace access are not implemented.

Packages are extracted into private staging directories with limits of **128 MiB per archive**, **256 MiB total extraction**, **32 MiB per file**, **20,000 entries** and **1 MiB for package.json**. Traversal, absolute/nonportable paths, duplicate/case-aliased paths, symlinks and special files are rejected. Declared entry counts are checked before ZIP metadata allocation; the parser is restricted to that validated archive footer so malformed metadata cannot fall back to a different unchecked count. ZIP64, multi-disk and ambiguous-footer archives are unsupported. Installed manifest listings have cumulative limits of 8 MiB of serialized metadata and 50,000 JSON nodes; reaching either limit reports an error, while direct ID lookup and uninstall remain available. A failed extraction or invalid manifest leaves the prior installation selected. Mutations take a nonblocking exclusive storage lock, publish immutable generations and atomically replace the registry. Rollback switches the registry to the previous generation; running hosts keep using their original files until restarted. Uninstall removes the registry entry, while immutable files are retained to protect running hosts; automatic garbage collection remains outstanding. Power-loss durability across filesystems has not been qualified.

**Installed does not mean compatible.** The manifest classification describes runtime shape only. Engine ranges, proposed APIs, native module ABI and broad extension API coverage remain unqualified. Browser-only packages and dependency-bearing packages can be stored, but the host cannot run them. Only one CommonJS code extension can run at once; declarative contribution support depends on native adapters. The editor performs installation and host startup in bounded background jobs; canceled pending starts and replaced hosts are retired by those workers, and their slots remain occupied until teardown completes; native buffers retain their identity, dirty state and version checks during activation.

Automated evidence includes malicious-path/link and oversized-archive rejection, preservation of an existing install after failures, concurrent-operation rejection, upgrade/rollback/remove, noninteractive CLI operation, and an actual Unix PTY workflow installing a VSIX, explicitly activating it, running its F9 command, saving/undoing and uninstalling while its host is live. The picker is available from an empty welcome screen and from editor, Explorer and terminal focus. Dismissing its loading view prevents a late reply from reopening it over a later prompt; a successfully committed installation is reported even if refreshing another package’s metadata fails. A host lifecycle test verifies no fabricated documents at activation, correct mirrors after opening the first file, and rejection of edits to a closed final document. The real-protocol CI additionally packages the unchanged compiled **Sort Lines 1.12.0** entry files into a VSIX, installs it and checks F9 sorting plus native undo. This locally assembled archive tests installation and the named runtime workflow; it is not registry-download or publisher-signature qualification.

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

The flag explicitly runs that extension's code with your user permissions. Process isolation protects editor responsiveness and contains host failures; it is not a filesystem or network sandbox. VSCLI does not download packages or execute package installation scripts; native VSIX installation is described above. Node is unnecessary unless a code extension host is explicitly started. Runtime bridge files are embedded in the native executable and materialized in a temporary directory only for an enabled host.

F1 lists registered commands. Manifest keybindings retain their original combinations, platform overrides, arguments, and supported `when` expressions. They are layered above native defaults and below user bindings; user removal rules are reapplied when the extension activates. Invalid or unsupported expressions reject the contribution with a visible message. Stopping the host removes its defaults. User keybindings can also target original command IDs. Arguments remain one value, including arrays and explicit `null`; an omitted argument stays absent. Native editing and saving continue after a host crash. F1 → Extensions: Stop Host terminates it; an installed package can be restarted through the installed-extensions picker. Ordinary shutdown kills the process; guaranteed `deactivate` completion and persistent extension state are not implemented.

## Implemented behavior

The API reference target for this initial experiment is VS Code 1.95.0; the exposed `vscode.version` identifies that target, not complete compatibility. No complete baseline conformance claim is made. Coordinate clamping and argument dispatch were checked against the pinned [document implementation](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/workbench/api/common/extHostDocumentData.ts) and [keybinding dispatcher](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/platform/keybinding/common/abstractKeybindingService.ts); the first running-reference configuration cases are described in the [differential harness](../tests/vscode-reference/README.md). Document and dispatch differential coverage remains outstanding.

- Native installed snippet contributions, independently of the optional host.
- CommonJS `require('vscode')`, explicit eager activation, subscription disposal, extension path/URI, and extension mode.
- Command registration/disposal/execution, contribution titles in the native palette, and single-message notifications without choices.
- Active editor and selections as read-only mirrors; synchronous document text, lines, UTF-16 position/offset conversion, ranges, selections, URIs, and document lifecycle/change events.
- `TextEditor.edit` replace/insert/delete on open native buffers. Rust validates identity, version, UTF-16 boundaries, overlapping ranges, and size before one native undoable transaction. Stale requests resolve `false`; malformed transactions reject without partial changes.
- A single workspace folder and configuration reads from extension-declared defaults, imported user settings, and workspace `.vscode/settings.json`, including language overrides, `inspect`, and live change events.

Document versions increase across edits and undo/redo observations. State generations reject outdated document notifications. All mirrors are updated before document event callbacks run. Protocol v3 sends document text only for new or changed document revisions; selection, dirty-state, and path updates reuse cached text. Ordered state notifications precede edit acknowledgements, and the native baseline advances only after a message is queued successfully. Changed text still uses full snapshots rather than edit deltas; the total mirrored text budget is 4 MiB, transport frames are limited to 16 MiB, and pending command requests are capped at 64 with a 30-second timeout. Crossing a host limit stops or rejects extension work while preserving native buffers.

Unsupported service APIs throw explicit errors. Browser-only packages, extension dependencies/proposed APIs, providers, webviews, notebooks, custom editors, workspace edits, settings writes, storage/secrets, automatic activation rules, multiple simultaneous code extensions, extension menus, and built-in command delegation remain unsupported. Only the active editor is mirrored in `visibleTextEditors`; independent extension editor handles for split panes remain incomplete. Package engine ranges, native module ABI compatibility, and the full URI API are not yet validated. Local VSIX installation and rollback are implemented; registry search/download remains outstanding.

## Extension settings

Use the normal `--settings /path/to/user/settings.json` import and workspace `.vscode/settings.json`. Initial values are sent before extension activation; subsequent valid reloads update the host before firing `workspace.onDidChangeConfiguration`. Pending document mirrors are synchronized first so configuration listeners can read preceding native edits. The native loader checks files in the background every two seconds. Missing files become empty scopes, while malformed JSON retains the entire last valid configuration. Native input polling compares shared snapshot identities and only sends changed settings. Scope resolution is cached inside the optional host.

`workspace.getConfiguration(section, scope)` supports `get`, `has`, `inspect`, and direct section properties. General objects merge recursively, arrays/scalars replace earlier values, and explicit `null` remains a value. Extension property types supply defaults when no explicit default is declared. Application/machine settings ignore workspace overrides for effective reads and inspection. The pinned reference fixture verifies that a machine-scoped setting has no `workspaceValue` even when the workspace file supplies one. Extension-provided defaults do not replace the three built-in native setting definitions.

Reads held across a reload remain snapshots; request a fresh configuration in a change callback. `inspect` reads current scope values. Returned `get`/`inspect` objects can be mutated without altering stored settings. A URI alone does not infer a language; pass a document or `{ uri, languageId }` to apply language overrides. Identical combined-language groups merge in their first-seen position across scopes, with single-language groups applied last. Unscoped `affectsConfiguration` can report a changed setting even when a workspace value hides its effect; scoped queries additionally compare effective values. Retained event objects keep their original before/after states.

Behavior was checked against the pinned [extension configuration API](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/workbench/api/common/extHostConfiguration.ts), [configuration model](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/platform/configuration/common/configurationModels.ts), and [schema registry](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/platform/configuration/common/configurationRegistry.ts). These source checks and focused tests do not establish full configuration conformance. The [differential harness](../tests/vscode-reference/README.md) now compares 25 shared configuration observations with the actual VS Code 1.95.0 extension API. It caught and corrected inspection of ignored machine-scoped workspace values. This is a bounded fixture, not complete configuration qualification.

Only the three supported native defaults and the enabled extension's `contributes.configuration` defaults are registered. The complete built-in defaults catalog, `configurationDefaults` contributions, folder/multi-root settings, remote/application/policy layers, profiles, trust/restricted-setting handling, and schema validation of arbitrary extension values remain incomplete. Configuration writes explicitly reject. Importing an unknown native setting makes it readable by extensions; it does not enable its native editor behavior.

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

Separate synthetic fixtures test Unicode edits, version changes through undo, stale rejection, atomic rejection of overlaps, explicit unsupported API failures, host crashes, and continued editing. The Unix PTY suite also exercises activation-time settings, live configuration events, palette/F9 dispatch, save, undo, and host stop through the actual executable. Node API tests cover callback lifetime and out-of-order mirror updates.

The upstream-host extraction comparison, broader reference-editor differential tests, broader extension corpus, API capability reports, registry distribution, garbage collection, and native adapters for richer contributions remain required work. A single successful command extension does not establish compatibility with language services, Git providers, debuggers, or graphical extensions.

## Mirror performance measurement

`node scripts/bench_extension_mirrors.cjs` measures selection-only Node mirror updates for a 1.7 MB, 100,001-line document over 30 iterations. On Linux x86_64 with Node 26.10.0, a comparison against commit `3c7a698` measured 18.28 ms mean / 30.74 ms p95 before and 0.0125 ms mean / 0.0397 ms p95 after. The serialized state payload fell from 1,800,223 bytes to 213 bytes. These are component measurements, excluding Rust serialization, pipe transport, rendering, and end-to-end input latency; they are not a whole-editor speed claim. Timing is reported rather than used as a flaky CI threshold. A Rust regression test independently bounds selection-only wire payloads below 1 KiB for two open documents.

For a previous runtime checkout, set `VSCLI_BENCH_HOST=/path/to/extension-host` and use `--full-text` to reproduce the former update shape. Initial synchronization and actual content edits still have size-dependent costs; incremental content synchronization remains required performance work.
