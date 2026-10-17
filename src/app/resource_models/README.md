# App resource model coordinator

The first connected operation is native **File: Revert File**. Confirmation binds
the original document ID, resource epoch and displayed name. Another editor
opening behind the dialog cannot redirect the operation; path away→back rejects
before reading. The original model can remain visible, shared or hidden.

App captures the actual ResourceModelSource and original workspace/settings/
profile context, then starts the existing two-phase resource worker. Disk reads
and text/EOL diff preparation run there, with a nonrenewable ten-second deadline.
App checks the original source before authorizing the prepared reply and again
after the actual terminal receipt and thread join. Publication uses the exclusive
native restoration commit, preserves document/view/history identity, updates the
disk baseline lifetime and invalidates prior disk-watch publications. Revert
manufactures neither reference acquisitions nor replacement model identities.

One actual worker slot remains occupied through cancellation and exit/join.
Edit→Undo, language/resource ABA, model replacement, settings failure, expiry and
failed reads preserve current text, baseline and history. View navigation remains
independent. Pending reloads pin their original API-managed model through the
existing per-model pending-work predicate. Saving that model refuses while its
reader is occupied; autosave/refactoring-save dispatch waits so dirty pre-revert
text cannot overwrite the disk contents being read. Shutdown retires and joins
the worker rather than detaching it.

Native reads retain the existing 32 MiB file limit. A clean restored model and
baseline share one immutable Rope allocation; independent dirty contents and
baselines remain additive under the restoration budget. Optional-host sessions
keep their smaller aggregate mirror budget, checked at capture and publication.
Other models growing during preparation cannot admit an oversized public mirror.
The implementation uses Rust and requires no JavaScript runtime.

Nine App controls cover actual shared-model reload/Undo/Redo, original baseline
and save-proof retirement, held edit→Undo, retained occupied capacity, language
ABA, missing files, save exclusion, same-path model replacement, settings failure,
confirmation/editor redirection and confirmation resource ABA. One additional
restoration control verifies a 17 MiB clean payload shares its original backing
with both the native model and disk baseline. The existing orphan-file watcher
control now awaits the background command instead of assuming synchronous IO.

This connects native reload publication. Bulk resource execution, conditional
destination survival/editor replacement, full retired history/view archives,
ordered file/text/notebook groups, public resource-operation events and file
Undo/Redo still need integration. The extension resource-edit gateway remains
disabled. Neither these controls nor the native Revert terminal journey qualify
full resource-operation or VS Code parity.

Final Linux verification passes 1,550 Rust all-target tests across 67 reports
(1,201 library; 23 ignored), formatting, strict locked all-target Clippy and build.
Four terminal journeys pass: native EOL Revert/Undo/Redo/Enter/Save, Explorer
resource lifetime and two document/restart journeys with the optional host. The
two native journeys run without Node and restore terminal mode on exit. No local
VS Code instance was launched; CI work remains deferred.
