# Native save automation: implementation and follow-up plan

Native command Save and Save As now use a background worker. Native autosave
implements `files.autoSave = "off" | "afterDelay"`, including language scopes.
These are implemented runtime paths undergoing full integration, terminal and
platform qualification; this document does not establish full VS Code save parity.
The original plan has been replaced with the implemented contract and remaining
qualification work. Format-on-save and save-time code actions remain future work.

## Implemented integration

| Source | Implemented behavior |
| --- | --- |
| [App save ownership](../src/app/saving.rs), [command and prompt routing](../src/app.rs) | Original Save/Save As commands retain model identity and route persistence through one actual worker and one latest desired intent. Queued work recaptures text after the previous receipt. |
| [Document snapshot](../src/document.rs) | Cheap immutable Rope capture, exact preauthorization proof and successful-receipt publication perform no filesystem I/O. Explicit low-level synchronous document save APIs remain available. |
| [Save worker](../src/save_worker.rs), [persistence backend](../src/persistence.rs) | Worker resolves destination/aliases, stages bytes, waits for authorization, rechecks disk/lock/parent identity and commits. Capacity remains occupied until the actual thread exits, even after its terminal reply. |
| [Close/quit and panes](../src/app.rs), [pane ownership](../src/app/panes.rs) | Close-after-save targets its original pane/model. Quit and Close All defer while persistence is pending, then recompute dirty models. Escape cancels deferred closing while saves continue. |
| [File operations](../src/app/files.rs), [disk watching](../src/app/watching.rs) | File operations and pending saves cannot acquire conflicting ownership. Watch publication is fenced before authorization and at commit receipt; pending-save targets do not reload through the watcher. |
| [Language notifications](../src/app/language.rs), [LSP writer](../src/lsp.rs) | Successful Save and Save As notify the original saved URI with the exact persisted Rope, independently of newer text or active-editor changes. |
| [Settings loading](../src/settings.rs), [settings persistence](../src/app/settings_persistence.rs) | Every successful native receipt requests a fenced asynchronous settings reload. Native document writes and scalar settings patches use separate actual lanes and the same destination lock protocol. |
| [Extension mirrors](../src/extensions.rs), [host events](../extension-host/api.cjs) | Successful save generations feed existing document-save notifications. Change precedes save when published together; multiple commits before one mirror sync can coalesce. One extension event per commit is not claimed. |
| [Autosave scheduler](../src/autosave.rs) | Bounded per-model metadata and round-robin due selection include dirty hidden named models; shared panes schedule a model once. |

## Save transitions and data ownership

| Transition | Contract | Failure or supersession |
| --- | --- | --- |
| Capture → prepare | Capture model/path/revision/text epoch/save generation, shared text and baseline, request identity and optional continuation. Resolve filesystem identity and stage the captured bytes on the worker. | Validate 32 MiB text/baseline, 4 KiB paths, 128 distinct retained models and 512 KiB aggregate model paths before admission. One latest intent carries metadata, not a reusable old text snapshot. |
| Prepared → authorize | Check exact live snapshot and retained alias-model proofs; automatic work also checks policy/workspace generation. Fence watch publication before authorization. | Edit→Undo still changes the epoch and rejects old preparation. Dirty matching aliases, changed paths/models or retired policies cannot authorize. Rejected work retains actual capacity until cleanup and thread exit. |
| Authorized → commit | Recheck exact baseline bytes, file/parent/alias/lock identities and atomically persist. Missing destinations use no-clobber creation. | Authorized work can complete after subsequent typing or a profile switch. Failure leaves text, selections, Undo/Redo, saved generation and buffer path unchanged. |
| Commit → receipt | Return original captured text and canonical destination after lock/tempfile cleanup. Deliver the receipt only after actual worker settlement. | Successful disk facts are published even when the requested action is no longer latest. |
| Receipt → publish | Update only the originating retained model's captured baseline/revision and monotonic save generation. Preserve newer text, shared selections and history, then notify the persisted URI and recapture queued work. | Newer edits remain dirty. A close continuation requires its original pane/model and a clean target. A moved/replaced model receives an explicit persisted-destination notice instead of unsafe reassignment. |

Persistence adds no text edit or Undo entry. Postauthorization edits remain
editable and are not falsely marked saved. A later Undo back to the captured
saved revision can legitimately become clean. Successful-save generations are
reserved without wrapping, and stale or duplicate receipt publication is refused.

Save As retains the initiating model while the prompt is open, including editor
and pane switches. A different existing destination is refused; this slice has
no overwrite-confirmation flow. Saving to the same source retains ordinary
baseline protection. Missing parent directories fail rather than being created.

A Close issued while a matching Save/Save As is pending attaches to the latest
matching intent, otherwise to that model's actual save. Disk visibility alone
does not acknowledge completion. The continuation keeps the original pane/model
and generation and cannot replace a broader current Quit/Close All continuation.
Escape cancels the deferred close while actual persistence continues.

A delayed Close targets the original pane, preserving the current editor's
focus when possible. If newer edits remain, the pane stays open and a notice
asks the user to close again to review them. Canceling close retires destructive
continuations; authorized persistence still settles. Quit and Close All await
native saves and settings writes before revisiting dirty visible/hidden models.
Interrupt/error shutdown retires unapproved work and queued intentions, waits
for authorized receipts, and only then writes final recovery/session state.
This is actual-settlement waiting, not a promise of a fixed shutdown deadline
for a blocked filesystem operation.

## Autosave settings

Defaults are `files.autoSave: "off"` and `files.autoSaveDelay: 1000` ms. Native
`afterDelay` accepts integer delays from 0 through 86,400,000 ms (24 hours).
Zero means eligible on a subsequent poll; input handling performs no disk write.
The effective winning mode or delay is authoritative: malformed values,
unsupported `onFocusChange`/`onWindowChange` modes, or out-of-range delays disable
native automation with a compatibility notice rather than inheriting a lower
value that might enable writes.

User/workspace and supported composite/single-language precedence applies. For
example, enable autosave only for C++:

```jsonc
{
  "files.autoSave": "off",
  "[cpp]": {
    "files.autoSave": "afterDelay",
    "files.autoSaveDelay": 1000
  }
}
```

The pinned VS Code 1.95 settings declaration specifies off/1000 ms defaults and
language-overridable values; it also includes focus/window modes outside this
native slice. See [pinned settings source](https://raw.githubusercontent.com/microsoft/vscode/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/contrib/files/browser/files.contribution.ts)
and [official autosave documentation](https://code.visualstudio.com/docs/editing/codebasics#_save-auto-save).
Native delay limits and malformed-value refusal are explicit product boundaries,
not a claim that every upstream settings edge case matches.

Only dirty named files qualify; untitled buffers still require Save As. Accepted
text changes, including Undo/Redo epoch advancement, restart a model's delay.
Cursor motion, an older snapshot's successful receipt and activity in another
model do not restart that text's deadline. Due-model rotation prevents a hot
active file from starving a dirty hidden file. A newer edit during an authorized
save keeps its own debounce deadline and may receive a later fresh autosave.

Manual Save has priority over unapproved automatic work. A failed automatic or
manual snapshot suppresses retries for that exact live model/path/epoch/save
baseline/policy proof; explicit Save remains available, and changed text or
qualifying baseline/policy evidence can requalify automation. Policy/profile
changes retire unapproved interest, including A→B→A changes. Failed settings
reloads pause automation until valid settings arrive. Save As prompts, close
confirmation and deferred closing pause automatic dispatch.

The scheduler retains at most 128 distinct models, with no copied document text.
Common save admission also enforces the model/path budgets: exceeding them can
refuse explicit Save as well as automation; native editing retains unsaved work.
Settings-source identity caching avoids reparsing every effective policy for
unchanged models on every frame.

## Filesystem and extension boundaries

Regular-file, read-only, symlink/reparse, parent, alias and lock checks occur on
the persistence worker. Payload symlinks are refused. Dirty hidden/recovered or
hardlinked aliases identifying the target refuse preauthorization. Atomic
replacement affects one directory entry, not every hardlink; other retained
alias models keep their own baselines for subsequent conflict detection. Unix
mode/read-only preservation is implemented; complete ACL/xattr/metadata parity
remains unqualified. Directory-sync failures after successful persistence are
reported as durability warnings. Advisory locking cannot exclude an
uncooperative writer racing the final filesystem check and rename.

Settings patches and ordinary native saves cooperate via the same canonical
`.vscli-write.lock` protocol, but each has its own one-worker lane. There is no
global one-worker claim across the two. Successful document commits force a
fresh settings-loader read, including after a profile change.

Format-on-save, source code actions on save, extension `onWillSave`/wait-until
participants, recursive save arbitration, Save All, save reasons and dynamic
LSP save-registration parity remain outstanding. Ordinary formatting and
workspace edits stay reviewable dirty edits; they do not implicitly save.
Extension mirrors retain existing save-event coalescing semantics.

## Qualification status and follow-up

The current candidate passes 726 ordinary Rust tests across 39 suites (20
optional integration tests ignored), formatting and strict all-target Clippy,
133 extension-host tests and 25 Python tooling tests. The actual CLI passes all
35 baseline PTY reports, seven new native save/autosave journeys on both debug
and optimized builds, three settings-persistence reports and three Breadcrumbs
reports. Native Outline and Breadcrumbs consumers also pass against their existing
frozen reference captures, including all eight source-validation tests; this is
consumer regression evidence, not a new reference
capture. Fresh Linux/macOS/Windows CI, save-participant comparisons and performance
qualification remain pending. These counts describe this save candidate and do
not establish full VS Code parity.

The current suites exercise real staging/authorization/commit/thread-exit
boundaries, original commands, exact Unicode/CRLF bytes, edit→Undo fencing,
newer dirty text, shared views/Redo, original URI notifications, hidden aliases,
latest-intent recapture, policy/profile changes and final recovery baselines.
Two additional actual-worker tests qualify shared destination locking, dirty
settings refusal, simultaneous authorized native/settings shutdown and recovery
baseline integrity. Five held-worker tests qualify Close before a Save/Save As
receipt, original pane ownership across a switch, newer edits, Escape, latest
intent ownership and preservation of a broader Quit continuation. Further named
qualification should cover deferred Close All and prompt-policy transitions.
Reference save-participant and event-order captures need explicit setup/target
separation and must retain unsupported/no-op outcomes without desired-output
retries. Adding participants requires a separate bounded provider/edit pipeline
and reviewed failure policy; it is not part of this implementation.
