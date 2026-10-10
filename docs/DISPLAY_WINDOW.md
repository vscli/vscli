# Native immutable display window

This foundation prepares one bounded Wrap Off viewport from an immutable
DisplayRows projection. Painting, source hits and caret projection can use the
same prepared grapheme runs; current App rendering and input still use their
existing paths. Fold commands, wrapping and presentation authority are pending.

Clones retain private window identity; equal fresh preparations do not revive
it. The exact source projection is retained. Runs carry absolute byte, scalar
and logical cell ranges plus clipped viewport cells. Tabs retain configured
stops; CRLF, combining sequences and wide clusters retain native affinities.
Partial clusters expose complete source ranges and must be painted as blank
fragments rather than sliced UTF-8. Fold bodies and off-window positions cannot
produce source hits or a visible caret. Header decoration is separate metadata;
App must partition marker cells before permitting a source-cell action.

Preparation visits only visible logical lines. Their whole physical byte lengths
are checked before segmentation: 64 KiB per line and 256 KiB total. Up to 4,096
rows and 262,144 cells are admitted. Actual row/run vector capacities, Arc counters,
inline data and cross-chunk scratch capacity are accounted against 8 MiB.
Fallible reservations and invalid anchors/options reject unpublished candidates
without changing the old window. Zero dimensions are inert; a valid 1×1 window
remains representable. No whole-source fold discovery runs in this API.

Hit and caret queries binary-search these same prepared runs rather than
segmenting a prefix on each query. This is a bounded algorithmic foundation,
not an editor latency benchmark. App must still account for all unique windows
retained by current, cached and held frames, and seal exact document, membership,
view, geometry, settings and interaction ownership before publishing or clicking.

Nine local tests pass with independent Unicode/tab cell and source-offset
oracles. They cover every offset, multiple tab sizes and horizontal origins,
CRLF, cross-chunk clusters, folded bodies, private identities, invalid/zero
windows, capacity accounting, maximal admitted scans and refusal of an oversized
visible line while retaining prior authority. A large document with an off-window
long line demonstrates that preparation does not inspect unrelated rows.
These tests qualify this immutable API, not terminal gestures or upstream parity.


## Rebased foundation qualification

The five foundation commits are rebased onto nested-layout head
`c9682f5a1a879370803c86715fc6ebf623a303f0`, retaining merged stable save policy
and the separately bound corrected sticky observer. The rebased source passes
1,084 locked all-target Rust tests across 51 reports, with 23 optional
integrations ignored, formatting and strict locked all-target Clippy. Three
native debug terminal scripts pass 16 reports covering nested layouts, sticky
tabs and source actions on save. No local reference-editor instance was launched.

These checks establish foundation and existing-workflow regression evidence.
They do not establish visible folding: App command dispatch, shared frame/input
authority and aggregate reservations are being integrated in the separate
user-visible slice. Fresh exact-head protected CI and parent integration remain
required before this foundation merges. Earlier all-green CI results apply to
their original heads, not this rebase.

The foundation is now based on merged nested layouts in main
`fb360539a060f118db4a10239e9d5181410e0a2e`. Layout PR #69 passed all six
required checks on reviewed head `8664ba9` in
[run 38073389609](https://github.com/vscli/vscli/actions/runs/38073389609).
Foundation head `045cc6e` passed all six checks in
[run 38072484939](https://github.com/vscli/vscli/actions/runs/38072484939).
The main rebase changes inherited documentation only; native source, tests,
workflows and reference inputs remain byte-identical to that qualified
foundation head. Fresh protected exact-head checks remain required. Visible
folding has a separately prepared source candidate and is not yet qualified.
