# Extension configuration updates

The optional extension host routes `workspace.getConfiguration(...).update(...)`
to the native settings writer. Native editing still runs without Node. This batch
adds a usable local write path; the entire stable API and seamless migration remain
required.

```js
const config = vscode.workspace.getConfiguration('editor', {
  uri: editor.document.uri, languageId: editor.document.languageId
});
await config.update('tabSize', 3, vscode.ConfigurationTarget.WorkspaceFolder, true);
```

The promise resolves with `undefined` after the native writer finishes, native
settings reload, open target settings models refresh through the existing disk
reader, and configuration/document/editor notifications have been sent before the
acknowledgement. A retained configuration object's `get` remains a snapshot;
`inspect` and newly requested configurations observe current values. Writes do not
optimistically mutate host state.

Global/`true` writes use the configured native profile's `settings.json`.
Workspace/`false` and an omitted target use the current single folder's
`.vscode/settings.json`. WorkspaceFolder requires a captured local resource inside
that folder. `overrideInLanguage: true` writes a captured language override;
`false` writes the root; omission retains an existing target override, including
an existing `false`. `undefined` removes a property; JSON `null`, arrays and
objects remain values. The bidirectional `ConfigurationTarget` enum is exposed.

The native JSONC patcher changes only the selected property/value/comma spans,
retaining unrelated comments, language blocks, Unicode and CRLF bytes. Missing
removal creates no directory. Existing no-op writes preserve file identity. The
existing settings writer retains filesystem/model identity checks, external-write
protection, atomic replacement and process-local locking without project lock
sidecars. A dirty settings model refuses the update and retains its native Undo
history. Unrelated dirty editors remain usable. Shared and API-hidden clean
settings models retain their identity and gain an undoable disk refresh. The
controller waits for a current read/publication after the committed write; a
retained pre-write reader cannot satisfy it. A hardlink at another resource keeps
its original bytes after atomic replacement. Closing or moving the original model
does not redirect its receipt to another model at the same path.

If settings edits become dirty after the authorized file write, the write cannot
be undone by pretending it never happened. The promise rejects with “Setting
persisted” and an explanation; the dirty text and its Undo history stay intact.
The new file remains on disk. Accepted watcher publications are retained only for
current model IDs, and obsolete source/epoch replies never acknowledge a refresh.

Configuration writes share the eight-operation/eight-MiB host service budget.
The native controller runs one actual settings worker, serializes against native
settings toggles, and checks the original profile, workspace, owner/session and
settings model proofs before authorization. Values are bounded to 64 KiB,
4,096 nodes and depth below 64; settings files are bounded to one MiB. A timeout
reports an unknown outcome and retains actual work until a valid native receipt.
Profile A→B→A cannot revive an unapproved write; an already authorized filesystem
operation may finish for its original profile and cannot be described as rolled
back merely because its caller retired.

Qualification is recorded in [the write receipt](validation/configuration-update.json)
and [the open-model refresh receipt](validation/configuration-model-refresh.json).
The synthetic public fixture checks event/read/void order against real disk writes,
language override behavior against actual native editor options, and dirty/shared
history retention. Terminal checks cover Unicode/CRLF save/Undo/Redo, persisted
native language settings after restart without Node, original source preservation,
absence of lock sidecars, and dirty-settings refusal.

Remaining obligations include complete schema/default registration and scope/error
fidelity, dirty-settings prompting/merge behavior,
multi-root/workspace-file/remote/profile/default-target
semantics, exact configuration event fidelity, large values, and upstream-runtime
and other-platform qualification. Personal extensions and arbitrary visual setup
migration have not been qualified by these synthetic workflows.
