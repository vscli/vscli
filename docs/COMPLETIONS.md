# Native IntelliSense: resolved completions and snippets

Automatic and explicit suggestions retain the native caret popup and existing
Tab/Enter/Escape bindings. The selected item can resolve documentation and import
edits in the background. Selection changes coalesce; accepting waits for that
item's result, while editing, focus changes, settings changes, edit→Undo, provider
replacement or source restart invalidate the request. Escape cancels acceptance.
The native LSP path works without JavaScript.

## Insertion and presentation

Plain and snippet completions support one primary replacement and additional
text edits in the same document. All UTF-16 ranges, overlaps, expansion sizes and
EOL conversion validate before any text, selection or undo mutation. Snippet fields
remain correctly positioned after imports and Unicode/CRLF normalization. Repeated
fields edit together; Tab/Shift+Tab move through the existing native field engine.
Insertion and imports share one Undo, and files remain unsaved until explicit save.
Completion targets the primary cursor; field mirrors create their own selections.
Undo restores the original selections and shared-view positions. Redo restores text
and selections under the existing snippet policy; it does not revive a canceled
interactive snippet session.

The Details panel shows bounded, sanitized detail/documentation next to the popup
in wide editor areas and beneath it in medium layouts. Small editor areas retain
the suggestion list. Markdown is inert text; no HTML or command links execute.
Filtering ranks exact matches, prefixes, token-boundary abbreviations such as
`gc` → `getClient`, and other subsequences, with server `sortText` breaking ties.
Explicitly navigated selection retains its identity while filtering. This ranking
is not qualified as identical to VS Code.

## Resolution ownership and limits

LSP initialization advertises snippet support and resolution of `detail`,
`documentation` and `additionalTextEdits`. These are the only mutable resolved
properties. Labels, sorting, filtering, primary edits and opaque item data retain
their original identity; a resolver changing them rejects without document edits.

The optional CommonJS adapter calls `resolveCompletionItem` with the original
JavaScript item, including its opaque data. Native handles bind the exact owner,
session, provider registration, originating request and mirrored document version.
At most 300 handles and 2 MiB of normalized completion snapshots are retained;
obsolete handles are purged on adapter operations after six seconds or when their
document/registration changes. Original JavaScript object graphs remain in the
optional extension heap; the wire budget does not bound that heap.

The controller keeps one resolution in flight and coalesces selection intent.
Its six-second deadline cancels the UI request. Native transport retains its slot
until the real response or its 15-second deadline; a transport deadline disables
further resolution until server restart. Optional host callbacks share the existing
eight-slot budget. Cancellation/deadline rejects their caller while an uncooperative
callback retains its actual slot until it settles.

Popup limits remain 300 items, 4 KiB presentation fields, 64 KiB serialized item and
2 MiB total. Documentation is capped at 32 KiB; rendering previews at most 8,192
characters and 64 source lines. Completion edits are bounded by 4,096 edits, 1 MiB
combined normalized replacement text, 10,000 snippet markers and the native file
size limit. Identifier prefix scans remain capped at 1 KiB.

## Remaining scope and qualification

Follow-up completion commands, insert/replace range pairs, `insertTextMode: 2`,
completion-list defaults, clipboard variables, multi-cursor replication, choice
menus/nested snippet merging, provider aggregation, ghost text and complete VS Code
IntelliSense behavior remain unsupported or unqualified. Unsupported insertion
forms reject before mutation.

Behavior tests live in the shared completion transaction module, native suggestion
tests and extension provider tests. Native and CommonJS PTY scenarios exercise
resolved imports/snippets, exact Unicode/CRLF save and undo, held stale replies and
responsive input. Passing synthetic fixtures qualifies their named contracts;
unchanged package and real-server workflows remain separately recorded in
[provider evidence](EXTENSION_PROVIDERS.md) and [compatibility](COMPATIBILITY.md).
CI results apply to the checked commit, not to future source changes.

Local qualification of this candidate passes 400 ordinary Rust tests (13 opt-in
cases remain separate), 87 Node tests, formatting and strict Clippy. The recorded
terminal run covers 34 baseline editor scenarios, six native suggestion scenarios,
two resolved CommonJS scenarios, four native surface scenarios, one provider
scenario, real clangd completion and three unchanged NPM/SQL corpus scenarios.
The unchanged package corpus also passes its two opt-in native integrity tests.
The final getter-publication hardening is additionally checked with actual native
host and terminal tests. Platform CI and reference comparisons apply separately
to the eventual PR head; these counts do not claim full VS Code parity.
