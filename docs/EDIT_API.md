# Extension edit objects and native transactions

The optional host implements source-guided `TextEdit`, `SnippetTextEdit`,
`WorkspaceEdit` and `NotebookEdit` value objects from pinned stable VS Code
1.141.0. Shared constructors also provide `NotebookRange`, `NotebookCellData`,
`NotebookData`, `NotebookCellKind` and the bidirectional `EndOfLine` enum.
Creating an object does not edit a native document or write a file.

`TextEdit` supports replace/insert/delete/setEndOfLine, mutable fields and JSON.

Snippet edits retain their snippet and whitespace settings. Notebook edits retain
cell arrays and metadata references. TextEdit, WorkspaceEdit and NotebookEdit
support the upstream ES5 function-subclass call/apply convention. All extension
facades and provider adapters use the same constructors.

`TextEditor.edit` supports native before/after Undo-stop options and no-op builder
handling. [Editor Undo boundaries](EDITOR_UNDO_STOPS_API.md) records the usable
subset. [Editor EOL changes](EDITOR_EOL_API.md) covers `setEndOfLine` before text,
separate default Undo steps and remaining global-history/platform obligations.

`WorkspaceEdit` stores operations in order. Following the actual pinned source,
`set(uri, edits)` appends; an empty array is a no-op, while null/undefined removes
that resource's non-file operations. This differs from the declaration prose
saying set replaces existing edits. `has`, `get`, `entries`, `size` and `toJSON`
expose text edits only. File-only and snippet-only collections have size zero.
Entry metadata, file options, text edits and notebook arrays retain references;
returned outer arrays are copies.

Native code-action serialization accepts the existing plain-text transaction
subset. It captures primitive text, then retains independent coordinate, UTF16,
owner, version, overlap, workspace and Undo checks. Metadata requiring a
confirmation, EOL changes, snippets, notebook mutations and file operations are
explicitly unsupported by that backend. A mixed action is disabled as a whole;
its supported prefix is never applied. Formatting and completion adapters also
reject unsupported edit fields and EOL edits, instead of converting them to empty
text changes. Completion captures its primary edit and newText once so insertion
and replacement cannot disagree through changing getters.

Workspace collection publication is staged with generation checks and bounds:
4,096 entries, 128 resources, 4 KiB serialized URIs and 4,096 cells per insertion.
Rejected outer mutations leave intentional reentrant inner mutations intact.
These bounds and strict native admission are explicit adaptations; source-guided
value methods do not establish whole API compatibility. Arbitrary extension
getters cannot be interrupted inside the optional host.

Local evidence: all 286 host tests pass, including 21 edit-value tests and four
adapter/facade regressions. The broader native extension selection passed 134
tests; after the final value corrections, five native extension-code-action
integration tests pass. Fixture-gated ignored corpus cases are not new evidence.
Actual terminal checks pass for both unchanged official code-actions sample
scenarios, seven provider workflows and the queued native-only code-action
workflow, with Unicode/CRLF save and Undo. A focused real terminal regression
proves mixed text/EOL rejection leaves the native dirty buffer, original disk
bytes and pre-existing Undo history intact; a subsequent supported action saves
and undoes correctly. Build, formatting and strict Clippy
pass. These checks qualify the documented subset only.

Public `workspace.applyEdit` now routes retained-document text transactions
and explicit EOL edits through atomic native publication and grouped Undo/Redo; see
[native workspace edits](NATIVE_WORKSPACE_EDITS.md). File/notebook/snippet
transactions and annotation confirmation remain pending. EOL Undo and actual
persistence are available through this public gateway; code actions, F2 and
TextEditorEdit retain their separately documented backend limits. Public object
reflection, complete upstream runtime comparison and platform qualification also
remain outstanding. No local VS Code instance is launched for these checks.

[Source receipts](../extension-host/edit-types.provenance.json) record immutable
source hashes and the MIT license. The implementation embeds the complete notice.
Relevant pinned upstream sources:

- [Text edits](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTypes/textEdit.ts)
- [Workspace edits](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTypes/workspaceEdit.ts)
- [Snippet edits](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTypes/snippetTextEdit.ts)
- [Notebook values](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTypes/notebooks.ts)
- [Native conversion contract](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTypeConverters.ts)
