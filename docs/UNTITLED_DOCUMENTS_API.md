# API-created untitled documents

`workspace.openTextDocument()` and the content/language options overload create an
editable native untitled model. An explicit `untitled:` URI retains its resource
identity; a file-associated untitled URI records a proposed destination separately
from the native saved path. Opening it does not create a file. Existing destinations
are refused during worker admission; a later foreign file creation is protected
by the native create-only save transaction.

Initial content becomes the model's starting baseline without a synthetic Undo
entry. The first real edit's Undo returns to that content. API-created models are
initially dirty even when empty. After an actual edit or Undo, an unassociated
empty model becomes clean; an associated model stays dirty. A no-op empty insert
does not clear the initial dirty state. Ordinary keyboard-created untitled
models retain their existing native behavior.

Initial text retains Unicode and uses one consistent EOL: CR/CRLF making up more
than half the line breaks selects CRLF, otherwise LF; text without line breaks
uses the native platform default. A language option persists as an explicit
model override. The associated filename supplies the native tab name, while the
actual saved path remains absent until a successful save receipt.

Public `TextDocument.save()` uses the existing joined native persistence worker.
A proposed associated destination can be saved directly; content/options-created
models use native Save As. Escape returns false without writing. Only actual
successful persistence clears untitled URI/association metadata and advances the
saved baseline. Native buffer identity, newer edits and Undo history survive.
The old public untitled `TextDocument` closes cleanly with its original URI/text;
a distinct file document becomes current. Native mirrors publish before ordered
`onDidCloseTextDocument(old)`, `onDidOpenTextDocument(new)` and
`onDidSaveTextDocument(new)` callbacks, and before the original authorized save
resolves true. Later saves or editor edits on the closed identity reject. Same-URI
saves preserve the existing public identity. Explicit language metadata remains. Recovery retains untitled identity, association,
language and dirty initial content, including an empty initial model. Legacy
recovery journals remain readable. A malformed later metadata record refuses the
whole journal and preserves its original bytes.

## Evidence and limits

Five focused Document/recovery tests and three native creation-input tests pass.
Four actual selected-host/App workflows pass: content/noarg/options opens and
initial Undo, Save As Cancel/accept and public resource transition, direct
associated saves, existing/late-file overwrite protection, and fresh names when a
retained recovered URI is already occupied. Twenty focused host cases and one
framing case were added; all 524 host tests and 1,197 Rust all-target tests pass
(894 library tests; 23 ignored cases provide no qualification).

Three actual native PTY journeys pass content/language creation, Unicode/CRLF
initial Undo, editable new file Undo/Redo, associated direct saves and no-clobber
refusals. Every journey preserves the original dirty file's shared identity,
keyboard Undo/Redo and disk bytes; exit answers each retained dirty model's
Save/Discard/Cancel prompt and restores terminal mode. Four existing public-save
PTY journeys and the virtual-document PTY also pass with the new public lifecycle.
Build, formatting and strict locked all-target Clippy pass. Source comparisons
and local execution remain distinct from upstream runtime or platform parity.

The public gateway admits 1 MiB of content, 4,096-byte URIs and paths, 128-byte
language identifiers and eight queued native document services. It shares the
actual native worker and the 4 MiB mirror/16-hidden-model allowances. Core model
construction separately allows 32 MiB; that does not enlarge the public gateway
limit. NUL-containing content is explicitly refused. UTF-8 is supported;
non-UTF-8 encoding options are rejected until implemented. Display, Save As and
native editing remain separate from arbitrary virtual/custom filesystem editing.
Full untitled label formatting, encoding fidelity and upstream/platform execution
remain outstanding.

Source comparisons are pinned to stable 1.141.0:
[document creation](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/browser/mainThreadDocuments.ts),
[untitled model lifecycle](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/services/untitled/common/untitledTextEditorModel.ts),
[public document lifetime](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostDocumentData.ts),
[closed editor refusal](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTextEditor.ts)
and [EOL normalization](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/editor/common/model/pieceTreeTextBuffer/pieceTreeTextBufferBuilder.ts).
No local VS Code instance was launched.
