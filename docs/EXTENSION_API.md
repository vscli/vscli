# Full extension API workstream

The entire public stable VS Code extension API is the current implementation
priority. The [broad feature todo](FEATURE_TODO.md) remains the project backlog;
CI repair and release qualification are deferred until after feature work.

The current [native file-participant slice](NATIVE_FILE_PARTICIPANTS.md) connects
all three public will-file subscriptions to real Explorer preflight, configured
timeout, interactive review and grouped ordinary-text application. It preserves
the complete source DTO and refuses unsupported selected kinds as a whole.
Ordered resource application and public file history remain required; the
resource-operation batch is not complete. Historical milestone statements below
retain their own source and executed-evidence scope.

Delivery follows usable API slices with explicit completion criteria. Shared
native mutation/protocol ownership is an integration dependency; independent
environment/localization, read-only language-provider and tooling families need
not wait for every resource backend. Prefer existing native foundations and
evaluate pinned upstream host/type/converter reuse before expanding handwritten
compatibility machinery. The full upstream-host comparison remains outstanding.
Use focused behavior checks while iterating, then required formatting/strict
Clippy and regression checks on the final coherent source. Preserve test commands,
source revisions/digests, binary identity and results so unchanged qualification
is not silently repeated or attributed to a different build.

The native participant integration passes all 1,604 Rust tests across 67 reports
(1,255 library tests; 23 ignored cases provide no qualification), all 783 host
tests, four new participant terminal journeys and four existing file/event,
no-Node path/reset and public workspace-edit journeys. Formatting, strict locked
all-target Clippy and the native build pass. All eight terminal journeys use the
same binary. [Validation receipts](validation/native-file-participants.json)
record the source/binary digests, commands, results and the corrected test-only
reload-gate readiness race. No platform or upstream-runtime qualification is
inferred, and the full resource-operation batch remains unfinished.

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

The separate committed-runtime-name check currently finds **132 of 421 unique
runtime declaration names (31.35%)** in committed native file-event milestone `b8f71c1`. It excludes
322 type-only declarations and collapses repeated overload names. This measures
name presence only: constructors, members, overload behavior and native backends
remain unqualified by this check. It is not a percentage of implemented or fully
compatible API behavior. Provider registrations separately route thirteen of 32
stable families (40.63%); their existing implementations still have documented limits.
To inspect any committed revision without counting unfinished working-tree code:

```sh
python3 scripts/extension_api_surface.py --revision HEAD --output /tmp/vscli-api-surface.json
```

The [resource-edit workstream](RESOURCE_EDITS_PLAN.md) now includes actual Linux
filesystem execution, durable checkpoints, guarded rollback and an original-file
DTO adapter, alongside native resource-lifetime guards. The combined work has
166 focused Rust controls and 1,555 full Rust tests passing. Ten new adapter controls
include real joined-worker execution; the 26 new executor controls cover actual
filesystem and cold-recovery outcomes. Public resource gateway integration,
model lifetimes and operation history remain unfinished. These additions provide
no new facade names or provider families. [Model lifetime contracts](RESOURCE_MODEL_LIFETIMES.md)
record the conditional destination reuse and reference ownership required next.
Seventeen ledger controls, twenty App controls and two checked-reload integrity
controls now qualify native API acquisition, last-tab retention, expiry, customer
teardown and retained-file Discard. Three App controls use actual selected hosts
for unchanged cache deadlines, expiry close publication and virtual refresh.
Matching native resource mirrors now return before provider/backend routing,
with five new resource-cache controls alongside the earlier eight file controls.
All 740 host tests and two native document PTY journeys pass. The PTY includes
reopen with all eight service slots busy, then shared identity, Undo/Redo, host
crash, dirty recovery and save; the strengthened document PTY also passes clean
tab close/reopen and dirty Discard/reopen/Undo on the original public document.
Formatting and strict locked all-target Clippy pass. File misses now share a
customer-owned native read by original URI, with eight load lanes and 128 pending
waiters. Seven new host controls and an exact retained-wire control cover
coalescing, independent caller retirement, timeout capacity and refused-admission
retry. Three new Rust controls cover native caller/session/deadline admission,
original resource/profile proofs and an actual two-extension concurrent-load
workflow with one acquisition per URI. Native read/state workers settle and join
before freeing their occupied slot. Shared virtual/untitled miss loading, encoding
reload and exact disposal/history archival remain outstanding. Resource-operation
model events and conditional destination reuse still need their controller.
All twenty-one selected terminal journeys pass: two document/recovery, four
public tab-close, three untitled, one virtual, six preview and five sticky-close
policy journeys. These preserve native/shared identity, exact Unicode CRLF disk
bytes, Save/Discard/Cancel and Undo/Redo; they do not qualify resource execution,
mouse input, other platforms or full extension compatibility. Fresh build passes.
That twenty-one-journey set is the earlier retention milestone. Three journeys
were rerun on the customer-load continuation: two document/recovery and one
provider-resource workflow. The document journey now starts with concurrent URI
and relative-path opens returning the same public model; the provider journey
verifies cached access after provider disposal without another callback.

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

The navigation follow-up uses one stateful optional-host ordering authority for
public queries and native actions. Cached candidate order, competing exclusives,
registration and disposal follow the pinned source rather than independent
host/native rankings. All 676 host and 1,322 Rust all-target tests pass (995
library; 23 ignored), including real framed metadata/source/slot controls and a
public/native interleaving App workflow. The new registry PTY, four navigation
PTYs and original provider PTY pass, as do build, formatting and strict Clippy.
This improves behavior behind existing
API names; the committed name-presence figure is unchanged.

[Global workspace symbols](WORKSPACE_SYMBOLS_API.md) add selector-independent
queries without an editor, fresh public symbol values and original-object lazy
resolution through native Ctrl+T. Hidden indexing may await native document
opening without dismissing the picker or changing focus. Actual callback slots
survive cancellation until settlement, and pre-wire admission errors reject the
whole cohort rather than publish partial success. All 694 host and 1,337 Rust
all-target tests pass (1,003 library, 23 ignored), including seven actual
selected-host/App workflows. Four new workspace-symbol terminal journeys, three
native-only Outline journeys and the real installed-clangd symbol test pass.
The relevant original provider/navigation/registry/refactoring journeys also
pass, bringing this cohort to sixteen terminal journeys.
Build, formatting and strict Clippy pass. Mixed native/extension aggregation and
unsupported URI editor backends remain incomplete.

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
| Workspace text transactions | Public `workspace.applyEdit` for retained local/untitled documents; atomic native multi-model publication, mirror/events before success, grouped Undo/Redo and independent actual Save | All 428 host tests and 1,120 Rust tests pass (866 library tests; 23 ignored cases grant no qualification), including 29 new native/document/real-host tests. The public transaction/keyboard/Save PTY and formatting/strict Clippy/build pass. Details are recorded in [native workspace edits](NATIVE_WORKSPACE_EDITS.md). That historical milestone excluded unopened resources; the rename milestone below adds local unopened text files. Custom resources, file/snippet/EOL/annotation/notebook backends and upstream/platform parity remain outstanding. Refactoring autosave is added in the continuation milestone below. |
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
| Workspace symbols | Global provider/query APIs, settlement-order sorting/group deduplication, private original-object resolution, hidden indexing and original native Ctrl+T navigation | All 694 host and 1,337 Rust tests, seven selected-App cases, four new terminal journeys and real clangd regression pass; build/fmt/strict Clippy pass. Mixed native/extension aggregation and unsupported URI editor backends remain outstanding; see [workspace-symbol APIs](WORKSPACE_SYMBOLS_API.md). |
| Rename providers and unopened text cohorts | Public prepare/execute commands, full ordered rich query values, original native F2/LSP preparation, atomic dirty/shared/hidden/unopened text edits and grouped history; negotiated actual alias receipts | All 714 host and 1,361 Rust all-target tests pass (1,019 library; 23 ignored), with six selected-host/App workflows, two no-Node LSP workflows, fourteen native transaction tests and two real framed protocol tests. Nineteen terminal journeys and installed clangd 23.1.1 rename pass; build/fmt/strict Clippy pass. At that milestone default multi-resource autosave remained unfinished; the continuation below implements it. Resource/snippet/EOL/annotation/notebook application, custom word patterns and wider package/platform fidelity remain unfinished; see [rename contracts](RENAME_PROVIDERS_API.md). |
| Refactoring save continuations | Default multi-resource public applyEdit/F2 uses scoped native saves, hidden native LSP participants, Save As transition receipts and applied-true warnings after save failures | All 719 host and 1,389 Rust all-target tests pass (1,040 library; 23 ignored), including seven new selected-extension cases and four framed hidden/retained LSP cases. Seven new terminal journeys, build/fmt and strict locked all-target Clippy pass. Rich native resource/snippet/EOL/annotation/notebook backends and complete extension save participants remain outstanding; see [refactoring saves](REFACTORING_SAVES_API.md). |
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

The generated-load continuation now grants original customer admission before
provider callbacks/untitled creation, with one-use resource/kind/session/settings
tickets and a nonrenewable 60-second clock. It shares virtual and explicit
untitled URI misses without borrowing the first consumer's lifetime; fresh
noarg/options documents stay distinct. Provider disposal still refuses its
creator's work, and publication timeout retains the original shared entry until
actual terminal response. Five ticket tests, one real three-package failed-
activator/other-consumer test, six host workflow and two retained-wire controls
pass. All 748 host and 1,519 Rust tests pass (1,170 library; 23 ignored), using
four Rust test threads after an unrelated parallel theme timeout passed alone.
The three untitled and one provider terminal journeys pass. Formatting, strict
locked Clippy and build pass. Full resource/model/history integration, encoding
reload, provider corpus/platform behavior and remaining SDK/provider families
remain open; CI work remains deferred.

The native resource model archive boundary now captures original live Document
authority with private history/source guards and COW dirty text/baseline. Its
worker encoder retains exact bytes, EOL, selected language and baseline under
the original deadline and shared encoded-byte bounds. Eight new integrity tests
plus production-source worker and physical-overwrite controls pass. All 1,527
Rust all-target tests pass (1,178 library; 23 ignored); formatting and strict
locked all-target Clippy pass. Host sources remain at the separately qualified
748-test generated-load milestone. Full native history/view archival, surviving
destination restoration, App publication and public resource edits remain open.

Native destination restoration now has eleven behavioral controls plus two EOL
history controls. Original surviving targets keep their identities, version
lineage, shared views and history; absent targets get fresh initial models. Dirty
source text restores, clean sources use actual disk postimages, and equal text can
become dirty without a fabricated text version. Explicit/inferred language and
ordinary/grouped EOL history remain separate native metadata. Actual Linux
overwrite-move, displaced dirty archives and physical rollback are coupled in one
control. App survival arbitration, complete retired history/view archives,
operation events, mixed-group application and public resource edits remain open.

All 1,540 Rust all-target tests pass (1,191 library; 23 ignored) with two test
threads; all 748 host tests, formatting, strict locked all-target Clippy and build
pass. A new native EOL terminal journey verifies Revert LF→Undo CRLF→Redo LF,
future Enter/Save, exact Unicode bytes and disk baseline preservation without
Node. The native Explorer resource-lifetime journey also passes. An initial full
run caught a new snippet Redo count-shadowing regression, which was corrected;
the four recorded snippet trace controls pass unchanged. A subsequent parallel
theme timeout passed alone and in the bounded final suite. No local reference
editor was launched; CI work remains deferred.

Native File Revert now connects the restoration kernel to an App-owned joined
resource worker. Confirmation binds original document/resource identity; disk
reads and diff preparation stay off input/rendering. Original model/history/
language/resource/settings proofs are checked before authorization and after
thread exit/join. Accepted publication keeps shared identity, advances baseline
lifetime and invalidates prior disk-watch replies. Pending reads pin their model
and exclude saves; retirement retains occupied capacity until actual exit.
Nine App controls and a 17 MiB shared-backing control pass. All 1,550 Rust
all-target tests pass (1,201 library; 23 ignored), with formatting, strict locked
all-target Clippy and build. Four terminal journeys pass: native EOL Revert/
Undo/Redo/Enter/Save, Explorer path lifetime and two extension-document/restart
journeys. Host sources retain their separately qualified 748-test milestone.
Bulk file effects, surviving-destination arbitration, retired history/view
archives, ordered mixed groups, public resource events/history and the resource
extension gateway remain unfinished.

Three public file operation completion events now deliver joined native Explorer
create/rename/trash outcomes after accepted document mirrors. Source event objects,
Uri payloads, mutable arrays/pairs and scoped listener lifetimes are covered
independently of watcher notifications. A bounded queue retains original messages
through actual writer pressure; completed move/delete hooks remove source-tree API
references independently of models/destination ownership. Five native and five
host controls pass, including a real extension callback, plus a create/rename/
trash terminal journey. All 1,555 Rust tests (1,206 library; 23 ignored), 753 host
tests, formatting, strict locked Clippy and build pass. See
[file operation events](FILE_OPERATION_EVENTS_API.md). Will participants, bulk/
mixed apply, public file history and exact lifetime/event/platform qualification
remain open.

The private will-participant engine now implements sequential awaited delivery,
synchronous wait registration, cancellation, original owner leases, bounded
actual occupancy and full versioned WorkspaceEdit collection. Fifteen engine
controls and two converter controls pass. Public will names remain unavailable
until native pre-operation application, timeout/cancellation and confirmation
are connected; this foundation does not increase the public API name metric.
See [participant contracts](FILE_OPERATION_EVENTS_API.md#will-participant-engine-foundation).

The opt-in [native participant transport](FILE_PARTICIPATION_TRANSPORT.md) now
delivers full source DTOs from real selected extensions, preserving
conversion-time target identities/versions and original contributor lifetimes
through writer pressure, cancellation and separate reply/completion receipts.
Ten host and nine native controls qualify this boundary. Default editor
negotiation remains disabled until
pre-operation review/application and filesystem authority are connected.

The [review/preferences foundation](FILE_EDIT_REVIEW.md) now implements the
source initial choice table, complete original-edit overview and joined profile
storage. Native F1 and selected extensions can reset the remembered policy with
`files.participants.resetChoice`, including without an editor. Six review, five
storage, five App and one host controls qualify this layer. Actual diff review,
full apply and default will subscriptions remain unfinished.

The [committed runtime-name report](extension-api/1.141.0/runtime-name-presence.json)
records the three added file completion events at `b8f71c1`: 132/421 names
(31.35%), with unchanged 13/32 provider-family routing. The report excludes
working-tree changes and grants no overall behavioral compatibility percentage.
