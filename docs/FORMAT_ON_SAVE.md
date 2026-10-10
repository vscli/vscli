# Native format-on-save

## Behavior

Set `"editor.formatOnSave": true` in user or workspace settings, optionally in
a language block such as `[cpp]`. The default is false. `editor.formatOnSaveMode`
defaults to `"file"`; modified-lines modes and an explicitly selected extension
`editor.defaultFormatter` are read and reported, but are not implemented on save.
An explicit Save uses the already synchronized native language-server formatter
for an ordinary named visible model. It then captures a fresh snapshot and uses
the existing guarded background save worker. No JavaScript runtime is needed.

After-delay autosaves skip formatting. First save, Save As, hidden targets,
unsupported servers and unsupported formatter selection still save current bytes
with a bounded skip notice. Formatter errors and invalid edit batches also fall
back to saving current bytes with a notice. The notice survives the successful
save receipt. These fallback policies are native product behavior, not a claim
of complete upstream failure-policy parity.

Formatting has a 1500 ms native save deadline. An expired or canceled callback
continues to occupy its actual native formatter lane until a valid matching
response arrives or the server retires. A later Save can persist raw bytes while
that lane is occupied. A late reply cannot mutate the document. A reply delivered
after the deadline is also discarded when the callback has already settled.
The transport's separate 15-second deadline sends cancellation once and retains
actual occupancy; neither deadline proves the server has stopped working.

Escape cancels an unapproved formatting save and its close continuation, retaining
unsaved work. Close/Quit waits for a current formatted save and the real persistence
receipt. Interrupt shutdown cancels formatting without waiting for an advisory
server cancellation, then settles authorized filesystem work before recovery.

## Ownership and bounds

One latest save intent carries model/destination/continuation metadata. One logical
formatter participant retains the originating server identity and request token,
model/path/revision/text epoch/saved revision/save generation, settings ownership
and workspace. The filesystem snapshot is captured only after a valid edit batch.
Typing and Save in the same input batch are admitted after normal language polling
synchronizes the current model, without adding full-text synchronization to the
Save key callback.

The reply must retain its originating synchronized lifetime. Edits, edit→Undo,
close/reopen, settings ownership changes and server replacement retire stale work.
Switching to another file, editing that unrelated model or moving the caret does
not redirect an otherwise current save. Valid edits map the current selections
and shared views through one native undoable batch.

All TextEdits are staged before mutation: at most 4096 edits, 4 MiB aggregate raw
and EOL-expanded replacement text, and a 32 MiB final native document. Coordinates
are strict UTF-16 scalar boundaries within logical line contents; CR/LF terminator
units, surrogate interiors, malformed fields, overlaps and equal-start ambiguity
are rejected. Indexed Rope slices avoid scanning a long line for every coordinate.
Validated byte-identical, empty and null results preserve epoch and Redo and add
no Undo step. Ordinary native Format Document shares the transport lane and strict
staging helper; its existing user-invoked ownership still requires the active model.

The existing LSP full-text synchronization path still flattens and serializes changed
documents on the caller. Native editable files can reach 32 MiB, while escaped LSP
frames must fit 16 MiB. This feature does not solve that performance gap or qualify
formatting every native-editable file. Transport queues remain bounded independently
of the logical save participant.

## Reference and qualification

Pinned [VS Code 1.95 save participants](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/contrib/codeEditor/browser/saveParticipants.ts)
skip short-delay AUTO formatting and register source actions before formatting.
The new [actual reference observer](../tests/vscode-reference/save-formatting.README.md)
records four Linux cases and 18 snapshots. Explicit formatting adds a separate
Undo step after typing; null formatting adds none. After-delay autosave makes
zero target formatter calls. In the source-action case, organize imports runs
before formatting and the first Undo reverses formatting while preserving the
source-action edit. Source-action execution is an outstanding native feature;
capturing that reference behavior does not implement it.

The frozen Linux trace SHA-256 is
`c73214599b36ff19ca3668a9878ec6d9a430dbf3ace004443c03a75dd4024346`;
raw evidence SHA-256 is
`6302870577407d375f542f2cbbad73056b457e9ec310df62283cce5fec7406dd`.
Setup-only provider registration probes are separated from original target
commands; failures and null outcomes are retained. The native consumer matches
three implemented cohorts and 13 snapshots against both the frozen Linux trace
and its retained fresh raw capture. It checks exact text/disk bytes, dirty state,
primary selection, Undo/Redo, formatter inputs/counts and committed save
notifications. The fourth source-action cohort is retained as an explicit future
boundary. Fresh macOS/Windows reference captures and full platform qualification
remain pending. CI captures all four cases on each platform and runs the native
consumer against that fresh evidence.

Local checks currently pass eight staging integrity tests, six actual framed
transport tests, language-scoped settings tests and twelve controller journeys,
including a positively settled response delivered after the save deadline.
Five native terminal journeys pass with exact Unicode/CRLF
bytes, original Save/Undo/Redo, formatter errors, deadline/late-response behavior,
after-delay exclusion, no-LSP/no-Node fallback and terminal restoration. Actual
installed clangd 23.1.1 also passes settings-based automatic startup without
`--lsp`, exact C++ Unicode/CRLF formatting, one Undo, settings-disabled raw Save,
Redo and clean restart with empty PATH and missing Node. The complete local suite
passes 757 ordinary Rust tests across 40 suites (20 optional integrations ignored),
formatting and strict all-target Clippy. All five new terminal journeys also pass
on the optimized executable. The [isolated routine-editing benchmark](PERFORMANCE.md#native-format-on-save-core-baseline-2026-10-10)
records 40 successful launches and 1,600 keys with mixed measurements and no speed
ranking. Fresh platform qualification remains pending.

## Outstanding work

Save-time source actions and action resolution/commands, extension formatters and
`willSaveWaitUntil`, trim-whitespace/final-newline participants, modified-range
formatting, competing formatter selection, dynamic registration, Save All, and
Save As/hidden-target participant ownership require separate implementation and
qualification. Each valid participant batch is atomic; an entire asynchronous
participant chain is not an all-stage rollback or a single Undo transaction.
This milestone does not establish full VS Code parity.

## Long workspace paths and visible save notices

The save receipt now displays a workspace-relative path for ordinary in-workspace targets. The macOS PR64 failure saved the expected raw bytes but its absolute temporary path consumed the status row before the formatter-failure notice. The terminal fixture now exercises a deliberately long workspace path at its existing width, retaining exact-byte and committed didSave checks and requiring the complete failure notice. Fresh local and platform qualification remains required; outside-workspace paths and arbitrarily long notices still use the terminal display bounds.

## Merged platform qualification

PR #64 passed all six required checks for reviewed head
`6ceac94b7c28038feaf0ee9abf4fdcb6e93435e3` in run 38032272972 and
merged through protected main. Linux/macOS/Windows freshly captured the pinned
reference and ran their native consumers; Linux/macOS also ran the terminal
journeys. The deliberate long-workspace-path regression passed locally and on
those terminal platforms. Earlier pending statements describe the historical
candidate; later source-action participants need their own qualification.
