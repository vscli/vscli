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

The Unix parsers also preserve the second prefix when Escape followed by an
SS3/CSI shortcut arrives in one read. Upstream consumed both adjacent Escape
bytes as one event, turning an original Escape followed by F1 into Escape and
literal `OP`. This was observed in the macOS extension-provider workflow. The
patch emits the first Escape and parses the second as a fresh prefix, with no
new timeout. Both Mio and `use-dev-tty` share this rule. Two plain Escapes remain
two events; legacy Alt-prefixed function keys can have the same bytes as Escape
followed by that function key, so this rule does not claim to distinguish them.
Enhanced explicit modifier reporting remains separate.

`tests/adjacent_escape_pty.py` deliberately sends Escape and original F1 in one
write, from both Inspector and editor focus, in legacy and enhanced sessions.
It verifies palette/Inspector transitions and exact saved Unicode/CRLF bytes.
The same unchanged regression fails on the unpatched executable. Extension
workflow command submission also waits for the actual palette before typing.

An unnecessary pair of closure parentheses in `src/terminal/sys/unix.rs` is also
removed to avoid an upstream compiler warning when building this path dependency.

The PTY suite sends more than 1 KiB of raw editor input plus Save, and a shell
command containing 1,200 payload characters. Both must finish without another
input event. Keep these regressions when upgrading. Remove the local patch once
an upstream release fixes this behavior and passes the same tests; do not edit
unrelated vendored sources. The packaging script includes the crate's license
through Cargo metadata, just as for registry dependencies.

`unicode-segmentation/` contains the published unicode-segmentation 1.13.3 crate,
including its Apache-2.0/MIT licenses, copyright notice, and Unicode 17 test data.
The root patch also applies to transitive users of this version.

- Upstream: https://github.com/unicode-rs/unicode-segmentation
- Published source commit: `66a032fd8d667bc47ac5b640b151dff3f5356d07`
- Archive: https://static.crates.io/crates/unicode-segmentation/unicode-segmentation-1.13.3.crate
- Archive SHA-256: `c6f5d3c3b1bf09027a88a6bc961fc00497d651009560b5463668dc81b0fa87a8`

Only `src/grapheme.rs` is modified, with two minimal upstream backports:

- [5dfedef](https://github.com/unicode-rs/unicode-segmentation/commit/5dfedefcdd8a2d888843ae3fe88a8f2615408f7f)
  preserves an already-known regional-indicator count across chunk changes.
  Without it, moving through rope chunks can split a flag emoji in half.
- [048d51f](https://github.com/unicode-rs/unicode-segmentation/commit/048d51fe1d9bac5ca7c56226d3a4b42f21d70be2)
  applies control/CR/LF breaks before the Prepend rule in supplied context.

Unicode tables and unrelated upstream changes are intentionally retained at the
published version. `tests/grapheme_conformance.rs` runs the bundled Unicode 17
expected boundaries through forward, backward, and boundary-query cursors with
each scalar in a separate chunk, for both extended and legacy graphemes. It also
covers a cached regional-indicator regression. Native document tests exercise
real rope leaves, including clusters spanning several leaves.

Remove this patch once a released dependency includes both fixes and passes
these tests. Dependency notices are collected through Cargo metadata during
packaging, including the licenses and copyright notice from this directory.
