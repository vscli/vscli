# Native editor options, visible ranges and reveal

The standard App/CLI publishes resolved `TextEditor.options` and measured
`TextEditor.visibleRanges` for each visible editor, with scoped
`onDidChangeTextEditorOptions` and `onDidChangeTextEditorVisibleRanges` events.
`TextEditor.revealRange` scrolls the addressed native pane without changing its
selections or the active group. These adapters require the optional extension
host; core rendering, indentation and editing remain native Rust without Node.

## Options and native editing

`tabSize`, `indentSize` and `insertSpaces` belong to the shared document model.
Changing them through either pane updates both panes showing that model.
`cursorStyle` and `lineNumbers` belong to the addressed view and do not change a
sibling pane. A retained editor exposes a stable options object; both whole-object
assignment and individual property setters are supported. Confirmed native
changes publish all mirrors before events and acknowledgements. Numeric setters
have optimistic getters; failed requests retire their own optimistic overlay.

Tab and indent widths follow the source's numeric flooring and decimal string
parsing; nonpositive and unparseable widths are ignored at the public boundary.
`indentSize: 'tabSize'` follows the current tab width. `tabSize: 'auto'` or
`insertSpaces: 'auto'` invokes source-derived indentation detection against the
actual native text and creation defaults. The auto branch resolves both fields,
ignores a simultaneously supplied indent size, and publishes its numeric result.
`insertSpaces: 'false'` resolves false; other nonspecial values use Boolean
conversion. Automatic/string modes wait for native confirmation rather than
inventing resolved values.

Native indentation commands, bracket-aware Enter, Tab/outdent and line-comment
alignment use the resolved indent width. Display columns and mouse positioning
still use tab width. LSP formatting receives the indent width as its `tabSize`
option. Changing options does not edit text, dirty the document or alter its
version, persistence generation or Undo/Redo history. Extension assignments are
ephemeral native model/view changes; they do not write settings files.

User/workspace language settings additionally support `editor.indentSize`
(1–16 or `"tabSize"`) and `editor.cursorStyle` (`line`, `block`, `underline`,
`line-thin`, `block-outline`, `underline-thin`). Existing `editor.tabSize`,
`editor.insertSpaces` and `editor.lineNumbers` continue to set native defaults.
Line-number public values are Off=0, On=1, Relative=2 and Interval=3.

## Actual viewports and scrolling

Ranges use UTF-16 coordinates and the whole logical lines in the actual vertical
text viewport. Horizontal clipping does not shorten those line ranges. An editor
without a measured native frame exposes `[]`; the bridge does not assume terminal
dimensions. Resizing or replacing a native view retires its old measurements.
Only a subsequent actual frame can supply new dimensions. Presentation metadata
does not authorize mouse or document hits.

`showTextDocument` accepts prior mirror generations only across a contiguous
series of measured-visible-range-only publications. A source, membership,
editor identity, focus, selection or options change resets that allowance.
Native source/version/view/context checks still apply at admission and completion;
measuring a newly shown pane therefore does not spuriously invalidate the next
Show request.

Reveal supports Default, InCenter, InCenterIfOutsideViewport and AtTop. Native
positions are clamped to the retained model; horizontal scrolling accounts for
actual tab stops and grapheme widths. Explicit scrolling survives redraw without
pulling the viewport back to an offscreen caret. Real caret navigation, selection
changes and editing resume normal caret following, including movement away and
back to the same position. An inactive visible pane can be revealed while focus
and both panes' selections remain unchanged.

A reveal received before measurement retains its original bounded native request
until a real frame is available or the existing service deadline retires it.
Original caller, document/version/resource, language/settings and native view
checks apply before any effect. Retired handles cannot target a replacement pane.
Accepted operations retain their receipts through mirror/reply backpressure;
acknowledgement retries do not repeat the mutation.

## Bounds and remaining qualification

Native widths currently admit 1–16. This is a VSCLI safety bound, not the upstream
maximum. Auto detection scans at most 10,000 lines and 256 KiB of accumulated
content; exceeding the byte budget refuses before mutation. Public option strings
admit 4,096 bytes, enums require declared values, and native integers must be safe
and nonnegative. These explicit refusals differ from upstream's broader coercion.
Horizontal reveal prefixes admit 256 KiB. Existing four-group, 128-model,
selection, service queue and mirror byte limits remain in force.

Terminal cursor shapes use steady bar, block and underline. Thin and outline
variants map to the nearest terminal shape; exact browser appearance is not
claimed. Prompt/noneditor cursors and terminal cleanup restore the user shape.
Smooth/pixel scrolling, wrapped/folded visible-range projection, precise upstream
horizontal reveal rules, configuration-reload fidelity and richer editor APIs
remain outstanding. [Editor decorations](EDITOR_DECORATIONS_API.md) adds native
per-view range overlays; its richer render fields retain separate display gaps.
Legacy direct Client embedders without explicit
native inventory do not receive fabricated resolved options or reveal capability.
Windows/macOS and upstream runtime parity remain unqualified.

## Executed evidence

All 578 host tests pass, including nineteen new cases for stable options objects,
coercion/auto/alias modes, optimistic rollback, actual scoped events, private source
and getter reentrancy checks, readonly ranges, malformed publication refusal,
retired handles, exact acknowledgement identity and retained timeout occupancy.
Two actual selected-host/App workflows pass native shared/per-pane options, real
Tab and original Undo/Redo, every reveal mode, measured UTF-16 ranges, resize and
retired-handle refusal. The two existing inventory and four editor-state App
workflows pass alongside them. All 1,244 Rust all-target tests pass (933 library;
23 ignored cases grant no qualification), including ten new native library
regressions for source/history integrity, options/detection, scoped settings,
offscreen caret rendering, measured-frame retirement and resize. The existing
real LSP formatter test additionally checks a distinct indent width on the wire.

Two actual Unix PTY journeys pass shared/per-pane options, auto/alias modes,
native two-space Tab, original Unicode/CRLF Undo/Redo and protected disk bytes;
inactive-pane reveal in all four modes, measured ranges, resize and retired-handle
refusal also pass. The resize observer waits through the temporary unknown range
for a measured replacement frame. Twelve existing inventory, editor-state,
untitled, save and virtual-document PTY journeys pass again. Native build,
formatting and strict locked all-target Clippy pass. This is local Linux
qualification, not an upstream runtime or cross-platform parity claim.

Source comparisons use immutable stable 1.141.0:
[public editor options and reveal](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTextEditor.ts),
[native option application](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/browser/mainThreadEditor.ts),
[indentation detection](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/editor/common/model/indentationGuesser.ts)
and [visible model ranges](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/editor/common/viewModel/viewModelImpl.ts).
No local VS Code instance was launched. CI remains deferred under the requested
feature-first workflow. This milestone does not establish complete API parity.
