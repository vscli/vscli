# Extension filesystem watchers

Selected extensions can use `workspace.createFileSystemWatcher(pattern,
ignoreCreateEvents, ignoreChangeEvents, ignoreDeleteEvents)`. Each watcher exposes
readonly ignore flags, individually disposable `onDidCreate`, `onDidChange` and
`onDidDelete` subscriptions, and disposal of the watcher itself.

String globs receive events within the native workspace. RelativePattern bases
can refer to directories outside it, individual files or registered custom
filesystem providers. Simple relative patterns watch nonrecursively; `**` or a
path separator requests recursion. Recursive watches honor `files.watcherExclude`;
nonrecursive watches bypass those exclusions. A later `false` entry disables a
standard exclusion. The matcher follows the pinned upstream glob parser rather
than applying Quick Open's hardcoded directory skips.

External local changes are observed through OS notifications and background
metadata reconciliation. Local watch setup, traversal and metadata work run in
dedicated workers in the optional compatibility process. Watches with the same base/recursion/exclusions share one OS backend; its last
subscriber owns shutdown. Initial contents do not
produce a synthetic create flood. Missing/deleted bases are monitored for later
creation. Child symlinks are not traversed; an explicit symlink base can be watched.
Custom providers receive actual `watch(uri, {recursive, excludes})` calls and their
`onDidChangeFile` events are routed to matching consumer watchers. Disposing the
registration also disposes its watches.

Callbacks retain their original extension caller lease. Watch disposal stops
delivery immediately; local worker capacity is released only on actual worker
exit. Late messages from a replaced backend cannot rebind to its replacement.
Configuration updates refresh watch exclusions. Watching and reporting external
changes do not edit or save native buffers, replace document identity, or add
Undo history. Native editing remains usable without JavaScript.

## Evidence and limits

All 482 host tests pass, including 12 glob-parser cases, eight facade/provider/
shared-session cases, one retired-cleanup regression and seven real OS backend
tests. The selected-host native integration and terminal journey pass external
create/change/delete and disposal while preserving dirty Unicode/CRLF document
identity, original Undo and disk bytes. All 1,173 Rust all-target tests pass
(876 library; 23 ignored cases grant no qualification). Build, formatting and
strict locked all-target Clippy pass.
Full extension API parity, upstream runtime behavior and Windows/macOS execution
are not established by this implementation. OS event ordering/coalescing varies
by platform. Metadata reconciliation does not guarantee observation of every
transient create-delete or intermediate same-byte rewrite. Admission is bounded to 64 public watchers and 32 actual local workers, including
workers retiring after disposal. Each worker has at most 256 indexed directories and two additional watch anchors,
10,000 indexed entries, 4 MiB of indexed paths, 64 directory levels and a ten-second
scan deadline. Event queues admit 4,096 events/2 MiB, with four in-flight batches
of at most 64 events/64 KiB. Exclude inputs admit 128 patterns/32 KiB; individual
globs are capped at 4,096 UTF-8 bytes with bounded parser expansion. Overflow and
backend failures report errors; they do not invent a successful observation.
Local watching requires Node 22 or later; core editing has no Node dependency.

File-operation `onWill`/`onDid` events are distinct from filesystem
watchers and remain separate work, as do remote/multi-root backends.

[Upstream event service](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostFileSystemEventService.ts)
and [stable declarations](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vscode-dts/vscode.d.ts)
pin the source comparison. No local VS Code instance is launched.
