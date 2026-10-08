# Patched dependencies

`crossterm/` contains the published Crossterm 0.29.0 crate, including its MIT
license. The root Cargo patch applies it to both VSCLI and Ratatui.

- Upstream: https://github.com/crossterm-rs/crossterm
- Published source commit: `36d95b26a26e64b0f8c12edfe11f410a6d56a812`
- Archive: https://static.crates.io/crates/crossterm/crossterm-0.29.0.crate
- Archive SHA-256: `d8b9f2e4c67f833b660cdb0a3523065869fb35570177239812ed4c905aeff87b`

The behavioral change is in `src/event/source/unix/mio.rs`. The original Mio
reader returns a parsed event before draining the ready file descriptor. After
the parser consumes its 1,024-byte batch, bytes still in the terminal can wait
indefinitely for another readiness edge. This was reproduced with a 1,200-byte
editor burst followed by Save in the same write: Save did not run until an extra
Escape arrived. It also stranded the end of an embedded-shell command.

The patch checks actual unread bytes with `FIONREAD` before waiting for another
Mio event and before reading the descriptor. This is necessary because stdin can
be blocking: retaining readiness alone would block on an already-drained fd.
It keeps the existing parser-batch size and does not alter the shared descriptor's
blocking flags, key decoding, Windows input, or the alternative `use-dev-tty`
backend. The input API's single-reader assumption remains in force.

An unnecessary pair of closure parentheses in `src/terminal/sys/unix.rs` is also
removed to avoid an upstream compiler warning when building this path dependency.

The PTY suite sends more than 1 KiB of raw editor input plus Save, and a shell
command containing 1,200 payload characters. Both must finish without another
input event. Keep these regressions when upgrading. Remove the local patch once
an upstream release fixes this behavior and passes the same tests; do not edit
unrelated vendored sources. The packaging script includes the crate's license
through Cargo metadata, just as for registry dependencies.
