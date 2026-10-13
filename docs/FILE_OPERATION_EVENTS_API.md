# Public file operation events

`workspace.onDidCreateFiles`, `workspace.onDidRenameFiles` and
`workspace.onDidDeleteFiles` now receive completed native Explorer operations.
File/folder creation emits create, rename emits `{ oldUri, newUri }` pairs, and
successful system-trash deletion emits delete. Failures do not emit success.
These events describe native operations; arbitrary filesystem changes belong to
the independent `createFileSystemWatcher` channel.

Native file jobs retain their actual thread through terminal receipt, exit and
join. Model/editor mirror publication is admitted before the operation event.
A create callback observes the newly opened model; a rename callback observes
the old public document closed and the new resource mirrored. Native dirty text/
history and shared identity keep their existing path-operation behavior. Exact
upstream model disposal/reuse and editor-event timing remain separate obligations.

Public event objects are frozen; arrays and rename-pair objects retain ordinary
source mutability. Resources revive as real Uri objects. Scoped subscriptions
retain normal listener/context/disposal semantics; disposed owners cannot revive
old callbacks. Session/monotonic-sequence checks reject foreign/replayed frames.
Whole-frame validation precedes dispatch, preventing malformed later rows from
publishing partial events.

The per-host native queue retains at most 32 events/1 MiB, with 64 KiB per single
native-operation frame. New operations wait when capacity is exhausted. Writer
byte/channel pressure retains original messages for retry without renewing
sequence or duplicating accepted frames. Host teardown discards that session's
queue. State/configuration backpressure defers operation dispatch while the host
continues draining incoming messages. Mirror suspension prevents callbacks from
observing unadmitted state.

Completed working-copy move/delete cleanup removes source-tree customer API
references. Destination references and model ownership remain independent.
Create leaves references unchanged. Internal overwrite deletion must not use the
explicit working-copy-delete hook. Conditional bulk-operation lifetime arbitration
still needs integration.

Source: VS Code 1.141.0 commit `2a59476c9bfcb90b3ddc372c36762471b7dfad1c`,
[file event delivery](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostFileSystemEventService.ts)
and [reference removal](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/browser/mainThreadDocuments.ts).

Five host controls cover payloads/URI revival, outer-only freezing, malformed/
foreign/replayed/bounded frames, emitter disposal, scoped ownership and watcher
separation. Five native controls cover joined workers, bounded queues, actual
writer backpressure, source-tree reference removal and a real extension observing
create/rename after model publication. A real terminal journey covers native
create/rename/trash, exact event order/URIs, dirty Unicode/CRLF disk preservation,
isolated system trash and terminal restoration.

All 1,555 Rust all-target tests pass across 67 reports (1,206 library; 23 ignored),
all 753 host tests pass, and formatting, strict locked Clippy and build pass.
No local VS Code instance was launched; CI work remains deferred.

Will-operation participants/contributed WorkspaceEdits, copy/multi-operation
public backends, mixed file/text/notebook apply, public file Undo/Redo, precise
lifetime/event races and broader package/platform qualification remain open.
The resource-edit gateway stays disabled; native completion events are usable.
