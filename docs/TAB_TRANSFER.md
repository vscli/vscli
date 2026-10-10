# Native cross-group editor transfer foundation

The prepared transfer engine stages destination admission and source retirement
as one bounded transaction. App can inspect the projected group order, prepare
layout and document views, then publish after exact proof revalidation. No command
or terminal binding is exposed by this foundation.

Existing destinations admit a committed editor right of the active tab, outside
the sticky prefix. Duplicate documents reuse the destination tab identity, become
committed and inherit source sticky state without unpinning an already sticky
target. Unrelated preview tabs remain. New destinations are nonempty groups after
the source, with fresh group/tab identities; source groups collapse only when
empty. Source membership always retires. An approved Save→Close for that identity
cannot follow the document to its destination.

Runtime limits remain four groups, 128 tabs per group and 512 memberships.
Destination deduplication is allowed at tab capacity. Creating a new group at the
four-group limit is refused even if subsequent source collapse would return to
four. Exact private lineage, current UI/ordered group proofs, counters and original
staged state guard publication. Foreign engines, stale replies and divergent
clones cannot publish. No-op plans validate their source/destination without stage
allocation or counter increments. Changed plans use bounded fallible storage;
failed preparation or late validation leaves the live engine intact.

Document view transfer uses an exclusive prepared lease. It copies at most 10,000
current selections and scroll state, with checked snippet clocks, before group
publication. Publication retires source snippet/pair/cursor-history ownership,
removes the source view and stages the destination projection without changing
focus, text epoch, document identity, baseline or Undo/Redo. Unrelated historical
views remain owned independently. Fold intent starts expanded at the destination;
future folding integration needs its own checked quota/admission hooks. Exact
upstream caret restoration through Undo after transfer remains unqualified.
Private editing sessions have a fresh view lifetime plus checked source/destination
generation retirement. Undo cannot revive generated brackets or snippet sessions
from an old target view, including one closed before transfer. Exhausted clocks
refuse before publication and leave the source and destination intact.

Local focused evidence: 57 editor-group tests pass, including thirteen new
transfer integrity tests; four new Document tests pass with concrete Unicode/CRLF
saved-byte, save-snapshot, Redo, dropped-lease and malformed/capacity oracles.
The full locked all-target suite passes 1,032 tests across 51 reports with four
test threads (20 optional tests ignored). Formatting and strict all-target locked
Clippy pass. The following App integration separately exposes commands and keys; see [native transfer workflows and evidence](TAB_TRANSFER_APP.md).

The App integration adds original platform keys, right/down side-opening policy,
joint create-before-remove layout publication, save-worker gate tests and native
PTY journeys. The foundation itself grants no UI authority or full VS Code group
parity. Directional transfer/left-up creation,
copy wrappers, generic argument schemas, nondefault insertion and retained empty
groups remain separate scope.
