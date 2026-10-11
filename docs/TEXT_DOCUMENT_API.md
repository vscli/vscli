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

`TextDocument.save()` still rejects explicitly because its public acknowledgement
hook is not connected. The [native foundation](NATIVE_DOCUMENT_SAVE.md) now
captures exact model/caller interests and real persistence receipts. It never
writes around the native save pipeline, returns invented success or clears
dirty text optimistically.
Installed language `wordPattern` and `languages.setLanguageConfiguration`
integration, broader encodings/BOM handling, public object reflection and full
upstream runtime comparison remain outstanding. A zero-width regex that cannot
advance rejects instead of looping indefinitely. A single catastrophic JS regex
cannot be interrupted by the soft budget; it runs in the isolated optional host.
These are documented limits, not full document/API qualification.

[Source receipts](../extension-host/document-types.provenance.json) pin stable
VS Code 1.141.0 at `2a59476c9bfcb90b3ddc372c36762471b7dfad1c`:

- [Document implementation](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostDocumentData.ts)
- [Word lookup](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/editor/common/core/wordHelper.ts)
- [Logical text model](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/editor/common/model/mirrorTextModel.ts)
