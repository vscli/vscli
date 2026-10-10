# Same-group tab reordering

The native candidate implements the original **View: Move Editor Left** and
**View: Move Editor Right** commands. Linux and Windows profiles use
Ctrl+Shift+PageUp/PageDown; the macOS profile uses Cmd+K, Cmd+Shift+Left/Right.
The original bindings have no `when` clause. Terminal applications retain the
existing terminal-input routing; the terminal journeys exercise editor input.

A command captures the current group's exact ordered proof and active membership,
then moves that tab one position without changing its active document. An edge
no-op does not publish a group change, invalidate a sealed frame or commit a
preview. The ordinary physical-key input lifecycle still applies. Actual movement
commits a preview; crossing into or out of the sticky prefix pins or unpins the
moved tab. These policies follow the pinned VS Code 1.95.0
[move wrappers](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/workbench/browser/parts/editor/editorActions.ts)
and [group model](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/workbench/common/editor/editorGroupModel.ts).
No new upstream behavioral capture is claimed.

The engine rotates an existing bounded group after checked generations and
fallible result reservation. Group/tab/document identities, activation MRU,
layout ratios, document history and per-group views are retained. A changed order
retires stale presentation targets and the remaining captured batch-close work.
An already approved Save→Close continues against its captured original membership;
its actual save receipt remains valid even after another tab moves. A newly sticky
original rejects an ordinary captured close after the saved receipt publishes.
Reordering cannot retarget that close to the newly active or neighboring tab.

Eight focused engine tests, five App integrity cases and the original-platform
key test pass. The App cases use actual BeforeCommit/BeforeFinish worker gates,
shared Unicode/CRLF views and pending Redo. All fourteen existing sticky-save
ownership tests pass unchanged. The candidate passes 1,001 all-target Rust tests
across 51 reports with four test threads (20 optional cases ignored), formatting
and strict locked all-target Clippy. The first default-concurrency run timed out
in the unchanged relative-theme test; it passed alone, and the bounded-concurrency
full run passed. The failure log is retained; no deadline or assertion was relaxed.

Four terminal journeys pass across five isolated native-only sessions with empty
PATH, missing Node, disabled LSP, original commands, byte-exact saves, Undo/Redo,
clean-session visual order, sticky crossings and observable preview replacement.
The existing sticky, preview, group-tab and nested-layout PTY suites also pass:
25 reports across the five scripts. Fresh platform CI remains required before
merge.

Cross-group transfer, mouse tab dragging/reordering, graphical mouse-close policy,
spatial focus and the distinct upstream Close Group merge behavior remain separate
work. The macOS key mapping has native unit coverage prepared; the Unix PTY suite
uses the Linux profile and does not claim native macOS GUI or Windows ConPTY input
qualification. The editor stays usable without JavaScript.

The original terminal's `allowChords` behavior can intercept macOS chord sequences
independently of its shell-skip list. This slice retains native terminal routing;
terminal-child chord/configuration equivalence is outstanding. Editor input
qualification does not establish that equivalence or physical shortcut delivery
through every host terminal.


## Qualification on merged native reliability repairs

The reordering implementation is rebased onto main
`ba024b1d98b3cbcab3d47739bd92c04844b3e492`, including merged display
foundations and the fixture-publication and syntax-cancellation repairs in PR
#79. The feature implementation and terminal fixtures remain byte-identical to
preceding head `2c5b0ab`; inherited native repairs change the integrated source.
Current local qualification passes 1,101 locked all-target Rust tests across 51
reports, with 23 optional integrations ignored, formatting and strict locked
all-target Clippy. Five debug terminal scripts pass 21 reports covering
reordering, nested layouts, sticky tabs, source actions on save and syntax color
stability. Reordering and sticky terminal scripts pass nine reports on the
optimized executable. No local reference-editor instance was launched.

The first full run on this parent timed out in the existing relative-theme
fixture. The unchanged focused test passed. Test-only timeout diagnostics now
retain the actual call site, pending-receiver state, theme and notice; the
original five-second deadline and assertion remain. The full integrated run
above passed, but the original timeout's cause is unproven. Its failure log is
retained; these diagnostics do not claim to repair theme loading.

A scoped read-only source review found no actionable blocker in staged rotation,
sticky/preview transitions, document/view identity, accepted Save-to-Close
ownership, source-hit retirement or original platform bindings. These tests and
review do not establish cross-group transfer, physical keyboard delivery on all
terminals or full workbench parity. Fresh protected exact-head checks and parent
integration remain required before merge.

Reordering's earlier head `7370e03` passed all six checks in
[run 38072995481](https://github.com/vscli/vscli/actions/runs/38072995481).
That historical result does not qualify the new integrated head. Fresh exact-head
CI is required before this candidate merges.
