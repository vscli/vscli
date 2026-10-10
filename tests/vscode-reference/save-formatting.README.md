# Pinned save formatting observer

Run `node tests/vscode-reference/save-formatting-run.cjs target/save-formatting-reference`
from the repository root after installing the existing reference harness dependencies.
Linux needs a display, for example an owned Xvfb session. The existing supervisor
owns and retires the complete VS Code process tree on completion or failure.
The four profiles start fresh and use VS Code 1.95.0, commit
`912bb683695358a54ae0c670461738984cbb5b95`.

The named synthetic provider cases cover explicit file formatting, a null formatter,
formatting exclusion for after-delay autosave, and a source-organize-imports edit
before formatting. Targets use the original `type`, Save, Undo and Redo commands;
the autosave case waits for an actual `onDidSaveTextDocument` event. Each save event
records document text and independently reads committed disk bytes. One setup-only
public execute-format-provider call proves registration readiness without applying
its edits; exact text, version, selections, dirty state and disk remain unchanged.
This API is callable even though the pinned `getCommands(true)` inventory omits it;
that inventory fact is recorded separately. A null provider's setup API returns
`undefined`, which is retained as an observed no-op.

The Linux baseline contains four cases and 18 snapshots. Explicit formatting adds
one Undo step after typing. Null formatting adds none. After-delay autosave has no
target formatter callback. The source-action case calls organize imports before
formatting; its first Undo reverses formatting while retaining the source edit.
These results describe the captured named fixtures. They do not qualify unchanged
formatter extensions, native formatting equivalence, modified-lines modes, provider
timeouts, competing formatters, Save As transitions or every save participant.
Fresh Windows and macOS captures remain unqualified until executed.

Each case limits callback, change and save traces to 128 events. Public-state
settlement is bounded to three seconds and uses 100 ms minimum plus 50 ms unchanged
state after command acknowledgement, without preferred-output predicates or target
retries. Save acknowledgement has an eight-second bound. The outer supervisor has
a 120-second bound. These are observer limits, not VS Code formatter timeout claims.
Profiles disable trimming and final-newline participants, format-on-type/paste and
unrelated automatic editing. Raw setup failure logs from observer preparation stay
under `/tmp`; successful raw evidence and provenance are preserved separately.

`baselines/1.95.0/save-formatting/` preserves the successful Linux trace, raw evidence
and provenance. Every case records hashes for the observer, cases, runner, suite,
shared supervisor, package manifests and extension entry point. The aggregate
provenance also hashes the trace and evidence. No existing reference capture was
changed to create this corpus.
