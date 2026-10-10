# Native per-view folding ownership

This foundation keeps fold intent and prepared row maps in document views, separate
from text Undo/Redo. It does not expose Fold/Unfold commands or change terminal
rendering, movement, clicks or popup placement. Those paths must use the same
row mapping before the feature is usable.

Each view owns a fresh private lifetime, checked publication/selection clocks,
bounded desired regions and optional immutable rows. Document source epochs and
policy lifetimes guard asynchronous results. Returning to identical text through
Undo, returning to old options, or reusing a numeric view ID cannot revive an old
result. Checked publication uses an exclusive Document lease; dropped leases
preserve state. Explicit split preparation copies bounded selections, viewport and
fold intent with fresh ownership. Ordinary new views start expanded.

Edits immediately retire prepared rows in every retained view. A scalar journal
lets worker preparation map strictly disjoint fold anchors; touched or boundary
anchors retire permanently even through Undo. Journal rollover discards intent
whose provenance is no longer complete. Complete primary and secondary selected
intervals remain visible. Selection observation fences replies, while direct
hidden assignments conservatively make a map unavailable. App reveal, completion,
snippet and mouse paths still need completed-interaction hooks.

Preparation admits Wrap Off only, sources up to 2 MiB/100,000 lines, five retained
views, 5,000 regions per view, 10,000 selections and 256 journal changes. Document
publication checks at most 20,000 desired anchors locally; this is not App-wide
memory accounting. Worker replay is bounded at 256×5,000 mappings with cancellation
and deadline checks. No discovery or region replay runs in ordinary source edit
hooks. Core 32 MiB unwrapped editing, native saves and text history remain usable
when folding preparation is refused.

The Document bridge now supplies borrowed exact model/view/selection proofs to
one owned worker lane. Discovery and preparation share the original snapshot,
private handoff authority and absolute deadline; a later intent cannot be
replaced by a stale discovery reply. Public outcome fields cannot redirect that
handoff. Clear/Unfold All renews private lifetime without scanning source or
selections, even when pending work or exhausted clocks would refuse preparation.
Finite Fold/Unfold actions target the original collapsed mask simultaneously;
Fold All keeps complete selected intervals visible. These APIs remain separate
from App command registration and source-cell authority.

Qualification: sixteen actual Document integrity tests pass locally. They cover
Unicode/CRLF persistence, pending Redo, stale edit→Undo replies, touched-anchor
no-revival, split versus ordinary insertion, close/recreate lifetimes, selection
ABA, complete secondary intervals, malformed batches, journal rollover, policy
ABA, generation exhaustion, oversized selection refusal and cancellation/deadline
refusal. New cases cover borrowed proof ownership, simultaneous nested targeting,
clear after oversized-source/cohort refusal and pending-only no-op clear retirement.
Full-suite evidence is recorded in [the display-row foundation](DISPLAY_ROWS.md).
These tests qualify Document ownership rather than App gestures or upstream parity.

App integration must retain the existing single actual folding worker, add global
quotas, preserve original save receipts, and atomically publish complete paint/hit/
caret/movement mappings. Fold mementos, wrapping, language-provider ranges and
upstream cursor-adjustment semantics remain outstanding.
