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
the worker protocol, not resource editing or the future journal engine.

The journal foundation is implemented in `src/resource_journal.rs`. All 23 focused
integrity tests pass: binary/tree/displaced/dirty preimages, exact option presence,
durable staging and complete recovery inspection, foreign replacements, parent
identity/alias ABA, missing-prefix bounds, payload/schema corruption and late
malformed records. Two cases use actual native document snapshots on the joined
worker and preserve original COW text after edit→Undo and cancellation.

Capture/staging/recovery inspection are not an execute engine or public workspace
integration. Every successful journal still reports NotStarted. No actual resource
API, operation Undo/Redo, recovery after destructive execution or new terminal
qualification is claimed. Payload symlinks, special/reparse entries and non-UTF-8
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
