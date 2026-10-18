# Extension editor end-of-line changes

`TextEditorEdit.setEndOfLine(EndOfLine.LF | EndOfLine.CRLF)` changes the retained
native document. The last call in the builder wins. Invalid values, retained
builders and stale captures refuse before dispatching an applied text prefix.
The optional host requires the native editor-EOL capability; it does not simulate
a successful change over an older backend.

The native order follows pinned VS Code 1.141.0: change EOL, apply the requested
before stop, edit text, then apply the after stop. With the default stops, a mixed
EOL/text request therefore has two Undo steps. Replacement strings normalize to
the resulting EOL. EOL and text mirrors/events precede the successful promise.
Later editor requests cannot overtake an occupied EOL preparation; their original
versions determine whether they can apply.

Preparation reuses the workspace Rope converter in one worker. Up to eight
intents, including actual work, retain an aggregate 8 MiB payload budget and a
five-second lifetime. A completed reply does not release a worker that is still
running. Both native history stages, current live-view mapping and the complete
ordered output receipt are prepared before publication. Invalid later ranges,
conversion bounds or output-admission failure cannot publish only the EOL stage.
Owner/session, model, exact view, resource/language, save/history and text-epoch
proofs reject obsolete replies, including edit/Undo ABA.

The existing 4 MiB mirror budget, combined 4,096 scalar/history steps, 8 MiB
metadata budget and bounded live-view mapping remain explicit limits. Larger
requests refuse as a whole. Disk bytes stay unchanged until native saving.
Shared line-end carets and reversed selections keep their logical positions
through conversion and Undo/Redo, including edits to an inactive view.

An EOL-only change versions a single-line model and controls future Enter input.
Requesting the existing convention adds no content/history entry and preserves
Redo; its requested before/after boundaries still apply. A source with mixed
physical line endings normalizes when its convention changes; requesting its
existing convention leaves the original bytes intact.

[Validation receipts](validation/editor-set-eol.json) record the named local
host/native/terminal workflows, source/binary and retained failed observations.
These do not qualify a whole API row. Large conversions, complete model-global
grouping, exact incremental/EOL event payloads, snippet/marker presentation over
every EOL transition, arbitrary private transport/UTF-16 inputs, arbitration with
other mutation families, and upstream-runtime/platform/release qualification
remain open. The entire stable API and seamless personal setup migration remain
required; native editing continues to work without Node.
