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

The next full batch remains ordered resource edits. Its acceptance criteria
include public create → edit → rename → edit → delete, will-file/review policy,
dirty/shared/hidden models and retired history, genuine model/operation events,
public Undo/Redo, truthful partial-failure receipts, data-integrity tests and a
terminal workflow. EOL edits are a usable adjacent checkpoint, not completion of
that batch. Other-platform, custom filesystem, trash and large-conversion
obligations stay visible after a local backend becomes usable.
