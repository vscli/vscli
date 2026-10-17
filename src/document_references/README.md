# Native API document references

The ledger is now wired into native App acquisition, retention, expiry and
customer teardown for file, untitled and readonly virtual documents. Reference
release remains separate from editor, dirty/history and pending-worker ownership.
Seventeen ledger tests, twenty App tests (including three real selected-host
sessions), and two checked-reload integrity tests pass. The combined source passes
1,510 Rust tests across 67 reports (1,161 library; 23 ignored), all 727 host tests,
formatting and strict locked all-target Clippy. Terminal evidence is recorded
below and in the API ledger; this is not full model/resource-operation parity.
All twenty-one selected document/tab/untitled/virtual/preview/sticky terminal
journeys and a fresh native build pass. Broader platform behavior is unqualified.
The ledger and native ownership code need no JavaScript process, timer or thread;
optional real-extension tests run the isolated host. Explicit retained-file
Discard uses one joined native background reader.

The source pin is VS Code 1.141.0,
`2a59476c9bfcb90b3ddc372c36762471b7dfad1c`. The collection and acquisition paths
are in [MainThreadDocuments](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/browser/mainThreadDocuments.ts).
Cache and concurrent-loader behavior comes from
[ExtHostDocuments.ensureDocumentData](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostDocuments.ts).
The resource comparison inputs follow
[ExtUri](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/base/common/resources.ts).
The source was read through the immutable GitHub contents API; source-derived
contracts do not qualify actual editor/reference timing.

## Authority and evidence

`References::new(customer_epoch)` belongs to one native extension-host customer,
shared across its extensions. A genuine acquisition calls `acquire` with the
original model ID/resource epoch, original acquisition Instant, trusted resource
identity, and captured text-model getValueLength units. The length includes
model UTF-16/EOL units; it is neither UTF-8 bytes nor current memory consumption.
Capture it once, at successful native acquisition, rather than after didOpen
callbacks or on later polls. Each acquisition has its own checked monotonic ID
and three-minute deadline. Duplicate resource acquisitions remain independent.

Length cleanup releases oldest acquisitions while the captured sum exceeds
80 * 1024 * 1024 units. Count cleanup runs afterward: reaching 60 releases ten
oldest entries to 50; 51 through 59 are valid. An oversized acquisition can evict
every older acquisition and itself. At most 59 entries remain after a call.
Resource strings/components retain at most 128 KiB per entry; returned evidence
is bounded by the original collection plus one admitted entry.
This receipt budget accommodates the existing 16 KiB string-path admission after
URI escaping without reducing that gateway's accepted paths. It does not enlarge
the public gateway's URI/input limits. A trusted provider resource may have an
empty path, such as `preview:?revision=1`; its nonempty original URI, scheme and
query/fragment namespace still distinguish identity.

`ResourceKey::new` preserves the public URI as receipt evidence and accepts
caller-normalized identity components. The caller must use the actual native
URI/provider authority to normalize aliases, case and UNC semantics, with slash
path boundaries. This module performs no filesystem resolution or speculative
platform folding. Scheme, normalized authority, query and fragment namespaces
must match before path ancestry is considered.

`cached_open` is a read-only observation: it neither acquires nor renews a lease.
Regular native polling calls `expire_due`; `next_expiry` permits scheduling without
a global timer. An individual timer can use `expire_acquisition` with its original
ID. A stale timer cannot release another acquisition, customer or resource epoch.
Expiry scans actual deadlines independently of insertion order.

Completed working-copy moves call `remove_move_source` for source resources;
completed working-copy deletes call `remove_deleted_resource` for targets.
Each removes equality and descendants, preserving destination references and
same-prefix siblings. CREATE, COPY, raw watcher notifications and an overwrite's
internal file-service deletion are not removal events for this ledger.
`untitled_revert` takes the original model key and exact untitled resource;
it does not remove descendants or a replacement binding at the same URI.
This added native epoch check prevents an old callback from retiring newer work.

`retire_customer` drains an owned snapshot. `replace_customer` requires a strictly
newer epoch and preserves the acquisition sequence. Neither ordinary extension
owner unload nor routine request/input cancellation is customer teardown.
Old-customer acquisition, expiry-poll and removal requests cannot affect the
replacement; already stale individual timers simply return no releases.

Every Decision carries released original Acquisitions and a complete retained
ownership summary grouped by original ModelKey. These receipts authorize only
API-reference release. Before changing model storage, the caller must separately
arbitrate dirty text/history, editor memberships, pending saves/resolves/workers,
and any other owners. Compare exact model ID/resource epoch before consuming
release evidence; path equality alone cannot authorize retirement after ABA.
The pinned resolver likewise waits on dirty/pending ownership rather than
equating last API-reference expiry with model destruction:
[resolver disposal](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/services/textmodelResolver/common/textModelResolverService.ts).

## App and host wiring

- Native file Open acquires the actual accepted model after path resolution or
  an existing-model result, before mirror publication and extension callbacks.
  Successful untitled and virtual opens also acquire; virtual refresh does not.
  Immutable acquisition length uses logical UTF-16/EOL units. A bounded private
  length cache reuses only the same model/resource/text/EOL proof for later
  genuine acquisitions; older reference lengths never change.
- Matching public resource mirrors return before provider/backend routing, without a native
  request or renewal. Showing/promoting a hidden model likewise acquires nothing.
  All extensions in a session share the customer collection; unloading one owner
  or routine input/request cancellation does not retire it.
- Last-tab close, preview replacement and fallback removal retain a referenced
  Document privately, preserving native ID, resource epoch, text and history.
  Storage is reserved before changing memberships. Native-only preview opening
  reserves no API retention storage when no referenced preview is replaced.
- Native Open still admits at most 16 hidden models. Moving an existing visible
  model into hidden storage does not increase the existing total 128-model/4 MiB
  mirror admission; these bounds are separate from the 59-reference policy.
- App polling releases exact original deadlines, then removes only eligible
  API-managed hidden models. Editor memberships, dirty text, native history,
  queued services, saves/resolve workers and real pending Discard protect storage.
  Unrelated native hidden models are not adopted into this collection. Old
  resource-epoch evidence cannot release a replacement binding.
- Host stop, crash and session replacement drain that customer's references.
  Teardown does not discard protected native buffers. Quit/CloseAll and recovery
  retain their Save/Discard/Cancel flow independent of host availability.
- Explicit Discard of a retained file reads the current disk on a bounded joined
  worker, preserving its original deadline, model/text/resource/history/baseline,
  settings and membership proof. Only a current reply reverts and closes the
  selected tab. Missing/invalid files or changed proofs leave original work;
  accepted reload keeps prior text in native Undo without writing disk bytes.
  Automatic/refactoring saves wait while that reader is occupied. Untitled
  Discard releases only its exact original model/resource references.

Twenty App controls qualify exact expiry boundaries, editor retention, genuine
acquisition/unchanged cache deadlines, unrelated/dirty/history ownership,
customer replacement, source-path ABA, preview replacement, captured Unicode/EOL
length, long escaped native paths, late/refused Discard, independent peer work,
public close/readable-old-document publication and provider refresh without
renewal. Three use actual selected Node-host sessions. Offset clocks qualify the
native deadline boundary; no three-minute wall-clock/upstream race claim is made.
The strengthened native document PTY passes clean tab close/reopen, dirty
Discard/reopen/Undo, eight busy slots, exact public shared identity, host crash,
recovery and save. Broader terminal evidence appears in `docs/EXTENSION_API.md`.

## Customer-owned file loading

Negotiated file cache misses share one runtime-private load by original URI across
extensions, with eight actual lanes and 128 waiting callers. The native admission
validates the original selected caller and command; accepted read ownership then
belongs to that session. Every waiter keeps its own original caller/mirror guard.
Retirement/replacement of the initiating caller cannot authorize its old promise
or poison a separate live caller. Legacy non-negotiated opens retain their old
caller-bound behavior. This authority grants no native edit/save permission.

Native read context preserves the original workspace/settings/profile and
existing resource binding. Unrelated model/input changes do not cancel a read;
resource-path ABA and profile replacement refuse the old binding. Independently
loaded URIs can publish sequentially without invalidating each other's inventory.
Actual reads and native state workers join before their slot/receipt is released.
A timeout retains the original shared entry and wire capacity until an exact
typed terminal reply; refused pre-admission work leaves no permanent URI entry.

Seven sharing controls, five resource-cache controls and one wire-retention
control pass; all 740 host tests pass. Three new Rust controls cover native
admission/session/deadline, resource/profile proofs and actual cross-extension
coalescing with one acquisition per URI. All 1,513 Rust tests pass (1,164 library;
23 ignored), formatting and strict locked Clippy. The two document and one
provider terminal journeys pass, including concurrent file opening and cached
provider access after disposal. These are distinct from the earlier twenty-one
retention journeys; no new resource-operation/platform qualification is claimed.


## Generated-document admission

Native admission now precedes provider callbacks and untitled creation. Each
one-use customer ticket captures its original URI/kind, selected caller, session,
workspace/settings/profile and absolute 60-second deadline. Eight outstanding
tickets bound native metadata; issuance and consumption never renew that clock.
Completion validates the exact ticket/resource/kind without reacquiring a retired
consumer's lease. Provider registration/creator authority remains independently
checked, including after native publication before a retained acknowledgement.
Caller retirement rejects only that caller's promise; provider disposal still
refuses its work. Unused tickets cancel without creating a model.

Virtual misses share one callback/native publication by URI, with eight actual
callback lanes and 128 consumers. Explicit untitled URI misses share creation;
noarg/options creations remain distinct. Publication timeout retains the original
shared entry and occupied slot until its typed terminal reply. Begin/cancel wire
receipts likewise retain capacity and reject malformed responses. Legacy hosts
without negotiation keep their previous caller-bound path. Core Rust ownership
and admission require no JavaScript runtime.

Five native ticket tests cover forged/replayed ownership, issuance, exact resource/
kind, distinct creation, bounds and original deadline. A real three-package host
case proves a failed lazy activator cannot poison the other consumer's document
or trigger a second provider call. Six host workflow controls and two wire
controls pass. All 748 host tests and 1,519 Rust tests pass (1,170 library;
23 ignored), with four native terminal journeys: three untitled and one provider.
An unrelated theme timeout in the default parallel Rust run passed alone; the
complete run passes with four test threads. Fmt, strict locked Clippy and build
pass. This qualifies these named workflows, not full extension compatibility,
resource-edit model publication or other-platform/upstream timing.

## Remaining qualification

Negotiated virtual and untitled misses now have original customer admission before callbacks/creation. File misses share their actual native read. Broader loader races and backend fidelity remain unqualified. Encoding reload remains UTF-8 only. The
future ordered resource controller must remove genuine completed MOVE/DELETE
source references, distinguish overwrite's internal destination deletion, and
publish surviving-target reuse and model events. CREATE/COPY remove no references.
These resource hooks are implemented in the ledger, but their public native
operation controller remains unfinished.

History and global pending-work protection currently postpone native model and
public-mirror disposal. Exact upstream disposal/history archival and per-model
resolve ownership, broader replacement races and
platform timing remain unqualified. This work does not expose the public
resource-operation gateway or establish full extension API compatibility.
