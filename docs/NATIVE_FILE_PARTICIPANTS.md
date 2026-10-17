# Native file-operation participants

Selected editor extensions now receive `workspace.onWillCreateFiles`,
`onWillRenameFiles` and `onWillDeleteFiles` before native Explorer effects.
Ordinary text contributions reach real retained/unopened native text transactions,
including reviewed text annotations. This is a usable intermediate workflow in
the **entire stable extension API** workstream. Full ordered resource, snippet,
EOL and notebook application remains required and is not implemented here.

## Operation and edit authority

Before dispatching a callback, a joined preparation worker uses the existing
resource-journal capture to validate the original operation and retain canonical
resource/parent identities and disk bytes. An existing rename destination or a
missing parent refuses before callbacks or edits. The existing bounded capture
limits apply: 32 MiB shared resource data and 4,096 filesystem nodes. Symlinks,
special nodes and unsupported paths refuse this participating path explicitly.
Native operation without an extension host keeps its existing behavior.

The source setting `files.participants.timeout` is root-scoped, defaults to
60,000 milliseconds, and disables callbacks at zero or a negative/non-numeric
winning value. Native representable budgets extend through 2,147,483,647 ms;
larger values disable with a settings notice. Positive fractional milliseconds
truncate with a one-millisecond floor. Language overrides do not change it.
Dispatch retains the transport's original deadline and actual callback capacity;
review does not renew an expired callback budget.

The complete returned DTO remains immutable through review. Original target and
contributor proofs are checked at receipt, during review and immediately before
text publication. This includes service polling while the actual Client is
temporarily owned outside App; moving the handle is not host retirement.
Target edit/Undo ABA, replaced contributors or workspace/settings/profile changes
cannot authorize an old review. A stale review cancels the original operation and
retains unsaved work rather than rebinding its authority.

## Review and continuation

Without a remembered choice, ordinary contributions offer Apply, Preview and
Skip, plus Remember. Confirmation annotations require preview, including when a
stored choice requests direct application. Preview shows worker-prepared removed
and replacement ranges from actual retained dirty text or bounded unopened file
loads. It handles UTF-16 boundaries and aliases, escapes control/bidi characters,
and reports unavailable or clipped ranges explicitly. Rich file/snippet/EOL/
notebook rows retain their original details; a range preview is not a simulation
of an unsupported backend. Preview selection keeps original DTO indices.

Apply uses the existing grouped native text transaction and preserves shared
panes, hidden targets, dirty state and ordinary Undo/Redo. Additional text is not
implicitly saved. File effects follow accepted text publication; completion
events follow accepted document mirrors. Save, autosave, Revert and disk-watch
publication cannot race the pending original operation.

Skip or Escape in review skips additional edits and continues the original
operation. Escape during callback delivery cancels the logical contribution while
retaining actual callback occupancy until settlement. Timeout and participant
errors likewise allow the original operation to continue. Native preparation and
final effect admission recheck the original filesystem proof; a foreign write
cannot be silently adopted after review. These checks retain the existing
filesystem backend's limits against uncooperative races between its final checks
and effect syscalls; this is not a new atomic filesystem transaction claim.

A selected unsupported file/snippet/EOL/notebook entry refuses the **whole
additional selection before text effects**, with a visible reason. It is not
filtered away or converted to successful text-only application. The original
Explorer action can still continue under participant failure isolation. Public
mixed-resource `workspace.applyEdit`, public file history and applying all source
edit kinds remain explicit obligations in [the resource plan](RESOURCE_EDITS_PLAN.md).

Remembered choices use the existing joined profile writer; reset remains available
through `files.participants.resetChoice`. Failed storage does not hide a native
warning. No JavaScript runtime is needed for ordinary Rust editing/file commands.

## Evidence and remaining qualification

Ten actual selected-host/App controls cover shared/hidden application and grouped
history, mirror-before-completion events, review/skip, original preflight failure,
stale target and profile ABA, external original writes, whole mixed-selection
refusal, remembered choices/confirmation, configured disablement and actual held
callback cancellation/timeout occupancy. Separate native controls cover range
preview, preflight effects and root policy. All 1,604 Rust and 783 host tests pass,
with four new participant and four existing regression terminal journeys on one
native binary, plus formatting, strict locked all-target Clippy and build.
[Validation receipts](validation/native-file-participants.json) record the source
and binary identities and exact commands. A test-only entered/release handshake
corrects a reload fixture that could cancel before its claimed held phase;
existing integrity assertions remain intact.

Broader platform/package behavior, activation during callbacks, whole ordered
file/text/snippet/EOL/notebook execution, full file history and upstream runtime
qualification remain open. No local VS Code process is launched.

Reference: immutable VS Code 1.141.0 commit
`2a59476c9bfcb90b3ddc372c36762471b7dfad1c`,
[participant timeout and failure isolation](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/services/workingCopy/common/workingCopyFileOperationParticipant.ts),
[review and original-operation continuation](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/browser/mainThreadFileSystemEventService.ts),
and [timeout default](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/contrib/files/browser/files.contribution.ts).
