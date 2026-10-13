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
surviving destination restoration, full history/view archives, public file events,
WorkspaceEdit resource admission and public Undo/Redo remain unfinished. No new
terminal, third-party-extension, cross-platform or full API parity is implied.
