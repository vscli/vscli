# Extension Input Box passwords

Extensions can use `window.showInputBox({password: true})` to request masked native
input. Initial values, typed text and pasted text render as one `*` per grapheme,
including during horizontal scrolling. Navigation, selection, deletion and paste
operate on the actual value. Enter returns the exact accepted string; Escape
returns `undefined`. `password: false` retains ordinary visible input.

The password option must be a boolean and is captured once before dispatch. Input
and presentation retain the existing 4 KiB text limit, eight queued prompts and
64 KiB shared presentation budget. Password input is available only for Input Box;
the native receiver rejects a password-marked Quick Pick.

Both Input Box and Quick Pick handles now capture their original extension caller
lease. Retired handles, delayed item promises, option getters and late native
replies cannot acquire a replacement owner's authority. Native session/request
checks, FIFO presentation, cancellation and host retirement remain in force.

The implementation reuses the existing native prompt controller, input editing,
queue and extension transport. The pinned stable declaration's password contract
is to hide typed text; graphical masking appearance is not qualified. Input
validation callbacks, explicit value selections, cancellation-token arguments,
`ignoreFocusOut: true`, `createInputBox`, and the rest of Quick Input remain required.
This change does not implement authentication providers or migrate account secrets.

The new App control checks normal/narrow rendering, long values, Unicode grapheme
editing, exact extension receipts, cancellation/stop and unchanged native document
identity, revision and Undo history. The actual terminal journey checks screen and
raw output using synthetic values, alongside the existing FIFO/crash/restart and
module-evaluation workflows. [Source-bound qualification](validation/input-box-password.json)
records results and remaining platform/upstream qualification.
