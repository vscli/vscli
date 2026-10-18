# Extension filesystem API

The optional compatibility host now exposes `workspace.fs` (`stat`,
`readDirectory`, `createDirectory`, `readFile`, `writeFile`, `delete`, `rename`,
`copy`, `isWritableFileSystem`) and `workspace.registerFileSystemProvider`.
`FileType`, `FilePermission`, `FileChangeType` and `FileSystemError` are public.
These contracts are compared with stable VS Code 1.141.0 at
`2a59476c9bfcb90b3ddc372c36762471b7dfad1c`.

```js
const vscode = require('vscode');
const folder = vscode.workspace.workspaceFolders?.[0];
if (folder) {
  const uri = vscode.Uri.joinPath(folder.uri, 'generated', 'example.txt');
  await vscode.workspace.fs.writeFile(uri, new TextEncoder().encode('hello\n'));
  const bytes = await vscode.workspace.fs.readFile(uri);
}
```

Local `file:` operations use asynchronous filesystem calls in the isolated
optional host. Core Rust editing still runs without Node. A filesystem write
changes disk; it does not implement `TextDocument.save`, mutate editor text,
clear dirty state or manufacture save events. Reads and admitted writes own
their byte arrays. Parent directories are created for writes/transfers;
overwrite defaults to false for transfers. A cross-provider move deletes its
source only after every destination write acknowledges completion.

Provider registration validates the required methods, prevents duplicate
schemes, tracks the registering owner and disposes its event subscription and
capabilities with that owner. Readonly providers reject mutations. Queued calls
retain provider identity and cannot silently switch to a replacement provider.
Provider change events are filtered by scheme and copied into bounded URI
snapshots. Public watchers now route those events through actual provider watch
registrations; writable filesystem-provider document routing remains outstanding.
Separate readonly text-content providers now support [native virtual documents](VIRTUAL_DOCUMENTS_API.md).
See [filesystem watchers](FILE_WATCHERS_API.md).

Admission is bounded to 64 occupied operations and 64 MiB of admitted write
buffers. Slots remain occupied until the actual promise settles. Mutations
currently share a serial queue; reads run independently. Generic copy traversal
is limited to 256 levels. These bounds and serialization are explicit current
compatibility limits, not claims of upstream scheduling equality.

Local evidence: 18 host tests cover real binary I/O, parent creation, symlinks,
overwrite rejection, readonly providers, byte isolation, disposal, replacement
providers, failed cross-provider moves and occupied admission. The complete
host suite passes 223 tests (including the URI foundation); all 37 native extension tests pass.
`tests/extension_filesystem_pty.py` passes the real terminal workflow: disk
writes preserve a dirty native model, undo/redo and host-independent recovery,
without inventing save events. Formatting and strict Clippy checks pass.

Outstanding qualification includes trash (`useTrash: true` rejects explicitly),
writable filesystem-provider editors, automatic scheme activation, document
links, remote authorities, actual Windows/macOS behavior, provider errors and
filesystem races. Local rename with `overwrite: false` checks destination
existence before rename; atomic exclusion against another OS process is not
qualified. Ancestor destinations and resolved local aliases containing the
source reject before deletion. Directory overwrite replaces stale children.
Cross-provider directory copies can leave partial destinations on
failure, and cannot promise rollback or symlink preservation. Full extension
API compatibility remains the goal, not an achieved milestone.

Comparison sources:

- [Consumer operations and errors](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostFileSystemConsumer.ts)
- [Provider registration and lifecycle](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostFileSystem.ts)
- [Public filesystem errors](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTypes.ts)
- [Disk provider](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/platform/files/node/diskFileSystemProvider.ts)
