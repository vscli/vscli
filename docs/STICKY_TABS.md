# Sticky tabs: candidate contract and qualification

The native sticky-tab subset is implemented on the feature branch. Its engine,
App save/close ownership, settings, original shortcuts and tab presentation pass
local Rust and terminal qualification. The optimized executable passes the same
sticky journeys and adjacent workflows. Fresh protected platform checks remain
required before merge. This is a bounded editor
slice, not full VS Code workbench parity.

## Native behavior

Sticky state belongs to an exact group/tab/document membership, rather than the
shared document. Pinning promotes a preview to committed, moves the membership to
the leading sticky section and preserves its document identity, text, dirty state,
selection, Undo/Redo and save ownership. Unpinning moves it to the beginning of the
ordinary section. Splitting the shared document creates an ordinary membership.
Metadata-only movement retires old tab-hit geometry through the group UI proof.

`workbench.action.pinEditor` and `workbench.action.unpinEditor` use the original
Ctrl/Cmd+K then Shift+Enter chord with opposite `activeEditorIsPinned` conditions.
The actual active membership supplies that context; no active editor reports false.
The command helper must reject or safely ignore pinning with no active membership.
`workbench.action.keepEditor` retains its separate Ctrl/Cmd+K then Enter chord.
`workbench.action.closeActivePinnedEditor` is the original forced-close command:
it can close the active membership in either mode through ordinary dirty handling.

The tab strip displays ◆ before a sticky filename, keeps ● for native revision
dirtiness, and keeps preview italics independent. Tiny strips retain a visible
marker and an exact bounded hit region. Rendering changes no group/model state
and performs no filesystem work. It retains existing bounded groups, memberships,
filename segmentation and proof-qualified focus hits. Presentation reuses the existing bounded
group/tab iteration.

## Root close policy and retained work

`workbench.editor.preventPinnedEditorClose` accepts exactly `keyboardAndMouse`
(default), `never`, `mouse` and `keyboard`. The highest valid root user/workspace
value wins; malformed higher values warn and fall back to lower valid values.
Language overrides warn and are ignored without rewriting imported source files.
The typed policy retains independent keyboard/mouse flags, but this candidate has
no mouse-close gesture: current tab mouse hits only focus tabs. Parsing the mouse
flag is not evidence that graphical mouse-close behavior is implemented.

For keyboard protection, ordinary Close focuses a nonsticky recent membership
in its current group, then another group, without closing either tab. With no
eligible fallback it does nothing, including a dirty all-sticky group. Forced
Close still follows Save/Discard/Cancel. Close All Editors and Close Editors in
Group capture only nonsticky memberships, not a later sweep over the group.

Save acceptance owns its captured document and destination. A successful receipt
publishes even after unrelated group changes. Remaining batch work can retire
while an already accepted exact close completes; current nonsticky eligibility
is checked independently from membership identity. Pinning a captured nonsticky
close target protects its current membership without canceling disk receipt
publication. No new tab is included retroactively in a close batch. Fourteen actual SaveWorker gate tests cover these App ownership rules
independently from presentation tests.

## Evidence and outstanding qualification

The independent pinned VS Code 1.95.0 Linux observer records **18 cases, 65 target
snapshots and 91 setup observations**. Its raw corpus remains unchanged; setup
observations are not additional native target gestures. Trace SHA-256:
`e8aa4a1ed449ebbf2fd6b7e2910156efb1835cc8d88d19582b8ff2ee9d1175db`.
The observation and primary command/configuration source informed this scope.
The local all-target run passes 936 Rust tests across 51 reports, with 20 optional
integrations ignored. This includes 36 engine tests, seven strict consumer
integrity/replay tests, five admission-refusal tests, fourteen real save-worker
ownership tests and six settings/key/presentation tests. Formatting, strict
all-target locked Clippy and six observer path/event tests pass. The complete
source-matched Linux native comparison covers all 65 target and 91 setup states;
four named final-empty-group boundaries retain raw upstream inventories.

Five new terminal reports across nine isolated sessions pass on both debug and
optimized binaries.
They use original Linux-profile enhanced keys with empty PATH, no Node and no LSP:
pin/unpin metadata and dirty history; protected local/global MRU and forced close;
captured group/all subsets; all four root policies and workspace/language scope;
and shared Unicode/CRLF carets/Undo/Redo. Existing original-group close and held
save assertions remain unchanged. The complete ordinary Unix terminal regression passes all 31 scripts and
145 reports. The optimized executable passes 71 reports across eight scripts:
sticky, preview, groups, saving, formatting, source actions, adjacent Escape and
the 35-report smoke suite. The [isolated paired core benchmark](PERFORMANCE.md#native-sticky-tabs-core-baseline-2026-10-10)
passes 80 launches / 3,200 keys, including a focused repeat, with mixed results
and no editor ranking or active-sticky performance claim. Fresh platform CI
remains required before merge. Mouse-close gestures and physical terminal/key delivery are unqualified.

Sticky state is transient; clean-session restoration commits historical tabs and
does not persist stickiness. Nested/resizable layouts, graphical sticky rows,
double-click behavior, mouse closing, arbitrary upstream editor types and complete
VS Code workbench parity remain outside this subset. Native editing and these
settings/commands require no JavaScript runtime.

Primary pinned source: [editor command registrations](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/browser/parts/editor/editorCommands.ts#L1167-L1218).
