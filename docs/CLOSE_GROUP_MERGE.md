# Native Close Group merge

Implemented with local native qualification; fresh platform checks and parent
stack integration remain gates. This operation on existing nonempty editor
groups needs no Node or language server.

F1 exposes **View: Close Group**, invoking the original
`workbench.action.closeGroup` on the current group. It merges every source tab
into the most recently active other group, then removes the source layout leaf.
Dirty documents remain retained; no save/discard dialog or document close is
introduced. The sole native group is an owned no-op. Recovery overflow refuses
without removing buffers.

This native palette exposure uses the original toolbar title and command ID.
Pinned VS Code 1.95 does not list that ID in its command palette. Its
**View: Close Editor Group** palette entry invokes the distinct
`workbench.action.closeEditorsAndGroup`, which this slice does not register.
**View: Close All Editors in Group** invokes `closeEditorsInGroup` and retains
its existing guarded close/review behavior. The formerly misleading native
Close Editor Group label is corrected; these operations are not aliases.
The upstream Ctrl/Cmd+W Close Group binding only applies to an empty group;
native groups do not retain empty groups, so no such default binding is added.

## Transaction and retained views

An owned Groups plan stages destination tab order/modes/MRU and exact original
source-to-target Membership mappings. The nested Layout removal and all at most
128 disjoint Document view leases are prepared before any live publication.
Fallible reservation and an 8 MiB aggregate prepared-view allowance include
lease vector capacity and newly prepared view payload; retained document/history,
HashMap capacity and allocator headers are excluded. This is not an allocator
OOM recovery guarantee or a whole-editor memory cap.

The original globally active source tab copies its exact source selections and
viewport to its destination, with fresh private pair/snippet authority. Inactive
rows keep an exact retained target view, including private sessions. If no exact
target view exists, they receive a fresh native origin; a target tab Membership
alone never proves stored geometry. This origin policy is a documented gap from
upstream editor memento-cache restoration. Historical views are never temporarily
activated during preflight, and the operation does not loop single-tab transfers.

After proof revalidation, Groups publishes once, followed by the already prepared
Layout and infallible document leases. A merge-aware pane projection skips normal
new-tab origin resets. Source group historical views retire, pointer authorization
is invalidated, and all source text, document IDs, paths, save generations and
Undo/Redo histories remain retained. A later refusal drops stages without a
logical model/group/view publication; preparatory backing-map capacity may grow.

Shared target tabs reuse their exact TabId. Source previews become committed;
an unrelated target preview remains. Pinned default-right sticky/indexed-open
semantics follow the prepared engine rather than a simple sticky union. Native
global membership MRU mapping is explicitly native, not an upstream editor-service
MRU fidelity claim.

Accepted saves and Save As retain their original Document/destination and publish
successful receipts even after merge. Retired source Memberships cannot close a
reused destination tab. Remaining captured close batches and unapproved source
modals retire without canceling independently approved persistence work.

## Scope and evidence

Changed merges require root-scoped `workbench.editor.openPositioning: "right"`;
other valid policies refuse before publication. Language overrides are ignored
by the existing workbench settings rules. Explicit merge removes its source even
when `workbench.editor.closeEmptyGroups` is false. Runtime bounds remain four
groups, 128 tabs per group and 512 memberships. No copy/drag, context-menu group
arguments, retained empty groups, join/maximize aliases or pixel-layout parity
is added.

The pure engine's twelve focused tests and thirteen historical-view tests pass
separately. All eleven App integrity tests pass, including actual gated source
Save/Save As receipts, late preflight refusal and independent approved closes.
The candidate passes 1,088 locked all-target Rust tests across 51 reports with
four test threads; twenty optional tests remain ignored. Formatting and strict
all-target locked Clippy pass.

All three new native-only terminal workflows pass in debug. Five existing group,
sticky, transfer, nested-layout and smart-typing scripts also pass with their
data/history assertions intact: 29 successful debug reports across six scripts.
All three new journeys also pass against the optimized executable. Fresh
platform CI and parent stack integration remain gates. No local upstream editor
instance was launched for this slice.

Pinned source audit uses commit `912bb683695358a54ae0c670461738984cbb5b95`:
[mergeGroup](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/browser/parts/editor/editorPart.ts#L873),
[original command handler](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/browser/parts/editor/editorCommands.ts#L716),
[distinct menu and palette titles](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/browser/parts/editor/editor.contribution.ts#L578).
