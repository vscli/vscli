# Ordered native resource edits

Status: the resource-operation API is still unavailable. The existing public
gateway refuses unsupported mixed resource edits as a whole. The last usable
milestone is native multi-resource text refactoring with save continuations.
This work extends that gateway; it does not replace the full extension API goal.
CI work remains deferred and local VS Code instances remain prohibited.

## Source contracts

The reference is VS Code 1.141.0 immutable commit
`2a59476c9bfcb90b3ddc372c36762471b7dfad1c`.

- Execution preserves contiguous file/text/notebook groups; file groups further
  batch contiguous operation kinds. Text resource grouping must not cross those
  boundaries. This differs from the public query reconstruction's files-first
  arrangement. See [bulk execution](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/contrib/bulkEdit/browser/bulkEditService.ts)
  and [conversion](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTypeConverters.ts).
- Existing destinations are ignored only when overwrite is omitted and
  ignoreIfExists is truthy. Preserve explicit false. Binary contents and absent
  contents differ. Preserve folder, recursive, copy, skipTrashBin and maxSize.
- Create batches run folders before files. Delete inverse contents come from
  disk. Overwrite inverses do not restore displaced destination bytes. Public
  file-history inverses run forward within an element; outer group Undo runs
  newest elements first. Partial effects can survive failure. These are source
  contracts, not proof of an atomic filesystem transaction. See
  [file operations and history](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/contrib/bulkEdit/browser/bulkFileEdits.ts).
- Move preserves a loaded dirty source snapshot at its destination; dirty
  destination snapshots are not similarly retained by upstream managers. Our
  repository requires unsaved work preservation, so displaced/deleted native
  models need a private recovery archive. See
  [working-copy operations](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/services/workingCopy/common/workingCopyFileService.ts)
  and [text-file manager](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/services/textfile/common/textFileEditorModelManager.ts).

## Native design and implementation stages

1. The source stream planner preserves contiguous file/text/notebook groups,
   file-kind batches and original option/content/version distinctions. It borrows
   a bounded private DTO; it performs no filesystem work or editor mutation.
   A bounded filesystem worker retains the original admission deadline and
   captured request. Preparation happens in that worker; an exact native
   controller validates the original caller/model/settings proofs before
   authorizing effects. Cancellation before authorization preserves resources.
   Authorized work settles even after caller retirement. Prepared/Finished
   channels, one actual slot, exit and join precede its terminal receipt.
2. Capture and stage original resources, canonical parents, filesystem identities,
   bounded binary trees and dirty model archives. Persist preimages and the
   original operation stream before destructive effects. Keep the journal outside
   every operated subtree, including aliases and descendants. Recovery inspection
   verifies bounded schema, payloads and authority before exposing originals.
3. Add actual ordered effects and durable per-step postimage checkpoints. Preserve
   partial execution receipts; a failure does not manufacture rollback or prove
   absence of effects. Track physical operations rather than rewriting a net
   final filesystem image. Internal recovery uses dependency-correct inverse
   records; public history separately follows pinned source operation grouping.
4. Integrate original native model proofs and private displaced/deleted archives,
   shared panes, text history, path/language/baseline changes and recovery. Perform
   ordered text/file groups without stale asynchronous text publication. Publish
   genuine URI close/open, editor, watcher and LSP transitions before success.
   Destination reuse and API-reference provenance are conditional, not a universal
   source-close/new-target-open rule. See [model lifetime contracts](RESOURCE_MODEL_LIFETIMES.md).
5. Negotiate the optional host gateway, retaining original version suppression,
   caller/configuration proofs, options and receipt shape. Extend original F2/LSP
   resource edits through the same controller. Count performed resource arrays
   before autosave deduplication; preserve the existing save continuation.
6. Qualify mixed create→edit→rename→edit→delete workflows, dirty/shared/hidden
   identity, overwritten/deleted archives, dependent histories, raw binary bytes,
   directories/trash, aliases/parent ABA, failed/canceled/partial work, recovery
   after interruption and real terminal input. Arbitrary source options, custom
   filesystems and full platform fidelity remain obligations, not silent no-ops.

## Current evidence

The native API-reference ledger now passes seventeen behavioral controls for
original customer/model/resource identity, independent expiry and eviction,
move/delete removal, exact untitled revert and teardown. It returns reference
release evidence independently of model ownership. Twenty App controls and two
checked-reload integrity controls qualify acquisition, last-tab/preview retention,
expiry, customer teardown and retained-file Discard, including three actual hosts.
Resource-operation model effects and exact disposal/history archival remain
unfinished. Matching public file cache hits now bypass the native open bridge,
with eight host regressions and an actual eight-busy-slot terminal check. The
combined current source passes 1,550 Rust tests (1,201 library; 23 ignored), all
748 host tests, two native document PTY journeys, formatting and strict locked
all-target Clippy. The document PTY also passes clean tab close/reopen and dirty
Discard/reopen/Undo with the same public document. These additions do not expose
the public resource gateway.
Three additional controls qualify customer-owned file load admission, original resource/profile proofs and actual cross-extension sharing. File misses now share one native read per URI; provider-resource cached opens validate their original private mirror before argument getters. Generated provider/untitled misses now have one-use native admission before callbacks/creation; public resource effects remain unfinished.
See [reference contracts and integration points](../src/document_references/README.md).

The native model-to-journal boundary now uses
[ResourceModelSource](../src/document/resource/README.md), captured from the actual
Document. Private history head identity and source clocks reject stale native
authorization; worker encoding preserves original dirty bytes and the disk
baseline under the original deadline and shared 32 MiB/8 MiB bounds. Eight new
integrity tests and production-source substitutions in the existing worker and
physical overwrite/rollback controls qualify this boundary. Full native history
archives, destination restoration and public resource admission remain open.

Native destination restoration now has a worker-prepared kernel. Surviving
targets retain identity/history; absent targets get fresh identities. Dirty
snapshots, actual clean disk postimages, equal-text dirty state, explicit/inferred
language and ordinary/grouped EOL history have eleven restoration and two history
controls. One couples actual Linux overwrite move, displaced dirty archives and
physical rollback with restoration into the surviving destination. App must
still select actual survival, archive complete retired history/views, apply
ordered groups and publish events; the public resource gateway remains unavailable.

The [App model coordinator](../src/app/resource_models/README.md) now connects
File Revert to the actual resource worker and restoration kernel. It binds
confirmation to original model/resource identity, keeps IO/diff work off input/
rendering, validates native source/settings proofs before authorization and after
join, and preserves shared model/history identity while publishing the new disk
baseline. Pending reloads exclude saves and retain actual capacity through
retirement. Nine App controls and one large shared-backing control qualify this
connection. Bulk file effects, destination survival, operation events, complete
retired archives and mixed-group/public-history integration remain unfinished.

The Linux resource executor now performs actual create, rename, copy and delete
operations with durable intent/postimage checkpoints, explicit partial outcomes
and guarded reverse physical rollback. Create batches run folders first;
rename/copy ignore decisions and source/overwrite preflight use the original
contiguous kind batch before effects. This preserves batch refusal and prefix
failure behavior rather than silently treating later collisions as skips.
Twenty-six new filesystem tests pass, alongside the prior 23 journal controls.
Cold recovery checks the complete record chain and owned filesystem state;
unmatched forward/rollback intents remain ambiguous and refuse automatic replay.
Internal rollback preserves displaced/deleted bytes and reverses dependent owned
steps. Public Undo/Redo, trash, cross-volume execution, custom providers, non-Linux
backends, model disposal/reuse and editor publication remain unfinished.

`src/workspace_resource_edits.rs` adapts original file groups on the worker.
`prepare_groups` revives all file descriptors before effects, enforces a whole-stream
1 MiB file/scalar-text raw budget, preserves original indices/annotations/options,
and retains original kind batches including skipped untitled-create barriers.
Absent/empty/raw binary contents stay distinct. Unknown options and nonnumeric
maxSize retain their original values while applying source-ignored semantics;
missing data-transfer resolvers reject even in later or skipped/non-create rows.
Ten adapter tests pass, including actual DTO→journal→filesystem execution on the
joined worker after exact authorization. Postauthorization owner retirement keeps
the real committed receipt until thread exit/join. This is a backend coupling
control: it does not authorize App models, confirm annotations, apply notebook
cells or publish the resource-operation extension API.

The combined final Rust suite passes 1,471 tests across 67 reports (1,122 library;
23 ignored), with global formatting and strict locked all-target Clippy passing.
There are 36 new executor/adapter regressions. No new resource-operation terminal
or platform qualification is claimed; previous native path/save/rename terminal
evidence below retains its original milestone scope. No local VS Code instance
was launched and CI work remains deferred.

Native documents now retain an independent resource epoch across live Explorer
rename/trash and Save As publications. Returning to the same path does not renew
original save/workspace, LSP/provider, editor-service or disk-refresh proofs.
Seven new regressions execute actual rename workers and held process/disk replies,
preserve dirty Unicode CRLF text, history and baselines, and check exhaustion
before file creation/overwrite. The full Rust suite passes 1,435 tests across
67 reports (1,086 library; 23 ignored), with global formatting and strict locked
all-target Clippy passing. A new native terminal journey verifies F2 rename
away/back, original Undo/Redo/Save, committed disk baselines and terminal
restoration with missing Node. This qualifies resource guards on existing native
workflows; it does not qualify extension resource execution or model reuse.
Eighteen existing terminal journeys also pass again: six extension/native rename,
seven refactoring-save, four public document-save and one installed clangd 23.1.1
rename journey. Together with the new native path journey, all nineteen verify
their existing identity/history/save contracts on the resource-guard change.

The worker is implemented in `src/resource_worker.rs` and its eight focused tests
have executed successfully against real filesystem writes. They cover explicit
authorization, wrong/replayed identity refusal, original deadline expiry, external
write conflicts, cancellation during preparation, preauthorization retirement,
postauthorization retirement, failure after a real effect, held Finished capacity
and shutdown joining. All eight cases pass again against final worker sources.
Formatting and strict locked all-target Clippy also pass. These results qualify
the worker protocol; execution evidence is recorded separately above.

The journal foundation is implemented in `src/resource_journal.rs`. All 23 focused
integrity tests pass: binary/tree/displaced/dirty preimages, exact option presence,
durable staging and complete recovery inspection, foreign replacements, parent
identity/alias ABA, missing-prefix bounds, payload/schema corruption and late
malformed records. Two cases use actual native document snapshots on the joined
worker and preserve original COW text after edit→Undo and cancellation.

Capture/staging remain NotStarted foundations; a separate execution header and
ledger represent authorized effects and guarded recovery. Public resource API,
operation Undo/Redo and App model/event integration are still unavailable. No new
resource-operation terminal qualification is claimed. Payload symlinks, special/reparse entries and non-UTF-8
paths remain explicit foundation refusals. No native resource capability is
advertised to extensions. The full API goal remains active; this foundation grants
no new API-name or provider-family count. The full Rust all-target suite passes
1,428 tests across 67 reports, including 1,079 library tests; 23 ignored cases
grant no qualification. Fresh build, global formatting and strict locked
all-target Clippy pass. There is no new terminal interaction to qualify until the
executor/controller is connected; earlier usable refactoring-save evidence stays
separately recorded in [its contract](REFACTORING_SAVES_API.md).

The independent planner is implemented in `src/workspace_edit_stream.rs`. Eight
native parser tests pass, covering create→edit→rename→edit→delete boundaries,
contiguous file batches, absent/false/null overwrite, unknown option retention,
binary/empty/data-transfer contents, create-driven text-version suppression,
notebook/annotation boundaries, malformed late records and aggregate bounds.
Formatting and strict locked all-target Clippy pass. These qualify parsing and
planning only; text/file/notebook execution still needs its actual backends.
