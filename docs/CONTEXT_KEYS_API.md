# Extension context keys

Selected extensions can call `commands.executeCommand('setContext', name, value)`
to control native conditional shortcuts. Boolean expressions such as
`editorTextFocus && myExtension.ready` react to the accepted value before the
command promise resolves. A missing/undefined value removes the key; false, zero
and null remain distinct stored values. URI values, including nested values,
become their public strings, following the pinned upstream
[API command](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostApiCommands.ts)
and [context service](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/platform/contextkey/browser/contextKeyService.ts).
The command returns no value and does not open an editor.

Module evaluation and activation may await this command at welcome. Context
values share the existing eight occupied service slots. Values are copied before
awaiting; invalid or oversized replacements preserve the previous map. A timed-out
request reports an unknown outcome and retains its slot until its exact terminal
reply. Malformed acknowledgements do not release it. Native admission validates
session, selected/live owner and any originating command before mutation.

The current window stores at most 1,024 extension keys and 256 KiB of serialized
values; each UTF-8 key is at most 1,024 bytes and each JSON value 64 KiB, 16,384
nodes and 32 levels deep. No filesystem work occurs for an update. Native focused
editor/platform facts take precedence over root extension keys. Context values
are local to the selected host session and clear when it stops; they are not
persisted into user configuration.

Local evidence covers public module/activation calls with no open editors,
scalar/URI/removal behavior, queue and malformed/timeout controls, stale
session/owner refusal, a conditional contributed shortcut, actual edit/save/Undo,
restart and original shared model identity. The terminal journey uses an invocation
trace and a later acknowledged host command to prove an inactive shortcut did not
invoke its extension. See [exact validation receipts](validation/extension-context-keys.json).

Native conditions now support boolean groups, equality/inequality, numeric
comparisons, array/object membership and the nonbacktracking regex subset.
Boolean false and numeric zero are falsy; an empty array or object is truthy.
Unquoted `== true`/`== false` follow upstream truthiness, while quoted strings
retain string comparison. The same parser serves imported user bindings and
extension defaults. Validated keymaps retain compiled conditions, and contributed
conditions are precompiled during session preparation.

The checked-in MIT upstream parser/evaluator is a test oracle, isolated from the
native runtime. Six hundred exact comparisons cover the listed operators and
JSON value cases. Native terminal journeys cover no-Node resource/numeric/regex
conditions and public extension context/membership conditions, including actual
Unicode/CRLF edit/save/Undo. See [condition receipts](validation/native-when-expressions.json).

This does not qualify every `executeCommand` target or the whole context system.
JavaScript regex lookaround/backreferences, complete UTF-16/Unicode/coercion
semantics, every native `config.*`/workbench key,
scoped context hierarchy, lifecycle equivalence across host failure/restart,
arbitrary non-JSON objects and the entire stable API remain open.
