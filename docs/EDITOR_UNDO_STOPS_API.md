# Extension editor Undo boundaries

`TextEditor.edit(callback, options)` supports all four combinations of
`undoStopBefore` and `undoStopAfter` through native document history. Omitting
`options` requests both stops. Supplied option properties are read once before
the callback and converted to booleans; an empty object requests neither stop.
An empty builder returns true without dispatching native work or changing history.

With neither stop, an edit can join preceding and following native typing.
Two consecutive API edits can also form one Undo/Redo group. Registered extension
command dispatch leaves this decision to the callback's options. Native navigation,
palette opening and other ordinary history boundaries still close groups.

Grouping reuses the native snippet history mechanism. Original editor-view,
language, resource and save-generation proofs prevent an old group from borrowing
a replacement view or changed source. Joining an API group is limited to 4,096
scalar changes. Existing model/version, overlap, mirror-byte and caller-lifetime
checks remain in the edit gateway. A rejected edit preserves native text and Redo.
Inactive-editor edits record their own view's history and map shared selections
without changing the displayed view.

The implementation follows the pinned 1.141.0 editor adapter's option capture and
before/after boundary order. Named host, native and actual terminal workflows
qualify defaults/coercion, reentrant-source refusal, all four stop pairs,
Unicode/CRLF save/Undo/Redo, shared models and contributed F9 dispatch.
[Recorded validation](validation/editor-undo-stops.json) distinguishes that local
evidence from fresh platform qualification.

`TextEditorEdit.setEndOfLine`, full model-global grouping across arbitrary views
and lifecycle transitions, incremental event fidelity, exotic UTF-16/private
transport behavior and the rest of the stable API remain open. The optional
extension process remains separate from native editing without Node.
