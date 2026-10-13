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
- Matching public file and explicit untitled mirrors return without a native
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

## Remaining qualification

Concurrent misses still need a shared customer-owned loader, independent of the
initiating extension's cancellation. Encoding reload remains UTF-8 only. The
future ordered resource controller must remove genuine completed MOVE/DELETE
source references, distinguish overwrite's internal destination deletion, and
publish surviving-target reuse and model events. CREATE/COPY remove no references.
These resource hooks are implemented in the ledger, but their public native
operation controller remains unfinished.

History and global pending-work protection currently postpone native model and
public-mirror disposal. Exact upstream disposal/history archival and per-model
resolve ownership, cached provider-resource behavior, replacement races and
platform timing remain unqualified. This work does not expose the public
resource-operation gateway or establish full extension API compatibility.
