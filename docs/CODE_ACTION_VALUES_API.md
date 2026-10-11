# Code-action values

`CodeActionKind` and `CodeAction` use source-derived runtime semantics from
stable VS Code 1.141.0, pinned to commit
`2a59476c9bfcb90b3ddc372c36762471b7dfad1c`. Public constructors retain arbitrary
JavaScript values and references. Kind instances and static constants are
mutable; `CodeAction` owns its `kind` property even when it is undefined.
The shared ES5 helper preserves legacy subclasses.

Kind append uses the source separator and JavaScript `+` coercion. `contains`
matches equality or a dot-separated descendant. Consequently, Empty does not
universally contain other kinds. Prototype methods retain the source ordering.

## Native admission

Provider metadata and returned rows independently capture a kind's value once
and require a primitive string of at most 128 UTF-8 bytes. Oversized strings are
refused by length before byte measurement. Invalid metadata cannot publish a
partial registration; invalid rows retain a disabled explanation without an
invalid kind or edit. Permissive public constructors do not bypass native bounds.

Native kind filtering uses the original detached request and normalized result
strings. Mutating `context.only`, public methods, constants or the separator
cannot broaden the admitted request. An empty request exposes undefined
`context.only`, following the source adapter. Native filtering currently drops
mismatched rows; the upstream adapter warns before conversion. That difference
remains explicit.

Resource, snippet, notebook, EOL and combined edit/command backends are still
outstanding. Unsupported edits are refused whole, preserving document versions
and existing Undo history; this value correction adds none of those backends.

## Evidence

Thirteen focused tests and all 332 local host tests pass. They cover mutable
values/constants, coercion, property descriptors, legacy construction, malformed
metadata/rows, UTF-8 budgets, single getter reads, disposal during normalization,
and request mutation. All 134 native extension-related tests and five native
code-action integration tests pass. Actual terminal checks pass both unchanged
official sample scenarios, whole mixed text/EOL refusal followed by legacy
WorkspaceEdit/TextEdit application with original Undo history, and 32 queued
native-only intents behind one actual callback. Unicode/CRLF save and terminal
restoration pass. Build, formatting and strict all-target Clippy pass.
These are local observations and pinned source comparisons, not an upstream
runtime comparison or full code-action API qualification. Ignored corpus cases
grant no new qualification.

Exact source URLs, lengths, hashes and native adaptations are retained in the
[source receipts](../extension-host/code-action-values.provenance.json).
The port retains Microsoft's MIT attribution and embedded license.
