# Extension caller ownership

The optional compatibility host keeps extension caller ownership separate from
native command correlation. A command registered by extension B runs as B when
extension A invokes it. The original native command still supplies its own
request ID and owner for correlation. Completing that command does not remove
the caller identity from promises and timers created by the callback.

Caller scope is private runtime state. Extensions cannot set it through the
`vscode` facade. Activation and module evaluation, registered command and
provider callbacks, and registered event listeners enter the selected extension's
scope. Shared document objects and public value constructors keep their identity;
caller attribution does not require cloning them. Provider adapters preserve the
original provider as `this`, including frozen providers.

Each runtime has distinct owner leases. Disabling an extension retires its lease;
re-enabling the same extension ID creates a fresh lease. Old scheduled work must
not acquire the new lease through a cached facade or a newly registered command.
Retired event and cancellation listeners are inert. Raw disposable cleanup still
runs after retirement.

Provider return values remain extension-owned code: a Thenable's `then` getter,
its resolution callback, and getters on returned items need the same caller
scope as the provider. Runtime cancellation and actual-work completion are
separate. A canceled invocation can return a cancellation error while its
callback is still pending; it retains the occupied slot until the actual callback
settles. Owner retirement cannot suppress runtime release notifications.

## Qualification

The corrected implementation passes 26 focused tests and all 319 local host
tests. Three returned-value regressions also fail against the unfixed code,
establishing that they exercise the corrected behavior. Tests cover stale timer
requests, queued disk-write refusal, cross-extension delegation, Thenable and
metadata getters, original receivers, and cancellation with an occupied actual
callback and a later release notification. All 134 native extension-related
tests pass with the corrected module embedded; ignored fixture cases do not
grant additional qualification. Build, formatting and strict all-target Clippy
pass. Actual native terminal workflows pass for seven language-provider actions,
output/status/tree interactions and disposal, filesystem disk receipts, shared
hidden documents, delegated Undo/Redo, host crashes, recovery and persistent
Mementos. Unicode/CRLF bytes and prior history remain intact in those workflows.
These are local host/native observations, not a running upstream comparison or
qualification of every callback family and platform.

Custom filesystem stat/directory results are snapshotted as primitive metadata
under the original provider lease. Malformed coercion objects are rejected before
copy writes. Directory snapshots have explicit native limits of 65,536 entries
and 8 MiB; oversized strings are refused before UTF-8 measurement. These limits
and retired-listener behavior are native adaptations, separate from upstream
runtime qualification.

Caller ownership is a prerequisite for shared-document request methods. Native
[`TextDocument.save`](NATIVE_DOCUMENT_SAVE.md) now uses actual joined persistence
receipts and shared mirror publication; caller context alone does not establish
save success. The native Rust editor continues to work without Node.

Native queued services also capture a selected owner's private retirement flag
at receipt. Failed/disposed activation retires that flag irreversibly; explicit
reactivation receives a different flag. A queued request cannot regain authority
merely because its owner ID becomes active again. Dropping the native client
retires all remaining flags, including those retained outside its queue. This
does not cancel an already authorized physical disk commit or release an
occupied worker early. The existing session and command-origin checks still
apply independently.
Admitting additional packages into a running selected cohort creates lifetimes
only for the new owners, after activation request admission succeeds. It preserves
existing and retired flags. A real activation regression opens a hidden Unicode/
CRLF document through the added extension, awaits its native save and verifies
the actual saved generation and exact file bytes without opening an editor.

Three native regressions and the complete 831-test native library suite pass:
owner failure/reactivation and isolation, registry/client-drop retirement, and a
real optional-host Client whose queued save remains rejected after reactivation
while a newly admitted save receives fresh authority. These do not establish
individual in-process hot unloading, broader extension shutdown parity, or
upstream/platform qualification.
Formatting, strict locked all-target Clippy and the native build pass. The four
public-save terminal workflows pass with these native lifetime checks enabled,
preserving exact Unicode/CRLF disk bytes, shared identity, Undo/Redo and terminal
restoration.
