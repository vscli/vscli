# Pinned committed editor-group tab reference

This standalone observer captures unchanged VS Code **1.95.0**, product commit
`912bb683695358a54ae0c670461738984cbb5b95`. Its evidence is public text-tab and
editor behavior, not a private workbench-layout/MRU implementation or complete
VS Code tab parity claim.

The local Linux capture passed **8 named cases / 70 snapshots** in eight fresh
isolated profiles/processes (24.434 seconds). Platform captures and native
consumer qualification remain separate; this reference pass is not their pass.
The preserved raw run is `target/editor-group-tabs-first-reference`; the local
log is `/tmp/vscli-editor-group-tabs-reference.log`.

```sh
cd tests/vscode-reference
npm ci --ignore-scripts
cd ../..
node tests/vscode-reference/editor-group-tabs-run.cjs target/editor-group-tabs-reference
```

Linux requires an owned usable display. The local capture used an isolated
Xvfb with `-displayfd`, then terminated only that owned server. The existing
supervisor bounds the worker to 180 seconds and cleans its process tree. The
runner creates isolated user-data, workspace, and installed-extension folders;
macOS starts under `/tmp` and the observer resolves existing fixture paths to
avoid `/var`/`/private/var` alias ambiguity. An optional second runner argument
selects a named case for diagnosis; a complete corpus still requires all eight.

Every case explicitly disables preview and sets right insertion, recent-tab
close fallback, close-empty-groups, and active-group opening (`revealIfOpen`
false). Effective values and the inventory of the actual original target
commands are checked and retained before targets. Each case begins with one
separately recorded `openTextDocument`/`showTextDocument` API setup for a.txt.
Every following open, split, focus, traversal, cursor motion, type, Undo, and
close uses its original public command once. There is no output-shaped target
retry, private editor-stack access, hidden command substitute, dialog discard,
or preferred-result wait.

The cases cover:

- Right insertion beside a historical active tab, and focusing an existing tab
  without changing its ordered membership.
- Splitting only the active tab, then opening in the current group.
- Global next/previous crossing groups versus in-group wrap.
- Historical caret restoration, copied split selection, independent shared-file
  views, and a first opening in another group starting at its own selection.
- Closing a dirty membership while another group retains the same model, then
  native Undo and a clean last close. The observer positively requires shared
  dirty membership before invoking that close, avoiding a dirty-last-tab dialog.
- Recent-tab close fallback that differs from the adjacent tab.
- Removing an empty secondary group and closing the final clean group.
- Opening a resource already present elsewhere into the active group.

Each snapshot keeps ordered `window.tabGroups`, active membership, raw public
pinned/preview/dirty flags, active and visible text editors, resource, exact
text, UTF-16 and scalar selections, document version/EOL, and exact fixture
disk bytes. Observer-local `documentObject` ordinals record equality of public
TextDocument objects within the run; they are not native model IDs or a claim
of matching allocation/close lifetimes. All four fixture disks remain unchanged.
The raw evidence includes setup, per-target gestures/events, aggregate events,
and independent state-settlement timings. Acknowledged commands are followed
by 100 ms unchanged public snapshots, bounded to three seconds; command
acknowledgment is bounded to five seconds. The observer never waits for a chosen
membership, caret, text, or dirty outcome.

The aggregate projection currently preserves every observation. Consumers must
validate trace/evidence/cases/source hashes, setup separation, case order/count,
and per-case original command/configuration evidence before projecting fields.
Do not compare native model versions/public object ordinals as if they were
identical implementation identities. API tab columns expose group order, not
pixel sizes, nested layouts, or terminal geometry.

The actual final close retains **one empty active VS Code group**, with no
active/visible text editor. A native engine representing its welcome workbench
with zero groups has a deliberate internal-layout boundary. A consumer may
compare the final absent editor/text/disk/welcome behavior only if it explicitly
asserts and reports upstream one-empty-group versus native zero-group; it must
preserve this raw observation and cannot silently claim matching group geometry.

Preview replacement, sticky tabs, non-default positioning/close policies,
reordering/dragging, arbitrary/resizable layouts, group moves, Back/Forward,
dirty-last-tab dialogs, partial batch-close cancellation, Save receipts, session
restore and recovery remain outside this reference. The local capture itself
contains no native implementation or test of those features.
