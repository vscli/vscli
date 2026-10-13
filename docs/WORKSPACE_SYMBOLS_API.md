# Workspace symbol providers

The implementation connects
`languages.registerWorkspaceSymbolProvider` to
`vscode.executeWorkspaceSymbolProvider(query)` and original native
`workbench.action.showAllSymbols` navigation (Ctrl+T on Linux/Windows, Cmd+T on
macOS, subject to terminal key constraints). JavaScript remains optional for core
editing. The executed synthetic workflows below qualify named behavior; they
do not establish arbitrary-extension or full VS Code compatibility.

The source contract is stable VS Code 1.141.0 at immutable commit
[`2a59476c9bfcb90b3ddc372c36762471b7dfad1c`](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostLanguageFeatures.ts).
The global collection and comparison rules are in
[`search.ts`](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/contrib/search/common/search.ts).
Native selection follows the lazy resolve/open contract in
[`symbolsQuickAccess.ts`](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/contrib/search/browser/symbolsQuickAccess.ts).
These primary-source receipts are separate from historical 1.95 runtime captures.

## Global queries and lazy navigation

Workspace providers receive the query string and a real cancellation token.
Queries are global: an empty workbench has no fabricated document, language or
text version. Public execution returns fresh `SymbolInformation` values and
preserves name, kind, tags, container name, URI and range. A missing initial
range stays absent. Public execution does not eagerly call the resolver, focus
an editor, open the returned resources or edit files.

All registered workspace providers participate; ordinary errors and null results
are isolated. This registry is independent of document-selector score ordering.
Collected results sort by name, kind, URI, nonintersecting range and container
name. Intersecting ranges compare equal at the range step; an absent range can
prefer the provider with a resolver. Equal groups retain their first collected
item, with asynchronous provider settlement affecting collection order.
Different resolver capabilities can keep otherwise identical symbol names in
distinct groups when a range is absent; preference does not delete that second
group. Exact equal complete symbols still deduplicate.

Selecting a lazy native row invokes its original provider's
`resolveWorkspaceSymbol` with the exact original symbol object. Opaque extension
data, including cycles, remains private to that object. Native selection uses a
real resolved range and collapses the caret at its start. If the range remains
absent, it opens the original URI without changing that editor's selection. It preserves retained
dirty/shared/hidden model identity and history. Public arbitrary URIs do not
imply native backends for every URI scheme; unsupported native navigation must
refuse rather than substitute a file or invent a range.

## Lifetime and bounded adaptation

Native searches retain original workspace/settings/profile, provider registry,
owner and list/resolve identities. Query supersession, Escape, disposal and owner
or workspace retirement cannot authorize a late list or resolver. An obsolete
callback that ignores cancellation continues to occupy an actual shared slot
until settlement. Global queries share the eight active callback limit with
other language-provider work; a ninth simultaneous public query refuses rather
than masquerading as an empty successful result. Metadata/results retain the
implementation's explicit provider, row, byte and deadline limits; excess data
must not silently truncate the workspace or modify native buffers.
Pre-wire admission failures reject the whole cohort explicitly; they cannot
publish a silently incomplete successful list. Ordinary provider exceptions
remain isolated after genuine callback dispatch.

Bounds are 128 providers, eight shared active callbacks, 512 results and 1 MiB
normalized collection metadata. Native query input is capped at 1 KiB; the
host's public query boundary is 4 KiB. Lazy caches hold at most 512 original
objects and 2 MiB canonical metadata, without serializing opaque provider data.
Native cohorts have a 15-second logical deadline. File loading retains its one
actual worker lane until exit and join, even after a result has arrived or the
user cancels. Closed extension targets must fit the remaining 4 MiB retained
model budget; admission is rechecked before visibility changes.

## Executed vertical qualification

The selected synthetic extension lives in
`tests/fixtures/workspace-symbols-extension`. Its public query runs with zero
editors; its original lazy symbol holds cyclic data and has no initial range.
It uses real mirrored documents, callbacks, cancellation and registry disposal,
and does not write document files.

`tests/extension_workspace_symbols.rs` passes seven actual selected-App checks:
zero-editor typed global collection; original native lazy and hidden target
navigation; latest held list and canceled/disposed/profile-ABA resolver; selected
owner retirement; eight held callbacks followed by actual settlement/reuse;
on-demand hidden indexing through a real awaited `workspace.openTextDocument`
inside the original Ctrl+T callback; and explicit whole-cohort admission refusal
while an obsolete callback remains held. The indexing case checks both an empty
workbench and existing shared dirty models: the real opened event precedes the
callback's acknowledgement, the prompt/query and active editor stay intact, and
the indexed model becomes visible only when the user accepts its row.
The native cases preserve Unicode CRLF text, shared memberships, selections,
Undo/Redo, save proofs and original disk bytes.

`tests/extension_workspace_symbols_pty.py` passes four real terminal journeys
for global queries, original Ctrl+T navigation, ignored-cancellation list/resolve
and actual callback capacity. It uses absolute optional Node with an empty PATH,
disabled LSP and terminal restoration.

All 694 host tests and 1,337 Rust all-target tests pass (1,003 library tests,
64 result reports, 23 ignored cases). Ignored cases grant no qualification. The
real clangd C++ document/workspace-symbol test separately passes with installed
clangd. Build, formatting and strict all-target locked Clippy pass on Linux.
Actual host/native controls cover zero-document callbacks, exact opaque resolve,
eight shared slots, cancellation/expiry, malformed release identities and
registry retirement. Native worker controls cover received-result-before-exit,
cancellation with responsive painting, over-budget target refusal and an
equal-valued settings reload that cannot revive an old hidden-open result.

The selected-App latest-list workflow accepts the real latest row while the
obsolete callback remains held, then verifies the same target after its late
settlement. It does not claim to observe that late reply while a newer picker is
still open: invoking another user command deliberately closes symbol pickers.
The indexing callback performs its event/mirror/focus checks before returning;
the App separately checks the still-open original prompt and query before Enter,
then verifies accepted identity through the public fixture command.

The existing `tests/symbol_navigation.rs` and its framed
`tests/fixtures/symbol_server.py` are separate native LSP regression targets:
global symbols without an editor, latest-query cancellation and shared-buffer
navigation remain usable without Node. Its six framed native tests pass; the
seventh installed-clangd case passes in the separate explicit run. All three
native-only Outline terminal journeys also pass with the freshly built editor.
Four original navigation terminal journeys, the shared-order registry journey,
the original provider journey and three single-resource refactoring journeys
also pass. This cohort has sixteen passing terminal journeys in total.

Rename/prepareRename, unopened-target transactional rename, refactoring auto-save
and the remaining language-provider families follow separately. This milestone
does not claim complete extension or VS Code parity.

Provider and public-query timers now check their original monotonic deadline
when a timer fires. Early delivery re-arms only the remaining interval; it cannot
cancel a callback early or let its isolated timeout produce an empty successful
query. Deadline rejection precedes cancellation notification, and unfinished
callbacks retain actual capacity until settlement. One deterministic provider/query
test delivers both timers half a millisecond early and then at the original
deadline, covering the race without increasing the timeout or renewing authority.
A second holds timer delivery past the original deadline: a late provider result
is refused before any extension-owned result getter is read.
