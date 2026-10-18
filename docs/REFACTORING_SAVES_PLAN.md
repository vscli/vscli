# Refactoring autosave integration plan

Status: native continuation, private owner/cohort save authority, hidden LSP
participants, negotiated public acknowledgement and owned Save As transitions are
implemented. **719 host tests and 1,389 Rust all-target tests pass across 67
reports**, including 1,040 library tests; 23 ignored cases grant no qualification.
The 20 focused native controls, all seven selected-extension cases and all seven
new PTY journeys pass. Fresh build, global format and strict locked all-target
Clippy pass. Fresh existing rename, direct-save and native save-participant
terminal regressions also pass, for 34 terminal journeys combined. This is an intermediate milestone, not completion of
the extension API goal.

The implementation contract is [REFACTORING_SAVES_API.md](REFACTORING_SAVES_API.md).
CI investigation/repair remains deferred. No local VS Code/Electron process is
launched.

## Pinned behavior

VS Code 1.141.0 commit `2a59476c9bfcb90b3ddc372c36762471b7dfad1c`,
`bulkEditService.ts` `apply/_saveAll` and `bulkTextEdits.ts` `apply`:

- Respect autosave only when the operation opts into refactoring, the root
  `files.refactoring.autoSave` setting is exactly `true` (default true), and the
  performed resource array length exceeds one. The count precedes set dedup.
- Save dirty working copies involved in the performed operation. A dirty no-op
  peer in a multi-model text operation remains involved; unrelated dirty copies
  must not be saved. Single-resource refactoring does not automatically save.
- Wait for all attempted saves. Rejected saves warn; already-applied edits remain
  applied, and successful writes remain committed. Do not roll back editing or
  report `applied:false` for a post-publication persistence failure.
- Native save participants, actual worker authorization/Finished/join receipts,
  baseline/alias integrity and Save As are required; an ad hoc file writer or a
  Boolean manufactured at enqueue time does not implement this behavior.

## Architecture

`workspace_transactions` retains its five-second original preparation/queue
lease until atomic model publication. At publication, a bounded native save
cohort retains the actual transaction token, original origin, requested-URI to
actual-document receipts and lightweight proofs for every involved dirty model.
Source proofs are captured together before future queue waits; subsequent edits,
edit+Undo, text/language epoch ABA, path/baseline changes or settings/profile
replacement cannot silently
retarget a deferred save. No queued Rope is retained just to save a later version.

The cohort submits one original model at a time to existing document save
interests, using an explicit private native caller authority (no fabricated
extension session/owner). It checks the original owner retirement flag and a
separate cohort cancellation flag at actual worker authorization. It respects
existing eight-interest and one-worker bounds, save participants, normal human
Save As, original deferred proofs, and queued/actual receipt ownership. Generic
extension cleanup and completion acknowledgement cannot steal native cohort
interests. An already-authorized disk save still publishes after retirement.

Workspace capacity stays occupied until all saves settle and the terminal
cohort receipt is consumed. Publication ends the preparation deadline; a later
save timeout/cancellation does not become `applied:false`. F2 source validation
must distinguish the operation's own published edits from a stale provider
reply. Ordinary UI retirement after publication must not revoke already-owned
persistence or clear the cohort's Save As prompt.

The optional host negotiates refactoring-save support. It lifts its existing
whole-request gate only for that capability and retains exact native request
capacity through the save/Save As acknowledgement phase. Failed or malformed
terminal frames cannot release original physical work. Mirror/change/save/URI
transitions precede successful public acknowledgement.

Native LSP save participants must synchronize and target hidden involved models
without showing or cloning them. Existing visible-source behavior remains qualified. Four hidden/retained framed
LSP cases pass in the full suite, and the three hidden participant cases also
pass in a focused run; these are outside the 20-case refactoring filter. Complete extension save participants, all URI schemes, resource/file
journals, snippets/EOL/annotation/notebook apply models and broader platform
behavior remain obligations of the full goal.

## Qualification required

Actual native tests: default/true/false/non-Boolean root policy; single-resource
no-save; dirty/no-op/unopened/shared/hidden peers; unrelated dirty preservation;
all-target publication before save; saved bytes/baselines and grouped Undo/Redo;
actual conflict with committed peer preservation; deferred edit+Undo and settings
ABA; native/extension owner retirement before/after authorization; no receipt
stealing/replay; native Save As accept/cancel; capacity retention, shutdown joining
and save-participant behavior including a hidden target.

Actual selected-host tests: public default refactoring apply and original F2,
mirror/didSave before awaited success, default and disabled settings, unsupported
mixed whole refusal, Save As transitions, partial save failure without rollback,
original callback/worker capacity and pristine unrelated disks.

Real terminal journeys: public default multi-resource refactoring and F2, real
Save As/Cancel, Unicode CRLF saves, original Undo/Redo and terminal restoration.
Re-run relevant save/workspace/rename regressions, full host and Rust all-target
suites, fresh build, cargo fmt --all --check and strict all-target locked Clippy.
Source review alone, authored tests and plan entries do not count as execution.

## Current execution ledger

- Implemented: original publication proofs, private native authority with two
  lifetime flags, bounded sequential existing-save admission, exact receipt
  ownership, post-publication warnings/partial persistence, original F2 completion,
  negotiated public acknowledgement and actual Save As URI transitions.
- Executed: 719 optional-host tests and 1,389 Rust all-target tests across 67
  reports, including 1,040 library tests and the 20 focused native controls.
  Ignored Rust cases (23) grant no qualification. Four hidden/retained framed LSP
  controls pass in the full suite, with three hidden participant cases also run
  focused; they are outside the 20-case filter. Fresh build, global formatting
  and strict locked all-target Clippy pass.
- Executed selected API/terminal controls: all seven cases in
  `tests/extension_refactoring_saves.rs` and all seven journeys in
  `tests/extension_refactoring_saves_pty.py` pass with the selected synthetic
  `tests/fixtures/refactoring-save-extension` fixture. The controls use actual
  native persistence and final awaited receipts, including Save As and conflicts.
- Executed terminal regressions: rename (6), single-resource refactoring (3),
  public workspace edits (1), installed clangd rename (1), native formatting (5),
  native save actions (7) and public document saves (4), alongside the seven new
  refactoring journeys: 34 passes combined. Named scripts and controls are listed
  in the implementation contract.
- Pending: fresh platform/package qualification and remaining rich apply,
  extension-save and broader upstream behavior backends.

Earlier rename/direct-save qualification remains historical evidence. No new
coverage percentage, complete parity or platform pass is inferred from these
implementation and local-test results.

The 30-second save queue clock governs the next-admission phase and resets after
an actual member receipt. Repeated polls do not renew it. The original five-second
text preparation deadline ends at publication; post-publication save failures
retain applied true and their actual committed outcomes.
