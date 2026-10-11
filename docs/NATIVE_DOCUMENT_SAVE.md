# Extension document saves

Public `TextDocument.save()` now routes the retained shared document through the
native save controller. It captures the invoking extension's private caller
lease, document ID and exposed version synchronously. The facade that first
opened the document does not own later callers. Callback ownership and the
originating command are independent; retired callers cannot reuse a replacement
lease or receive successful acknowledgement from an old invocation.
Queued native services also retain their original
[selected-owner lifetime](EXTENSION_CALLERS.md): failure followed by same-ID
reactivation cannot renew an earlier save's authority. Already authorized native
persistence still retains its worker and publishes the genuine receipt.

The internal `nativeDocumentSave` service connects one retained native document
ID/version to the existing asynchronous save controller. Admission captures the
arrival mirror and native path, text epoch, revision, saved baseline, EOL,
settings/profile and workspace ownership. It does not focus or promote an
inactive or hidden model. The eight-entry caller table retains completed
receipts until their exact service response enters the bounded transport FIFO.

The controller retains one actual persistence worker until its terminal result
and thread join. It publishes the original model's successful receipt before
synchronizing mirrors and answering true. Newer edits remain dirty. Retirement
before authorization rejects work; retirement after authorization preserves the
real commit and native publication. Backend ambiguity and committed-but-unpublished
outcomes reject explicitly rather than returning an invented false or true.

Untitled documents use the original native Save As prompt and document identity.
Path selection has a 300-second human-wait phase; acceptance starts the
30-second preauthorization phase before synchronous dispatch. Failure clears
only that exact admission, and a service-tagged intent/worker cannot run without
its matching caller interest. Existing manual/autosave/close/quit saves keep
ordinary native ownership and require no JavaScript runtime.

The promise returns true only after the actual worker finishes and joins, the
original model receives its receipt, and the same shared JavaScript document
receives a greater save generation. `onDidSaveTextDocument` precedes true.
Sequential clean saves also use native persistence, advance the generation and
preserve text version, selections and Undo/Redo. Native Save As cancellation
returns false; closed documents, external-write conflicts and ambiguous backend
outcomes reject. Newer text typed after authorization remains dirty even when
the earlier captured content was saved successfully.

Save As suspends only the exact originating command during its bounded human
wait. Repeated polls do not renew it; acceptance, cancellation or expiry resumes
that command's clock once. Timer/event callbacks can save without an originating
command. The public hook requires a live extension caller; core editing and
ordinary native saving remain usable without Node.

Save shares the eight-call document-service budget and the 64-request host
budget. A 360-second acknowledgement timeout rejects with an unknown outcome
while retaining occupied capacity until the exact terminal reply. Malformed
replies cannot release capacity, reset clocks or retry the operation. Native
response backpressure retains the original receipt. Host shutdown never undoes
an already authorized disk commit. Service FIFO can delay later extension
document operations; command/preauthorization deadlines can still end a caller
while an authorized physical worker remains occupied.

Virtual/custom filesystem saves, broader encodings and all upstream save
participants remain incomplete. Existing first-save participant limitations
remain; this is not complete save-lifecycle or extension API compatibility.

Local qualification: fourteen real-worker native ownership/integrity tests and
all 812 native library tests pass, with formatting, strict all-target locked
Clippy and a native build. Terminal save automation, format-on-save and
code-actions-on-save journeys pass, covering Unicode/CRLF, Undo/Redo, disk
restart, autosave, external conflicts, dirty close/quit, original keybinds,
late actual callbacks and terminal restoration. The ownership fixture bypasses
optional-host registration only; these tests do not establish public save API,
actual caller/session identity or didSave-before-promise compatibility.
A separate real-host transport regression fills the real 64-slot output queue,
retains the canceled Save As receipt across failed response admissions, then
observes exactly one false reply after release and a real ping. The same test
fails against the original removal-before-send implementation at the receipt
retention oracle. It also preserves the hidden Unicode/CRLF model, Undo/Redo,
active editor and disk bytes. This exercises the internal protocol; public
TextDocument.save and human command-wait behavior have separate tests.
No local VS Code instance or CI investigation was used.

Public gateway qualification: eleven host tests and all 360 host tests pass.
Three negative controls fail on the unfixed implementation: alternating identity
getters, retirement/reacquisition during metadata access, and retirement while a
true reply is held. A real two-extension framed host tests independent caller
and command origins, timer/event/provider callbacks, newer dirty publication,
repeated clean saves and didSave ordering. Its parent supplies synthetic native
state; physical disk evidence is separate. The native library suite passes 828
tests, including exact service human-wait and caller/origin matching tests.
Formatting, strict locked all-target Clippy and the native build pass.

Four actual native CLI PTY sessions pass with an absolute Node path, empty PATH
and no language server: repeated clean save/Redo, inactive A save while B is
dirty, untitled cancellation/acceptance/Undo, and external-conflict recovery plus
closed-document rejection. They verify exact Unicode/CRLF file bytes, shared
identity, didSave-before-true and restored terminal state; fixture JavaScript
calls the API and observes state, never writes document files. Existing shared
hidden-document/recovery/Memento and native save-automation terminal journeys
also pass. The inactive fixture waits for the post-acknowledgement shared
identity assertion and active-file status before typing.

[Source receipts](../extension-host/document-save.provenance.json) pin the
public delegation and closed-document behavior to stable VS Code 1.141.0,
commit `2a59476c9bfcb90b3ddc372c36762471b7dfad1c`. No local upstream instance
was run; broader upstream runtime and platform qualification remain outstanding.

Five additional real App/two-extension host regressions qualify Save As deadline
handling using deterministically aged actual native clocks. Caller B saves under
originating command A; A survives more than its ordinary 30-second command limit
without renewing the human wait, while an unrelated stalled B command still
times out. Escape and actual path acceptance reset only A's clock once. Aging
both the exact service wait and App Path interest beyond 300 seconds expires the
prompt, returns false and cannot renew it or write later. Acceptance verifies the
actual joined native worker, generation, Unicode/CRLF disk bytes and Undo/Redo.
Removing the native origin-protection guard makes the first regression fail at
the exact original-command timeout oracle. These tests do not represent 300
seconds of wall-clock runtime or additional terminal/platform qualification.
The combined native library suite passes 837 tests; formatting and strict locked
all-target Clippy pass.
