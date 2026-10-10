# Editor groups and committed tabs

VSCLI now implements an ordered tab list for each editor group. The native
editor works without Node or an extension host. This document describes the
implemented slice. Local native, debug and optimized terminal qualification passes;
fresh platform CI remains a separate pending check.

## Opening, splitting and navigating

Opening a file selects its existing tab in the current group, or inserts a new
tab immediately to the right of that group's active tab. A file already open
in another group can therefore also have a tab in the current group. Opening
the same file again in the same group does not duplicate or reorder its tab.
All tabs are committed: opening another file does not replace a preview tab.

Splitting creates an adjacent group containing only the active tab. Historical
tabs in the source group stay there. Up to four groups share the available
space equally, using the existing horizontal or vertical split orientation.
Each group has its own tab strip above its Breadcrumbs and editor. The selected
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
workbench and every buffer, with an explicit layout-unavailable notice. It does
not truncate recovered work to satisfy the tab limit.

Recent-file and Reopen Closed Editor admission failures retain existing models,
pending Redo and closed-file history. Rejected asynchronous loads also resume
navigation-history recording, so later Back/Forward remains usable.

## Clean-file session continuity

Session schema 2 records an ordered clean-file table, ordered groups and tabs,
each tab's view, each group's active tab and recent-tab order, active group and
split orientation. Restore reads each unique file once, installs shared native
documents and allocates fresh runtime group/tab identities after validation.
It retains inactive-tab views as well as visible views.

Schema 1 remains readable. Its complete global file order becomes the first
group; additional legacy panes become groups containing their visible file
and recorded view. The previously active pane selects the active group. Since
schema 1 stored no historical group-tab usage, migration initializes recent
order with the active tab followed by the remaining file order. Reading v1
does not overwrite its metadata; replacement occurs only when a complete,
valid schema-2 snapshot is successfully published.

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
Fresh platform checks remain pending. Windows ConPTY, graphical input and every upstream tab policy are not
qualified by these Unix terminal checks.

The final upstream close retains one empty active group. VSCLI's welcome
workbench has zero engine groups and no document. This is an explicit internal
layout boundary: absent editor/text and unchanged disk behavior may be compared,
while the group inventories differ. The raw observation is preserved.

Follow-ups include preview replacement, sticky/pinned tabs, tab movement and
reordering, drag-and-drop, resizable or nested layouts, non-default opening and
close policies, multi-workspace/hot-exit continuity and terminal persistence.
This slice does not establish complete VS Code editor-group or workspace parity.
