# File-participant review policy and profile preferences

The native review model now retains the complete original WorkspaceEdit DTO and
indexes create/rename/copy/delete, text, snippets, EOL and notebook rows without
performing edits or filesystem operations. This is an overview foundation for
the editor coordinator, **not a completed diff preview or apply backend**.

The initial participant decision follows the pinned main-thread policy. No edits
means no dialog. With no remembered choice, ordinary contributions offer Apply,
Preview or Skip Changes, with optional remembering. Confirmation-marked
contributions instead offer Preview or Skip; that confirmation dialog does not
persist a choice. Skip never persists. A stored boolean supplies the requested
preview policy directly. This initial decision is separate from the later bulk
edit service's annotation/preview requirements, which must still be enforced.

Metadata confirmation flags retain source JavaScript truthiness. Original arrays,
versions, labels, binary contents, file options and group order remain in the
immutable DTO; summaries do not replace the payload. Compound text/EOL rows keep
both effects. Source-size nested metadata is accepted under a bounded native
depth of 96, including the DTO's wrappers around converter metadata.

Profile storage uses `file-participant-preferences.json` alongside the native
user settings file, with schema 1 and a `values` object. The preview key preserves
the pinned source spelling, `file.particpants.additionalEdits`. A missing key is
the unremembered choice. Other bounded profile values survive updates and reset.
The file is separate from extension Memento namespaces and workspace settings.

All storage reads/staging/commits belong on workers. The existing native
persistence guard retains a cooperating-writer lock, original destination
baseline and canonical parent authority. Writes stage without changing payload
bytes, then commit only after original native profile authorization. Corrupt,
oversized, nonboolean, symlink and nonregular payloads are refused. A changed
destination after staging cannot be overwritten. The guard's existing limits
against uncooperative check-to-rename races remain separately documented.

**F1 → Reset choice for 'File operation needs preview'** now invokes the stable
`files.participants.resetChoice` command. Selected extensions can invoke it with
`vscode.commands.executeCommand`, including from an empty editor. It takes no
arguments and cannot be replaced by an extension command registration. The reset
is admitted into the current profile's cache immediately; persistence completes
on a joined worker, with failure reported by the editor. This acknowledgement
does not promise completed disk durability. Profile changes do not carry that
cache marker into another profile or revive it on A→B→A.

The write coordinator refuses an unapproved reset when a matching preference
buffer is dirty. Cancellation/profile change before authorization preserves
stored bytes. Accepted work still finishes in its original profile after caller
retirement; its terminal result appears only after actual thread exit/join.
The command does not enable will-file subscriptions or intercept Explorer work.

Six review controls cover the source decision table, remember/skip semantics,
every edit kind, original DTO preservation, JavaScript confirmation truthiness,
whole-review refusal and metadata depth. Five storage controls cover complete
roundtrips/reset with unrelated state, canceled work, corrupt/bounded payloads,
external changes and filesystem type checks. Five App controls cover no-editor
native reset, original-profile authorization, A→B→A, accepted retirement, dirty
data and actual selected-extension delegation. A host control covers command
listing/reservation and argument-free delegation. Terminal qualification uses
the blank screen, no Node, both remembered values, restart, Unicode state and
terminal restoration.

All 1,580 Rust all-target tests pass across 67 reports (1,231 library tests;
23 ignored cases provide no qualification), along with all 783 host tests,
formatting, strict locked all-target Clippy and native build. The new native
terminal reset/restart journey passes with Node unavailable. No local VS Code
instance was launched; CI remains deferred.

Next: render the actual review/preview, use the profile choice after participant
delivery, connect original file operations and filesystem preflight, apply full
ordered groups, and qualify history/event order. The complete extension API
objective remains open.

Reference: VS Code 1.141.0 commit
`2a59476c9bfcb90b3ddc372c36762471b7dfad1c`,
[initial choice, profile key and reset command](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/browser/mainThreadFileSystemEventService.ts).
