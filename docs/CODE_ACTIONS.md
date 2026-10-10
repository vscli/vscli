# Native and extension code actions

Quick Fix (`Ctrl+.`; `Cmd+.` on macOS) and Refactor (`Ctrl+Shift+R`)
request the active native server and every ready matching extension provider.
The optional JavaScript host is unnecessary for native editing or native actions.
Rows show their source. Enter/Tab selects a row; Escape cancels. Unsupported
extension commands, combined edit-and-command actions, resource operations and
annotated edits have individual disabled rows, preserving usable sibling fixes.

The extension facade implements `CodeAction`, hierarchical `CodeActionKind`,
`CodeActionTriggerKind`, `registerCodeActionsProvider`, textual `WorkspaceEdit`
methods and lazy `resolveCodeAction`. A provider receives a real Selection,
Invoke context, optional requested kind, cancellation token and current original
extension Diagnostic objects intersecting the selection. Custom diagnostic data
and original action identity remain in the host, including circular objects.
Resolution supplies edits/commands while retaining displayed metadata. Native
LSP diagnostics are not yet exposed through this extension callback context.

Each source owns its session, registration and original document versions.
The native picker additionally captures settings, workspace, focus, pane/view
selection, retained model identities, revision and monotonically increasing text
epochs. An edit followed by Undo cannot revive old authorization. Reading a
completed picker has no six-second UI expiration; expired lazy handles require
another request. Late arrivals append without moving the highlighted item.

Text edits stage every target before changing any model. Visible, hidden and
untitled extension targets must already belong to the captured mirrored set;
native targets must belong to the original synchronized set. Identity, URI/path,
epoch, source version, strict UTF-16 boundaries, overlaps and all budgets are
checked first. Text adopts each target's EOL convention, shared identity and
selection mapping stay native, disk stays untouched, and Undo is per file.
Saving an active document does not save hidden targets. Closed files, renames,
creates/deletes and edit confirmations remain unsupported.

Bounds are 128 retained models and 16,384 aggregate document/pane selections;
eight matching optional providers and actual unresolved optional callbacks;
300 displayed/retained rows, divided fairly among successfully requested sources;
4,096 edits; 4 MiB cumulative raw and EOL-normalized replacement text; the existing
32 MiB per-document bound; and the 4 MiB aggregate extension mirror budget,
including unedited documents. Enabled/preferred rows precede disabled rows within
each source. Exceeding row shares is reported. A source failure leaves independent
sources usable. Native requests/resolution retain their one actual action slot
until a response, and a timeout disables further actions until server restart.
Optional callbacks that ignore cancellation retain occupancy until settlement.
These bounds do not bound arbitrary extension-owned JavaScript object graphs.

Native server command actions retain the separately documented explicit-version
`workspace/applyEdit` boundary; extension command execution remains outstanding.
There is no whole-command rollback of an earlier accepted callback. Broader
context/documentation metadata, command execution, closed-file edits, native
diagnostics in extension contexts and full VS Code differential parity remain
qualification and implementation gaps.

## Qualification

`extension-host/code-actions.test.cjs` exercises original object identities,
metadata/Selection, independent registrations, per-row rejection, getters,
workspace versions, handle retirement and actual held callback occupancy.
`src/app/workspace_edits.rs` tests stage-before-mutation, invalid secondary targets,
Unicode/EOL, dirty/hidden/untitled identity, shared views, source lifetime/version,
edit→Undo, normalized byte limits and no-op history preservation.
`tests/extension_code_actions.rs` adds framed real-host workflows, failure
isolation, lazy resolution, hidden/untitled edits and an opt-in actual clangd
coexistence workflow. Existing native action tests retain strict integrity checks.

The unchanged MIT official **vscode-samples.code-actions-sample 0.0.2** is pinned
to Microsoft/vscode-extension-samples commit
`73e249b3c1ba8422aa5713f1a5db23ed0eab4f7a`. Preparation verifies source, license,
committed lockfile, compiled output and installed compiler hashes. Two native
and two PTY workflows cover its original two providers, three emoji edits,
individual disabled commands, empty newer-provider results, Unicode/CRLF,
shared dirty history and explicit save/Undo. This is official sample conformance,
not qualification of a published production extension or ecosystem parity.

Run `python3 tests/prepare_code_actions_sample.py`, set
`VSCLI_CODE_ACTIONS_SAMPLE` to the absolute
`target/code-actions-sample-upstream/code-actions-sample` directory, then run
`cargo test --locked --test code_actions_sample -- --ignored` and
`python3 tests/code_actions_sample_pty.py target/debug/vscli`. Set `VSCLI_CLANGD`
to an installed clangd and run
`cargo test --locked --test extension_code_actions -- --ignored` separately.
CI runs synthetic contracts on its platform matrix and official-sample/actual
clangd workflows in Linux's real-protocol job. Platform checks, optimized builds
and test results must pass before merging; this document describes the scope,
without claiming full parity or project completion.
