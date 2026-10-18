# Refactoring edits and native save continuations

Default multi-resource text refactorings now continue through the existing native
save controller after atomic workspace publication. This applies to public
`workspace.applyEdit(edit, {isRefactoring:true})` and original F2 rename. Native
editing, LSP rename, preparation, persistence and grouped history remain usable
without a JavaScript runtime. Invoking an extension still uses the isolated
optional host.

## Policy and visible behavior

Automatic refactoring saves run when the operation requests refactoring, the root
`files.refactoring.autoSave` value is exactly Boolean `true`, and more than one
resource group participates. The missing setting defaults to true. Explicit
false and non-Boolean values disable this continuation; language overrides do
not replace the root policy. A single-resource refactoring remains an unsaved
text transaction under default, true and false policies.

```javascript
const edits = new vscode.WorkspaceEdit();
edits.insert(first.uri, new vscode.Position(0, 0), 'first ');
edits.insert(second.uri, new vscode.Position(0, 0), 'second ');
const applied = await vscode.workspace.applyEdit(edits, { isRefactoring: true });
```

The text transaction first validates and publishes all admitted models together.
The continuation then attempts to save the involved dirty working copies. A dirty
participant with a byte-identical edit remains involved; unrelated dirty models
are excluded. Retained visible, shared, inactive and hidden models keep their
native document identity. Existing unopened local regular files are loaded by the
bounded transaction worker and remain hidden. Canonical aliases use the original
retained dirty model rather than replacing it with disk content.

The public awaited true and native F2 completion follow settlement of the entire
save cohort. An intermediate clean source or idle save worker is insufficient to
establish completion. Successful saves publish actual baselines and mirror/save
events before the final acknowledgement. For a successfully saved text cohort,
grouped Undo restores the prior text as dirty working copies while committed disk
bytes remain unchanged; grouped Redo restores the saved text and cleanliness.
Native source-action and formatting edits retain their own existing Undo
boundaries when those participants actually mutate text.

## Native ownership and participants

Every involved dirty model receives an original source proof at workspace
publication, before waiting for later save admission. That proof retains the real
model identity, resource/text/language epochs, revision, path and associated path, saved
baseline/save generation, EOL, settings/profile and workspace. Effective-language
changes followed by reversal also invalidate a deferred proof. Moving away and
back invalidates the resource epoch without editing text or erasing history.
Edit followed by Undo, path or
baseline changes, and settings/profile replacement cannot authorize a newer
version through a deferred original save.

Each save enters the normal source-action, formatting, snapshot and native disk
worker stages. The private native caller checks both the original owner lifetime
and a separate cohort cancellation flag at actual worker authorization. Stopping
one cohort does not retire the extension owner or unrelated operations. Generic
extension cleanup and response handling cannot take the cohort's exact save
interests or their receipts. An already-authorized disk worker keeps its outcome
and publishes its genuine receipt even if the owner or cohort retires.

Native LSP synchronization includes actual retained hidden models through the
same iterator as visible models. Hidden save actions and formatters use the
original document ID, source/version and server proof; they do not show an editor
or construct a second model. Owned edits are synchronized before the next
participant and disk capture. Stale edit/Undo replies are rejected, and advisory
cancellation keeps the actual LSP callback lane occupied until its reply or server
retirement. Existing first-save, Save As and after-delay participant restrictions
remain. Extension-provided save participants and complete formatter/action policy
parity remain outstanding.

## Save As, failures and partial persistence

An involved untitled model uses the normal native Save As prompt. Acceptance
preserves its native document ID through actual persistence. A negotiated save
receipt authorizes the original untitled URI's closed mirror and the saved file
URI's new mirror, with the actual save generation. The receipt uses the same
canonical public extension URI as that mirror, including Windows drives and
reserved path characters. No exposed document version is
invented. The close/open/save events precede the public success acknowledgement.
Cancellation retains the applied dirty untitled edits; another participant may
already have committed successfully.

Save failure happens after text publication. Cancellation, a deferred stale
source, an external-write conflict or a failed native save therefore retains
`applied:true` with a warning. Successful peer writes stay committed, failed peers
retain unsaved text, and foreign disk bytes remain protected by the native save
checks. The controller does not roll back editing or automatically retry a failed
save. A committed-but-unpublished receipt is reported as a warning describing
that distinct outcome.

This differs from direct `TextDocument.save()`, whose own cancellation/failure
contract is documented in [native document saves](NATIVE_DOCUMENT_SAVE.md).
Malformed acknowledgement or transport timeout does not prove rollback. The
caller can have an unknown outcome while native edits or writes remain retained;
occupied request capacity is released only by the exact valid terminal frame or
physical owner/session retirement.

## Bounds and current evidence

Text admission retains the existing 128-model/resource, 4,096-edit, 1 MiB raw
replacement, 2 MiB expanded/wire, 4 MiB aggregate model and 8 MiB view/history
metadata limits. One actual preparation worker retains its slot until exit and
join. Its five-second original deadline ends at atomic publication; later save
failure cannot reclassify the published edit as unapplied.

The save cohort retains at most 128 lightweight original proofs and admits one
model at a time through the existing eight-interest table and single actual disk
worker. The next-admission phase has a 30-second queue wait, starting at cohort capture
and resetting after an actual member receipt; repeated polls do not renew it.
Individual save interests retain their existing 30-second preauthorization and 300-second human
Save As phases. The negotiated refactoring acknowledgement uses the existing
360-second retained save-style timeout. Repeated polls do not renew these clocks.
Native LSP save participants keep their existing 1,500 ms participant deadlines
and actual callback occupancy rules. Shutdown joins authorized filesystem work
before recovery.

Executed evidence for this implementation is **719 host tests and 1,389 Rust
all-target tests passing across 67 reports**, including 1,040 library tests. The
23 ignored Rust cases grant no qualification. The 20-case focused refactoring
filter covers actual joined saves, original proof and owner/cohort lifetime
checks, effective-language ABA, exact receipt isolation, Save As and conflicts. Four hidden/retained framed
LSP controls pass in the full suite; three hidden save-participant cases also
pass in a separate focused run.
The fresh native build, `cargo fmt --all --check` and strict
`cargo clippy --all-targets --locked -- -D warnings` pass.

All seven selected-extension tests in `tests/extension_refactoring_saves.rs` and
all seven terminal journeys in `tests/extension_refactoring_saves_pty.py` pass
with `tests/fixtures/refactoring-save-extension`. They exercise default public/F2
four-model saves, didSave-before-true, shared/hidden/unopened identity, exact
Unicode CRLF disk bytes, original grouped Undo/Redo, disabled and single-resource
policy, whole unsupported refusal, real Save As acceptance/cancellation and
partial conflict without retry. Fixture JavaScript calls APIs and observes
events; the test harness owns external conflict setup. PTY sessions restore
terminal mode.

Fresh terminal regression executions also pass against the final sources:
`extension_rename_pty.py` (6), `extension_single_refactoring_pty.py` (3),
`extension_workspace_edits_pty.py` (1), `clangd_rename_pty.py` (1),
`native_format_on_save_pty.py` (5), `native_code_actions_on_save_pty.py` (7), and
`extension_document_save_pty.py` (4). Combined with the seven new refactoring-save
journeys, **34 terminal journeys pass**. The installed clangd control uses version
23.1.1 with no Node and an empty PATH. No CI investigation or local VS Code process
was used.

This is implementation and focused local evidence, not full VS Code parity or a
completed extension API. Resource/file-operation journals, snippet/EOL/annotation/
notebook apply backends, writable custom filesystems, broader encodings, extension
save participants, all upstream configuration and error-policy behavior, arbitrary
extension packages and fresh Windows/macOS qualification remain outstanding. No
local VS Code/Electron process or CI investigation was used.

The source behavior is pinned to VS Code 1.141.0 immutable commit
[`2a59476c9bfcb90b3ddc372c36762471b7dfad1c`](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/contrib/bulkEdit/browser/bulkEditService.ts),
including its exact-true root setting, multi-resource condition and awaited save
attempts. Native bounds, ownership, sequential scheduling and explicit unsupported
backends are local adaptations. See the [integration plan](REFACTORING_SAVES_PLAN.md)
for remaining qualification.

Reproduce the new selected and native controls with:

```sh
node --test extension-host/*.test.cjs
cargo test --all-targets --locked
cargo test --locked --lib refactoring
cargo test --locked --lib hidden_tests
cargo test --locked --lib lsp::retained_tests
cargo test --locked --test extension_refactoring_saves
cargo build --locked
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
python3 tests/extension_refactoring_saves_pty.py target/debug/vscli
```
