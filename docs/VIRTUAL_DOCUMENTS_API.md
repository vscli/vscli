# Virtual text documents

The current implementation connects `workspace.registerTextDocumentContentProvider`
to native read-only models for custom URI schemes. Extensions open a resource with
`workspace.openTextDocument(uri)` and display its shared document with
`window.showTextDocument(document)`. A provider's `onDidChange` event requests fresh
content for that URI; a successful update retains the native model and shared
public document identity.

Provider strings normalize CRLF and lone CR to LF, matching the upstream logical
line model. Native provider refresh advances the text epoch/version, keeps the
model clean, clamps selections in all retained views and creates no user Undo
entry. Native editing, snippet/completion insertion, workspace text edits and
file-save capture/publication reject read-only virtual models. Public
`TextDocument.save()` returns false; it does not open Save As or write a file.
An editable copy through Save As is separate, unimplemented work.

Disposing a provider removes its registration and retires its pending refresh
interest. It does not close or erase already retained documents: their cached
content and URI identity remain available. A later open can reuse that retained
model. Original caller/provider lifetimes and exact model/version proofs guard
asynchronous publication. Refresh bursts coalesce into the latest intent per URI;
cancellation does not release callback capacity until the actual callback settles.
Core Rust editing remains usable without JavaScript; content-provider callbacks
run only in the optional extension host.

## Evidence and limits

All 503 host tests and 1,185 Rust all-target tests pass on Linux (886 library
cases; 23 ignored cases grant no qualification). Nineteen focused provider tests
cover cached/opening identity, same-content suppression, latest-first fallback,
coalescing, ignored cancellation, original provider/Thenable/event cleanup scope,
URI getter retirement, native mirror proofs and bounds. Two framing tests prove
that timed-out virtual operations retain shared capacity until an exact typed
terminal receipt. Five native registry tests cover atomic registration updates,
original provider retirement/ABA, hidden-model source proofs and limits.

Three native Document integrity tests exercise Unicode/CRLF normalization,
read-only refusal before source or selection changes, snippet/completion resolver
nonexecution, save and forged-receipt refusal, shared-view clamping, clean refresh,
no Undo/Redo history and atomic refusal on exhausted counters or invalid metadata.
Two further regressions protect distinct virtual/untitled tab identity and native
grammar highlighting under virtual refresh epochs.

Two actual selected-host App tests and one real terminal session exercise public
registration, opening, mirror/events before acknowledgement, showing, refresh,
held obsolete callback suppression, readonly editing/saving and cached reopen
after disposal. The terminal runs with an empty PATH and an explicitly selected
Node executable; ordinary shared dirty Unicode/CRLF text, native keyboard
Undo/Redo and original disk bytes survive. Build, formatting and strict locked
all-target Clippy pass. These local tests do not establish upstream runtime parity
or actual Windows/macOS behavior.

Native provider metadata admits at most 128 registrations; the current host
registration cap is 64. Content is capped at 1 MiB and encoded resource URIs at
4,096 bytes. Native admission has at most eight queued requests and 8 MiB of
queued input, with a 4 MiB shared mirror budget and at most 16 hidden retained
models. The host permits eight actual content callbacks, including callbacks
retiring after cancellation, and at most 128 mirrored resources/refresh interests.
Overflow is an explicit refusal, not a successful partial document. The native
Document primitive separately supports up to 32 MiB; this does not enlarge the
public provider gateway's 1 MiB allowance. NUL-containing URIs or content reject
explicitly; arbitrary binary content is not a text-provider document.

Custom filesystem-provider document editing, automatic scheme activation, remote
and multi-root resource behavior, complete provider scheduling/event fidelity and
exact upstream/platform qualification remain outstanding. This is a text content
provider route; it does not establish arbitrary virtual filesystem editing.

The source comparison is pinned to stable 1.141.0:
[content-provider host](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostDocumentContentProviders.ts),
[native-side content-provider service](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/browser/mainThreadDocumentContentProviders.ts)
and [public declarations](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vscode-dts/vscode.d.ts).
No local VS Code instance was launched for this work.
