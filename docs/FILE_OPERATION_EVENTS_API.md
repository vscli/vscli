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

## Will-participant engine foundation

The engine now also has an opt-in [native coordinator transport](FILE_PARTICIPATION_TRANSPORT.md).
Real selected extensions deliver full DTOs through conversion-time target guards
and independent reply/completion receipts. Default editor sessions keep will
subscriptions unavailable until the pre-operation application path is connected.

The isolated compatibility runtime now contains a private create/rename/delete
participant engine. It is embedded in the native host package, but **the public
`workspace.onWillCreateFiles`, `onWillRenameFiles` and `onWillDeleteFiles` names
remain unavailable**. Native Explorer still follows its qualified completion-event
path above. This foundation does not intercept operations or apply edits yet.

The engine follows the same pinned
[AsyncEmitter](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/base/common/event.ts)
and extension-host file event sources. Each listener gets a mutable event with
shared Uri-array data, a cancellation token and `waitUntil`. Normal callbacks
must register waits synchronously. Delivery waits for all registered promises
before advancing to the next listener. Callback return values and non-WorkspaceEdit
promise values do not contribute edits. Contributions from one listener are
collected in settlement order. Callback failures and rejected waits are reported
without preventing later listeners from running.

Conversion occurs after awaited delivery. The complete original WorkspaceEdit
DTO survives, including file options/binary contents, annotations, snippets, EOL
and notebook edits. Text/snippet versions come from current private native
mirrors. Create rows suppress versions for that resource across their own edit,
including preceding text rows; suppression does not leak into another listener's
edit. Notebook version lookup is supported by the converter; VSCLI's participant
runtime supplies undefined until a real notebook model exists. Conversion and
thenable assimilation stay under the original registering extension lease.
Retiring/re-enabling an owner cannot renew that authority.

The source extension-host layer's timeout argument only diagnoses slow waits.
The future native coordinator must separately cancel and race its own timeout,
as the pinned main-thread participant does. Cancellation discards contributed
edits, while an unsettled promise continues occupying the engine. Logical result
and actual completion have separate receipts. Even a callback throwing after
registering an unfinished wait cannot release capacity prematurely.

Two non-obvious source behaviors are retained and tested: ordinary listener
disposal does not remove an already snapshotted async delivery, and cancellation
leaves undelivered entries with their original data in that kind's queue. A
synchronous callback throw also skips the source collection freeze/await step;
its already accepted work remains independently observed. Lifetime and capacity
guards bound later admissions rather than silently rewriting those behaviors.

Native adaptations admit one actual invocation, at most 1,024 registered/queued
listeners, 1 MiB of conservatively charged queued data and 4,096 waits per
invocation. Combined results retain at most 4,096 entries/128 resources, 1 MiB
of raw content and 2 MiB of wire JSON; individual-edit wire charges include
wrapper overhead. Overflow rejects the complete conversion without truncation.

Fifteen new engine controls and two version-converter controls cover these
contracts, including runtime-owned subscription cleanup. They are isolated Node
behavior tests, not evidence of native pre-operation participation. Native
integration still requires original request/model/settings proofs, real timeout
and cancellation handling, preview/confirmation with remembered preference,
contributed-edit application before file effects, and public history/event order.

With this foundation and the separate workspace-symbol monotonic-deadline fix,
all 772 host tests and all 1,555 Rust all-target tests pass (67 reports; 1,206
library tests; 23 ignored cases provide no qualification). The completion-event
Explorer terminal journey is separately rechecked against the rebuilt executable.

Will-operation participants/contributed WorkspaceEdits, copy/multi-operation
public backends, mixed file/text/notebook apply, public file Undo/Redo, precise
lifetime/event races and broader package/platform qualification remain open.
The resource-edit gateway stays disabled; native completion events are usable.
