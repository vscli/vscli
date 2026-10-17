# Public workspace edits and grouped native history

Selected extensions can call `await vscode.workspace.applyEdit(edit)` with text
edits for retained native documents and existing unopened local regular files.
One transaction can edit visible, inactive, hidden and untitled models together.
Native preparation runs on a bounded Rust worker; all targets are validated and staged before any text changes. The complete
mirror is published before change events and the successful awaited Boolean.
Shared document identities survive the operation.

```javascript
const edit = new vscode.WorkspaceEdit();
edit.insert(first.uri, new vscode.Position(0, 0), 'first ');
edit.insert(second.uri, new vscode.Position(0, 0), 'second ');
const applied = await vscode.workspace.applyEdit(edit);
```

Existing local regular files can be included directly by URI. A bounded native
worker loads unopened targets and publishes real hidden models in the same
atomic cohort. Canonical aliases reuse original retained dirty models. Opening
with `workspace.openTextDocument` also retains a hidden model without changing
the active editor. Ordinary plain text edits change native buffers, preserving
their EOL convention unless an explicit EOL edit changes it. Multi-resource refactoring metadata can additionally request
the [native save continuation](REFACTORING_SAVES_API.md); explicit
`await document.save()` uses [native persistence](NATIVE_DOCUMENT_SAVE.md).
Normal native autosave remains governed by its own settings.

Undo or Redo from any changed participant restores the entire cohort, including
hidden peers. Original platform shortcuts use this native history route. If a
peer has independent history or was closed, the entire grouped operation is
refused and retains the other work; undo the independent edit first or keep the
peer open. Confirmation dialogs and history splitting are not implemented.
Nonempty no-op text transactions return true without consuming Redo or adding a
history entry; an empty edit returns false.

## End-of-line edits

The negotiated public gateway accepts `TextEdit.setEndOfLine(EndOfLine.LF)` and
`EndOfLine.CRLF`, including `newEol` on an ordinary text edit. For each resource,
all replacements use the original EOL convention, then the last requested EOL
converts the resulting buffer. This follows the pinned
[bulk text application order](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/contrib/bulkEdit/browser/bulkTextEdits.ts).

```javascript
const edit = new vscode.WorkspaceEdit();
edit.set(document.uri, [
  vscode.TextEdit.insert(new vscode.Position(0, 0), 'header\n'),
  vscode.TextEdit.setEndOfLine(vscode.EndOfLine.LF),
]);
await vscode.workspace.applyEdit(edit);
```

Conversion runs on the existing native worker, preserves shared and hidden
document identity, maps shared selections and joins the same grouped Undo/Redo.
It changes buffer bytes and `TextDocument.eol` before the public acknowledgement;
the disk baseline stays unchanged until an actual save. An EOL-only change also
advances the document version when the buffer contains no line breaks, setting
the convention for future Enter input. Requesting the existing EOL is a no-op
that preserves Redo. A source with mixed physical line endings is normalized when
changing its EOL convention; requesting its existing convention leaves it intact.

EOL conversion shares the existing 4,096 scalar/history-step and bounded view
mapping limits, including ordinary replacements and the EOL metadata step. A
larger conversion refuses the entire cohort without changing peers or history.
This limit remains a full-API obligation. `TextEditorEdit.setEndOfLine`, EOL edits
returned by file participants, exotic invalid-enum conversion, upstream-runtime
comparison and other-platform qualification remain outstanding.

Single-resource `{isRefactoring:true}` edits remain unsaved under default,
enabled and disabled refactoring autosave settings. This follows the pinned
[bulk-edit resource-count condition](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/contrib/bulkEdit/browser/bulkEditService.ts#L239-L244).
Multi-resource refactorings now continue through actual native save interests
when the root user/workspace setting `files.refactoring.autoSave` is exactly
Boolean true, with true as the missing-setting default. Explicit false and
non-Boolean values disable this continuation; language overrides do not change
this root policy. All involved dirty models, including dirty no-op peers and
worker-opened files, are considered. Unrelated dirty working copies are excluded.

Workspace text publication remains atomic. Persistence follows publication and
can partially succeed: a conflict, canceled Save As or stale deferred source
retains applied true with a warning while successful peer writes stay committed.
The full save cohort settles before public success or native F2 completion.
Grouped Undo changes buffers and dirties successfully saved peers without
rewriting committed disks; Redo returns the saved text clean. See
[save ownership, transitions and current evidence](REFACTORING_SAVES_API.md).

## Admission and failure behavior

The native gateway captures the complete original model inventory, target
identity, URI, exposed version, source epoch,
saved baseline, history heads, settings/profile and workspace at receipt, before
FIFO waiting. Later dispatch uses that original capture. A target edit followed
by Undo cannot renew the proof. Navigation/focus changes alone do not invalidate
a text transaction. The selected native caller lifetime cannot be renewed by
reactivating the same extension ID.

Invalid UTF-16 ranges, split surrogate pairs, overlapping replacements, duplicate
target groups or changed targets refuse the whole transaction. Same-position
insertions preserve their original order. No supported prefix of a mixed
unsupported edit is applied. File operations, custom/virtual
filesystem documents, snippets, annotations/confirmation metadata and
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

The EOL checkpoint passes eight named criteria in the
[live behavior ledger](extension-api/progress.json), with eight new native
behavior cases and fourteen new host cases. The complete final source passes
1,612 Rust tests across 67 reports (1,263 library; 23 ignored) and 797 host tests.
The EOL terminal journey verifies last-EOL-wins, mirror/event-before-return,
hidden identity, grouped CRLF/LF Undo/Redo and actual LF disk saves. The original
text journey, four file-participant journeys and no-Node path/history journey
also pass against the same binary. Formatting, strict locked Clippy, build,
frozen-inventory verification and three live-ledger scope/evidence checks pass.
[Validation receipts](validation/workspace-eol-edits.json) record source/binary
digests, exact commands and the initial host capability-forwarding failure that
was fixed before the successful final runs. This is local subset evidence;
platform, upstream-runtime and complete API qualification stay open.

Historical retained-text and rename qualification covers actual preparation,
atomic publication, shared-view mapping, grouped history, original source proofs,
worker joining, selected optional-host behavior and terminal persistence controls.
Those earlier totals and terminal runs do not qualify this new save continuation.

For the current refactoring-save changes, 719 host tests and 1,389 Rust all-target
tests pass across 67 reports, including 1,040 library tests; 23 ignored cases grant
no qualification. The 20 focused refactoring cases include actual disk workers,
original deferred text/language proofs and owner/cohort lifetime gates, exact save
receipts, conflicts and Save As. Four hidden/retained framed LSP cases additionally
pass in the full suite; three hidden participant cases pass in a focused run. A fresh build, global
format check and strict locked all-target Clippy pass. Seven selected-extension
tests and seven terminal journeys pass; their named cases and limits are recorded
in [the refactoring save contract](REFACTORING_SAVES_API.md). Fresh existing
rename, direct-save and native save-participant terminal regressions also pass:
34 terminal journeys in total, with exact script counts in that contract.

This evidence does not establish full `workspace.applyEdit` parity. Change events
currently describe full-document mirror replacements rather than upstream
incremental deltas. Exotic non-Boolean refactoring metadata is projected by
truthiness rather than reproducing every upstream RPC serialization behavior.
Rich apply backends, extension save participants, broader upstream runtime,
Windows/macOS and arbitrary extension qualification remain outstanding. No local
VS Code process is used. Rust editing, transactions, saving and history need no
JavaScript runtime; extension invocation uses the isolated optional host.

Source contracts use immutable upstream commit
[`2a59476c9bfcb90b3ddc372c36762471b7dfad1c`](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostBulkEdits.ts)
and the [edit value receipts](EDIT_API.md).

The [rename milestone](RENAME_PROVIDERS_API.md) extends this backend to genuine
unopened local text files. Fourteen new native tests cover bounded loading, canonical
dirty hidden aliases, all-target refusal, original inventory changes, cancellation
with actual worker joining and grouped Undo/Redo. Missing, binary, oversized and
nonregular targets refuse before publication. The negotiated public gateway omits
identity/version for unopened URIs; successful original-URI-to-document receipts
resolve canonical aliases against actual published models. It never invents a
JavaScript document version.
