# Directional editor-group focus

The native candidate implements the original View: Focus Left/Right Editor Group
and View: Focus Editor Group Above/Below commands. Linux and Windows profiles use
Ctrl+K followed by Ctrl+Left/Right/Up/Down; macOS uses Cmd+K followed by
Cmd+Left/Right/Up/Down. The original profile rules have no `when` clause. These
four wrappers focus an existing group and wrap at the outer boundary. They do
not split, open an editor or create a missing group. Numeric-group focus and
side-open creation semantics remain separate.

The source policy is pinned VS Code 1.95.0:
[wrappers](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/workbench/browser/parts/editor/editorActions.ts#L276-L403),
[group MRU selection](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/workbench/browser/parts/editor/editorPart.ts#L275-L295),
[boundary overlap](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/base/browser/ui/grid/grid.ts#L76-L120)
and [wrapping](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/base/browser/ui/grid/grid.ts#L625-L645).
The native query uses strict positive perpendicular overlap, excludes corner-only
contact and chooses the most recently active group when several neighbors share
an edge. It uses no center-distance or nearest-pane heuristic.

Native adjacency follows the retained nested tree's exact integer weight ratios
on a logical unit square. Visual one-cell dividers have zero logical width for
this query. Private checked rational intervals fit the four-leaf/seven-node/
three-level limits and require no dynamic result allocation, filesystem operation
or JavaScript. The query changes no layout identity, generation, counters or
models. The App validates current group/layout agreement and exact source
membership before synchronously focusing the selected group's current active tab.
A changed focus retires old pointer seals. Text, Undo/Redo, shared document identity,
per-group selections and layout ratios remain owned by their original models.

This logical geometry differs from actual terminal floor/minimum-size clamps and
upstream CSS pixel boxes. Very close weighted boundaries or tiny clamped panes
may select different candidates; pixel-equivalent spatial navigation is not
qualified. Tiny terminal projection still navigates retained hidden groups and
redraws the newly active one. Whole-screen tiny mode grants no pointer authority.
Recovery-overflow mode has no supported group tree and refuses directional group
focus while preserving buffers. A single group wraps to itself and enters editor
focus without creating another group. Empty welcome is harmless.

Focus does not mutate tab lists or retire captured remaining group-close batches.
Independently approved Save/SaveAs receipts and original Save→Close ownership stay
intact after another group receives focus. Native restored group MRU is seeded
active-first then appearance order; the prior upstream session's MRU is not
reconstructed. Terminal child input keeps existing routing. The macOS terminal
chord/allowChords configuration policy is separate from editor-profile key mapping.

Local qualification passes eight pure adjacency tests, five App
integrity cases (including actual BeforeCommit/BeforeFinish save-worker gates),
one platform-profile rule test, and three native Unix terminal journeys. These
cover strict corners, weighted/extreme/deep adjacency, all outer directions,
unknown/self/empty cases, invalid private trees and checked arithmetic, MRU ties,
shared Unicode/CRLF carets and pending Redo, stale seals, tiny topology, original
receipts and remaining close batches. The terminal suite uses missing Node,
disabled LSP and empty PATH with original chords/palette commands and exact saved
bytes/Undo/Redo. The full locked all-target Rust suite passes 1,015 tests across 51 reports
(20 optional tests ignored), using four test threads. Formatting and strict
all-target locked Clippy pass. Five native Unix PTY scripts pass 22 reports: three
new focus journeys plus tab navigation, reordering, nested layouts and sticky tabs.
The tiny-screen fixture uses four retained groups so it actually exceeds the
projection minimum before testing hidden-group focus. Fresh platform CI remains
pending.
No new upstream editor instance or behavioral capture is claimed.

Cross-group tab transfer, mouse divider/tab dragging, left/up split commands,
spatial movement of groups and the distinct original Close Group merge command
remain separate work. This feature does not establish full VS Code layout parity.
