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

## Implementation and evidence

| Surface | Implemented behavior | Evidence and remaining qualification |
| --- | --- | --- |
| `SnippetString` | All public builder methods; nested callback numbering; choices; variable defaults; exact source transfer into completion results | Six local host tests and ordinary provider/framed-host tests pass. Native nested navigation, choice UI, variable expansion and `TextEditor.insertSnippet` remain outstanding; see [snippets](SNIPPETS.md). |
| `CancellationTokenSource` | Lazy stable token identity; cancel once; disposal; parent propagation; active and late subscribers; receiver binding; individually disposable late callbacks | Twenty local lifecycle/facade tests and ordinary provider/framed-host tests pass. Source comparisons cover pinned 1.95 and 1.141. The host reports active-listener exceptions to stderr; upstream telemetry/error-handler behavior is not qualified. |
| `Position`, `Range`, `Selection`, `Disposable`, `EventEmitter` | Public overloads, unchanged-object identity, readonly getters, JSON forms, disposal retry/order and event delivery/removal/reentrancy | Eighteen value/lifecycle tests plus four native-boundary regressions pass. Emitter monitoring, broader reflection/constructor fidelity and upstream global error routing remain outstanding. URI and document API completeness are separate work. |
| Filesystem | Asynchronous local `workspace.fs` operations and owner-scoped custom filesystem providers; filesystem values/errors | Eighteen local filesystem tests, 37 native extension tests and the dirty-buffer/undo/recovery PTY workflow pass. Virtual documents, watches, trash, scheme activation and further scheduling/platform qualification remain outstanding; see [filesystem API](FILESYSTEM_API.md). |
| `Uri` | Source-derived parsing, file/from/joinPath/with, encoding, fsPath, JSON caches and identity | Syntax checking, byte-identical generation, 18 POSIX/Windows-branch tests, the 223-test host suite, 133 native extension-related tests and the filesystem PTY workflow pass. Actual Windows/macOS and upstream runtime qualification remain outstanding; see [URI API](URI_API.md). |
| Text documents | Word lookup, UTF16 offsets, logical EOL, strict validation/line snapshots and native EOL/encoding metadata | Fifteen document tests, 238 host tests, 134 native extension-related tests and document/filesystem PTY workflows pass. Language word-pattern registration, wider encodings and reflection/runtime qualification remain pending; see [text document API](TEXT_DOCUMENT_API.md). |
| Workspace scope | Folder lookup, relative paths, live folder/name/root getters and configuration-file absence for the actual single-folder workspace | Twenty module tests, three facade tests, 261 host tests, 134 native extension-related tests and the filesystem/workspace PTY workflow pass. Native multi-root, trust, mutation and workspace events remain pending; see [workspace API](WORKSPACE_API.md). |
| Edit value objects | TextEdit, SnippetTextEdit, WorkspaceEdit, NotebookEdit and related notebook/EOL values; ordered append/reference semantics and shared facade identity | Twenty-one value tests, four adapter/facade regressions and all 286 host tests pass; native code-action and actual sample/provider/queue terminal checks pass. Evidence and limits are recorded in [edit API](EDIT_API.md). General workspace.applyEdit and file/snippet/notebook/EOL/confirmation backends remain outstanding. |
| ES5 value subclasses | Exact shared call/apply helper on the 25 already-exported classes decorated in pinned upstream; preserves constructor identities and private initialization | Seven focused tests and all 293 host tests pass, with 134 native extension-related tests, build/formatting/strict Clippy, and actual terminal legacy WorkspaceEdit/TextEdit application plus CRLF save/Undo. Undecorated classes stay unchanged; this does not correct every existing constructor difference. See [source receipts](../extension-host/es5-class-compat.provenance.json). |
| Extension caller lifetime | Private caller scope independent of command origin; registered callbacks, Thenables and result getters retain their original owner lease and receiver; stale descendants cannot borrow replacement leases | Twenty-six focused tests and all 319 host tests pass, including three regressions that fail on the unfixed code; 134 native extension-related tests and actual provider/surface/filesystem/document terminal workflows pass. Build/formatting/strict Clippy pass. See [caller ownership](EXTENSION_CALLERS.md) for resource limits and remaining qualification. |
| Code-action values | Source-derived mutable CodeActionKind values/constants, coercion/containment and owned CodeAction.kind; native kind validation and request filtering remain independent | Thirteen focused tests and all 332 host tests, 134 native extension-related tests, five native code-action integration tests and actual sample/edit-integrity/queue terminal checks pass. Build/formatting/strict Clippy pass; see [code-action values](CODE_ACTION_VALUES_API.md) for filtering differences and unsupported backends. |
| Document saves | Public `TextDocument.save()` through exact caller/document admission and actual joined native worker receipts; shared didSave publication before true, native Save As cancellation, retained timeout capacity | Eleven gateway tests and all 360 host and 828 native library tests pass, including three unfixed negative controls, framed caller/origin ordering and exact human-wait tests. Four public-save terminal sessions plus shared-document and native save-automation journeys pass; build/formatting/strict Clippy pass. Virtual/custom saves, full save participants, broader encodings and upstream/platform qualification remain outstanding; see [document saves](NATIVE_DOCUMENT_SAVE.md). |
| Markdown values and native documentation | Source-derived MarkdownString constructor/builders and shared ES5 identity; typed bounded inert projection for hover, completion and signature documentation | Seventeen value tests, thirteen native projection/model/lifecycle regressions, all 349 host and 826 native library tests, twenty integration tests and actual Markdown/provider terminal journeys pass. Build/formatting/strict Clippy pass. Full Markdown rendering, icons, HTML, command links, baseUri actions and upstream/platform qualification remain outstanding; see [documentation display](MARKDOWN_DISPLAY.md). |
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
