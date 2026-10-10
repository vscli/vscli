# Pinned preview and sticky editor-tab reference

This standalone observer captures unchanged VS Code **1.95.0**, product commit
`912bb683695358a54ae0c670461738984cbb5b95`. The actual Linux run passed **11 cases /
86 public snapshots** in 37.929 seconds. Native preview/sticky implementation,
consumer comparison and fresh platform qualification remain separate and pending.
The existing native group feature still uses committed tabs.

```sh
cd tests/vscode-reference
npm ci --ignore-scripts
cd ../..
node tests/vscode-reference/editor-preview-tabs-run.cjs target/editor-preview-tabs-reference
```

Linux needs a usable owned display. The local capture used isolated Xvfb display
199, and terminated only that owned server after the supervised editor tree
retired. The runner bounds the whole worker to 180 seconds, launches a fresh
profile/process for each case, and cleans isolated workspace/profile/extension
folders. macOS session roots use `/tmp` and a short prefix to respect Unix socket
path limits. An optional second argument selects one named case for diagnosis;
complete qualification requires all 11 cases.

The raw complete run is `target/editor-preview-tabs-final-reference`; its log is
`/tmp/vscli-editor-preview-tabs-final-reference.log`. The unchanged actual Linux
trio is stored under [baselines/1.95.0/editor-preview-tabs](baselines/1.95.0/editor-preview-tabs).
Its metadata records product identity, platform/architecture, all nine observer /
harness source hashes, case input hash, 44 fixture-file hashes, per-case actual
run identities and exact trace/evidence digests.

## Capture contract

Each case sets preview enabled, Quick Open preview disabled, code-navigation
preview disabled, right insertion, recent-tab close fallback, close-empty-groups
and `revealIfOpen=false`. These match the
[pinned defaults](https://raw.githubusercontent.com/microsoft/vscode/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/browser/workbench.contribution.ts)
relevant to the corpus.
Effective configuration and the inventory of every original target command are
checked before targets. Preview-from-Quick-Open and code-navigation settings do
not establish those UI workflows: neither is invoked here.

The initial `openTextDocument` / `showTextDocument(preview:true)` is separately
recorded API setup. Every subsequent target uses its original public command
once, including `vscode.open` with explicitly recorded preview options, Keep
Open, pin/unpin, splitting, group focus, cursor motion, type and Undo. There are
at most eight target gestures per case, four groups and eight tabs per group.
No target is retried until it produces a preferred tab state. Command completion
is followed by 100 ms unchanged public observations, bounded to three seconds;
command acknowledgement is bounded to five seconds.

The observer preserves ordered public groups/tabs and their active, preview,
pinned and dirty flags; active/visible editors; exact Unicode/CRLF text;
UTF-16 and scalar selections; document versions/EOL; public TextDocument-object
equality ordinals; actual tab/group/editor/document events; and fixture disk
bytes. Untitled resources receive observer-local names and `disk:null`. Public
object ordinals and version allocation are evidence, not native identity counters.
All four actual file disks remain unchanged at every recorded snapshot and at
the end of each case. No dirty-dialog discard or save target is used.

## Actual observed behavior

- Clean preview opens replace the current group's previous preview. Cursor
  motion and repeating an explicit preview open do not commit that tab.
- `workbench.action.keepEditor` commits a preview without making it sticky.
  Reopening a committed tab with preview requested does not demote it.
- Editing commits a preview; Undo to clean bytes keeps it committed. New
  Untitled tabs remain committed even after Undo makes them empty and clean.
- A committed open can coexist with another preview. Replacement inserts the
  new preview to the right of the currently active tab, which can differ from
  the previous preview's index.
- Splitting a clean preview leaves the source preview and creates a committed
  destination copy. Editing through that copy promotes the source preview too;
  Undo leaves both committed. Dirty-before-split tabs are committed in both groups.
- With `revealIfOpen=false`, a resource committed elsewhere can still open as
  preview in the current group. Each group independently retains at most one
  preview in these observations.
- `workbench.action.pinEditor` makes a tab committed and sticky. Pinning a
  historical middle B changes `[A,B,C]` to `[B(sticky),A,C]`; unpinning leaves
  `[B,A,C]`, without restoring its old index or making it preview.

The public `Tab.isPinned` flag means **sticky**, whereas `Tab.isPreview` exposes
commitment separately. The naming distinction is also verified in the pinned
[public-tab bridge](https://raw.githubusercontent.com/microsoft/vscode/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/api/browser/mainThreadEditorTabs.ts)
and [command handlers](https://raw.githubusercontent.com/microsoft/vscode/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/browser/parts/editor/editorCommands.ts).

## Preserved setup correction

The first actual run, `target/editor-preview-tabs-first-reference`, passed nine
cases / 69 snapshots with the unrelated Quick Open preview setting explicitly
true. The corrected default run, `target/editor-preview-tabs-default-reference`,
sets Quick Open preview false and explicitly fixes code-navigation preview false.
Its nine public projections are exactly equal to the first run. Both original
actual outputs and exact SHA-verified nine-file source bundles are preserved.
The source-backed default correction concerns input qualification; it was not a
retry of a target outcome. The final fresh run adds two named integrity cases
and executes the complete expanded 11-case corpus once.

## Qualification boundary

The raw projection preserves every observation. A future native consumer must
preflight exact trace/evidence/input/source hashes, resource inventory, ordered
cases, setup/configuration, original commands and gesture counts before fixture
writes or field projection. It must state any excluded fields and retain raw
public object/version observations. The current baseline is actual evidence,
not a native expected-output test that mirrors an implementation.

Graphical single/double-click, Quick Open, code navigation, multiple-sticky
placement, sticky close-prevention policies, preview settings reconfiguration,
dirty last-tab dialogs, Save receipts, stale provider/loader ownership, session
restore/recovery and persistence of preview/sticky modes are outside this capture.
It does not establish full VS Code tab behavior or terminal UI parity.
