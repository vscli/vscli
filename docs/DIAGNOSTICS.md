# Diagnostic collections and native result identity

VSCLI renders current native language-server diagnostics and diagnostics supplied
by selected CommonJS extensions in the editor gutter and **Language: Problems**.
Problems includes visible and hidden native document models, including untitled
buffers. Selecting a problem retains the document's identity and rejects a row
after its text changes or its document closes. Messages remain inert terminal text.

Native editing, saving and language-server diagnostics do not require Node.js.
An optional extension process owns extension collections; its failure or retirement
removes those collections while independently owned native diagnostics remain.
Restarting a native server removes its publications without removing extension
collections.

## Native identity and limits

Each native publication retains its originating server identity and the document
ID, text epoch and protocol version from synchronization. Display and action
acceptance compare these with the current native model and synchronization entry.
An edit followed by Undo still advances the text epoch and creates a new protocol
version. Closing, reopening, hiding or replacing a server cannot make an old
version-qualified publication current again.

The LSP permits publishers to omit their version. Such a publication can be tied
to the synchronization entry at receipt, but the client cannot prove which version
an asynchronous server actually analyzed. This weaker provenance remains an
outstanding qualification limit. A present malformed version is rejected rather
than treated as omitted.

Malformed and oversized publications preserve the previously accepted source
instead of becoming an empty clearing publication. Per-publication limits are
5,000 diagnostics, 2 MiB of serialized diagnostic data and 4 KiB per message.
The retained native cache also caps its combined publications at 128 resources,
5,000 diagnostics and 2 MiB; closed and stale source entries are evicted.
Problems presents at most 5,000 rows and explicitly labels a truncated list.

## Optional collection API

The host provides `Diagnostic`, `DiagnosticSeverity`, `DiagnosticTag`,
`DiagnosticRelatedInformation`, `languages.createDiagnosticCollection`,
`languages.getDiagnostics` and `languages.onDidChangeDiagnostics`. Collections
retain their original JavaScript diagnostic objects; wire serialization is a
separate bounded presentation representation. Custom JavaScript fields are not
silently discarded from the collection objects.

Collections support single-resource and bulk `set`, `get`, `has`, `delete`,
`clear`, iteration, `forEach` and disposal. Duplicate collection names remain
independent. Bulk URI groups follow the pinned VS Code collection behavior,
including intervening undefined entries and empty groups. Reads are shallow:
present-resource arrays are copied and frozen, while their diagnostic objects
retain identity. These semantics do not freeze arbitrary extension objects.

Each publication is scoped to its session and extension owner. Native acceptance
requires the current mirror version and document text epoch. Cursor movement and
editor focus changes do not invalidate diagnostics. Updates are staged and
bounded before replacing accepted state. Collection lifetime and document stamps
prevent queued old notifications from reappearing after retirement or an edit.
The collection API itself carries no analyzer-start version: an extension that
finishes old analysis and calls `set` against the current document can still
publish logically stale analysis. This requires cooperation from the extension.

The initial native display scope is mirrored open models. Unopened workspace
resources, native LSP diagnostics inside the extension's `getDiagnostics`,
extension code-action providers, and a full diagnostics panel remain separate
work. The host's collection reads can retain diagnostics after a document closes,
as the pinned API does; that does not make a closed document's old publication
visible in the native editor.

The optional host caps all collections together at 64 collections, 128 resources,
5,000 diagnostics and 2 MiB of normalized presentation data, with 4 KiB messages.
These limits do not measure arbitrary custom JavaScript object graphs retained
by extensions. Normal pending change events cover at most 128 distinct URIs;
retirement can add up to 128 retained resources to that bounded event cohort.

`workspace.onDidSaveTextDocument` follows a successful native persistence
operation. Undo becoming clean and a failed save do not produce a save marker.
The mirror records successful save generations independently of text versions.

## Evidence

Synthetic Node tests exercise the collection API, save-event ordering, metadata,
bounds and adversarial getter behavior. Three Rust collection tests cover staged
wire acceptance, currentness and the actual framed optional host with hidden and
untitled models. Native protocol tests in `tests/native_diagnostics.rs` exercise
synchronization identity, version handling and publication budgets; native action
integrity tests preserve text, selections, Undo/Redo and disk bytes after rejection.

Pinned executable collection observation and unchanged real-extension native/PTY
qualification are separate acceptance work. Source inspection or synthetic API
success alone does not establish complete extension compatibility.
