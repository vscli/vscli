# Extension host experiment

VSCLI now has an optional CommonJS extension process. It is an independent, original compatibility shim; the Rust executable remains the owner of native buffers, rendering, input, and persistence. This is an experiment toward the full extension requirement, not completion of that requirement.

## Running an extension

Build VSCLI normally, install Node 24, and pass an already unpacked, built extension directory:

```sh
vscli --extension /path/to/extension .
```

The flag explicitly runs that extension's code with your user permissions. Process isolation protects editor responsiveness and contains host failures; it is not a filesystem or network sandbox. VSCLI does not download packages or execute package installation scripts. Node is unnecessary without `--extension`. Runtime bridge files are embedded in the native executable and materialized in a temporary directory only for an enabled host.

F1 lists registered commands. Manifest keybindings retain their original combinations, platform overrides, arguments, and supported `when` expressions. They are layered above native defaults and below user bindings; user removal rules are reapplied when the extension activates. Invalid or unsupported expressions reject the contribution with a visible message. Stopping the host removes its defaults. User keybindings can also target original command IDs. Arguments remain one value, including arrays and explicit `null`; an omitted argument stays absent. Native editing and saving continue after a host crash. F1 → Extensions: Stop Host terminates it; restart currently requires relaunching VSCLI. Ordinary shutdown kills the process; guaranteed `deactivate` completion and persistent extension state are not implemented.

## Implemented behavior

The API reference target for this initial experiment is VS Code 1.95.0; the exposed `vscode.version` identifies that target, not complete compatibility. No complete baseline conformance claim is made. Coordinate clamping and argument dispatch were checked against the pinned [document implementation](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/workbench/api/common/extHostDocumentData.ts) and [keybinding dispatcher](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/platform/keybinding/common/abstractKeybindingService.ts); reference-editor differential testing remains outstanding.

- CommonJS `require('vscode')`, explicit eager activation, subscription disposal, extension path/URI, and extension mode.
- Command registration/disposal/execution, contribution titles in the native palette, and single-message notifications without choices.
- Active editor and selections as read-only mirrors; synchronous document text, lines, UTF-16 position/offset conversion, ranges, selections, URIs, and document lifecycle/change events.
- `TextEditor.edit` replace/insert/delete on open native buffers. Rust validates identity, version, UTF-16 boundaries, overlapping ranges, and size before one native undoable transaction. Stale requests resolve `false`; malformed transactions reject without partial changes.
- A single workspace folder and configuration reads from extension-declared defaults, imported user settings, and workspace `.vscode/settings.json`, including language overrides, `inspect`, and live change events.

Document versions increase across edits and undo/redo observations. State generations reject outdated document notifications. All mirrors are updated before document event callbacks run. Protocol v3 sends document text only for new or changed document revisions; selection, dirty-state, and path updates reuse cached text. Ordered state notifications precede edit acknowledgements, and the native baseline advances only after a message is queued successfully. Changed text still uses full snapshots rather than edit deltas; the total mirrored text budget is 4 MiB, transport frames are limited to 16 MiB, and pending command requests are capped at 64 with a 30-second timeout. Crossing a host limit stops or rejects extension work while preserving native buffers.

Unsupported service APIs throw explicit errors. Browser-only packages, extension dependencies/proposed APIs, providers, webviews, notebooks, custom editors, workspace edits, settings writes, storage/secrets, automatic activation rules, multiple extension packages, extension menus, and built-in command delegation remain unsupported. Only the active editor is mirrored in `visibleTextEditors`; independent extension editor handles for split panes remain incomplete. Package engine ranges, native module ABI compatibility, and the full URI API are not yet validated. There is no VSIX/registry installer yet.

## Extension settings

Use the normal `--settings /path/to/user/settings.json` import and workspace `.vscode/settings.json`. Initial values are sent before extension activation; subsequent valid reloads update the host before firing `workspace.onDidChangeConfiguration`. The native loader checks files in the background every two seconds. Missing files become empty scopes, while malformed JSON retains the entire last valid configuration. Native input polling compares shared snapshot identities and only sends changed settings. Scope resolution is cached inside the optional host.

`workspace.getConfiguration(section, scope)` supports `get`, `has`, `inspect`, and direct section properties. General objects merge recursively, arrays/scalars replace earlier values, and explicit `null` remains a value. Extension property types supply defaults when no explicit default is declared. Application/machine settings ignore workspace overrides for effective reads; `inspect` exposes raw scoped values. Extension-provided defaults do not replace the three built-in native setting definitions.

Reads held across a reload remain snapshots; request a fresh configuration in a change callback. `inspect` reads current scope values. Returned `get`/`inspect` objects can be mutated without altering stored settings. A URI alone does not infer a language; pass a document or `{ uri, languageId }` to apply language overrides. Identical combined-language groups merge in their first-seen position across scopes, with single-language groups applied last. Unscoped `affectsConfiguration` can report a changed setting even when a workspace value hides its effect; scoped queries additionally compare effective values. Retained event objects keep their original before/after states.

Behavior was checked against the pinned [extension configuration API](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/workbench/api/common/extHostConfiguration.ts), [configuration model](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/platform/configuration/common/configurationModels.ts), and [schema registry](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/platform/configuration/common/configurationRegistry.ts). These source checks and focused tests do not establish full configuration conformance. There is no reference-editor differential suite yet.

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

The upstream-host extraction comparison, reference-editor differential tests, broader extension corpus, API capability reports, installation/rollback, and native adapters for richer contributions remain required work. A single successful command extension does not establish compatibility with language services, Git providers, debuggers, or graphical extensions.

## Mirror performance measurement

`node scripts/bench_extension_mirrors.cjs` measures selection-only Node mirror updates for a 1.7 MB, 100,001-line document over 30 iterations. On Linux x86_64 with Node 26.10.0, a comparison against commit `3c7a698` measured 18.28 ms mean / 30.74 ms p95 before and 0.0125 ms mean / 0.0397 ms p95 after. The serialized state payload fell from 1,800,223 bytes to 213 bytes. These are component measurements, excluding Rust serialization, pipe transport, rendering, and end-to-end input latency; they are not a whole-editor speed claim. Timing is reported rather than used as a flaky CI threshold. A Rust regression test independently bounds selection-only wire payloads below 1 KiB for two open documents.

For a previous runtime checkout, set `VSCLI_BENCH_HOST=/path/to/extension-host` and use `--full-text` to reproduce the former update shape. Initial synchronization and actual content edits still have size-dependent costs; incremental content synchronization remains required performance work.
