# Rename providers

This cohort connects `languages.registerRenameProvider`,
`vscode.prepareRename(uri, position)`,
`vscode.executeDocumentRenameProvider(uri, position, newName)` and original
`editor.action.rename`/F2 to native prepared workspace transactions. Local host,
Rust, terminal and installed-clangd workflows pass; broader package/platform
qualification remains outstanding. JavaScript
is optional for the native editor and its independent LSP rename path.

The source pin is VS Code 1.141.0 immutable commit
[`2a59476c9bfcb90b3ddc372c36762471b7dfad1c`](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/editor/contrib/rename/browser/rename.ts).
The provider adapter is in
[`extHostLanguageFeatures.ts`](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostLanguageFeatures.ts),
and public value reconstruction is in
[`extHostTypeConverters.ts`](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostTypeConverters.ts).
Source fidelity is separate from historical reference captures and unchanged
extension-package qualification.

## Public preparation and queries

Preparation accepts a Range or `{range, placeholder}`. A Range derives its text
from the source; an object requires a truthy range and placeholder. The requested
line must be within the range's lines; character containment is not an additional
rule. Null/missing results fall through. Message-bearing string/Error rejections
remain distinct from generic failures. An undocumented `defaultBehavior` object
is not a new stable RenameProvider overload.

The source-ordered provider skeleton retains the successful preparation index.
Encountering a provider without preparation falls back to the captured source
word and resets the edit index to zero. Rejection reasons do not disappear into
successful word fallback. Executing rename prepares again, independently of an
earlier public preparation call, then tries edit providers from the selected
index until an edit is accepted. A branded empty edit is accepted; a wrong brand
converts to an empty DTO. With no providers, execution returns a fresh empty
WorkspaceEdit. Public names may be empty strings; provider policy determines
validity.

These commands query values: they do not focus an editor, apply changes or save
files. The family uses the same host-owned stateful selector ordering as native
navigation; it does not introduce a separate native provider winner cache.
Fallback text is captured privately rather than trusting extension-overridden
document presentation methods. Effective word-pattern fidelity and native
bounded single-line admission need their own explicit checks. The initial native
fallback uses the pinned default pattern; custom native language word patterns
remain explicitly unsupported.

## Returned values and native application

Provider conversion preserves the complete ordered DTO before native admission.
Public reconstruction follows pinned source behavior: resource rows precede
grouped text/snippet rows; versions, annotations and EOL metadata are not a public
round-trip promise. Notebook entries have no notebook-specific reconstruction
branch. Returning a value does not qualify an actual notebook, resource or snippet
apply backend. `WorkspaceEdit.entries()` exposes text groups and is not a complete
inspection API for private resource/snippet records. Source-bound host tests must
independently verify those ordered conversion details.

The first native application slice prepares local regular-file text targets,
including genuinely unopened files, in one bounded worker. It captures retained
dirty/shared/hidden models before provider execution, stages all new models and
history, and publishes the full admitted cohort once. Keyboard Undo/Redo owns that
whole cohort; separate per-file Undo is not presented as grouped history. New
worker-loaded targets stay hidden until genuinely shown, and existing model,
membership and disk-baseline identity remain authoritative.

Resource operations, snippets, EOL, confirmation annotations, notebook changes
and unsupported schemes require their actual backends. A mixed unsupported edit
must refuse as a whole rather than apply its text subset. These remain explicit
dependencies of the full API goal.

Native F2 respects refactoring save policy. The qualified multi-resource workflow
sets `files.refactoring.autoSave: false`, preserving dirty working copies and
original disk bytes. A separate single-resource workflow leaves the default
policy intact and checks that it does not autosave. Genuine default multi-resource
refactoring autosave, its receipts/conflicts/SaveAs continuations, and disk-history
semantics are not qualified by these tests; that owned save cohort remains a
separate integration obligation.

## Executed integrity checks

`tests/fixtures/rename-provider-extension` is a selected synthetic extension using
real mirrored documents, public registration, callbacks and cancellation tokens.
It does not write source files. `tests/extension_rename.rs` passes six workflows:

- Inactive-source public preparation/fallback/index tests, repeated preparation,
  empty names, generic/message errors, private word fallback, fresh branded values,
  public conversion boundaries and unsupported mixed apply refusal.
- Original F2 across dirty source, shared dirty target, hidden dirty model and a
  genuinely unopened target, with complete grouped Undo/Redo and exact Unicode
  CRLF disk preservation under explicit autosave-off policy.
- A single-resource default-policy F2 change retaining dirty identity, save
  generation and original/rename histories without an automatic save.
- Held preparation/edit callbacks across source edit+Undo, settings ABA and
  provider retirement, preserving intentional Redo and original text/disks.
- Eight real held public callbacks, a ninth refusal, actual settlement and fresh
  query reuse without source mutation.
- An unopened symlink URI editing the original dirty retained model, with native
  URI-to-document receipts, mirror publication before success, grouped history
  and unchanged disk bytes.

The control command used after disposal also intentionally cancels native UI;
that case does not isolate registration disposal from ordinary command retirement.
Exact provider/source proof controls belong to the independently executed native
and host tests too. Logical cancellation does not fabricate actual callback
capacity, and no test retries the target to obtain a preferred result.

`tests/extension_rename_pty.py` passes five selected-extension terminal journeys
and one separate native LSP control using the existing framed
`tests/fixtures/lsp_server.py`. The LSP control starts absolute Python with no
extension or Node and an empty PATH; it checks original F2 fallback, native
Undo/Redo and exact explicit-save bytes. Other journeys use absolute optional
Node, disabled LSP, original F2/Enter/Escape, real receipts and terminal restoration.
All six journeys pass, including terminal-mode restoration.

`tests/native_rename.rs` and `tests/fixtures/rename_server.py` additionally prepare
two passing genuine no-Node App tests, covering six capability/response cases: boolean
rename support without preparation, preparation Range, supplied placeholder,
default behavior, null rejection and error rejection. The actual framed server
checks advertised client capabilities and records whether preparation/rename
were called. Null/error cases must never reach the name prompt or edit request.
These LSP response contracts are described in the official
[3.17 rename specification](https://github.com/microsoft/language-server-protocol/blob/gh-pages/_specifications/lsp/3.17/language/rename.md).

Bounds and unsupported operations are native adaptations, not a claim of complete
VS Code parity. Broader word patterns, every resource scheme, full value backends,
autosave and platform/package qualification remain separately recorded.

Fourteen focused native transaction tests cover unopened loading, dirty canonical
aliases, no-op model publication, bounded admission, target receipts, cancellation
with actual joining and all-target refusal. Target receipts contain the original
requested URI and the actual native document ID; they invent no exposed version.
They are negotiated internally and validated against the original submitted
cohort before host capacity can be released. The complete actual mirror must
exist before public success, including when an alias names an existing dirty
model. Failed/canceled transactions return no successful mappings.

The two actual framed protocol tests in `src/lsp_rename_tests.rs` pass held
preparation cancellation, malformed responses, original request authority,
logical timeout, retained physical capacity, late settlement and edit+Undo
staleness. The peer records client responses to malformed server requests rather
than echoing them as invented rename completion; both protocol barriers must be
observed before checking retained capacity.

At this milestone **714 host tests and 1,361 Rust all-target tests pass**, including
1,019 library tests across 66 reports; 23 ignored cases grant no qualification.
Build, `cargo fmt --all --check` and strict all-target locked Clippy pass. Nineteen
actual terminal journeys pass: six rename, one installed-clangd rename, one
workspace-edit, three single-resource refactoring policy, four navigation and
four workspace-symbol journeys. `tests/clangd_rename_pty.py` uses installed clangd
23.1.1 through native user settings, without `--lsp`, with an empty PATH and
missing Node. Original F2 prepares the source, renames both C++ occurrences,
retains prior dirty work, groups Undo/Redo and preserves exact Unicode CRLF Save
and committed disk-baseline behavior. No VS Code/Electron process is launched.
This is one real language-server workflow, not full C++ extension qualification.

Reproduce the focused qualification with:

```sh
node --test extension-host/rename-providers.test.cjs extension-host/workspace-edit-values.test.cjs extension-host/workspace-edits.test.cjs
cargo test --locked --lib unopened_tests
cargo test --locked --lib lsp::rename_tests
cargo test --locked --test extension_rename --test native_rename
cargo build --locked
python3 tests/extension_rename_pty.py target/debug/vscli
python3 tests/clangd_rename_pty.py target/debug/vscli
```
