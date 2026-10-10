# Automatic parameter hints

Typing a registered trigger character now requests parameter hints automatically
from the selected native LSP or optional extension provider. The existing
Ctrl+Shift+Space (Linux/Windows) or Cmd+Shift+Space (macOS) command invokes manually.
The character passed to the provider is the original typed character, including
when native smart typing inserts a closing delimiter at the same time.

The panel highlights the active parameter, renders inert plain-text documentation,
and retains every qualified overload. Up/Down cycles locally; Alt+Up/Alt+Down
also cycles when completion suggestions are open. macOS retains Ctrl+P/Ctrl+N.
Completion's Up/Down and first Escape take precedence while its popup is open.
Escape/Shift+Escape dismiss hints. Cycling never issues another provider request.

`editor.parameterHints.enabled` and `editor.parameterHints.cycle` are booleans,
defaulting to true, with user/workspace/language overrides. Disabling automatic
initial hints still permits manual invocation and retriggering its active help.
With cycle disabled, moving beyond the first/last overload closes the panel.

## Requests and document integrity

Automatic work coalesces for 120 ms and keeps one latest queued intent. Initial
invocation, trigger-character, and content-change contexts are distinct, with
registered retrigger characters and previous help used for active interactions.
Normal typing can retain previous help for the next callback while stale display
and stale edits remain forbidden. Keyboard movement retriggers active help;
mouse selection changes dismiss it. Save and unrelated retained-buffer edits do
not invalidate an otherwise unchanged active editor.

Acceptance guards include active document identity, text epoch, revision,
selections, pane/focus, workspace, input interaction, effective hint settings,
and exact provider/server identity. Edit followed by Undo, source replacement,
or settings A→B→A cannot revive an earlier reply. Hints never edit or save files.

Native signature requests have their own one-actual-request lane, separate from
completion and other language features. Cancellation and a 15-second native
timeout retain actual capacity until an exact terminal result/error or server
restart. A late reply can release capacity but cannot reopen timed-out help.
The optional host likewise retains its signature lane until its callback really
settles; a matching session/owner/provider/request release acknowledges it. Its
existing five-second callback and six-second transport deadlines remain distinct.
Repeated invocations therefore cannot accumulate canceled-but-running callbacks.

The optional adapter retains the original JavaScript SignatureHelp object, including
opaque/circular fields, and revives it as `activeSignatureHelp` on valid retriggers.
Local navigation updates its active indices. Handles are owner/provider/document
scoped, explicitly released, and purged on lifecycle changes. The cache retains
at most 32 handles and 2 MiB of normalized metadata; arbitrary opaque object heap
size is not measured by that metadata budget.

## Validation and qualification

All overloads validate before publication, including unselected/empty overloads.
Limits are 32 signatures, 128 parameters each, 8 KiB per label/documentation field,
64 KiB aggregate signature labels, and 256 KiB serialized help. Trigger/retrigger
metadata allows at most 16 single-character entries per set. Active indices must
be unsigned integers; valid out-of-range indices use the pinned fallback rules.
UTF-16 parameter offsets reject surrogate splits, and substring labels use the
pinned ASCII word-boundary matcher. Native selection capture is capped at 16,384.

`tests/signature_hints.rs` checks automatic original-character contexts, local
cycling, queued invocations despite ignored cancellation, settings/source/epoch
guards, and preservation across Save/unrelated edits. Host tests cover original
object revival, reentrancy, malformed data, exact release ordering, timeouts, and
bounded retained callback capacity. Unix `tests/signature_hints_pty.py` checks
native and synthetic extension workflows, completion/hint coexistence, unchanged
dirty bytes, CRLF/Unicode save/Undo and terminal restoration. Its explicit
`--real-clangd` mode checks installed automatic C++ tooling with no CLI LSP launch
and an empty PATH, so executable Node is unavailable.

The highest-scoring ready extension provider takes precedence; absent a matching
extension, native LSP supplies hints. Multi-provider null fallback/aggregation,
Markdown rendering, exact widget placement, all real language servers, and a
production signature-extension corpus remain unqualified. Source-informed
contracts target the pinned [VS Code 1.95 hint bindings](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/editor/contrib/parameterHints/browser/parameterHints.ts)
and [extension adapter](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/workbench/api/common/extHostLanguageFeatures.ts).
Passing named synthetic/native workflows does not establish full signature parity.
