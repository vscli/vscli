# Native nested editor layout

The current candidate integrates bounded nested groups, terminal presentation and
keyboard/palette resizing. Split Right replaces the selected leaf with two side-by-side
groups; Split Down replaces it with two stacked groups. Other branches retain their
layout and document views. App/UI integrity tests pass locally; native terminal
journeys and platform checks are qualified separately. This page does
not claim full VS Code workbench parity or completed platform qualification.

## Commands and terminal behavior

Ctrl+\ / Cmd+\ runs the original `workbench.action.splitEditor` command. F1 →
**View: Split Editor Right** and **View: Split Editor Down** run the corresponding
original IDs. Splitting copies only the current tab and its view into a new adjacent
group. Up to four groups retain independent ordered tab inventories, selections and
scroll positions, while shared documents retain one text buffer, Undo history and
save identity. Ctrl+1 through Ctrl+4 (Cmd on macOS) focus existing groups.

F1 exposes these original commands without inventing default resize shortcuts:

| Palette command | Native operation |
| --- | --- |
| View: Increase/Decrease Current View Width | Grow/shrink the current group by four terminal columns at its nearest column split. |
| View: Increase/Decrease Current View Height | Grow/shrink the current group by two terminal rows at its nearest row split. |
| View: Reset Editor Group Sizes | Reset same-axis sibling weights while retaining mixed-axis branches. |

All operations use the shared allocator's bounds and may clamp or remain unchanged.
Width/height resizing requires editor focus. A palette command whose overlay retired
pointer geometry can retain one original resize intent until the next actual editor
repaint. It expires after two seconds or if its original group/focus/layout changes;
a second pending resize does not replace it. Ratio changes retire old hit areas but
do not change document text, selections, Undo, group membership or native save/source
ownership. Native four-column/two-row increments are an explicit terminal adaptation,
not equality with upstream CSS-pixel resize increments.

Every group has a strip; focused Breadcrumbs reserve their existing row where space
allows. Input maps use exact current group/tab/document membership and the actual
text rectangle after the line-number gutter. A divider consumes pointer input
without editing text. **Mouse divider dragging is not implemented.** Left/up splits,
layout presets, merge/maximize, spatial group focus and outer workbench-part sizing
also remain outside this slice.

The legacy palette label **View: Close Editor Group** invokes
`workbench.action.closeEditorsInGroup`: it reviews the captured nonsticky tab subset
through existing dirty-close protection. It does not implement the distinct upstream
`workbench.action.closeGroup` command. Closing a group's final actual membership
collapses its leaf; surviving subtree/group identities, ratios and shared models
remain. Pinning, changed membership and asynchronous Save→Close retain their existing
exact-target guards. No extra close alias or inferred upstream group-removal behavior
is introduced by layout rendering.

## Bounded geometry and authorization

`editor_layout::Layout` holds at most four native GroupId leaves, seven tree nodes,
three levels and three dividers. Structural preparation validates Groups and Layout
before publishing either, then applies existing document/view/close behavior. Rendering
never calls membership reconciliation or creates groups. Ratio-only plans validate
private immutable revision identity and checked counters, rejecting foreign layouts,
retired plans and equal-generation forks before mutation.

Integer geometry reserves one terminal cell per divider. Preferred leaf minima are
sixteen columns by four rows; hard minima are one cell per axis. If the entire tree
cannot fit hard minima, only the active group receives the available rectangle;
hidden leaves remain retained with zero rectangles. A whole-screen terminal below
20 columns or six rows shows the existing enlargement message. Both presentations
retain topology, ratios, dirty models and historical views. Existing viewport
keep-caret-visible clamping remains; rendering is not claimed to preserve every
scroll offset.

A sealed Presentation owns a checked geometry epoch and immutable frame identity,
with at most four pane hits and three divider hits. All final rectangles are validated
before publication; a failed or covered frame grants no source authorization. Resize,
sidebar/panel/overlay changes, gutter or tab-width changes, displayed text epochs and per-view
viewport changes retire old maps before another input. A→B→A cannot revive a stale
hit. Fixed-size presentation and layout guards perform no filesystem or JavaScript
work, and no whole-document flattening.

Recovery exceeding the group-tab capacity retains every model. Its current authoritative
buffer is drawn in an explicit recovery view and remains keyboard-editable/navigable;
no fake GroupIds or source/tab/divider pointer maps are granted. The existing native
save inventory limit remains 128 retained models. More models cause an explicit
pre-dispatch refusal with work and disk intact; safely closing enough dispensable
models permits the normal bounded save. Layout does not bypass this guard or drop
recovered work to satisfy it.

## Metadata and restoration boundary

The bounded SavedNode DTO contains group indices and positive integer weights, not
runtime GroupId, TabId, SplitId or revision identities. Whole-wire validation checks
order, exact leaves, counts, depth, weights and empty-tree rules before allocating
runtime identities or reading files. Encoded metadata still requires caller byte
bounds.

The schema-3 candidate records mixed topology, positive integer split weights and
per-tab sticky flags alongside clean group/tab/view metadata. Sticky flags must form
an ordered prefix in each group; preview modes remain transient and restore committed.
Native document, group, tab, split and revision identities are never read from disk:
newly restored documents and staged Groups/Layout receive fresh runtime identities.

Strict schema-1 and schema-2 DTOs remain readable with their original field sets;
they do not accept tree, sticky or preview additions. Older flat orientations normalize
to a bounded native tree with nonsticky committed tabs. Reading or failed migration
never rewrites the original slot. A complete schema-3 snapshot is published only after
successful bounded validation and atomic replacement. The 1 MiB write limit counts the
whole new envelope, including workspace, tree and each sticky flag, before allocating
serialized bytes or touching the slot. A legal near-budget legacy slot remains readable
and recoverable even if its larger schema-3 replacement would exceed that limit.

Capture omits dirty/untitled memberships and prunes their empty leaves on a bounded
clone, collapsing removed branches while preserving surviving nested axes and weights.
No live group/model/view or pending save is changed by capture. If no clean files remain,
an incidental empty capture does not erase the prior session; an explicit final close
can record an empty workbench. Existing recovery buffers, views, topology, tab modes
and pending save receipts remain authoritative: appended clean files do not replace
that live layout with the saved tree. A fresh workbench instead validates/configures the
whole file batch and stages memberships, sticky prefixes, views and tree before publishing.
Pending restore guards include exact immutable layout identity as well as document,
selection, membership and interaction proofs, so ratio-only changes and equal-generation
forks cannot authorize an old restore. The fourteen new integrity tests pass locally;
platform qualification remains separate.

## Evidence and remaining qualification

All fifteen pure-layout cases pass, covering mixed splits/collapse, exhaustive
small/extreme rectangles, same-axis reset, resize limits, private geometry identity,
foreign/equal-generation forks, ABA, malformed late whole-wire data, capacity and
counter rollback. The integrated candidate passes 987 ordinary all-target Rust tests
across 51 reports, with 20 optional integrations ignored. Formatting and strict
locked all-target Clippy pass on the same source. The App/UI cohort adds 22 cases,
and schema 3 adds fourteen integrity cases. The full session-module filter passes
32 tests; platform checks remain separately recorded qualification.

Actual App/TestBackend checks cover shared Unicode/CRLF carets and Undo, stale source
geometry, sidebar/panel/Inspector and resize ABA, tiny projection, recovery, original
palette resizing and authorized saves through ratio changes. The tab-width regression
rejects stale pointer input through a 4→2→4 setting round trip without changing text,
group ownership, history or disk. Four native-only nested terminal reports pass:
local mixed splits/collapse, shared Unicode/CRLF Save/Undo/Redo, and original
palette resizing/reset, and tiny fallback followed by clean nested-ratio restart.
These cover five sessions with empty PATH, missing Node and language services disabled.
The crash/recovery extension-document workflow caught a missing startup group
initialization; moving it to recovery admission passes both original restart
reports. All 31 prior Unix scripts passed across 145 reports during App integration;
after schema 3, eight relevant scripts pass across 40 reports on the final source.
The optimized executable passes 75 reports across nine Unix scripts, including
the four nested journeys. The [isolated paired core measurements](PERFORMANCE.md#native-nested-layouts-core-baseline-2026-10-10)
retain 80 launches and 3,200 keys with mixed results and higher candidate typing
tails in the large-file repeat; active nested-pane performance is unqualified.
Platform qualification remains separate.

A local raw Linux capture against VS Code 1.95.0 commit
`912bb683695358a54ae0c670461738984cbb5b95` completed ten cases, 39 target commands,
49 initial/target frames and 28 setup observations. It retains the
raw public n-ary layouts/ratios, group memberships and document/view observations;
the raw capture is not a completed native comparison or a macOS/Windows result.
Native tests can proceed without launching VS Code. Upstream CSS/n-ary trees and native cell/binary
trees need an explicit bounded comparison scope; raw geometry must not be rewritten
or normalized into fabricated equality. Drag, session serialization and full layout
parity are outside that capture's stated scope.
