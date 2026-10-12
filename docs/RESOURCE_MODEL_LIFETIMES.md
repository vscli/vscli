# Resource operations and native model lifetimes

This is a source-derived integration contract, not runtime qualification. It
uses VS Code 1.141.0 commit `2a59476c9bfcb90b3ddc372c36762471b7dfad1c`.
No local VS Code instance was launched. Actual ordered execution, model retention
and resource-specific editor/event qualification remain unfinished in VSCLI.

## Conditional destination reuse

A move restores into a surviving destination model, preserving its public
document identity and version lineage. If no destination model survives,
restoration creates one. Source model identity is not assigned to the destination
URI. Dirty source snapshots, folder descendant mappings and selected language
state are restored; clean sources reload disk. See
[model restoration and resolution](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/services/textfile/common/textFileEditorModelManager.ts).

Overwrite complicates survival: the filesystem layer deletes the destination
before renaming. Its delete event can dispose destination editor inputs after
soft revert cleared dirty flags. Independent API or other references can keep
the destination model alive. Editor replacement can reuse an existing destination
tab and retains source view/group placement; asynchronous editor replacement does
not prove a single universal ordering relative to working-copy restoration. See
[filesystem overwrite](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/platform/files/common/fileService.ts)
and [editor event handling](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/services/editor/browser/editorService.ts).

Public structural close/open follows model removal/addition. A surviving
destination is not universally closed and replaced. Language notifications can
emit close/open on the same public document without setting isClosed, so event
pairs alone cannot establish replacement. See
[model synchronization](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/browser/mainThreadDocumentsAndEditors.ts)
and [document notifications](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostDocuments.ts).

A surviving destination keeps its own encoding preference; source encoding
initializes a newly created destination. Equal target text can preserve its
version; diff/EOL updates otherwise follow destination history, rather than copy
source versions. Create-overwrite reloads a surviving target; creating an unopened
file does not independently create a text model. Untitled create rows are skipped
by bulk file execution, leaving later text handling responsible for them. See
[target resolve](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/services/textfile/common/textFileEditorModel.ts),
[model updates](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/editor/common/services/modelService.ts)
and [bulk creation](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/contrib/bulkEdit/browser/bulkFileEdits.ts).

## API reference provenance

A cached mirrored open with matching encoding acquires no native reference or
fresh expiry. Concurrent misses share one loader by URI. Actual native opens,
encoding reloads and untitled creation acquire references in a collection shared
by the extension-host customer, independent of extension ownership. See
[public cache](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostDocuments.ts).

Each acquisition expires after three minutes. Captured text-model length totals
above 80 Mi units evict oldest entries; reaching sixty entries evicts ten. Length
is captured once, not tracked as disk bytes or current process memory. Genuine
acquisitions can duplicate a canonical URI. Move/delete working-copy completion
removes affected references; create/copy and last-tab close do not. Overwrite's
internal file-service deletion is distinct from explicit working-copy deletion.
Untitled revert removes its references. See
[acquisition, bounds and removal](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/browser/mainThreadDocuments.ts).

Reference expiry need not destroy the model: other owners and dirty/pending-resolve
protection can retain it. Native implementation must therefore distinguish API
leases, editor memberships and dirty/history ownership. See
[resolver disposal](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/services/textmodelResolver/common/textModelResolverService.ts).

## Native integration obligations

Preserve displaced/deleted unsaved text in private archives. Capture actual model
and history proofs before effects; durable numeric IDs alone do not authorize a
new model after restart. Native documents now carry an independent checked
resource epoch through Save As and Explorer path publications. Save/workspace,
provider/editor-service and disk-refresh proofs validate it alongside their
text/language/history and saved-generation guards. A return to the original path
cannot restore an earlier proof. Baseline publications retain their independent
save-generation guard. This does not yet implement resource-edit model disposal,
conditional destination reuse or API reference acquisition.
Keep source and destination lifetimes separate. Implement surviving-target reuse,
reference provenance, expiry and teardown before claiming exact resource/API
document compatibility. Teardown must drain an owned snapshot rather than mutate
the collection being iterated. Precise editor/watcher/refcount timing still needs
runtime evidence; a source trace is insufficient to qualify those races.
