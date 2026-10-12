# Full extension API workstream

The entire public stable VS Code extension API is the current implementation
priority. The [broad feature todo](FEATURE_TODO.md) remains the project backlog;
CI repair and release qualification are deferred until after feature work.

The [complete pinned stable inventory](extension-api/1.141.0/README.md) records
776 declarations and 2,366 member/overload/inline-signature entries, with proposed
APIs tracked separately. These are declaration counts, not feature counts or
compatibility percentages. Static source presence and tested behavior have
separate evidence; the inventory alone grants no new behavioral qualification.

The existing host reports `vscode.version = '1.95.0'`. That identifier is not a
claim of full 1.95 compatibility. The complete stable API inventory is being
tracked separately against immutable upstream declarations, including newer
members. Neither installing a VSIX nor finding an exported name establishes
behavioral compatibility.

Progress reports distinguish usable public API workflows, implemented subsets,
and outstanding backends. Constructor/namespace export counts measure surface
presence only; changes to document saves or workspace transactions can deliver
new behavior without changing those counts. There is no measured overall API
compatibility percentage. The frozen declaration audit is a separate source
snapshot, not a live implementation dashboard.

The separate committed-runtime-name check currently finds **127 of 421 unique
runtime declaration names (30.17%)** in committed milestone `7af9165`. It excludes
322 type-only declarations and collapses repeated overload names. This measures
name presence only: constructors, members, overload behavior and native backends
remain unqualified by this check. It is not a percentage of implemented or fully
compatible API behavior. Provider registrations separately route eleven of 32
stable families (34.375%); their existing implementations still have documented limits.
To inspect any committed revision without counting unfinished working-tree code:

```sh
python3 scripts/extension_api_surface.py --revision HEAD --output /tmp/vscli-api-surface.json
```

The development audit uses Node to inspect the real facade over an empty
activation registry. It loads no extensions, opens no editor and invokes no API
operation. Native editor startup has no dependency on this script or Node.

## Live workflow progress

The declaration/export audit is frozen evidence of API shape, not a live count of
working workflows. Recent delivered behavior includes retained-document
[workspace transactions](https://github.com/vscli/vscli/commit/647b8f9), native
[file discovery](https://github.com/vscli/vscli/commit/5f40cba) and shared OS
[filesystem watchers](https://github.com/vscli/vscli/commit/86fe0a8). Their evidence
and outstanding backends appear below. [Virtual text documents](VIRTUAL_DOCUMENTS_API.md)
now pass public host/App and terminal workflows, including readonly protection
and preserved dirty-file Undo/Redo. [API-created untitled documents](UNTITLED_DOCUMENTS_API.md)
now also pass content/language/noarg opens, associated destinations, Save As with
public close/open/save transitions and metadata recovery. New workflow delivery
can advance without changing constructor/export figures or implying an overall
compatibility percentage.

Delivered editor-state integration adds
[language switching and selection setters/events](EDITOR_STATE_API.md), with
language metadata lifetimes that retire obsolete language-provider and LSP work
without changing text versions or history. Each milestone's evidence below is
updated after actual host/native/terminal execution; authored tests alone do not
count as delivered qualification.

[Visible editor inventory](EDITOR_INVENTORY_API.md) adds distinct public handles
and selections for shared native panes, all visible editors, view-column events,
showTextDocument column/preserveFocus overloads and retained show receipts. These
are new behaviors behind existing API names; they do not change the frozen
declaration/export audit or establish an overall compatibility percentage.

[Editor presentation](EDITOR_PRESENTATION_API.md) now adds native options and
events, measured UTF-16 visible ranges and events, and revealRange in all four
modes. Indentation is shared between panes; cursor/line-number styles and scrolling
target the actual addressed view. The live evidence below advances separately
from the frozen declaration audit.

[Editor decorations](EDITOR_DECORATIONS_API.md) adds live type lifecycle,
per-pane range setters, edit-aware UTF-16 markers and native colors/common
styles. Source-backed marker flags distinguish public inserts/deletes from
ordinary native typing. Rich render fields retained in the adapter still have
explicit native display gaps; exported API names alone grant no qualification.

[Public editor snippet insertion](EDITOR_SNIPPETS_API.md) now has a bounded host
adapter and native view-specific bridge for overloads, options, linked fields and
Undo/Redo. All 617 host tests and 1,279 Rust all-target tests pass, with four new
actual selected-host/App workflows and four new terminal journeys. The original
snippet corpus and frozen API audit keep their own scope.

[Public native tabs](EDITOR_TABS_API.md) now expose `window.tabGroups`, stable
tab/group objects, all seven stable TabInput value classes, shared/inactive text
memberships, change events and close overloads. Real Save/Discard/Cancel and
untitled Save As finish through native persistence and mirror-before-ACK receipts.
Welcome retains one real empty native group. All 641 host tests and 1,304 Rust
all-target tests pass (982 library, 61 reports, 23 ignored), including five actual
tab host/App workflows. Four new tab PTYs and 26 relevant regression journeys
pass, along with formatting and strict Clippy. This delivers text-tab workflows;
other input constructors do not establish their editor backends.

[Navigation provider APIs](NAVIGATION_PROVIDERS_API.md) add Implementation, Type
Definition and Declaration alongside Definition and References. All five families
have public query commands, complete Location/Link fields, matching-provider
aggregation and native location pickers with a collapsed target caret. Actual
callback slots survive logical cancellation until settlement. All 664 host and
1,316 Rust all-target tests pass (990 library; 23 ignored), with four new actual
selected-host/App cases and four new PTY journeys. Build, formatting and strict
Clippy pass. This routes eleven of the 32 stable provider families; remaining
providers and mixed native/extension aggregation are still open.

## Implementation and evidence

| Surface | Implemented behavior | Evidence and remaining qualification |
| --- | --- | --- |
| `SnippetString` | All public builder methods; nested callback numbering; choices; variable defaults; exact source transfer into completion results | Six local host tests and ordinary provider/framed-host tests pass. Public insertion is tracked separately below; richer choice/nested-session behavior and broader variable qualification remain outstanding; see [snippets](SNIPPETS.md). |
| `CancellationTokenSource` | Lazy stable token identity; cancel once; disposal; parent propagation; active and late subscribers; receiver binding; individually disposable late callbacks | Twenty local lifecycle/facade tests and ordinary provider/framed-host tests pass. Source comparisons cover pinned 1.95 and 1.141. The host reports active-listener exceptions to stderr; upstream telemetry/error-handler behavior is not qualified. |
| `Position`, `Range`, `Selection`, `Disposable`, `EventEmitter` | Public overloads, unchanged-object identity, readonly getters, JSON forms, disposal retry/order and event delivery/removal/reentrancy | Eighteen value/lifecycle tests plus four native-boundary regressions pass. Emitter monitoring, broader reflection/constructor fidelity and upstream global error routing remain outstanding. URI and document API completeness are separate work. |
| Filesystem | Asynchronous local `workspace.fs` operations and owner-scoped custom filesystem providers; filesystem values/errors | Eighteen local filesystem tests, 37 native extension tests and the dirty-buffer/undo/recovery PTY workflow pass. Custom filesystem document editing, trash, scheme activation and further scheduling/platform qualification remain outstanding; see [filesystem API](FILESYSTEM_API.md). |
| `Uri` | Source-derived parsing, file/from/joinPath/with, encoding, fsPath, JSON caches and identity | Syntax checking, byte-identical generation, 18 POSIX/Windows-branch tests, the 223-test host suite, 133 native extension-related tests and the filesystem PTY workflow pass. Actual Windows/macOS and upstream runtime qualification remain outstanding; see [URI API](URI_API.md). |
| Text documents | Word lookup, UTF16 offsets, logical EOL, strict validation/line snapshots and native EOL/encoding metadata | Fifteen document tests, 238 host tests, 134 native extension-related tests and document/filesystem PTY workflows pass. Language word-pattern registration, wider encodings and reflection/runtime qualification remain pending; see [text document API](TEXT_DOCUMENT_API.md). |
| Workspace scope | Folder lookup, relative paths, live folder/name/root getters and configuration-file absence for the actual single-folder workspace | Twenty module tests, three facade tests, 261 host tests, 134 native extension-related tests and the filesystem/workspace PTY workflow pass. Native multi-root, trust, mutation and workspace events remain pending; see [workspace API](WORKSPACE_API.md). |
| Edit value objects | TextEdit, SnippetTextEdit, WorkspaceEdit, NotebookEdit and related notebook/EOL values; ordered append/reference semantics and shared facade identity | Twenty-one value tests, four adapter/facade regressions and all 286 host tests pass; native code-action and actual sample/provider/queue terminal checks pass. Evidence and limits are recorded in [edit API](EDIT_API.md). Retained-document text workspace.applyEdit is now routed separately; file/snippet/notebook/EOL/confirmation backends remain outstanding. |
| ES5 value subclasses | Exact shared call/apply helper on the 25 already-exported classes decorated in pinned upstream; preserves constructor identities and private initialization | Seven focused tests and all 293 host tests pass, with 134 native extension-related tests, build/formatting/strict Clippy, and actual terminal legacy WorkspaceEdit/TextEdit application plus CRLF save/Undo. Undecorated classes stay unchanged; this does not correct every existing constructor difference. See [source receipts](../extension-host/es5-class-compat.provenance.json). |
| Extension caller lifetime | Private caller scope independent of command origin; registered callbacks, Thenables and result getters retain their original owner lease and receiver; stale descendants cannot borrow replacement leases | Twenty-six focused tests and all 319 host tests pass, including three regressions that fail on the unfixed code; 134 native extension-related tests and actual provider/surface/filesystem/document terminal workflows pass. Build/formatting/strict Clippy pass. See [caller ownership](EXTENSION_CALLERS.md) for resource limits and remaining qualification. |
| Native queued caller lifetime | Every queued native service retains its selected-owner retirement flag; failed/disposed owners and client drop retire old authority irreversibly, even across same-ID reactivation | Four native lifetime regressions and five actual App/host Save As deadline regressions, all 837 native library tests, formatting, strict Clippy and build pass. Four public-save terminal workflows pass with the native checks enabled. Individual hot unloading and broader shutdown/platform behavior remain outstanding; see [caller ownership](EXTENSION_CALLERS.md). |
| Code-action values | Source-derived mutable CodeActionKind values/constants, coercion/containment and owned CodeAction.kind; native kind validation and request filtering remain independent | Thirteen focused tests and all 332 host tests, 134 native extension-related tests, five native code-action integration tests and actual sample/edit-integrity/queue terminal checks pass. Build/formatting/strict Clippy pass; see [code-action values](CODE_ACTION_VALUES_API.md) for filtering differences and unsupported backends. |
| Document saves | Public `TextDocument.save()` through exact caller/document admission and actual joined native worker receipts; shared didSave publication before true, native Save As cancellation, retained timeout capacity | Eleven gateway tests and all 360 host and 828 native library tests pass, including three unfixed negative controls, framed caller/origin ordering and exact human-wait tests. Four public-save terminal sessions plus shared-document and native save-automation journeys pass; build/formatting/strict Clippy pass. Writable custom-resource saves, full save participants, broader encodings and upstream/platform qualification remain outstanding; see [document saves](NATIVE_DOCUMENT_SAVE.md). |
| Markdown values and native documentation | Source-derived MarkdownString constructor/builders and shared ES5 identity; typed bounded inert projection for hover, completion and signature documentation | Seventeen value tests, thirteen native projection/model/lifecycle regressions, all 349 host and 826 native library tests, twenty integration tests and actual Markdown/provider terminal journeys pass. Build/formatting/strict Clippy pass. Full Markdown rendering, icons, HTML, command links, baseUri actions and upstream/platform qualification remain outstanding; see [documentation display](MARKDOWN_DISPLAY.md). |
| Workspace text transactions | Public `workspace.applyEdit` for retained local/untitled documents; atomic native multi-model publication, mirror/events before success, grouped Undo/Redo and independent actual Save | All 428 host tests and 1,120 Rust tests pass (866 library tests; 23 ignored cases grant no qualification), including 29 new native/document/real-host tests. The public transaction/keyboard/Save PTY and formatting/strict Clippy/build pass. Details are recorded in [native workspace edits](NATIVE_WORKSPACE_EDITS.md). Unopened/custom resources, file/snippet/EOL/annotation/notebook backends, refactoring autosave and upstream/platform parity remain outstanding. |
| File discovery | Public `workspace.findFiles` and shared `RelativePattern`; native local traversal with default/null/additive exclusions, limits and retained cancellation | Nine native worker tests, exact cancellation-origin regression, 26 new host cases and two actual selected-host workflows pass. All 454 host tests and 1,172 Rust all-target tests pass (876 library; 23 ignored). Public search/cancel/dirty-buffer/Undo/Redo PTY, build/formatting/strict Clippy pass. Multi-root/custom search providers, nested glob braces, global Git ignores and upstream/platform parity remain outstanding; see [file discovery](NATIVE_FILE_SEARCH.md). |
| Filesystem watchers | Public create/change/delete events, ignore flags, string/RelativePattern scopes, custom provider watch routing, shared local OS workers and original caller cleanup | All 482 host tests pass, including seven real OS backend tests, 12 glob cases, eight facade/provider/shared-session cases and retired cleanup. Actual selected-host and keyboard PTY workflows preserve dirty buffers, native Undo and disk bytes. All 1,173 Rust all-target tests (876 library; 23 ignored), build/formatting and strict Clippy pass. OS coalescing, upstream/platform fidelity and remote/multi-root backends remain outstanding; see [watchers](FILE_WATCHERS_API.md). |
| Virtual text documents | Custom-scheme content-provider registration/open/show/change route; shared native URI identity, clean LF-normalized refresh and read-only edit/save protection | All 503 host tests and 1,185 Rust all-target tests pass (886 library; 23 ignored). Two actual selected-host workflows, the public keyboard PTY, build/formatting and strict Clippy pass; see [virtual documents](VIRTUAL_DOCUMENTS_API.md) for limits and outstanding backends. |
| API-created untitled documents | Content/language/noarg and explicit untitled URI opens; retained resource cache, fresh naming, initial Undo baseline, associated create-only saves, public close/open/save transitions and recovery | All 524 host tests and 1,197 Rust all-target tests pass (894 library; 23 ignored), including four actual selected-host/App workflows. Three new untitled PTY journeys, four existing public-save PTYs and the virtual-document PTY pass; build/formatting/strict Clippy pass. Encoding, writable custom resources, full label formatting and upstream/platform fidelity remain outstanding; see [untitled documents](UNTITLED_DOCUMENTS_API.md). |
| Document language and editor selections | Public language switching with same-object close/open events, explicit language-aware native settings/LSP; selection setters, clamping, inactive editors and confirmed scoped selection events | At this milestone all 542 host tests and 1,220 Rust all-target tests passed (913 library; 23 ignored), including fifteen editor-state host cases, three provider lifetime cases, nineteen native library cases and four actual selected-host/App workflows. Two new PTY workflows and eight save/untitled/virtual regressions, build/formatting/strict Clippy passed. Shared-pane public inventory is added in the following milestone; selection kind provenance and other editor APIs remain outstanding. See [editor state](EDITOR_STATE_API.md). |
| Visible editor inventory | Distinct shared-document pane handles, exact inactive selections/edit identity, full visible/active inventory, view columns/events, show overloads/preserveFocus and original retained Show acknowledgement | All 559 host tests and 1,232 Rust all-target tests pass (923 library; 23 ignored), including seventeen new host cases, ten native library cases and two new actual selected-host/App workflows. The four existing editor-state App workflows also pass. Two new inventory PTYs and ten editor-state/untitled/save/virtual regressions pass, as do build/formatting/strict Clippy. See [editor inventory](EDITOR_INVENTORY_API.md) for outstanding preview/sparse groups, decorations, selection kind provenance and upstream/platform fidelity. Options/ranges/reveal are added in the following milestone. |
| Editor presentation | Stable options setters/getters/events, shared indentation and per-view styles, measured visible ranges/events, exact inactive-pane reveal in all four modes | At this milestone all 578 host and 1,244 Rust all-target tests pass (933 library; 23 ignored), including nineteen new host cases, ten new native library regressions and two actual selected-host/App workflows. Two new PTY journeys and twelve inventory/editor-state/untitled/save/virtual regressions pass; build/formatting/strict Clippy pass. See [editor presentation](EDITOR_PRESENTATION_API.md) for width/scan bounds, terminal shapes and remaining wrapped/folded range/reveal fidelity. Decorations follow below. |
| Editor decorations | Private type lifecycle and per-pane setters, edit-aware UTF-16 markers including forced public inserts/deletes, native RGB/common styles and whole-line backgrounds | All 600 host and 1,260 Rust all-target tests pass (947 library; 23 ignored), including 22 new host cases, fourteen new library cases and two actual selected-host/App workflows. Two new decoration PTYs and fourteen presentation/inventory/editor-state/untitled/save/virtual regressions pass; build/formatting/strict Clippy pass. Actual native buffer tests verify styles. Hover/attachments, raster gutter icons, rulers, richer CSS/alpha, arbitrary theme colors and upstream/platform fidelity remain outstanding; see [decorations](EDITOR_DECORATIONS_API.md). |
| Public editor snippet insertion | `TextEditor.insertSnippet` overloads/options, private view-specific native insertion, linked fields and source/caller/version guarded mirror-before-ACK receipts | All 617 host tests (17 new) and 1,279 Rust all-target tests pass (962 library, 60 suites; 23 ignored grant no qualification), including twelve new core cases, three clipboard worker cases and four actual selected-host/App workflows. Four new PTY journeys and build/formatting/strict Clippy pass. Choice menus, command-driven nesting, complete regex and upstream/platform fidelity remain outstanding; see [editor snippets](EDITOR_SNIPPETS_API.md). |
| Public tabs and groups | `window.tabGroups` inventory/getters/events/close, all seven stable TabInput values, distinct shared memberships and actual empty welcome group | All 641 host tests (24 new), 1,304 Rust tests and 30 actual PTY journeys pass; five selected-host/App cases cover inventory/sticky/history, clean arrays/groups, Cancel/Discard, Save/Save As and original owner retirement. Native inventory, empty engine/session and exact retained-request tests pass. Full snapshots may coalesce events; diff/custom/webview/notebook/terminal tab backends remain outstanding; see [tabs](EDITOR_TABS_API.md). |
| Navigation providers | Five public execute queries; three new provider families; full Location/Link conversion, matching cohorts, sorting/dedup and native collapsed-caret reveal | All 664 host and 1,316 Rust tests, four selected-App and four new terminal workflows, build/formatting/strict Clippy pass. Mixed native/extension aggregation, all URI editor backends, notebook selectors and broader provider/package/platform fidelity remain outstanding; see [navigation APIs](NAVIGATION_PROVIDERS_API.md). |
| Other API surfaces | Existing partial document/editor, configuration, state, command, diagnostic, provider and native UI adapters | Existing focused documents describe their implemented subsets. Full API compatibility and arbitrary real-extension compatibility remain incomplete. |

Cancellation follows the pinned source lifecycle: an already-canceled parent
propagates on the next timer turn; disposing an unaccessed source leaves its token
uncanceled. Cancellation is advisory: it never releases a native provider slot
while the actual callback remains pending. Existing owner, generation and
document-version guards remain authoritative.

Public `Range` JSON uses the upstream two-position array. Native edit and
selection messages explicitly project `{start, end}` objects instead. Public
`Position` values match upstream's permissive JavaScript constructor; native
provider, diagnostic and edit boundaries independently require finite nonnegative
LSP integers. Rejected edits and diagnostic batches preserve the current document
and previously published diagnostics.

Comparison sources:

- [Cancellation in 1.95](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/base/common/cancellation.ts)
- [Cancellation in 1.141](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/base/common/cancellation.ts)
- [Stable declarations](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vscode-dts/vscode.d.ts)

No local VS Code instance is launched for this work. Source comparisons and local
host/native tests are reported separately from real upstream runtime observations.
Native Rust editing stays usable without Node; these modules are embedded only
for the optional compatibility process.

The ES5 helper restores function-style subclass construction without replacing
public classes. This matters for extensions compiled to older JavaScript.
Position, Range and Selection are each decorated; Disposable is actually
constructed so private state is initialized. CancellationTokenSource,
EventEmitter, Uri and the newer undecorated edit/notebook types keep upstream's
class-call behavior. Existing bounded symbol/diagnostic constructors still need
their own fidelity work; decorating a class does not qualify all of its behavior.
