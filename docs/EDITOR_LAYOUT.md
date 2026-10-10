# Native editor layout foundation

The native layout engine is registered and has focused qualification. App,
terminal presentation, original split/resize commands and clean-session schema
integration remain separate work. The current editor still uses its existing
equal group presentation until those callers are wired and tested. This page
does not claim an implemented nested workbench or full VS Code parity.

## Pure geometry and structural admission

`editor_layout::Layout` owns at most four native GroupId leaves, seven tree
nodes and three stable dividers. Local right/down splits retain the other
branch; removing a leaf collapses its empty branch and retains surviving
subtree identity, axis and weights. The foundation also represents left/up,
but the App's current group insertion order needs a separate operation before
those directions can be wired safely.

Prepared plans validate the entire replacement before mutation. A private
immutable revision identity plus checked counters rejects retired plans, foreign
layouts and independently staged equal-generation forks. Geometry-only resize
changes ratios without touching models, text, selections or Undo. Callers must
still retire terminal/container presentation authorization independently.

Integer terminal geometry reserves one cell per divider. Preferred leaf minima
are sixteen columns and four rows; hard minima are one cell per axis. If the
complete topology cannot fit, only the active group receives the available
rectangle. Hidden group entries remain present with zero rectangles; topology,
ratios and all document views remain unchanged. Overflowing public Rect fields
are rejected; Ratatui's valid constructor-clamped rectangles remain supported.

## Strict wire metadata

The separate SavedNode format uses bounded group indices and positive integer
weights; no runtime GroupId, TabId, SplitId or revision is deserialized.
`validate_saved_layout` validates counts, exact appearance order, every leaf,
weights, depth and empty-tree rules without allocating runtime identities or
reading files. Callers must bound encoded bytes before deserialization. The
runtime importer shares that walker and separately validates supplied native
identities before preparing any replacement.

This pure format is not a new session schema. Existing strict schema-1/2 readers
and schema-2 publication remain unchanged. A future version needs separate DTOs,
complete metadata byte accounting, fresh native IDs, clean-membership pruning on
a cloned tree and preservation of authoritative recovery models.

## Qualification and integration gates

Ten initial focused cases pass, including exhaustive small/extreme rectangle
partitions, local mixed splits/collapse, same-axis reset, resize bounds,
equal-generation fork/ABA proof rejection, malformed late wire data, capacity
and checked-counter rollback. The first overflow assertion incorrectly assumed
Rect::new preserved an overflowing width; it was corrected to construct actual
malformed public fields and retain a positive constructor-clamped case. Runtime
geometry behavior was unchanged.

All fifteen focused cases pass, including one additional single-leaf/foreign-
geometry identity case and four metadata-only whole-wire validation cases.
The registered foundation passes 951 all-target Rust tests across 51 reports,
with 20 optional integrations ignored; formatting and strict locked all-target
Clippy pass. These checks qualify the pure engine, not App or terminal behavior.

Required App integration stages both Groups and Layout before publishing models,
removing views or applying an accepted close. Structural publication must be
one infallible synchronous boundary; render must never reconcile topology.
Presentation must map exact group IDs, retire stale geometry on Resize/sidebar/
panel/overlay changes, preserve shared historical views and keep tiny-terminal
fallback observational. Original keyboard/palette resizing is the first planned
controller; pointer dragging remains deferred.

Actual pinned upstream layout observations and native terminal behavior need
separate evidence. Public upstream layout ratios describe CSS geometry, while
native geometry uses terminal cells. A binary/native and n-ary/upstream topology
projection must be explicit and retain raw trees; fabricated ratio equality is
not a compatibility test. The prepared reference contract has not been captured.
