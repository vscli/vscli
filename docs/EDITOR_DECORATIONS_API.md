# Extension decorations in native panes

The standard App/CLI supports `window.createTextEditorDecorationType` and
`TextEditor.setDecorations` with `Range[]` or `DecorationOptions[]`. Shared files
retain independent decoration sets for each visible editor. Setting a type
replaces its previous ranges in that editor; an empty array clears them. The host
avoids repeated empty setters for an editor without that type. Public decoration
types have a frozen key/dispose surface and runtime-private identity. Disposal
is idempotent and immediately retires the native type.

The public adapter snapshots typed render fields, light/dark overrides,
before/after attachments, theme color references, icon URIs and hover metadata.
Preserving those DTOs is separate from native rendering. Current native display
supports RGB foreground/background, bold, italic, underline, strike-through and
whole-line backgrounds, including blank cells after text. Native theme color
references cover the existing editor foreground/background, selection, current
line, line-number/description and focus colors. Selection backgrounds take
precedence over decoration backgrounds. Each frame projects sorted style boundary
events for its viewport rather than scanning all decorations for every cell.

All four `DecorationRangeBehavior` values map endpoints through actual native
edits and inverse Undo/Redo streams. Markers use UTF-16 offsets and replacement
common-length semantics, including markers inside UTF-16 surrogate pairs, with
native Rope metadata supplying scalar projections for display. Public editor
insert/delete edits force marker movement; replacements and ordinary native
typing use endpoint stickiness. Undo/Redo use inverse streams without forced
movement, matching the pinned text model. This avoids treating emoji replacement
lengths as scalar lengths. Undo does not
restore an obsolete extension publication from a history snapshot. Native source,
dirty state, document version, persistence generation and selections are not
changed by setting or disposing decorations.

Getter capture rechecks original caller, type creator, editor and document proofs.
A live type can be shared between extension facades; its original creator lease
and the consumer's lease remain independent. Forged keys do not acquire public
type authority. Native registration retains immutable type identity; reused keys
cannot revive it. Queued setters retain their original type and native view.
Retired editors, stale sources and disposed types refuse before effects. Type,
owner and host retirement remove rendering authority immediately. Every native
membership transition clears retired editor sets, including hide/show cycles
coalesced before host polling.

The void API reports asynchronous failures through a native notice. Typed native
receipts retain their original document/editor/type identity through reply or
mirror backpressure; timeout alone does not free occupied service slots or replay
effects. Legacy direct Client embedders without explicit inventory do not acquire
native decoration capability. Core editing and rendering remain Rust and usable
without Node.

`showTextDocument` tolerates intervening mirror publications that change only
measured visible ranges. Document/source, native editor identity, membership,
focus, selections and options changes still retire the prior generation. Native
admission and completion retain their source/view/context checks; this allows a
real frame to measure a newly shown pane without spuriously refusing its next
extension operation.

Limits: 256 live types and 4,096 lifetime type keys per host; 16 KiB of serialized
type options and 1 MiB across live options; 4,096 entries and 1 MiB per setter;
2 MiB of retained entry metadata per set; 16,384 entries and 8 MiB per document.
Nested DTO validation admits depth 12 and 4,096 nodes, accounting escaped UTF-8
bytes and conservative node overhead. It can refuse malformed/deep input before
mutation. Existing native
service and mirrored-model limits still apply.

## Outstanding rendering and fidelity

Hover presentation, before/after attachments, raster gutter icons, overview
rulers, border/outline/cursor/spacing/opacity properties and full CSS color/alpha
semantics are retained data with outstanding native display work. Thin browser
geometry, wrapping/folding integration, arbitrary theme registry colors,
grapheme-interior style boundaries, exact overlapping CSS cascade and upstream
runtime/platform qualification also remain incomplete. No full decoration or
extension API parity is claimed.

## Evidence

All 600 host tests pass, including nineteen new decoration cases and three
public edit-marker cases. These cover exact native receipts and occupied timeout
slots, getter reentrancy, cross-owner type sharing, original creator retirement,
typed hover/render snapshots, byte limits and public insert/delete marker flags.
All 1,260 Rust all-target tests pass (947 library; 23 ignored cases grant no
qualification). Fourteen new library cases cover source/history protection,
all four endpoint behaviors, forced movement, surrogate-interior and oversized
position clamps, metadata limits, theme/style projection, native type retirement,
unpolled editor replacement and measured-range-only Show generations.

Two actual selected-host/App workflows verify rendered RGB and text modifiers,
whole-line trailing backgrounds, independent shared-file panes, replacement,
clear, disposal, ordinary keyboard versus forced public insertion, Undo/Redo,
stale getter requests, pane and host retirement, and unchanged Unicode/CRLF disk
bytes. Both tests failed on the initial Show generation check; the narrowed
viewport allowance resolves that failure while source/identity/options/selection
changes still refuse.

Two new Unix PTY journeys pass actual public setters/type lifecycle, forced
public insertion, native keyboard editing, Undo/Redo, source reentrancy and
pane/host retirement with original dirty work and disk bytes preserved. Terminal
journeys qualify routing and lifecycle; actual buffer tests qualify cell styles.
Fourteen existing presentation, inventory, editor-state, untitled, public-save
and virtual-document PTY journeys pass again. Native build, formatting and strict
all-target locked Clippy pass. Broader upstream/runtime/platform equivalence
remains outstanding; authored tests alone do not grant it.

Source comparisons use immutable stable 1.141.0:
[type lifecycle and range setters](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTextEditor.ts),
[typed render/hover conversion](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTypeConverters.ts)
and [UTF-16 marker mapping](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/editor/common/model/intervalTree.ts).
No local VS Code instance is launched. CI remains deferred under the requested
feature-first workflow.
