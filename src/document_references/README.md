# Native API document references

This standalone ledger implements acquisition provenance and bounded reference
ownership. It is not wired into App or the extension host, and does not dispose,
hide, reopen or synchronize documents. Seventeen focused behavioral tests pass,
including deadline boundaries, duplicate acquisition, length/count eviction,
original resource/model epochs, stale timers and complete customer teardown.
The standalone milestone at `0583730` passed 1,487 Rust tests across 67 reports (1,138 library;
23 ignored), formatting and strict locked all-target Clippy. These results
qualify the ledger, not actual editor retention or expiry.
No VS Code runtime, JavaScript process, timer or thread is needed by the module.

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

## Later App and host wiring

The following are inspected current integration points, not implemented behavior:

- `src/app/extension_services.rs` handles Open using an immediate retained-model
  response or bounded path resolution followed by `install_service_output`.
  Each genuine native open needs one acquisition of the actual accepted model,
  including Existing/raced-existing results. Capture the length, epoch and
  acquisition Instant before callbacks and release any refused reference.
  Existing native output is distinct from a public cached mirror hit.
- `extension-host/document-services.cjs` returns matching file and explicit
  untitled mirrors from the private cache. These paths
  must send no native open request and acquire no new reference. Encoding reload
  and concurrent misses require separate real customer-owned loader authority;
  initiating-extension request cancellation is not shared-customer ownership.
  Current native decoding remains UTF-8 only. Eight new cache regressions and all
  727 host tests pass. The two native document PTY journeys pass, including
  reopening a hidden file with all eight native request slots busy and subsequent
  Undo/Redo, host crash, dirty recovery and save; these do not wire this ledger.
- `src/app/panes.rs::close_tab_membership` currently removes the last visible
  Document. With API references, closing the last editor membership must retain
  the same Document privately. App's close/remove fallback paths need the same
  ownership arbitration. Opening/promoting an existing hidden document in
  `src/app.rs` must preserve its IDs, epochs and leases without acquiring a lease
  merely because an editor becomes visible.
- Native Open currently bounds hidden storage to 16 models, alongside the
  128-model/4 MiB mirrored-text limits. Those admission limits remain independent
  of this source collection's 59-reference/80 Mi-unit policy. Do not silently
  drop dirty or leased models to reconcile limits.
- App's normal poll loop should expire exact references and arbitrate releases.
  Host stop/replacement should drain only that customer's epoch; ordinary input
  calls `cancel_extension_services`, so that method is not a teardown signal.
  Extension management host replacement owns the appropriate lifetime boundary.
- The future ordered resource controller should publish genuine completed
  working-copy MOVE/DELETE removal only after accepted effects. It must distinguish
  explicit DELETE from overwrite's internal destination removal. An untitled
  revert callback captures its original ModelKey and URI. CREATE/COPY have no
  reference-removal hook.
- Current Quit/CloseAll promotes hidden dirty documents into Save/Discard/Cancel
  handling. Preserve this recovery behavior and native ownership independent of
  host availability. API release alone must never discard unsaved/history work.

Customer replacement, model close/open event ordering, shared concurrent loads,
encoding reloads, actual hidden-model expiry and destination reuse require
integration and framed/PTY qualification. This module claims neither those
behaviors nor full API/resource-operation parity.
