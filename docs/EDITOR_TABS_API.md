# Public native tabs

The optional extension host exposes `window.tabGroups` over the actual native
editor groups, including inactive tabs and separate memberships for a document
shown in multiple groups. Native editing and tab commands remain usable without
Node. This milestone covers text tabs; it does not establish complete extension
API compatibility.

```js
const groups = vscode.window.tabGroups;
const inactive = groups.activeTabGroup.tabs.filter(tab => !tab.isActive);
const closed = await groups.close(inactive, true);
```

`all`, `activeTabGroup`, each group's `tabs`, `activeTab`, `viewColumn` and
`isActive`, and each tab's label, input, dirty, pinned, preview and active state
follow native inventory updates. Public tab/group objects retain identity for
their original membership lifetime. A shared document has a distinct tab in each
group. Native sticky tabs map to `isPinned`; ordinary committed tabs have
`isPinned: false` and `isPreview: false`.

Public tab/group objects and getter arrays are frozen. Event records are frozen;
their opened/closed/changed arrays are mutable, matching the pinned source.
`onDidChangeTabs` and `onDidChangeTabGroups` run after the shared document, editor
and tab mirrors have all been published. Bounded full native snapshots can
coalesce intermediate updates; this differs from upstream's individual tab
operation stream. Retired objects remain readable and cannot authorize a close
of a replacement tab or group.

All seven stable input value classes are exported: `TabInputText`,
`TabInputTextDiff`, `TabInputCustom`, `TabInputWebview`, `TabInputNotebook`,
`TabInputNotebookDiff` and `TabInputTerminal`. Constructors preserve provided
values without validation, cloning or freezing. Native inventory currently
produces text inputs. Exporting the other constructors does not implement diff,
custom editor, webview, notebook or terminal tab backends.

`close` accepts one native tab/group or a homogeneous array. An empty array
returns true without a native request. Forged objects, mixed arrays and retired
handles reject. Native requests retain the original caller, published tab/group
membership, engine, workspace and settings scope; public fields cannot retarget
them. Closing a shared view keeps the remaining document and its dirty history.

Closing a dirty final membership uses the existing native Save/Discard/Cancel
dialog. Cancel returns false; Discard closes only that membership. Save waits for
the actual native disk receipt, publishes document save/resource events, then
closes and acknowledges. Untitled Save opens native Save As; canceling it returns
false with the original document retained. Array/group closes are sequential:
canceling a later dirty tab can leave already approved earlier tabs closed.
This is a partial close, not an atomic multi-tab transaction.

For tab closes, `preserveFocus: true` restores the original surviving membership.
The pinned VS Code main-thread group-close implementation ignores preserveFocus;
VSCLI follows that group boundary. The native welcome screen and last-tab close
retain one real empty group and layout leaf, with no active tab or visible editor.
Empty runtime leaves are pruned from clean session metadata; they do not create
phantom saved documents.

Inventories are bounded to four groups, 128 tabs per group, 512 memberships,
128 models and 2 MiB of tab metadata. Close requests share the existing eight
retained service slots. Human dialogs pause only the originating command's wait;
close deadlines remain bounded. Timeout reports an unknown outcome and retains
host capacity until an exact terminal receipt. Caller retirement removes only
its pending close interest; an already authorized native save still publishes
its disk receipt.

## Evidence

All 641 host tests (24 new) and 1,304 Rust all-target tests pass, including 982
library tests and five actual selected-host/App tab workflows. The 23 ignored
tests grant no qualification. Four new tab PTY journeys and 26 relevant existing
group/welcome/editor/snippet/save/untitled/virtual journeys pass. Build, formatting
and strict all-target locked Clippy pass. Implementation and tests are tracked in
the live [extension API ledger](EXTENSION_API.md).
Host tests cover identities, values, events, forged/retired handles, ownership,
bounds, mirrored publication, and retained exact close acknowledgements. Native
tests and actual PTY workflows cover shared/sticky tabs, clean tab arrays and
group close, welcome, Cancel/Discard, file Save, untitled Save As, peer history
and disk integrity. Authored tests grant qualification only after execution.

Observable source contracts are pinned to VS Code 1.141.0 commit
`2a59476c9bfcb90b3ddc372c36762471b7dfad1c`:
[extension-host tabs](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostEditorTabs.ts),
[native tab bridge](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/browser/mainThreadEditorTabs.ts),
and [public declarations](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vscode-dts/vscode.d.ts).
