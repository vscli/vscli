# Extension system clipboard

The optional host exposes a frozen, owner-scoped `env.clipboard` with asynchronous
`readText(): Promise<string>` and `writeText(text): Promise<void>`. Writes resolve
with `undefined` after the native helper succeeds. They update the editor's
internal clipboard fallback only after that success. Reads and writes leave
native document text, selection, dirty state and history untouched.

The bridge reuses the existing snippet clipboard worker. Extension clipboard
operations and native snippet reads share its actual global permit. One helper
leader and its pipe transfer retain capacity through cancellation and exit/join;
a descendant holding a pipe cannot admit replacement workers around itself.
Work stays outside rendering/input. Requests share the host's eight-service and
eight-MiB queue, with a two-MiB encoded request limit and one-MiB text limit.
Unpaired UTF-16 surrogates reject before crossing the native UTF-8 boundary.
Malformed or oversized native receipts cannot release a timed-out request slot.

macOS selects `pbcopy`/`pbpaste`; Linux selects `wl-copy`/`wl-paste` or `xclip`.
Missing or failing helpers report an unavailable system clipboard; the API does
not claim a successful OS write through the editor's internal fallback.

Windows uses native `CF_UNICODETEXT` access on the same worker, without a shell or
JavaScript runtime. An invisible message-only owner supplies the required write
ownership. Movable, zeroed memory is prepared before clearing, unlocked before
transfer, and freed privately only if ownership has not passed to Windows. Reads
scan a bounded NUL-terminated UTF-16 prefix and reject malformed/oversized text.
Clipboard-open contention observes cancellation and a bounded wait. Thread-bound
guards close the clipboard and destroy the private owner; failures after clearing
report possible effects. An explicit isolated Windows CI control covers real OS
Unicode/CRLF, empty and boundary strings, contention and cancellation. It is
ignored locally unless explicitly selected in the isolated CI environment. Native
Windows linking/runtime and actual desktop/display-server qualification remain
separate gates; portable type checking alone is insufficient. Retired original owners/sessions cannot borrow a new caller's authority.
An already authorized system write may have completed when a caller retires;
errors do not imply that the global clipboard was rolled back.

[Validation](validation/clipboard-api.json) separates public facade/framing tests,
actual child-process transfer/retirement tests, and terminal/native editing.
The terminal fixtures replace OS helpers with isolated programs and private data,
then exercise the public extension API, native Ctrl+V, Unicode/CRLF saving,
Undo/Redo and restart without Node. They never read or change the personal system
clipboard. This validates the bridge and named workflows, not an actual display
server, every text size/platform, or whole stable-API compatibility.

The entire stable API and seamless personal migration remain in scope. Larger
values, arbitrary UTF-16 strings, broader system/helper behavior, simultaneous
clipboard mutations, platform/runtime fidelity and the remaining environment APIs
stay open. Existing foreground Copy/Paste integration is separate from the new
asynchronous extension worker path.
