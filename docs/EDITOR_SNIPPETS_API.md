# Public editor snippet insertion

The optional extension host connects `TextEditor.insertSnippet` to native snippet
parsing, linked fields and Undo/Redo. Native editing, user snippets and installed
snippet catalogs continue to work without Node. This adds a public editor workflow;
it does not complete the extension API or establish general snippet parity.

## Arguments and results

```js
const editor = vscode.window.activeTextEditor;
if (editor) {
  const applied = await editor.insertSnippet(
    new vscode.SnippetString('${1:name}($0)'),
  );
}
```

An omitted or falsy location, including an empty array, uses the addressed
editor's private current selections. A `Position` inserts at one point; a `Range`
or `Selection` replaces that range. Arrays support multiple positions/ranges and
preserve the captured input order before native expansion. Non-Range array entries
also accept source-compatible position-like fields. Explicit array insertion
replaces the prior snippet session rather than merging a new nested session.
This is the API array path's behavior, not an unimplemented merge operation.
Across file-ordered fragments, repeated fields share the first resolved
placeholder default; resolver cursor indices retain the original input order.
The API creates one implicit final stop for the combined insertion.

The whole omitted options argument defaults to `undoStopBefore: true` and
`undoStopAfter: true`, with `keepWhitespace: false`. When an options object is
provided, each field uses JavaScript truthiness: missing undo flags become false.
For example, `{keepWhitespace: true}` also requests both undo flags false;
include both flags explicitly when that is not intended. The adapter snapshots
each supplied property once and does not modify the options object.

Normal insertion applies the native common-indentation rules at each target and
normalizes line endings to the model's EOL. `keepWhitespace: true` preserves the
fragment's whitespace while retaining model EOL normalization. Template syntax,
variables and transforms are processed natively; the host sends the original
bounded template, including Unicode and NUL characters.

The result is a `Promise<boolean>`. True follows native insertion and shared
mirror/selection/change publication. Successful insertion focuses the addressed
native editor, including an inactive visible pane. Read-only or stale native
source refusal returns false. Invalid arguments, retired handles, unsupported
host capabilities and native errors reject. No host-side text simulation or
synthetic change event precedes the native result. Insertion leaves files unsaved.

UTF-16 coordinates clamp to native line bounds; positions past the last line
clamp to the document end, and surrogate-interior positions map to a native
scalar boundary. Overlapping or invalid insertion cohorts refuse atomically.
This bounded native mapping is not a claim of identical behavior for every
surrogate-interior or malformed upstream input.

## Ownership and bounds

The host captures the original caller lease, private document/version and exact
visible editor before extension getters run. It snapshots private selections,
location fields, template and options, then checks the original authority again
before sending. Public document metadata, editor getters and replacement panes
cannot authorize or retarget the request. Native admission additionally retains
source epoch/history, original view, settings/workspace and owner/session proofs.
Shared document identity and other views' independent selections remain native-owned.

The exact receipt contains the original document ID, editor ID and boolean
`applied`. Native mirrors precede the receipt; the host checks the original caller
and live editor again before resolving. A five-second host timeout rejects with
an unknown publication outcome and retains the occupied request until its exact
terminal response. It does not prove that no insertion happened. The shared
service lane admits eight calls within 8 MiB; framing admits 64 requests.

| Resource | Bound |
| --- | --- |
| Template | 64 KiB UTF-8 |
| Insertion ranges | 4,096 |
| Snippet request | 1 MiB escaped JSON |
| Aggregate native document mirrors | 4 MiB source bytes |
| Aggregate native selection mirrors | 16,384 selections |
| Native core document | 32 MiB; the public mirror bound can refuse earlier |
| Clipboard text | 1 MiB |
| Native parser/expansion | 64 nesting levels, 10,000 markers, 1 MiB per expansion |

Expansion and output-selection admission precede text, view, focus or history
publication. Active placeholder cohorts can be refused below the input range
limit to preserve the aggregate mirror budget.

`CLIPBOARD` uses a bounded native worker. Linux uses `wl-paste`/`xclip`; macOS uses
`pbpaste`. Unavailable helpers fall back to the captured internal clipboard;
oversized/non-UTF-8 output and failed worker retirement refuse. Source or original
selection changes while reading refuse the stale insertion. Cancellation or an
early reply does not release capacity while the actual worker or pipe reader is
still alive. Windows system clipboard integration remains outstanding.
Native user-command insertion also refuses while this public clipboard worker
is occupied. The worker joins before consuming its final reply; pipe descendants
retain the shared permit until they actually retire.

## Evidence and remaining work

Seventeen focused host tests and all 617 host tests pass locally. These cover
overloads/options, exact getter capture, stale source/view/caller rejection,
inactive shared-pane routing, mirror events before ACK, bounds and retained
eight/64-slot timeout capacity with malformed receipts refused.

All 1,279 Rust all-target tests pass across 60 suites (962 library tests; 23
ignored cases grant no qualification). Twelve new core cases and three clipboard
worker cases pass, including retained worker/join ownership, indentation,
selection and history integrity. Four actual selected-host/App workflows pass.
Native build, formatting and strict all-target locked Clippy pass.

Four new Unix PTY journeys pass: linked Unicode fields with Tab/Backtab/Escape
and history; inactive/shared panes and location arrays; stale getters plus
retired/read-only refusal; and default/keepWhitespace CRLF insertion with
false/false undo flags grouping API insertions A+B and native typing C into one
Undo/Redo. These workflows preserve original history and disk bytes. Sixteen existing decorations, presentation, inventory, editor-state, untitled,
public-save and virtual-document PTY journeys also pass, alongside two completion
resolve journeys. The combined terminal matrix passes 22 journeys.

Existing [snippet traces and native comparisons](SNIPPETS.md#evidence) retain their
original scope and evidence. They are not reclassified as public-host qualification.
Choice menus, command-driven nested session merging, full ECMAScript regex behavior, rich command
nesting and broader upstream/platform fidelity remain outstanding. The frozen
declaration/export audit remains unchanged.

Source comparisons use immutable stable 1.141.0 commit
`2a59476c9bfcb90b3ddc372c36762471b7dfad1c`:
[editor overloads and options](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTextEditor.ts),
[native editor bridge](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/browser/mainThreadEditor.ts)
and [snippet sessions](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/editor/contrib/snippet/browser/snippetSession.ts).
Source comparison is separate from actual upstream runtime evidence.
