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
| `Position`, `Range`, `Selection`, `Disposable`, `EventEmitter` | Public overloads, unchanged-object identity, readonly getters, JSON forms, disposal retry/order and event delivery/removal/reentrancy | Eighteen value/lifecycle tests plus four native-boundary regressions pass. ES5 inheritance, emitter monitoring and upstream global error routing remain outstanding. URI and document API completeness are separate work. |
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
