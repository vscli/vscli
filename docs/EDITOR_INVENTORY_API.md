# Extension editor inventory and shared native panes

The native App publishes a separate public `TextEditor` for each visible pane.
Two panes showing one file share the same `TextDocument`, text, dirty state and
Undo/Redo, while retaining independent selections and `viewColumn` values.
`window.visibleTextEditors` includes all visible panes; `activeTextEditor` identifies
the focused pane or is undefined in an empty workbench. Core editing remains
native Rust and usable without Node.

Public editor identities survive continuous visibility, focus changes, language
changes and column changes. Hiding, closing, replacing a pane's selected document,
replacing its native view owner or changing its document URI retires that editor.
Returning to the same file and group creates a new editor handle. Native membership
transitions are observed even when several changes occur before the next host
publication, so hide/show cycles cannot revive an old handle.

Selection setters and `TextEditor.edit` retain their original private editor
identity. An inactive visible pane can receive selections without changing focus
or the other pane's selections. Retired handles refuse rather than acquiring a
replacement editor for the same file or column. Caller, document version/resource,
language, settings and native view checks remain independent of public getters or
optimistic selection presentation.

`showTextDocument` accepts its options and positional view-column overloads.
`ViewColumn.Active`, `ViewColumn.Beside`, existing columns 1–4 and the next reachable
column use native editor groups. Beside reuses a right-hand group when available
and otherwise creates one. `preserveFocus: true` retains the original focused group
and native focus area while returning the actual shown editor, which may be
inactive. Selection ranges are validated as UTF-16 coordinates before mutation.
New groups, shared views and hidden-model admission are staged before publication.

The complete document/editor mirror publishes before events and acknowledgements.
Visible-editor events describe additions/removals; focus or column changes alone
do not fabricate a visible-list change. Active-editor events follow visible-list
events. `onDidChangeTextEditorViewColumn` identifies the retained scoped editor
whose column changed. Selection events identify the actual target pane.

Accepted show operations retain their original result while mirror or reply
admission is unavailable. Retrying acknowledgement does not reopen the document,
create another pane or apply selections twice. A retired original caller or shown
editor cannot be replaced by a newer handle to make an old request succeed.
Selection-operation receipts retain the same publication-before-acknowledgement
ordering. These queues remain bounded until terminal acknowledgement or retirement.

## Qualification and remaining work

All 559 host tests pass, including seventeen editor-inventory cases for independent
scoped identity, private proof capture, inactive setters, stale edit callbacks,
show overloads/acknowledgements, event ordering, column changes and retained timeout
occupancy. All 1,232 Rust all-target tests pass (923 library; 23 ignored cases grant
no qualification), including ten new native library tests and two new actual
selected-host/App inventory workflows. The four existing editor-state App
workflows pass too. These cover native shared history, retired-handle refusal,
group closing, empty inventory, explicit-language Save As and recovery. Two real
selected-host receipt regressions cover accepted Show mirror pause/resumption and
coalesced visibility A→B→A rejection without reapplying native effects or losing
Unicode/CRLF text, dirty state, Redo or disk bytes.

Two new actual Unix terminal workflows pass shared pane identity/selections,
preserveFocus, inactive target routing, retired-editor refusal, native movement,
group-column changes and empty visible/active inventory. Native keyboard Undo/Redo
retains original Unicode/CRLF dirty work and protected disk bytes. The existing two
editor-state, three untitled, four public-save and one virtual-document terminal
journeys pass again. Native build, formatting and strict all-target locked Clippy
pass. This is local Linux qualification; it does not establish upstream runtime
parity or Windows/macOS behavior.

Native mirrors admit 128 models, four visible groups, 10,000 selections per
model/view and 16,384 selections across model and pane copies in one publication.
Typed selection mutations admit at most 4,096 selections per target and preserve
the aggregate mirror limit. Exceeding mirror limits pauses publication while core
buffers remain usable; publication resumes within bounds. Legacy direct Client
embedders retain document-keyed mirrors; the standard App/CLI starts extensions
with the explicit inventory before activation.

Sparse columns requiring empty intermediate groups, preview tabs, configured
downward Beside placement, more than four groups and other editor
APIs remain outstanding. The subsequent [editor presentation milestone](EDITOR_PRESENTATION_API.md)
adds native options, measured visible ranges, scoped events and revealRange,
with separate bounds and terminal presentation differences. The
[decoration milestone](EDITOR_DECORATIONS_API.md) adds per-view overlays and
permits Show requests across measured-range-only publications while preserving
source, membership, options, selections and context checks. Native keyboard/mouse
selection-kind provenance, remote/custom editors and broader event/runtime/platform
fidelity remain unqualified. This milestone does not establish complete editor or
extension API parity.

Source comparisons use immutable stable 1.141.0:
[editor inventory and event publication](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostDocumentsAndEditors.ts),
[editor objects and selection setters](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTextEditor.ts),
[native show options](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/browser/mainThreadEditors.ts),
and [column mapping](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/services/editor/common/editorGroupColumn.ts).
No local VS Code instance was launched. CI remains deferred under the requested
feature-first workflow.
