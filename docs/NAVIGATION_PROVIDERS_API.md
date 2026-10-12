# Navigation provider APIs

This milestone connects optional language providers to native Definition,
References, Implementation, Type Definition and Declaration navigation, and to
the corresponding public query commands. JavaScript remains optional for core
editing. The bounded workflows below pass local host, native App and real Unix
terminal checks; broader upstream/platform and extension-package qualification
remains separate.

The source contract is VS Code 1.141.0 at immutable commit
[`2a59476c9bfcb90b3ddc372c36762471b7dfad1c`](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostLanguageFeatures.ts).
The related public declarations are in
[`vscode.d.ts`](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vscode-dts/vscode.d.ts).
This source contract is separate from historical 1.95 reference captures and
named completion/formatting package qualification.

## Public queries and native commands

| Provider registration | Public query command | Original native action |
| --- | --- | --- |
| `registerDefinitionProvider` | `vscode.executeDefinitionProvider` | `editor.action.revealDefinition` |
| `registerReferenceProvider` | `vscode.executeReferenceProvider` | `editor.action.goToReferences` |
| `registerImplementationProvider` | `vscode.executeImplementationProvider` | `editor.action.goToImplementation` |
| `registerTypeDefinitionProvider` | `vscode.executeTypeDefinitionProvider` | `editor.action.goToTypeDefinition` |
| `registerDeclarationProvider` | `vscode.executeDeclarationProvider` | `editor.action.revealDeclaration` |

Queries take a public `Uri` and `Position`, return locations, and do not focus an
editor or edit text. Reference queries pass `includeDeclaration: true` and return
`Location[]`. The other four queries can return `Location` and `LocationLink`
values. Links retain `targetUri`, full `targetRange`, `targetSelectionRange` and
optional `originSelectionRange`; those fields do not collapse to a single range.
Public queries can retain valid virtual and untitled resource URIs. Native
navigation can refuse a resource that has no supported native backing; returned
API values alone do not imply that every URI scheme has an editor backend.

All matching providers participate. Invocation follows selector score and newest
registration on ties. Public results are sorted by encoded URI and the **full**
location range or link target range, then deduplicated on that key. The first
provider wins equal keys even when link selection ranges differ. Individual
ordinary provider errors/null results do not discard another provider's valid
results. Provider invocation order is distinct from sorted result order.

A native location picker represents multiple targets. Accepting a target collapses
the caret at the start of its `range`, or `targetSelectionRange` for a link. It
does not select the full symbol. Existing dirty/shared/hidden target models keep
their identity and history, including when their backing file has been deleted.
Closed-file navigation still requires its bounded native loader and fresh range
validation.

The historical `editor.action.goToDeclaration` alias means **definition**; it is
not the new declaration route. The original Implementation shortcut is Ctrl+F12
on Linux/Windows and Cmd+F12 on macOS, subject to the repository's terminal key
constraints and user bindings. Type Definition and Declaration receive no
invented default shortcuts. All five actions are accessible from the native
command palette.

## Ownership and bounds

Provider creator and invoking caller retain their original selected-owner
lifetimes. Native replies also retain source document/version/text epoch,
effective language, registry, pane/view, workspace/settings and interaction
proofs. An edit followed by Undo, a language or view A→B→A transition, disposal or
owner retirement cannot revive an obsolete target or picker.

The registry is bounded to 128 providers. At most eight callbacks are actually
active, with bounded dispatch for larger matching cohorts; normalized navigation
results have a 512-location/1 MiB limit. Public queries also have a bounded
simultaneous admission. A callback that ignores cancellation continues to occupy
its actual slot until settlement. Logical cancellation/timeout does not fabricate
capacity. Malformed or over-budget data is refused without changing native text,
Undo/Redo or disks.

## Executed vertical checks

`tests/fixtures/navigation-providers-extension` is a selected synthetic extension,
not an unchanged third-party package. It registers all five families through the
public API, uses shared mirrored documents and real cancellation tokens, and
observes actual native/public effects without writing document files.

`tests/extension_navigation_providers.rs` passes four selected-App workflows:

- All five original native commands aggregate callbacks and reveal the collapsed
  selection start in a retained dirty target whose backing file is deleted,
  preserving exact original/target Unicode CRLF text, identity and Undo/Redo.
- Public queries preserve all Location/Link fields, sort/deduplicate, isolate
  provider errors, and query inactive/virtual/effective-language documents without
  focus or source changes.
- A real held callback survives logical cancellation while source edit+Undo,
  language/view ABA or registration disposal retires its native authority; late
  results cannot revive the UI or erase intentional history.
- Eight held public callbacks retain capacity; a ninth refuses, then actual
  settlement permits both original results and a fresh query with source/history
  intact.

`tests/extension_navigation_providers_pty.py` passes four real terminal journeys
for the same native commands, public queries, held source/language cancellation
and capacity. It uses absolute optional Node with an empty PATH and disabled LSP,
checks original bytes and keyboard Undo/Redo, and restores the terminal on exit.
The milestone passes all **664 host tests** and **1,316 Rust all-target tests**
(990 library; 62 reports; 23 ignored cases grant no qualification), native build,
`cargo fmt --all --check` and strict all-target locked Clippy. Four new terminal
journeys pass. Existing provider, signature-hint and navigation-history terminal
regressions also pass, including the original core workflows with no Node.
Completion resolution, shared group tabs, editor inventory/presentation and welcome
regressions pass too: **25 total terminal journeys** at this milestone.
These are Linux synthetic-fixture receipts, not a full upstream differential or
unchanged third-party navigation-extension qualification.

Reproduce the new behavior checks with:

```sh
node --test extension-host/navigation-providers.test.cjs extension-host/language-selector.test.cjs
cargo test --locked --test extension_navigation_providers
cargo build --locked
python3 tests/extension_navigation_providers_pty.py target/debug/vscli
```

This milestone does not complete the full stable language API. Workspace symbols,
rename/prepareRename, range/on-type formatting, semantic tokens, inlay hints and
the other provider families remain separately tracked. Native LSP aggregation,
all scheme backends, complete selectors/notebook support and every provider
extension's behavior also require their own implementation and qualification.

## Shared stateful provider ordering

Public queries and native actions now select through one optional-host registry
per navigation family. The source scans its current ordered entries for a
matching all-entry-exclusive selector **before** sorting. Registration appends
to that current array, disposal removes the original entry, and repeated URI/
effective-language candidates reuse the cached order. A public Rust query can
therefore change which exclusive provider wins the next native C++ action, even
when that winner has a lower selector score. Independent host/native caches,
newest-only selection and highest-score-exclusive selection cannot reproduce this
behavior. The shared adapter follows the pinned ordering and candidate-cache
algorithm; ordinary score ties prefer newer registrations.

Native actions first request bounded metadata ranking over the exact original
source and registry cohort. The response contains only original provider IDs and
owners; it invokes no extension callback or command and occupies no callback
slot. One actual ranking RPC remains retained through logical cancellation or
expiry, while one latest native intent waits for its real response. Malformed,
duplicate, foreign-family or borrowed-owner entries cannot start callbacks. A
valid empty match enters the original native language-server path once. Original
source/version/language/owner/context guards still protect subsequent callbacks.
Original URI constructors/getters are captured so extension changes to public URI
prototypes cannot introduce callbacks into trusted metadata ranking.

The follow-up passes **676 host tests** and **1,322 Rust all-target tests**
(995 library; 63 reports; 23 ignored). Eight ordering-unit tests, public/rank
interleaving controls, an actual framed rank/public-query test, four native framed
rank tests and the empty-rank App control pass. The new selected-App workflow
interleaves public/native candidates, cached requests, registration and disposal;
it checks exact original shared memberships, the legitimate new membership in
the source group, collapsed caret positions, Unicode CRLF text and both histories.
The new actual registry terminal journey, all four navigation terminal journeys
and the existing seven-route provider journey pass (six terminal journeys total
for this follow-up). Native build, formatting and strict all-target locked Clippy
also pass. Earlier milestone terminal receipts retain their separate scope.
Broader notebook selectors, native context-key glob matching across platforms,
built-in/extension mixed registries and upstream runtime qualification remain
separate gaps.

Additional shared-registry qualification commands:

```sh
node --test extension-host/provider-order.test.cjs extension-host/provider-wire.test.cjs
cargo test --locked --test extension_navigation_registry
python3 tests/extension_navigation_registry_pty.py target/debug/vscli
```
