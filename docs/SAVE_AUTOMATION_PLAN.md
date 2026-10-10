# Native save automation plan

This document specifies the next implementation and qualification work. It is
not evidence that asynchronous saves, autosave, format-on-save or save-time code
actions already work. The settings-persistence candidate is a separate feature;
its reviewed worker and publication guards provide patterns to reuse.

The native document foundation now exposes immutable `SaveSnapshot` capture,
preauthorization checking and successful-receipt publication without filesystem
I/O. Five native integrity tests pass for edit→Undo fencing, newer postauthorization
edits, shared reversed selections and existing Redo through Save As, stale or
duplicate receipts, and size/path/generation bounds. Formatting and strict
all-target Clippy also pass for this foundation. The current App save path remains
synchronous until the worker and ownership integration below are qualified.

## Current integration points

| Source | Current behavior | Required integration |
| --- | --- | --- |
| [`App::save`](../src/app.rs) | Calls `Document::save` synchronously; follows successful save with active-editor language notification and optional close. | Route by retained document ID through `app/saving.rs`; publish a receipt before running a continuation. |
| [Save As prompt](../src/app.rs) | Calls `save_to` synchronously, checks only literal visible-tab destination paths, reapplies settings and completes the pending action. | Capture the originating model before the prompt; resolve aliases and visible/hidden destination conflicts on the worker; add the same successful-save notifications as ordinary Save. |
| [`Document::save_to`](../src/document.rs) | Streams a Rope to an adjacent temporary, checks baseline bytes, preserves permissions, syncs and replaces; updates path, baseline, saved revision and save generation immediately. | Separate immutable capture, worker persistence and UI receipt application. Keep the synchronous API for explicit low-level callers until migration is complete. |
| [Close/quit confirmation](../src/app.rs), [panes](../src/app/panes.rs) | Dirty hidden models are promoted for quit/close-all; save completion currently removes the active document. Closing one shared pane retains the dirty model. | Continuations retain original pane/model identity; a delayed receipt must never close whichever editor became active later. |
| [File operations](../src/app/files.rs), [worker](../src/files.rs) | Rename moves retained document paths; trash detaches models. Save currently refuses while a file job runs. | Keep this guard and refuse conflicting rename/trash dispatch while an authorized save owns a destination. Do not drop a running save to make capacity. |
| [`language_saved`](../src/app/language.rs), [`Client::saved`](../src/lsp.rs) | Synchronizes visible models and sends `didSave` for the active one, including its current text. Save As does not invoke this hook. | Route successful receipts to the exact persisted URI/text and appropriate retained server. A newer active buffer is not the saved snapshot. |
| [Workspace edits](../src/app/workspace_edits.rs) | Stage buffer edits atomically; leave them dirty for review and do not persist. | Preserve this boundary. A save-time edit pipeline needs separate authorization and qualification. |
| [Extension mirror](../src/extensions.rs), [host events](../extension-host/api.cjs) | Actual successful `save_generation` changes produce save notifications; text change precedes save when published together. Multiple saves before one mirror sync can coalesce into one event. | Preserve owner/document identity and change-before-save ordering. Explicitly qualify save-event coalescing or add a bounded receipt-event route; do not claim one event per commit from the existing mirror alone. |

No `save_and_refresh` helper exists in the audited tree. Implement one receipt
publication path rather than giving this name to an active-editor shortcut.
Save All is not currently implemented by the native command dispatcher.

## Save state transitions

| Transition | Work and proofs | Failure or supersession |
| --- | --- | --- |
| Capture → prepare | On the UI thread, capture model ID, path, revision, monotonic text epoch/save generation, shared Rope, disk baseline, request ID, profile/workspace identity, effective save policy and optional pane continuation. The worker resolves paths, acquires the destination lock, validates native identity/permissions and streams the captured Rope into an adjacent temporary. | Validate size/path/model budgets first. Retain one actual save worker and one latest desired intent. Queued intent stores identity/reason/destination, not a reusable stale Rope. |
| Prepared → authorize | Worker returns canonical destination, baseline/file/parent/lock identity and matching retained model IDs. UI checks the exact current model/path/epoch/save generation, all relevant alias models, profile/policy and continuation. Fence disk-watch publications immediately before authorization. | Any preauthorization edit, edit→Undo, successful save, source replacement, conflicting dirty alias or destination change retires preparation. Keep the worker slot until cleanup completes. Recapture current state only when a new request can dispatch. |
| Authorized → commit | Worker rechecks exact baseline bytes and identities, parent alias/identity, matching-model alias membership and owned lock, then atomically persists. Existing target replacement and missing-target no-clobber are distinct operations. | Authorization cannot truthfully cancel a commit already in progress. Later typing is allowed and must remain in memory. A failed commit keeps bytes, selections, Undo/Redo, path and saved generation unchanged. |
| Commit → receipt | Worker returns a successful receipt containing exact persisted Rope/revision, canonical destination, model/request identity, captured path proof and durability result. Capacity remains occupied until temporary cleanup and lock release positively settle. | A success belongs to its original file even if the editor/profile changed. Never discard successful disk facts merely because an optimistic UI request is no longer latest. |
| Receipt → publish/continue | Fence disk-watch publications again, update only the matching retained model's baseline and successful-save counter, remember the saved file, notify exact language/extension ownership and request asynchronous index/settings refresh. Apply a captured close continuation only after proving its target remains safe. | Never assign `saved_revision = live.revision` unconditionally. The saved baseline is the captured Rope/revision; a newer revision stays dirty. A later edit→Undo returning exactly to that saved revision can legitimately become clean. A moved/replaced model needs an explicit saved-snapshot notice, not path reassignment or data loss. |

One worker means one actual document-save callback, not merely one visible
pending request. Retain its `JoinHandle` until finished, including cancellation,
timeout and terminal reply. A finite response channel and one latest desired
intent cannot license overlapping workers after an early reply. The existing
settings-write lane is separate; it can coexist, but both must cooperate through
the same canonical destination's `.vscli-write.lock` sidecar and lock-identity
checks. Do not advertise a global one-worker bound across these two lanes.

Receipt application preserves model ID, shared views, selection directions,
snippet state and Undo/Redo. Persistence itself adds no text edit. Check and reserve
a nonwrapping successful-save generation before authorization. Postauthorization
edits do not invalidate the successful disk receipt; they invalidate only its
ability to declare the *current* buffer saved or perform a destructive continuation.
If a receipt cannot update the model safely, retain unsaved work and report the
actual persisted destination explicitly.

## Close, pane and exit ownership

A Save As prompt retains its initiating model and continuation generation.
Typing a path or switching editors must not cause the prompt to save another
model. Closing an ordinary shared pane still needs no save if another view owns
the same model; a delayed close-after-save targets the captured pane/model, never
an array index or the then-active pane.

For Close, remove the captured model only when the successful receipt leaves it
clean and it still satisfies the original close ownership proof. If the user
typed after authorization, show the existing Save/Discard/Cancel confirmation
for the remaining changes. If the target pane already closed, retire the
continuation without removing its remaining shared document.

Quit and Close All use a bounded continuation generation and recompute dirty
visible/hidden models after every receipt. Canceling quit cancels its
continuations and unapproved intentions; authorized workers still settle and
publish success. Discard cannot terminate the process while an authorized save
is running. Keep `running = true` until actual save workers finish and the
existing recovery/session shutdown work settles. A canceled or failed save does
not count as completion of quit. Terminal restoration occurs after that safe
termination boundary; it must have PTY coverage.

## Initial autosave policy

The initial native setting scope is `files.autoSave = "off" | "afterDelay"`,
with default off, and `files.autoSaveDelay` default 1000 ms. VS Code desktop 1.95
uses these defaults and language-overridable settings. It also supports
focus/window-change modes; those are explicitly outside this first native slice.
See the [pinned settings declaration](https://raw.githubusercontent.com/microsoft/vscode/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/contrib/files/browser/files.contribution.ts)
and [official save documentation](https://code.visualstudio.com/docs/editing/codebasics#_save-auto-save).

Use a documented native delay budget of 0–60,000 ms, validated without wrapping.
Zero means eligible on a later poll, not an immediate filesystem operation in
input handling. Larger values and unsupported modes receive compatibility
notices rather than silently enabling another policy. A winning valid language
scope participates through the same settings precedence rules as other editor
settings; malformed values retain the existing lower valid value.

Track at most 128 authoritative model IDs across visible and hidden documents,
with one deadline/epoch/policy stamp per model and no copied text in the scheduler.
Shared panes schedule their model once. Only dirty, named native files are
autosave-eligible; untitled buffers continue to require Save As. Delay resets on
accepted text changes, including version advancement through Undo/Redo, not
cursor movement, save acknowledgments or unrelated document activity. Retiring
or disabling a policy removes unapproved interest; it cannot undo authorized
persistence.

At dispatch, choose a due model fairly, then recapture its current Rope and disk
baseline. Keep a round-robin cursor over due model IDs so editing one hot file
cannot permanently exclude a second dirty hidden file. Manual Save has immediate
priority over autosave, but replaces only unapproved desired work. Other due
models retain their bounded metadata and get subsequent turns. A failed external
conflict does not retry every frame: suppress that same epoch/path/baseline until
an edit, explicit retry or qualifying disk/policy change supplies new evidence.
Overflow of the model budget stops automation with a visible notice; manual
editing and explicit save remain usable.

## Filesystem boundary

Reuse the settings writer's platform-qualified regular-file, read-only,
reparse/symlink, parent and lock identity checks rather than adding UI `stat`,
canonicalization, directory creation or hashing. Existing canonical documents
remain resolvable. A payload that becomes a symlink is refused for persistence;
reading a symlink and writing one are separate compatibility behaviors. Save As
does not overwrite an existing destination in this initial slice. Missing parent
directories receive an explicit failure unless a separately qualified operation
creates them; do not make autosave create arbitrary directory trees.

Hardlink and recovered raw-parent aliases require native worker identity checks.
Other dirty retained models identifying the same file must refuse authorization.
An atomic replacement affects the named directory entry, not every hardlink:
receipt publication updates only the saved model, and other alias models retain
their own baselines for asynchronous conflict detection. Preserve Unix mode bits
and read-only flags; ACL/xattr/complete metadata parity remains unqualified.
Report directory-sync failures as durability warnings after an actual successful
replacement. An advisory lock cannot eliminate races with an uncooperative
external writer between the last check and rename.

Settings files can be ordinary native save targets. Their successful receipt must
force a fenced settings-loader read and disk-watch refresh, even after a profile
change. This must cooperate with the root-scalar settings-persistence worker,
without rewriting unrelated JSONC bytes or treating a stale loader read as the
new saved profile.

## Save participants and qualification

First ship background native Save/Save As plus off/afterDelay autosave. Do not
run asynchronous formatting, extension commands or source code actions as an
implicit save participant in this stage. Ordinary extension workspace edits
remain reviewable dirty buffers. A future safe participant pipeline needs
bounded selected providers, retained actual capacity through cancellation,
original object ownership, exact text/policy proofs, validated atomic edits,
explicit save reasons and a defined participant-failure decision.

Pinned VS Code describes `editor.formatOnSave` as explicit-save-only when
`files.autoSave` uses `afterDelay`. This distinction must be qualified before
adding format-on-save, alongside wait-until listeners, recursive save attempts,
unsupported commands and extension event order. Source registration alone is
not behavioral qualification.

Required integrity tests use real held workers, explicit gates and unchanged
preauthorization bytes, not timing sleeps or mock success flags:

- Unicode/CRLF, reversed multicursor/shared-view edits; successful save adds no
  Undo, subsequent Undo/Redo retains exact text and disk bytes.
- Edit→Undo before authorization rejects; typing after authorization persists
  the captured snapshot and leaves newer text dirty, with a later fresh save.
- Save A held while active editor switches to B; receipt, `didSave`, file history
  and close target A. Save As prompts retain the origin through pane changes.
- Same-file hidden/recovered/hardlink aliases, malformed historical paths,
  read-only/symlink targets, parent/lock retarget, exact external byte and
  same-byte native-identity conflicts; missing-file no-clobber race.
- Repeated requests retain one actual worker and one latest intent; canceled
  work holds capacity until cleanup; queued captures happen after publication.
- Delay reset, off/on and profile A→B→A, multiple dirty hidden/visible models,
  fairness and failed-conflict suppression, with a controlled scheduler clock.
- Settings-save/patch lock cooperation, forced-loader fence and held precommit
  watch reply rejection; saved generations advance only for successful commits.
- Close/Close All/Quit/Cancel while saving; failures and postauthorization edits
  keep the correct buffer and prevent premature process exit.

Public App tests and native-only PTYs must exercise original Save/Save As/close
commands, delayed typing, restart bytes and terminal restoration with Node absent
from PATH. Optional-host tests separately qualify exact document ownership and
save-event order. Pinned actual VS Code captures should isolate setup from target
gestures and preserve unsupported/no-op outcomes; no desired-output retries.
Run the repository's relevant integrity suites, formatting and strict all-target
Clippy before reporting implementation evidence. Fresh Linux/macOS/Windows CI,
optimized PTYs and matched performance measurements remain separate gates.
