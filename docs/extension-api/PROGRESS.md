# Stable API delivery progress

The scope is the entire stable VS Code 1.141.0 API at immutable commit
`2a59476c9bfcb90b3ddc372c36762471b7dfad1c`. All 776 declarations, 2,366
member/overload/type rows, sixteen namespaces and the ambient Thenable contract
remain in the generated live report. The frozen audit under `1.141.0` stays intact.

```sh
python3 scripts/extension_api_progress.py
python3 scripts/extension_api_progress.py --json > /tmp/vscli-api-progress.json
```

[progress.json](progress.json) records named acceptance criteria, their evidence
and remaining obligations. The report expands those references over **every**
pinned inventory row. Partial workflow evidence never closes a declaration,
overload or type-only contract. Unassessed rows are an evidence-reconciliation
backlog; existing implementations are not counted as absent or erased. The
[complete implementation order](1.141.0/implementation-order.json) continues to
include all remaining provider, UI, tooling, notebook, authentication, chat and
language-model families.

Each implementation checkpoint reports the new public behavior, passed/total
named criteria for that batch, exact tests and their source/binary cohort, and
the still-open contracts. Test totals, exported names, foundation modules and
file counts are never used as API completion percentages. Locally tested and
upstream-qualified remain separate; the current report grants no whole-API
qualification.

Workflows with a source receipt also compare its complete recorded source digest
with the current checkout. A mismatch labels the passed criteria as historical
evidence. New code does not silently inherit a previous run's qualification.

The current delivery priority is [migration usability and release](../MIGRATION_RELEASE_PLAN.md).
The entire stable API remains in scope. The retained ordered resource-edit batch
still has its complete acceptance criteria; its unfinished code is preserved. Those criteria
include public create → edit → rename → edit → delete, will-file/review policy,
dirty/shared/hidden models and retired history, genuine model/operation events,
public Undo/Redo, truthful partial-failure receipts, data-integrity tests and a
terminal workflow. EOL edits are a usable adjacent checkpoint, not completion of
that batch. Other-platform, custom filesystem, trash and large-conversion
obligations stay visible after a local backend becomes usable.

The [extension message localization slice](../validation/extension-localization.json)
passes 5/5 named local workflow criteria: three public translation overloads,
per-owner data/URI and fallback, preferred-language bootstrap, native editing and
restart, and a no-Node/translated terminal journey. Six environment metadata
values are exposed. Built-in language-pack resolution and the remaining
environment/whole-API contracts remain open.


Platform release repair: the latest local source passes 1,652 Rust tests across
69 reports (23 ignored), 11 focused host lifecycle controls, two document and two
welcome terminal journeys, formatting and strict Clippy. Completed-resource
references share the native extension URI spelling; Windows journal durability
uses identity-checked retained-object flush handles. Fresh platform CI remains
required. [Exact evidence and outstanding gates](../validation/platform-repair.json).


Source application migration passes 5/5 named local criteria, including built-in
package reconciliation, source-disabled native data, a 184-package native catalogue,
application-picker/dirty-buffer/rollback integration and regression checks. The
actual isolated copy contains 184/184 matching packages, 113 code packages and zero
personal execution grants. All 1,656 Rust tests pass locally. This is migration
integration evidence; whole stable-API qualification remains open.
[Source/binary and workflow receipts](../validation/builtin-profile-migration.json).


Extension context keys pass **4/4 named local workflow criteria**, including
module/activation calls at welcome, retained bounded requests, actual conditional
shortcuts and shared edit/save/Undo/restart. The final native suite passes 1,659
tests across 70 reports (23 ignored; four Rust test threads), all 811 host tests
pass on unchanged host JS, and the new terminal journey passes. The preceding
theme-helper timeout and source difference are retained in the receipt. Whole
expression/scoped context and entire stable API qualification remain open.
[Evidence](../validation/extension-context-keys.json).

Native condition integration passes **4/4 local batch criteria**: 600 exact
comparisons with the pinned upstream parser/evaluator, source-preserving imported
rules and diagnostics, native resource/public extension values in actual terminal
shortcuts, and required native regression checks. The full suite passes **1,661
Rust tests across 71 reports** (1,304 library; 23 ignored; four test threads).
Two new terminal journeys and the previous context-key journey pass. Conditions
are compiled and retained by validated keymaps. Full JavaScript regex/coercion,
context scope/population and the entire stable API remain open.
[Source and workflow receipts](../validation/native-when-expressions.json).

The follow-up Save As repair reproduces the Windows receipt mismatch on Linux
with Unicode/reserved path characters and passes the strengthened public workflow.
The receipt now uses the public extension URI function. Both terminal Save As
acceptance and cancellation pass; the unchanged native test count is 1,661 across
71 reports. An LSP timeout fixture also retains a cancellation barrier received in
the timeout poll, rather than expecting a duplicate. macOS main CI at `4e6f8b3`
passes; Windows passed 1,242 library tests before the reproduced Save As failure,
and Linux retained one LSP barrier failure. Fresh repair-head platform evidence is
required. [Repair receipts](../validation/refactoring-save-uri.json).

Packaged default manifest labels pass **5/5 named local criteria**. The native
catalogue includes a theme after 128 packages, source settings can match a
translated theme without an ID, and public packageJSON/running command metadata
reach actual edit/save/Undo. There are **1,668 passing Rust tests across 72 reports**
(1,309 library; 23 ignored; four test threads), **814 passing host tests**, two
new terminal journeys and the private 184-package no-Node journey. The real copied
profile finds the translated built-in Light 2026 theme; personal code grants stay
zero. Preferred-language manifests, language packs and the entire stable API
remain open. [Exact evidence](../validation/default-manifest-labels.json).
