# Extension progress in the status bar

`window.withProgress({location: ProgressLocation.Window, title}, task)` now shows
native status progress for the lifetime of the task. `progress.report({message})`
updates its label. The returned promise preserves the task's exact result object
or error, including values which cannot be serialized. Completion, failure and
extension-host retirement remove the progress item.

The implementation reuses the existing native status-item renderer, owned surface
transport and disposal. Editing, saving and Undo stay native and usable while the
task runs. An empty workbench does not acquire a document to display progress.

Window progress has no user cancel button or discrete percentage, as specified
in the pinned stable declaration. A boolean `cancellable` is accepted but does
not add cancellation at this location. The callback receives the existing inert
cancellation token; valid increments are accepted without a percentage display.
The native activity glyph is a text representation; VS Code's graphical/icon
rendering and accessibility behavior remain unqualified.

At most eight actual progress tasks remain unsettled across the selected session.
Retiring an owner removes its UI without releasing an unresolved task's slot.
Cached methods and reports retain their original caller lease; callbacks execute
under that facade's owner, including delegated calls. Completed reports are inert.
Reports coalesce to one queued label per task per event-loop turn. Titles and
messages each allow 512 UTF-8 bytes; the combined native label allows 1 KiB.
Progress also shares the existing status-item and transport budgets.

`ProgressLocation` exposes the stable numeric values. Notification, Source Control
and contributed-view locations still refuse before invoking the task or creating
UI. Their percentage/cancel/view integration remains required, alongside the
entire remaining stable API and seamless personal extension migration.

Host controls cover exact results/errors, invalid-input refusal, coalescing,
eight shared unresolved tasks, retirement and getter/delegated caller lifetime.
The App workflow checks actual rendering, dirty shared-document identity,
revision/Undo/disk preservation, failure/stop and an empty workbench. Three terminal
journeys cover concurrent native editing, empty workbench and a fresh no-Node
restart. [Source-bound results](validation/window-progress.json) distinguish local
evidence from fresh upstream/platform qualification.
