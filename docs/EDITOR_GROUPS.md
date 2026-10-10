# Editor groups and preview tabs

VSCLI now implements an ordered tab list for each editor group. The native
editor works without Node or an extension host. This document describes the
implemented slice. Local native, debug and optimized terminal qualification passes;
fresh platform CI remains a separate pending check.

## Opening, splitting and navigating

Opening a file selects its existing tab in the current group, or inserts a new
tab immediately to the right of that group's active tab. A file already open
in another group can therefore also have a tab in the current group. Opening
the same file again in the same group does not duplicate or reorder its tab.
Explicit opens are committed. Preview-enabled callers can replace the current
group's previous eligible clean preview, as described below.

Splitting creates an adjacent group containing only the active tab. Historical
tabs in the source group stay there. The current layout candidate places up to
four groups in a bounded nested Right/Down tree; only the selected leaf splits.
Keyboard/palette ratio resizing and its separate qualification are described in
[the layout contract](EDITOR_LAYOUT.md). Each group has its own tab strip above
its Breadcrumbs and editor. The selected
tab remains visible when the strip is narrow; long labels are clipped by
grapheme, controls are sanitized, and filename processing is bounded.

The commands use these distinct navigation rules:

| Original command | Behavior |
| --- | --- |
| `workbench.action.nextEditor` / `workbench.action.previousEditor` | Traverse tabs in display order, cross into the next or previous nonempty group at a boundary, and wrap across groups. |
| `workbench.action.nextEditorInGroup` / `workbench.action.previousEditorInGroup` | Wrap within the current group's tab list. |
| `workbench.action.focusFirstEditorGroup` through `workbench.action.focusFourthEditorGroup` | Focus the named existing group and its active tab. |
| `workbench.action.splitEditor`, `workbench.action.splitEditorRight`, `workbench.action.splitEditorDown` | Split the active tab into a new adjacent group within the four-group limit. |
| `workbench.action.closeActiveEditor` | Close the exact active tab membership. |
| `workbench.action.closeEditorsInGroup` | Review and close the captured group's tab memberships. |

The original default next/previous editor shortcuts are Ctrl+PageDown /
Ctrl+PageUp on Linux and Windows, and Cmd+Alt+Right / Cmd+Alt+Left on macOS.
The [keyboard rules](../README.md) and [usage guide](USAGE.md) explain profile
selection and terminal constraints. A terminal must deliver a distinguishable
shortcut to VSCLI; combinations intercepted by the terminal cannot be handled
by the editor. No additional default binding is invented for group-local
navigation. Commands are available through F1.

Mouse tab targets carry the exact group, tab and document membership plus the
engine's current UI-generation proof. A stale target cannot focus a replacement
tab after a group/tab A→B→A transition. Frame changes, including the Keyboard
Inspector and tiny-terminal fallback, clear targets for strips that are absent.

## Shared text, independent views and closing

A file shared by several groups has one native Document identity, text buffer,
Undo/Redo history, dirty state and saved baseline. Each group keeps its own
caret, selection and scroll view for that document, including historical tabs.
Switching tabs restores that group's view. Editing shared text maps retained
views through the same document edits; splitting does not copy text or history.

Closing one of several memberships removes that tab and its view, while the
other tabs retain the shared model and unsaved text. Closing the last dirty
membership uses Save / Discard / Cancel. Closing an active tab selects the
most recently used surviving tab in that group, rather than always selecting
the adjacent tab. An empty group is removed; closing the final editor returns
to the welcome screen without creating an Untitled buffer.

Save→Close captures the originating membership. A successful asynchronous
save receipt cannot close a different tab merely because focus moved, and an
old continuation cannot consume a close/reopened replacement membership.
Postauthorization edits remain dirty and require review; Escape can cancel a
deferred close while already authorized filesystem work continues. Saves still
publish against their original document identity, independently of group focus.

Close Group captures its membership list and structural generation. Tabs opened
or replaced while review is pending are excluded from an old batch. Dirty models
still owned by another group do not require global discard. Membership changes
retire stale review, preserving buffers. Earlier successful saves remain real
disk writes if a later review is canceled. This is a guarded native batch-close
contract; the pinned reference below does not qualify every upstream dirty
dialog or partial-close sequence.

Group identities, tab identities and presentation generations use checked
monotonic counters. The engine admits at most four groups, 128 tabs per group
and 512 memberships. Capacity or identity exhaustion rejects the operation
before changing ownership. Recovery remains authoritative: when retained
buffers cannot fit the bounded group representation, VSCLI keeps the legacy
workbench and every buffer, with an explicit layout-unavailable notice. Its active
recovery buffer remains keyboard-editable; no unproved source/tab/divider pointer
map is admitted. The existing native save inventory still requires at most 128
retained models. It does not truncate recovered work to satisfy either limit.

Recent-file and Reopen Closed Editor admission failures retain existing models,
pending Redo and closed-file history. Rejected asynchronous loads also resume
navigation-history recording, so later Back/Forward remains usable.

## Clean-file session continuity

The schema-3 candidate records an ordered clean-file table, groups and tabs, each
membership's view, active tabs/group, recent-tab order, nested axes/integer weights
and per-tab sticky flags. Flags must form each group's ordered sticky prefix. Restore
reads each unique file once and stages fresh native document/group/tab/split identities,
views and tree before publication. Preview memberships restore committed; their mode
remains transient.

Strict schemas 1 and 2 remain readable without accepting new tree, sticky or preview
fields. Schema 1's complete global file order becomes the first group; additional
legacy panes become groups containing their visible file and recorded view. The
previous active pane selects the active group, and recent order starts with its active
tab followed by file order. Older flat orientations normalize to a native flat tree
with nonsticky committed tabs. Reading or failed migration does not overwrite the
original slot. Only successful atomic publication installs a complete schema-3 encoding.
The complete write envelope, including tree/workspace/sticky flags, must fit the 1 MiB
limit. An already legal legacy slot remains readable if its new encoding is too large.

Capture omits dirty/untitled tabs and prunes their empty leaves on a bounded clone;
surviving topology/weights remain. It never changes live layout/views or pending saves.
All-dirty capture does not erase a previous clean session. Existing recovery retains
its live topology, mode and view authority; appended clean files do not replace those
with the saved layout. Fourteen new backend/App integrity tests and clean nested
restart checks pass locally; platform evidence remains pending.

Clean metadata is bounded to 32 distinct files, four groups, 128 selections per
view and 1 MiB of serialized metadata; restore reads at most 128 MiB of file
content in aggregate. Dirty and Untitled text and Undo history belong to the
separate recovery mechanism, rather than this clean-file layout. Existing
recovered models and their selections take precedence during session restore.
Already grouped recovered models can span all four groups. Appending clean
files counts only new or unassigned models against the active group's free
slots; a valid distributed layout is retained beyond 128 total documents.

Malformed references, duplicate memberships, unsupported schemas, missing files,
stale App context and budget failures reject publication without replacing the
live workbench or erasing prior session metadata. The existing leased-slot,
atomic-publication worker keeps filesystem work outside rendering and input.
Explicit final close can publish an empty layout; a failed restore does not
silently replace a saved session with empty state.

## Evidence and remaining scope

An actual unchanged VS Code 1.95.0 Linux capture records **8 cases / 70 public
membership frames**, product commit
`912bb683695358a54ae0c670461738984cbb5b95`. Preview was explicitly disabled;
each original target command ran once, with independent public-state settlement
and no preferred-output retry. Source, input, trace and evidence hashes are
retained with the [observer and qualification scope](../tests/vscode-reference/editor-group-tabs.README.md)
and [raw Linux baseline](../tests/vscode-reference/baselines/1.95.0/editor-group-tabs/linux.json).

The capture covers right insertion, current-group opening, split-active-only,
global versus local traversal, historical/shared views, shared dirty tab close,
recent-tab close fallback and empty-group removal. It does not qualify native
save continuations, session migration or recovery by itself. Native engine,
renderer, ownership, persistence and lifecycle tests pass locally. The candidate
passes 835 ordinary Rust tests across 42 reports (20 optional integrations ignored),
formatting, strict all-target locked Clippy, 133 extension-host tests and 25 Python
tooling tests. The strict native membership consumer also passes against the
fresh eight-case/70-frame raw capture, separately from the frozen baseline.

All 27 existing ordinary Unix PTY scripts pass with 126 workflow reports; six
new group-tab journeys pass across seven real terminal sessions. They cover
original shortcuts, per-group historical views, shared Unicode/CRLF Undo/Redo,
last-owned dirty closing, clean restart and exact originating-tab closure after
a held source-action Save. Existing Breadcrumbs tests now locate the exact row
following the group strip, retaining all symbol/picker/byte/history assertions;
the original stale-row failure is preserved locally. This totals 28 scripts and
132 successful workflow reports. A gated native save regression additionally
proves Close Group retains a model whose Undo made it clean while an authorized
snapshot was pending, then preserves its restored text as dirty after receipt.

The optimized executable additionally passes all six group journeys, seven
ordinary save journeys, five format-on-save journeys and five source-action
journeys, with original assertions and terminal restoration intact. Two reviewed
regressions cover full-group Recent/reopen loads and clean-session admission into
129 documents distributed across two groups. The isolated
[core comparison](PERFORMANCE.md#native-editor-group-tabs-core-baseline-2026-10-10)
records 40 successful launches/1,600 keys with mixed observations and no speed ranking.
The group-tab PR passed all six protected checks on its reviewed head and is
merged on main. Windows ConPTY, graphical input and every upstream tab policy are
not qualified by these Unix terminal checks.

The final upstream close retains one empty active group. VSCLI's welcome
workbench has zero engine groups and no document. This is an explicit internal
layout boundary: absent editor/text and unchanged disk behavior may be compared,
while the group inventories differ. The raw observation is preserved.

At the original group-tab milestone, sticky/pinned modes, preview policies and
nested resizing remained follow-ups. The later sections below describe sticky and
preview work; [the layout contract](EDITOR_LAYOUT.md) records current nested splitting
and keyboard resizing. Tab movement/reordering, drag-and-drop, broader close
policies, multi-workspace/hot-exit continuity and terminal persistence remain gaps.
This slice does not establish complete VS Code editor-group or workspace parity.


## Preview engine and reference qualification

The native membership engine supports explicit preview/committed admission,
exact authorized replacement, Keep Editor and permanent promotion of every
preview of an edited shared document. Admissions preflight capacities, fresh tab
identity and checked counters before replacing or promoting a membership. A split
keeps the source mode and creates a committed destination. App now enables Explorer left-click previews and accepted Quick Open previews
when their settings allow them. Sticky ordering and preview session persistence
remain pending.

The new actual pinned reference contains 11 cases and 86 frames. The strict
consumer verifies every raw frame and compares all 68 frames of the nine supported
preview/committed cases. The two sticky cases (18 frames) remain verified raw
evidence and are excluded explicitly by name. All 21 engine tests, nine consumer
integrity/replay tests, formatting and strict all-target locked Clippy pass.
Both unchanged frozen and fresh Linux captures pass native replay. Public version
and object allocation remain raw evidence outside the native comparison. The
capture records a product.json hash; it does not contain an executable hash.

Untitled public-document dirty state and tab cleanliness are compared separately.
After typing then Undo to empty, the captured document stays dirty while its tab
is clean. Native API snapshots now preserve that distinction without changing
revision-based tab/close decisions or Undo/Redo. See the
[reference contract](../tests/vscode-reference/editor-preview-tabs.README.md).


## Preview callers, Keep Editor and retained work

`workbench.editor.enablePreview` defaults to true, and
`workbench.editor.enablePreviewFromQuickOpen` defaults to false. These workbench
settings use user/workspace root scope; language overrides are preserved in the
imported values and reported as ignored. An ordinary Explorer left click requests
a preview when enabled. Enter/Right in Explorer, explicit Open File, CLI resources,
history/navigation, symbols, settings, new Untitled documents and extension
`showTextDocument` remain committed. Accepted Ctrl+P results request previews only
when both settings are enabled. Moving selection inside Quick Open does not open
resources in this slice; graphical double-click is not implemented.

A preview filename is italic, subject to terminal support. Cursor motion, rendering
and reopening that same preview preserve its mode and view. Original Ctrl+K then
Enter (Cmd+K then Enter on macOS), or F1 → **View: Keep Editor**, commits the exact
tab in place without changing text or adding an Undo step. This is separate from
sticky pinning; `activeEditorIsNotPreview` and `activeEditorIsPinned` stay distinct.
Typing, accepted completion/snippets and provider/workspace edits permanently
commit every preview of the edited shared document before follow-up UI work. Undo
to clean text never demotes a committed tab. Splitting keeps the source mode and
creates a committed copy with an independent historical view.

Replacement requires an exact clean file-backed membership and current model,
group, settings and loader ownership. Pending/authorized saves, source-action or
formatter participants, Save As, close review, settings writes and file operations
protect the old preview. If it cannot safely be replaced, a successful admission
keeps it committed before inserting the new preview. Full/counter-exhausted
admissions reject before removing any authoritative model or view. A failed atomic
multi-model promotion permanently disables further preview admission for that
instance, preserving Undo-clean models and Redo after a counter failure.

Disabling preview commits existing previews; enabling it again does not demote
historical tabs. Session schema 3 restores clean memberships committed, including
previous previews, while separately restoring saved sticky prefixes. Preview mode
remains transient; platform qualification, complete settings/open policies
and full workspace parity remain pending.

Local qualification on the main-based preview branch: 890 all-target Rust tests
across 50 reports pass, with 20 optional integrations ignored. This includes nine
preview integrity/replay tests. Formatting, strict all-target locked Clippy and
seven observer path/event tests pass. The refreshed actual Linux observer records
all 11 cases and 86 frames, exactly matching the original trace; its separate
source-matched baseline preserves the original evidence unchanged.

Helper, promotion-failure fence, original-profile chord/style, settings ABA,
Untitled API lifecycle and actual authorized-save tests cover native ownership
boundaries. All 30 ordinary Unix PTY scripts pass with 140 reports, including six
new preview journeys across eight sessions. These use original keys, empty PATH
and missing Node for the preview journeys. All 133 extension-host and 25 Python
tooling tests passed earlier on unchanged component source. The optimized executable passes all 66 reports across seven scripts: preview,
group tabs, saving, format-on-save, source actions, adjacent Escape and the full
35-report smoke suite. Its native source tree exactly matches this reviewed
branch; later changes only affect qualification, documentation and observers.
The [isolated paired benchmark](PERFORMANCE.md#native-preview-tabs-core-baseline-2026-10-10)
passes 40 launches and 1,600 keys with mixed measurements; it does not establish
an editor ranking or active-preview performance. All six required checks passed for the reviewed preview head in
[CI run 38046439264](https://github.com/vscli/vscli/actions/runs/38046439264),
including fresh Linux/macOS/Windows reference comparisons and Unix terminal
workflows. Preview tabs are merged on main in PR #67.

The previous Windows run failed in the observer's canonical-path equality, and
the macOS smoke test edited before positively observing Quick Open publication.
The observer now classifies known files through canonical containment, retains
bounded supplemental callback evidence and propagates callback failures. The
smoke test waits for the exact result and loaded document before editing.
The reviewed preview CI run passed afresh on all three platforms; a local Linux
pass alone would not qualify Windows or macOS. Native Explorer click is implemented; this keyboard corpus
does not qualify graphical single/double-click parity.


## Sticky engine and transactional admission foundation

The engine now supports a bounded sticky prefix, exact pin/unpin ordering,
committed split destinations, local and global nonsticky membership MRU, and
atomic captured nonsticky close subsets. Pin/unpin preserves native shared
identity and historical views. Original nonsticky transitions remain covered.
The App integrates original pin/unpin commands, native markers and captured
close-policy ownership. Its local qualification is recorded in
[the sticky contract](STICKY_TABS.md); platform checks and optimized evidence are
separate gates.

MRU storage can refuse admission recoverably. Untitled creation, loaded history
navigation, Problems reveal and loaded native rename targets now admit their
engine memberships before publishing models, focus or edits. Multi-file rename
stages all required memberships before changing any buffer. Five new refusal
integrity tests use a test-only one-shot reservation seam and preserve exact
models, selections, epochs, Undo/Redo, history and disks. They test error handling,
not a real allocator or memory-pressure workload; production has no test seam.
The legacy rename restrictions remain in place.

The strict consumer verifies the complete actual 39-file source-matched sticky
baseline before native replay. It compares all 18 cases, 65 target snapshots and
91 setup states. Four named final-empty-group observations preserve the upstream
one-empty-group/native-zero-group boundary explicitly; no target is removed.
Versions/object allocation and callback scheduling remain raw evidence outside
native equality. The observer hashes the selected executable, not an entire
installation. See the [reference contract](../tests/vscode-reference/editor-sticky-tabs.README.md).

The integrated candidate passes 936 all-target Rust tests across 51 reports,
including 36 engine tests, seven consumer integrity/replay tests, five admission
refusal tests and fourteen real save-worker ownership tests; 20 optional tests
are ignored. Formatting, strict all-target locked Clippy and six observer
path/event tests pass. A fresh complete unattended Linux capture has the exact
original target trace. All 31 ordinary Unix PTY scripts pass with 145 reports;
the optimized executable passes 71 reports across eight scripts. Five new
sticky reports cover nine native-only sessions. The isolated paired
[core benchmark](PERFORMANCE.md#native-sticky-tabs-core-baseline-2026-10-10)
passes 80 launches / 3,200 keys, including the retained small-file repeat.
Results vary; no editor ranking or active-sticky performance is established.
Fresh platform checks remain required before merge.

## Same-group reorder engine candidate

The `feat/native-tab-reordering` branch adds synchronous staged same-group
reordering, before App/key/terminal integration. Exact ordered group proofs and
Membership IDs are validated even for a clamped edge no-op. An actual move
commits a preview; crossing the sticky prefix pins or unpins that membership.
Tab/document/group IDs, active editor, activation MRU and historical document
views stay intact. Checked interaction/membership generations and fallible result
reservations precede bounded in-place rotation. No new tab/group/model is admitted.

Eight focused native tests cover visual order, sticky transitions, preview versus
edge no-op, stale order/inverse proofs, replaced membership, counter refusal, all
512 legal memberships and shared Unicode/CRLF document views with pending Redo.
The engine candidate passes 995 all-target Rust tests across 51 reports, formatting
and strict locked all-target Clippy, with 20 optional cases ignored. App commands,
original profile keys, accepted-save/batch-close integration and Unix terminal
journeys are separate pending qualification. This does not implement cross-group
transfer, drag or original Close Group merging, and claims no new actual upstream
behavioral capture. Policy follows the pinned original source.
