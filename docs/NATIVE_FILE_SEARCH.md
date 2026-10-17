# Extension file discovery

Selected extensions can call `workspace.findFiles(include, exclude, maxResults,
token)` through the optional host. `RelativePattern` accepts a path, file URI or
workspace folder. Native traversal runs in a bounded background Rust worker;
core editing does not require Node.

A string include searches the current native workspace. A RelativePattern
searches its explicit local directory, including directories outside the workspace.
Default and explicit-pattern exclusions honor `files.exclude`; an explicit pattern
adds to those defaults. Passing `null` disables exclusions. `search.exclude`
does not apply. Standard file exclusions can be disabled with a later `false`
setting. Conditional sibling exclusions and ordinary `*`, `?`, `**`, braces and
character classes are supported. Results are file URIs, and order is unspecified.

Cancellation returns an empty array. An already-canceled token creates no native
request. Cancellation after admission can resolve publicly before the worker
exits; actual request capacity remains occupied until the native thread exits,
is joined and its terminal response arrives. A timeout also retains capacity.
A new search cannot borrow a retired extension's original caller authority.
Searches retain their original workspace/settings profile across the queue;
changing those settings invalidates an outstanding search. Editor focus and
buffer revisions do not authorize read-only discovery. Searches leave dirty
buffers, document identity, Undo/Redo history and file contents untouched.

## Bounds and remaining compatibility

The current backend has one native workspace and local file directories.
Custom search providers, remote schemes, resource-specific/multi-root settings,
nested glob braces and global Git ignore configuration remain unsupported.
`search.experimental.useIgnoreFilesInFindFiles` opts into local ignore parsing;
ordinary discovery does not apply `.gitignore`. In that experimental mode,
`search.useIgnoreFiles`, `search.useParentIgnoreFiles` and
`search.useGlobalIgnoreFiles` control policy; enabled global ignores currently
produce an explicit error. `null` bypasses exclusions and ignores.
`search.followSymlinks` controls traversal; ancestor cycles produce an error.

Limits are 4,096 results, 2 MiB encoded results, 100,000 traversed entries,
16 MiB cumulative visited paths, 4,096-byte paths and 64 directory levels.
An explicit result limit stops enumeration; an omitted limit produces an error
if the safety cap is exceeded. Native glob inputs are limited to 1,024 bytes, 32 brace alternatives and 128
wildcards. Oversized/unsupported inputs fail explicitly.
The native deadline is ten seconds from receipt, including queue time; the host
acknowledgement timeout is five seconds. Filesystem calls cannot be forcibly
interrupted, so occupied capacity is retained until actual exit. Traversal is
observational rather than an atomic filesystem snapshot.

## Evidence

Nine native worker tests, an exact cancellation-origin regression and two actual
selected-host workflows pass. All 454 host tests and 1,172 Rust all-target tests
pass (876 library tests; 23 ignored cases confer no qualification). The native
terminal journey passes default/null/additive exclusions, relative bases, result
limits and cancellation while preserving dirty Unicode/CRLF buffers, original
keyboard Undo/Redo and every disk file. Build, formatting and strict locked
all-target Clippy pass.
No local VS Code instance is launched. Source comparison uses pinned upstream
[workspace API](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostWorkspace.ts),
[native search dispatch](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/browser/mainThreadWorkspace.ts)
and [query construction](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/services/search/common/queryBuilder.ts).
Source comparison, local execution and full upstream/platform parity are separate.
