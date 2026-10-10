# Native code actions on save

Status: local qualification passes; fresh required platform checks remain
necessary before protected-main merge.

Opt in through native user settings or workspace `.vscode/settings.json`:

```json
{
  "editor.codeActionsOnSave": {
    "source.fixAll": "explicit",
    "source.organizeImports": "explicit"
  },
  "editor.formatOnSave": true
}
```

The default is off. Original Save and explicit close-confirm Save can ask the
already running native language server for fix-all and organize-imports actions,
then formatting, then persistence. Language blocks such as `[cpp]` apply. Normal
editing, Save and autosave continue to work without a JavaScript runtime or a
language server. Configure language servers through the existing native settings
and automatic language-service startup; a command-line LSP argument is optional.

Object policies merge individual entries across user/workspace and matching
language scopes, including composite groups followed by single-language groups.
An empty higher object preserves lower entries; an array replaces the object.
Object policy runs fix-all first; legacy arrays retain the configured family order.
`explicit`, `always` and true enable an entry for an explicit save. Short-delay
autosave skips source actions, including always and array policies. Focus/window
autosave and focus-triggered actions are not implemented by this participant.

Legacy true is interpreted as `explicit` and false as `never` in the final native
executable policy. False excludes its kind and descendants even under an enabled
ancestor, matching the stable pinned 1.95 configuration cohort. Raw JSONC,
imported source files, merged layers and extension-visible values stay unchanged;
there is no persisted migration. Kinds match dot boundaries,
so `source.fixAllX` is outside fix-all. A broad `source` entry projects to the two
supported families with a notice about other unavailable source actions.
Malformed effective values fail closed with a notice; native limits include
32 entries, 128 bytes per kind, 8 KiB of kind text and bounded scope metadata.

This first native participant supports direct edits and lazy edit resolution.
It applies at most one mutating action per family, in server order after filtering
disabled, excluded and unrelated actions. Additional same-family mutations remain
a compatibility gap. The next family receives freshly synchronized text. Each
applied action and formatting operation has its own Undo boundary; a null or
byte-identical edit preserves existing Undo/Redo history. Persistence adds no text
Undo step. Errors in a later participant retain earlier valid edits.

Every edit must target exactly the original generated file URI. Versioned edits
must match the captured native synchronization version; multi-resource changes,
resource operations, ambiguous forms and nonempty annotations are rejected before
mutation. Command-bearing actions are skipped in their entirety, including an
action with both an edit and command. This avoids applying only half of an action.
Command lifetimes, versioned applyEdit during execution, secondary-resource edits
and extension-host save actions need separate implementation and qualification.

The action participant has one 1500 ms budget for discovery and resolution across
both families. Busy, unsupported, invalid or timed-out action work skips with a
notice while a still-owned save continues. Formatting has its own deadline. Actual
native action capacity is shared with the interactive picker and remains occupied
after advisory cancellation until an exact terminal response or server retirement.
Closing the picker cancels only its own request. Late replies can release capacity
but cannot apply text or reopen UI.

User edits, edit-to-Undo changes, saved-baseline changes, another accepted Save,
settings/profile changes, workspace changes and server replacement invalidate
old participant authorization. Unsaved work is retained and requires a fresh Save.
Switching panes alone cannot redirect the accepted Save to another document.
Escape retires unapproved participants and deferred close intent; filesystem writes
already authorized retain the existing persistence semantics. Save As, first saves
and hidden-target source actions skip with an explicit notice in this slice.

The strict edit limits are 4096 edits per action, 4 MiB cumulative raw/expanded
replacement text and 32 MiB final origin text. Discovery admits at most 300 items
and 2 MiB of action JSON; depth, labels, synchronized inventory, resolve count and
metadata are bounded before retention. No queued participant stores a Rope save
snapshot. The filesystem snapshot is captured after text participants finish.

The original fourteen-case Linux reference remains historical evidence with its
raw settings and callbacks intact. Current supported eligibility uses the separately admitted stable configuration cohort at revision
`133897055d5c524ed1fd0b51866edf95f1faffd8`, run `38063145339`: fourteen unchanged
cases / 162 frames per platform, exact ten source inputs and independent
configuration/participant readiness barriers. All three genuine platform archives
are retained. The two false-entry cases now have canonical `never` and matching
exclusions; after-delay targets retain zero callbacks. This does not establish
why the earlier transitional observations differed.

Native comparison is limited to supported-kind eligibility and at most one
mutating edit per supported family. Seven native terminal journeys pass on debug
and optimized executables,
including excluded child returned before an eligible root and excluded family
under broad `source`; each checks Unicode/CRLF saved bytes, independent Undo/Redo
stages and unchanged user/workspace JSONC. All 31 ordinary reference tests pass
with the default stable archive and each genuine second-cohort platform artifact.
See [reference validation](SAVE_REFERENCE_VALIDATION.md) for exact evidence and
fresh CI status. Native model
versions, graphical UI, unchanged extension behavior and complete save-pipeline
parity remain outside these comparisons. See the
[reference observer](../tests/vscode-reference/save-code-actions.README.md) and
[policy/evidence scope](SAVE_ACTION_POLICY.md).

For child-only policy, the native participant requests the family root and
filters descendants locally, whereas the pinned observer requests that child
kind directly. The consumer preserves and checks that difference, comparing
callback input/trigger and exact text/history for this cohort without claiming
the requested-kind parameter is identical. The other five implemented cohorts
compare their captured request kind and callback text directly.

## Historical participant qualification

The original participant candidate passed 784 ordinary Rust tests across 41 reports, with 20 optional
integrations ignored, formatting and strict all-target Clippy. Named coverage
includes five policy tests, five extraction integrity tests, six framed action
lane tests and eight controller journeys. All 133 extension-host tests and 25
Python tooling tests pass. Five original-key terminal journeys pass on the debug
executable with Unicode/CRLF persistence, per-participant Undo/Redo, null-edit
history preservation, combined-command refusal, deadline/late-capacity release,
Close/Escape cancellation and after-delay exclusion. The language server starts
from native settings with an empty PATH and missing Node.

All 27 ordinary Unix terminal scripts pass, with 126 reported checks. The five
source-action journeys and five existing formatting journeys also pass on the
optimized executable. Three existing automatic-service/suggestions/signature
terminal suites and two native/extension action integration tests pass against
installed clangd; these are existing workflow regressions, not real-server
fix-all parity. The final package rebuild repeats all 784 ordinary Rust tests,
strict Clippy and the fresh reference consumer successfully.

The [routine-editing comparison](PERFORMANCE.md#native-code-actions-on-save-core-baseline-2026-10-10)
passes 40 interleaved launches/1,600 keys without failures, with mixed observations
and no speed ranking. Fresh Linux/macOS/Windows checks still require qualification. These local results establish named supported workflows, not full
upstream save parity.

A guarded filesystem save that detects an external edit keeps its actionable
`Save failed` notice when a later watcher reports the identical conflict. The
notice is deduplicated only for the originating document, path, text epoch and
save generation; dirty buffers, foreign disk bytes and Undo/Redo remain intact.
A gated regression reproduces the prior overwrite with the fix disabled.
