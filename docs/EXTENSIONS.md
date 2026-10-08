# Extension host experiment

VSCLI now has an optional CommonJS extension process. It is an independent, original compatibility shim; the Rust executable remains the owner of native buffers, rendering, input, and persistence. This is an experiment toward the full extension requirement, not completion of that requirement.

## Running an extension

Build VSCLI normally, install Node 24, and pass an already unpacked, built extension directory:

```sh
vscli --extension /path/to/extension .
```

The flag explicitly runs that extension's code with your user permissions. Process isolation protects editor responsiveness and contains host failures; it is not a filesystem or network sandbox. VSCLI does not download packages or execute package installation scripts. Node is unnecessary without `--extension`. Runtime bridge files are embedded in the native executable and materialized in a temporary directory only for an enabled host.

F1 lists registered commands. User keybindings can target their original IDs. Native editing and saving continue after a host crash. F1 → Extensions: Stop Host terminates it; restart currently requires relaunching VSCLI. Ordinary shutdown kills the process; guaranteed `deactivate` completion and persistent extension state are not implemented.

## Implemented behavior

The API reference target for this initial experiment is VS Code 1.95.0; the exposed `vscode.version` identifies that target, not complete compatibility. No complete baseline conformance claim is made.

- CommonJS `require('vscode')`, explicit eager activation, subscription disposal, extension path/URI, and extension mode.
- Command registration/disposal/execution, contribution titles in the native palette, and single-message notifications without choices.
- Active editor and selections as read-only mirrors; synchronous document text, lines, UTF-16 position/offset conversion, ranges, selections, URIs, and document lifecycle/change events.
- `TextEditor.edit` replace/insert/delete on open native buffers. Rust validates identity, version, UTF-16 boundaries, overlapping ranges, and size before one native undoable transaction. Stale requests resolve `false`; malformed transactions reject without partial changes.
- A single workspace folder and configuration reads from extension-declared defaults. User/workspace overrides for extension settings are not yet imported.

Document versions increase across edits and undo/redo observations. State generations prevent delayed responses from replacing newer mirrors. All mirrors are updated before document event callbacks run. Full snapshots are currently used rather than incremental deltas; the total mirrored text budget is 4 MiB, transport frames are limited to 16 MiB, and pending command requests are capped at 64 with a 30-second timeout. Crossing a host limit stops or rejects extension work while preserving native buffers.

APIs outside this surface throw explicit errors. Browser-only packages, extension dependencies/proposed APIs, providers, webviews, notebooks, custom editors, workspace edits, settings writes, storage/secrets, automatic activation rules, multiple extension packages, extension-contributed keybindings/menus, and built-in command delegation remain unsupported. Only the active editor is mirrored in `visibleTextEditors`; independent extension editor handles for split panes remain incomplete. Package engine ranges, native module ABI compatibility, and the full URI API are not yet validated. There is no VSIX/registry installer yet.

## Named workflow evidence

| Item | Evidence |
| --- | --- |
| Package | Tyriar Sort Lines 1.12.0, MIT |
| Source | [Upstream commit eaf02bb](https://github.com/Tyriar/vscode-sort-lines/tree/eaf02bb141f1853d571b1e97574c6857e80a727c) |
| Preparation | `npm ci --ignore-scripts --no-audit --no-fund`, then `npm run compile`; source remains unchanged |
| Tested workflow | Select `zebra\napple\npear`, invoke `sortLines.sortLines`, observe `apple\npear\nzebra`, undo to original |
| Native data behavior | Edit stays unsaved; native undo/save remain authoritative |
| Local environment | Linux x86_64, Node 26.10.0; no terminal needed for the direct integration test |
| Automated gate | Real protocol servers CI builds this pinned source with Node 24 and runs the same workflow |
| Package status | Experimental; the named sorting/undo workflow passes, other package workflows are unqualified |

Reproduce after building the pinned upstream package:

```sh
VSCLI_TEST_SORT_LINES=/absolute/path/to/vscode-sort-lines \
  cargo test --test extension_host -- --ignored
```

Separate synthetic fixtures test Unicode edits, version changes through undo, stale rejection, atomic rejection of overlaps, explicit unsupported API failures, host crashes, and continued editing. The Unix PTY suite also exercises activation, palette dispatch, save, and undo through the actual executable. Node API tests cover callback lifetime and out-of-order mirror updates.

The upstream-host extraction comparison, reference-editor differential tests, broader extension corpus, API capability reports, installation/rollback, and native adapters for richer contributions remain required work. A single successful command extension does not establish compatibility with language services, Git providers, debuggers, or graphical extensions.
