# Public workspace edits and grouped native history

Selected extensions can call `await vscode.workspace.applyEdit(edit)` with text
edits for retained native documents. One transaction can edit visible, inactive,
hidden and untitled models together. Native preparation runs on a bounded Rust
worker; all targets are validated and staged before any text changes. The complete
mirror is published before change events and the successful awaited Boolean.
Shared document identities survive the operation.

```javascript
const edit = new vscode.WorkspaceEdit();
edit.insert(first.uri, new vscode.Position(0, 0), 'first ');
edit.insert(second.uri, new vscode.Position(0, 0), 'second ');
const applied = await vscode.workspace.applyEdit(edit);
```

Open a local file with `workspace.openTextDocument` before including it. Opening
alone retains a hidden model without changing the active editor. Plain text edits
change native buffers, preserving their EOL convention; they do not write files.
Use `await document.save()` for [native persistence](NATIVE_DOCUMENT_SAVE.md).
Normal native autosave remains governed by its own settings.

Undo or Redo from any changed participant restores the entire cohort, including
hidden peers. Original platform shortcuts use this native history route. If a
peer has independent history or was closed, the entire grouped operation is
refused and retains the other work; undo the independent edit first or keep the
peer open. Confirmation dialogs and history splitting are not implemented.
Nonempty no-op text transactions return true without consuming Redo or adding a
history entry; an empty edit returns false.

Single-resource `{isRefactoring:true}` edits remain unsaved under default,
enabled and disabled refactoring autosave settings. This follows the pinned
[bulk-edit resource-count condition](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/contrib/bulkEdit/browser/bulkEditService.ts#L239-L244).
Multi-resource refactorings still require the root user/workspace setting
`"files.refactoring.autoSave": false` until the multi-resource save continuation
is implemented. Other policies refuse the whole request with an explicit notice;
language overrides do not change this root policy.

The policy correction passes all 677 host tests and 24 native workspace
transaction/gateway tests on an isolated archive of its parent, independently
of the concurrent workspace-symbol feature. Three new terminal journeys cover
default/true/false single-resource policy, exact mirror publication, prior dirty
Unicode CRLF work, native Undo/Redo and explicit Save. The original multi-resource
workspace-edit terminal journey also passes. Build, formatting and strict
all-target locked Clippy pass. Reproduce the new terminal checks with
`python3 tests/extension_single_refactoring_pty.py target/debug/vscli`.

## Admission and failure behavior

The native gateway captures target identity, URI, exposed version, source epoch,
saved baseline, history heads, settings/profile and workspace at receipt, before
FIFO waiting. Later dispatch uses that original capture. A target edit followed
by Undo cannot renew the proof. Navigation/focus changes alone do not invalidate
a text transaction. The selected native caller lifetime cannot be renewed by
reactivating the same extension ID.

Invalid UTF-16 ranges, split surrogate pairs, overlapping replacements, duplicate
target groups or changed targets refuse the whole transaction. Same-position
insertions preserve their original order. No supported prefix of a mixed
unsupported edit is applied. File operations, unopened resources, custom/virtual
filesystem documents, snippets, EOL edits, annotations/confirmation metadata and
notebook edits still need native backends.

Limits are 128 retained models/targets, 4,096 edits, 1 MiB raw replacement text,
2 MiB normalized replacement text/request framing and 4 MiB total mirrored model
text. View/history staging has an 8 MiB metadata bound. The shared service FIFO
holds eight requests within an 8 MiB payload budget. One actual preparation worker
has a five-second absolute deadline covering FIFO waiting and publication; an
exact cancellation retains its occupied slot until the thread exits and joins.
Terminal receipts remain retained until their exact transport response is
admitted or the original caller/session retires.

A rejected Promise or acknowledgement timeout does not prove rollback. If edits
were published but the mirror/transport acknowledgement failed, native edits stay
retained and the outcome is ambiguous to the caller. Do not retry automatically.
Malformed acknowledgements do not release occupied host request capacity.
Shutdown cancels and joins native preparation without detaching it or canceling
an independently authorized disk save.

## Evidence and outstanding qualification

Local checks exercise the actual Rust worker, prepared all-target publication,
shared view mapping, grouped Undo/Redo, stale source/history proofs, late
cancellation, inventory growth and coexistence with actual native saves. Real
selected Node extension tests cover public requests, hidden/untitled targets,
original identity, all mirrors/events before success, refactoring policy, FIFO
waiting and host retirement. The terminal journey in
`tests/extension_workspace_edits_pty.py` exercises a visible and hidden target,
original Undo/Redo shortcuts, unchanged Unicode/CRLF disk bytes and later public
saves of both files. The host normalization/lifecycle suite covers 68 workspace
edit cases. All 428 host tests and 1,120 Rust tests pass, including 866 library tests; 23
ignored Rust cases grant no qualification. The actual PTY journey, build, format
check and strict all-target Clippy pass. Totals are also recorded in
[the API workstream](EXTENSION_API.md).

This qualifies the documented subset, not full `workspace.applyEdit` parity.
Change events currently describe full-document mirror replacements rather than
upstream incremental deltas. Exotic non-Boolean refactoring metadata is projected
by truthiness rather than reproducing every upstream RPC serialization behavior.
Broader upstream runtime, Windows/macOS and arbitrary extension qualification
remain outstanding. No local VS Code process is used. Rust editing, transactions
and history need no JavaScript runtime; extension invocation uses the isolated
optional compatibility host.

Source contracts use immutable upstream commit
[`2a59476c9bfcb90b3ddc372c36762471b7dfad1c`](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostBulkEdits.ts)
and the [edit value receipts](EDIT_API.md).
