# Pinned code actions on save observer

Run `node tests/vscode-reference/save-code-actions-run.cjs target/save-code-actions-reference`
from the repository root with the reference harness dependencies installed.
Linux requires a display, such as an owned Xvfb session. The shared supervisor
retires the VS Code process tree on success or failure. Fourteen fresh profiles
use VS Code 1.95.0, commit `912bb683695358a54ae0c670461738984cbb5b95`.

The Linux corpus preserves 14 named cases and 162 snapshots. Synthetic source
actions deliberately return organize-imports, root fix-all, child fix-all and an
unrelated `source.fixAllX` kind, leaving hierarchy/exclusion filtering to the
editor. The cases observe object versus legacy-array ordering, false versus never
under enabled ancestors, user/workspace object merging, composite/single-language
merging, array replacement, empty-object inheritance, child-only filtering and
after-delay autosave. Object policy prioritizes fix-all; arrays retain their order.
The captured boolean false does not exclude an enabled ancestor's descendants;
string never does. Dot boundaries distinguish fix-all from the unrelated prefix.
Both always and legacy-array policies have zero action callbacks on after-delay
save in these cases.

Targets use the original type, Save, Undo and Redo commands. Each explicit case
records exactly five Undo and five Redo commands, retaining extra no-ops rather
than issuing desired-output retries. Autosave waits for an actual didSave event.
Every snapshot records public text, dirty state, scalar selection and independent
disk bytes; save events prove text and committed bytes match. Provider callbacks
record requested kinds, trigger kind and the current document text, exposing fresh
text between action families. This corpus does not qualify native LSP equivalence,
unchanged extensions, cancellation, Save As, focus/window autosave or all action
providers. Fresh macOS and Windows captures remain unqualified until executed.

Setup invokes the public execute-code-action-provider API once to prove callback
readiness. It applies no edits, and exact text, version, selections, dirty state
and disk must remain unchanged. This API is callable even though the pinned
command inventory does not list it; both facts are retained. The initial
inventory-only setup rejection remains in the raw qualification logs. No target
was rerun to obtain a preferred output.

The observer bounds each callback/change/save list to 256 events. Public-state
settlement allows three seconds, with a 100 ms minimum and 50 ms unchanged state
after acknowledgement; it does not check preferred output. Actual save
acknowledgement has an eight-second bound. The outer supervisor allows 180 seconds.
These are capture limits, not editor timeout claims. Profiles disable unrelated
formatting, trimming, final-newline and focus-change participants.

`baselines/1.95.0/save-code-actions/` retains exact Linux trace, evidence and
provenance. Each case hashes the nine observer/harness sources and profile and
workspace settings. Aggregate provenance hashes the full trace and evidence.
Existing formatting and other reference corpora remain unchanged.
