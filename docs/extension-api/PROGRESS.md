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
