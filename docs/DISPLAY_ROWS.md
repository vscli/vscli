# Native display-row foundation

This candidate adds pure folding and display-row modules. It does not expose
Fold/Unfold commands or word wrapping in the editor. App rendering, navigation, reveal paths, popup anchors and source clicks still need
combined integration and terminal qualification.

`DisplayRows` owns an immutable Rope snapshot, options and private preparation
identity. A clone preserves identity; a fresh equal preparation does not revive
old authority. Document-owned publication now checks exact document, text epoch, policy and
private view lifetimes; App still needs membership and presentation guards. Visible anchors, ordinals, clamped movement, grapheme-aware cell
hits and logical-offset projection share one mapping. Hidden logical positions
identify their visible header and require a reveal before editing. Hidden rows
cannot produce source hits. CRLF source offsets and configured tab stops are
preserved; inside-cluster projection has explicit before/after affinity.

The ordinary unwrapped identity path accepts the native 32 MiB source limit and
uses Rope line metadata without discovering folds or building a line inventory.
Prepared folding separately admits at most 2 MiB, 100,000 lines, 5,000 regions and
256 nesting levels. Validation rejects stale, crossing and duplicate ranges as a
whole. Discovery accepts cancellation and a deadline. No worker or queue is
allocated by these display-row APIs. Word wrapping is explicitly refused. Width zero
permits row traversal but no actionable cell projection.

Cell mapping scans graphemes up to the requested line prefix, using the existing
chunked native helper; a cross-chunk cluster can allocate. This foundation is not
a cached long-line projection or a typing-latency claim. Per-view display state
must remain separate from document Undo. Edits must retire folded projections
before a new normal frame; pending preparation cannot hide new source text.

Qualification includes eight retained folding tests and nine new display-row
tests. Independent visibility intervals and contiguous Unicode/tab cell oracles
cover nested folds, EOF, CRLF, combining sequences, wide characters, control
characters, cross-chunk clusters, zero/narrow widths, extreme movement, malformed
late input, private preparation identity and the distinct source admission limits.
These are native foundation tests, not upstream command or desktop parity.

The additional native worker and controller remain separate from App. One actual
thread and a one-result channel serve one latest intent. Ordinary queued demand
retains metadata only. The authorized discovery-to-preparation handoff retains
its original Document-owned snapshot and absolute deadline; it never recaptures
a newer model or allows edited public outcome fields to redirect preparation. Cancellation retires interest without freeing capacity. Poll
joins the finished thread before releasing its slot, including a result sent
before thread exit. Private view/options lifetimes, exact membership, text epoch,
selection and interaction proofs guard publication. Ordinary polling borrows
selections; explicit preparation admits at most 10,000 protected selections.
No result itself changes a document or view. The caller must capture source from
the exact retained model and recheck the outcome immediately before publication.

If the last model closes, retire its interest and use `poll_retired` to drain the
actual worker without fabricating a current model. Runtime disable must retain
the same canceled worker until actual exit. Explicit bounded shutdown stops
admission; a timeout keeps that occupied stopped instance. Final Drop cancels
without blocking, and a detached thread owns its immutable source until exit.
The six-second deadline is checked around bounded preparation phases and does
not provide a hard wall-clock execution guarantee.

Seven real-worker tests and eleven controller tests cover held cancellation, replies
before exit, selection protection, bounded malformed admission, expiration,
edit/Undo epochs, options/view lifetime changes, latest/coalesced demand,
shutdown timeout and closing the last model while a canceled worker remains
occupied, original discovery handoff authority and stale-phase refusal. These tests
qualify controller contracts. Sixteen additional actual Document
integrity tests qualify per-view intent, immutable checked publication, edit
journal retirement, complete selection protection and independent split ownership;
see [per-view folding ownership](FOLD_VIEW_STATE.md). App integration is outstanding.

The candidate passes 1,038 all-target Rust tests across 51 reports (20 optional
cases ignored), formatting and strict locked all-target Clippy. No terminal
gesture changes in this foundation require a new PTY claim; the App feature does.
