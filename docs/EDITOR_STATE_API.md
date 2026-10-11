# Extension document language and editor selections

Selected extensions can call `languages.setTextDocumentLanguage(document, id)`
and assign `TextEditor.selection` or `TextEditor.selections`. Core selection,
editing and language settings remain native Rust operations; Node is needed only
for the optional extension process.

A language change preserves the public `TextDocument`, native buffer identity,
URI, text version, content, dirty state and Undo/Redo. The same public object is
reported by close/open events: the close callback sees the old language and
`isClosed === false`, then the open callback sees the new language. A request for
the current language is quiet. The resolved value is the retained document after
the native mirror and events publish. Known identifiers come from native builtins
and registered extension language contributions; an activation event alone does
not register a language. Unknown identifiers refuse without changing the model.

Explicit language metadata drives native syntax, language configuration and
language-scoped indentation/line-number settings. Existing automatic language
server selection observes it on the next ordinary poll. A metadata lifetime clock
retires held language-provider and native LSP requests even when content is
unchanged or a language changes A→B→A. Server close/reopen allocates a new
synchronized document version without changing the public text version. Actual
callback occupancy survives advisory cancellation until the work settles.

Selection setters validate their inputs synchronously. Assigning an empty array
normalizes to one origin selection. Getters immediately expose the assigned
selection cohort, matching the source's optimistic presentation. Native replies
and mirrors establish the actual clamped UTF-16 positions; confirmed selection
events are emitted only for actual native state changes. Direction and caller
order are preserved. Events contain the extension's scoped editor identity and
selections; command kind is emitted only when proven by a native API setter.
Other source kinds remain undefined until native provenance is available.

Requests retain the original caller, document resource, text/version/language
lifetime, settings/workspace and selection view proof. Invalid or stale requests
cannot modify another buffer or acquire a replacement caller lease. Updating an
inactive retained editor leaves the focused editor unchanged. A source view change
within the same document refuses rather than targeting its other native pane.
Native mirror publication and acknowledgement receipts are retained across output
saturation without applying a selection or language operation twice. Presentation
overlays have no authority over native source checks or event generation.
When native inventory exceeds mirror limits, ordinary host polling pauses document
publication and emits one notice; core editing and the isolated host remain usable.
Publication resumes after inventory returns within bounds. Explicit services keep
their original refused source proofs rather than adopting a later valid inventory.

## Qualification and remaining work

All 542 host tests pass, including fifteen editor-state cases and three new
language-provider lifetime cases. They cover language identity/events, selection
normalization, optimistic getters, native-only event confirmation, inactive editor
state, original ownership, stale acknowledgements, queue bounds and timeout occupancy.
All 1,220 Rust all-target tests pass (913 library; 23 ignored cases grant no
qualification), including nineteen new native library tests and four actual
selected-host/App workflows. Native cases cover source/view/settings proofs,
language-aware LSP lifecycle, Outline, hints and completion presentation retirement,
UTF-16 mapping, registered-language metadata and history integrity. Host/App cases
cover actual close/open events, inactive selections, keyboard movement, native
Save As, dirty recovery and mirror pause/resumption without losing the host.

Two new actual Unix terminal workflows pass language switching, selection setters
and events, explicit-language Save As and original Unicode/CRLF keyboard Undo/Redo
with protected disk bytes. Three existing untitled journeys, four public-save
journeys and the virtual-document journey pass again. Native build, formatting
and strict all-target locked Clippy pass. This is local Linux qualification; it
does not establish upstream runtime parity or Windows/macOS behavior.

Native mirrors admit 128 models, 10,000 selections per model and 16,384 total
selections. Typed editor-state mutations admit at most 4,096 selections per model
and retain the same aggregate limit. Native buffers outside these limits remain usable. Language
identifiers are bounded to 128 UTF-8 bytes. Current public editor inventory is
still one editor per document and one visible active editor; full shared-pane
editor identities and view columns are outstanding. Native keyboard/mouse kind
provenance, visible ranges/options/decorations, richer language providers,
remote/custom editor backends and upstream/platform fidelity remain work toward
the full extension API goal. Local behavior and source comparison do not establish
full upstream runtime parity.

Source comparisons use stable 1.141.0:
[language API entrypoint](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHost.api.impl.ts),
[language close/open lifecycle](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostDocuments.ts),
and [selection setters](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTextEditor.ts).
No local VS Code instance was launched. CI remains deferred under the requested
feature-first workflow.
