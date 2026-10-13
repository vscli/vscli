# Native resource model capture

`Document::capture_resource_model()` captures an actual writable native file
document without reading the filesystem or flattening text. The source retains
copy-on-write current text and the original disk baseline, native model/resource/
text/language/save clocks, next revision and original Undo/Redo head identities.
Caret movement and display changes do not invalidate that source. Edit→Undo,
path/language away→back, history replacement and accepted baseline publication do.

Keep the original `ResourceModelSource` through the worker's prepared reply and
call `check_current()` on the actual retained document before authorization.
Serialized `ModelProof` values describe recovery evidence; they cannot recreate
native history identity or authorize another live document after restart.

`archive()` materializes bytes on the existing joined resource worker, using its
original cancellation flag/deadline and the remaining aggregate payload/metadata
budgets. Current text is exact UTF-8. Versioned JSON metadata retains the original
EOL, selected language, dirty state and exact baseline chunks, including mixed
line endings and the distinction between a missing and empty baseline. JSON
escaping charges actual encoded bytes; bounded buffer growth never deliberately
reserves above the supplied limit. Text plus metadata share the remaining raw
budget, capped at 32 MiB; metadata is additionally capped at 8 MiB. The journal
still charges other operations/preimages against its whole-transaction budgets.

Eight controls cover original Unicode/CRLF data after edit→Undo, independent view
movement, path/language ABA, history/baseline retirement, encoded-byte budgets,
cancellation/deadline refusal, writable file admission, missing/empty baselines
and exact durable cold recovery. The two existing joined archive-worker controls
now use this production capture and validate the original live source. The actual
create-overwrite/rollback test also uses the production archive and preserves both
unsaved text and the original disk bytes.

The final Linux all-target Rust suite passes 1,527 tests across 67 reports
(1,178 library; 23 ignored), with formatting and strict locked all-target Clippy.

This is the native model-to-journal boundary. The App resource coordinator,
full history/view archives, public file events,
WorkspaceEdit resource admission and public Undo/Redo remain unfinished. No new
terminal, third-party-extension, cross-platform or full API parity is implied.

## Native destination restoration

`prepare_transfer()` prepares MOVE/COPY model contents against an original
surviving destination. Dirty source text restores; clean sources reload the
supplied actual disk postimage. Destination identity/version lineage, history and
current shared views stay owned by the destination. `prepare_reload()` prepares
CREATE-overwrite reload of an existing target. `restore_new()` creates a fresh
identity when no destination survives, without borrowed source history. These
methods perform no filesystem reads or App membership changes. The coordinator
must validate original caller/settings/model proofs and actual effect/postimage
receipts, select surviving membership and preserve displaced archives first.

Worker text preparation retains original cancellation/deadline. Whole-line
common prefix/suffix replacement and forced marker movement follow the pinned
[ModelService update structure](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/editor/common/services/modelService.ts).
Raw native EOL differences remain real byte edits; full logical-buffer
normalization and event-by-event upstream equivalence remain unqualified. Text
and baseline share 32 MiB; native publication metadata is capped at 8 MiB. Only a
private prepared payload enters an exclusive commit. Dropping it publishes
nothing. Retimed deadlines, stale source/history, cancellation, exhausted counters
and allocation refusal reject publication. Baselines retain independent clocks.

Unchanged text keeps its text revision/epoch, while dirty source contents can
still mark that equal target dirty. A fresh initially dirty model has no saved
alternative version until Save. EOL metadata now travels with ordinary and
grouped history, restoring future Enter behavior along with bytes.

Specific explicit source language restores as explicit. Inferred source language
is a separate native hint, applied only to plaintext targets other than `.txt`;
target-specific language wins otherwise. Explicit plaintext does not replace a
target language. Hints participate in native language consumers/archive metadata;
Auto clears them. Current captured inference uses built-in path classification;
custom associations/contributed detection still need App language/settings
authority. See the pinned [text-file manager](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/services/textfile/common/textFileEditorModelManager.ts)
and [text-file model](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/services/textfile/common/textFileEditorModel.ts).

Eleven restoration controls cover surviving/new identities, destination history and
shared views, equal-text dirty state plus Save, clean postimage reload, language
provenance, EOL-only Undo/Redo, stale/canceled/retimed work, budgets, dropped
commits and forced decoration endpoint movement. One executes an actual Linux overwrite move, restores into the original
destination and verifies displaced dirty archives/physical rollback bytes. Two
history controls cover ordinary/grouped EOL publication. This qualifies the
restoration kernel. App lifecycle arbitration, public operation events/history,
mixed groups, API-reference removals and the extension gateway remain unfinished.

The restoration continuation passes all 1,540 Rust all-target tests (1,191 library;
23 ignored), 748 host tests, formatting, strict locked all-target Clippy and build.
The final suite uses two test threads after an unrelated parallel theme timeout
passed alone. A snippet Redo count-shadowing regression introduced during the
change was corrected; the recorded trace fixtures remain unchanged. The native
EOL terminal journey passes Revert LF→Undo CRLF→Redo LF→Undo/Enter/Save with exact
Unicode bytes and baseline preservation. The existing Explorer resource-lifetime
journey also passes. Both run without Node and restore terminal mode on exit.
