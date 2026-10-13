# Extension text document behavior

The shared extension document mirror now implements source-guided word lookup,
strict line/position/range argument handling, immutable line snapshots, UTF16
offsets and logical EOL indexing. `getWordRangeAtPosition(position, regexp?)`
supports the pinned default word pattern and a custom regular expression,
including Unicode words, decimal numbers and word-end affinity. Lookup uses the
upstream 1,000-unit window and soft time budget.

Native snapshots publish the actual model EOL and current `utf8` encoding.
Metadata EOL changes advance the exposed version, publish text when necessary
and invalidate stale edit/provider/document replies, even before a mirror sync.
Public text joins logical lines with model EOL; raw native snapshot bytes and
disk bytes remain intact. The same document object survives mirror updates,
including metadata-only updates. Closed documents remain readable and clean.

Local evidence: 15 focused document tests and the complete 238-test host suite
pass. The native extension-related selection passes 134 tests, including an EOL
metadata/data-integrity test and stale-EOL rejection. Real terminal document
opening, shared identity, native edit/undo/redo, host crash, recovery/save and
Memento restart workflows pass; the filesystem dirty-buffer workflow also
passes. Formatting and strict Clippy pass.

`TextDocument.save()` now uses the [native save gateway](NATIVE_DOCUMENT_SAVE.md)
for retained local/untitled documents. True follows actual persistence and shared
mirror publication; native Save As cancellation returns false. Closed documents,
retired callers, external conflicts and ambiguous outcomes reject. Hidden or
inactive saves preserve focus; newer postauthorization edits remain dirty.
[Virtual content-provider documents](VIRTUAL_DOCUMENTS_API.md) open and refresh
as readonly models; their save returns false without a Save As prompt. Writable
custom filesystem saves and complete save-participant parity remain outstanding. The save gateway has separate ownership, host and native evidence.
Installed language `wordPattern` and `languages.setLanguageConfiguration`
integration, broader encodings/BOM handling, public object reflection and full
upstream runtime comparison remain outstanding. A zero-width regex that cannot
advance rejects instead of looping indefinitely. A single catastrophic JS regex
cannot be interrupted by the soft budget; it runs in the isolated optional host.
These are documented limits, not full document/API qualification.

## Cached file opening

`workspace.openTextDocument` returns the original runtime-private model for an
already mirrored matching file URI or resolved string path. The current dirty
text survives, no native open request or fresh API acquisition is made, and no
duplicate open event or focus change occurs. Cached opens remain usable when
all eight native document service slots are occupied. Caller retirement, model
replacement during argument getters and unsupported encoding still reject.
Public document metadata and public URI factory overrides cannot redirect the
private cache. Canonical aliases continue through native resolution; the cache
does not collapse URI queries/fragments or share concurrent misses.

Eight new regressions and all 727 host tests pass. Both native document PTY
journeys pass with an eight-busy-slot cached reopen added to the actual hidden
open/edit/Undo/Redo/host-crash/recovery/save workflow. Formatting and strict locked
all-target Clippy pass. Native acquisition expiry and last-tab retention are now
integrated separately below; concurrent misses and encoding reload still need
their backends.
The immutable source contract is
[ExtHostDocuments.ensureDocumentData](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostDocuments.ts).

## Native API-reference lifetimes

Successful native file, untitled and readonly virtual opens acquire customer-wide
references before publishing mirrors. Each stores original model/resource
identity, UTF-16/EOL length and an independent three-minute deadline. Cached opens,
showing a model and virtual refresh acquire nothing or renew no deadline. Closing
the last tab or replacing a referenced preview moves the same Document into
private storage. Expiry releases references; native editor, dirty/history and
pending-work ownership remain separate. Eligible clean hidden models close in
the public mirror, preserving the old closed object's readable text/URI. Host
stop/crash/replacement drains the customer without discarding protected buffers.

Explicit Discard of a retained file reads the current disk on one bounded joined
worker. Original text/resource/history/baseline, settings and membership proofs
must still match before revert and close. Failed or late reads preserve original
work; successful reload preserves prior unsaved text in native Undo and leaves
disk bytes unchanged. Untitled revert releases only its original identity.

Twenty App controls (three actual selected hosts), seventeen ledger controls and
two checked-reload integrity controls pass; all 1,510 Rust tests pass (1,161
library; 23 ignored). The strengthened document terminal journey passes clean
close/reopen, dirty Discard/reopen/Undo, eight busy slots, shared public identity,
host crash and dirty recovery/save. Full provider-resource cache, shared miss
loading, encoding reload and disposal/history archival remain unqualified.
See the [native ownership contract](../src/document_references/README.md).

[Source receipts](../extension-host/document-types.provenance.json) pin stable
VS Code 1.141.0 at `2a59476c9bfcb90b3ddc372c36762471b7dfad1c`:

- [Document implementation](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostDocumentData.ts)
- [Word lookup](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/editor/common/core/wordHelper.ts)
- [Logical text model](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/editor/common/model/mirrorTextModel.ts)

Cached opens now check any matching private native mirror before provider or
creation-capability routing. This preserves existing provider/untitled mirrors
without a new native request, even after provider disposal. The original mirror
is captured before argument getters; replacement at the same URI rejects, while
a legitimate update preserves its identity. Five new cached-resource tests pass.
Shared miss loading remains separately tracked in the native ownership contract.
