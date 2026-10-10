# Frozen monotonic sticky-tab observer

This package contains the exact eight runtime inputs for the corrected sticky
settlement contract. Seven inputs retain their historical bytes. The suite digest
is `2891fc2d0895ba2963db3b2f026c9f5b6b1b877106630d2cbab920a6c13299d8`.
Rust admission binds each input to a compiled digest and this separate source
root; the original observer and baseline remain independently accepted.
README prose is outside the runtime inventory.

The actual runtime settlement routine uses a monotonic clock and checks the
three-second deadline before sampling, after snapshot/fingerprint work and
before accepting 100 ms of unchanged public state. Polling remains 20 ms;
operation acknowledgement and outer supervisor budgets remain five/240 seconds.
Elapsed metadata is an integer in 100..2999 ms. No gesture, fixture, setting,
preferred-state predicate or target retry is added.

Twelve pure suite tests pass, including stalled snapshots/fingerprinting/timers,
exact quiet-state success and unchanged original error propagation. They do not
load VS Code. Four Rust context/timing guards distinguish original and explicitly
synthetic metadata; synthetic source receipts are never real observations.
Fresh all-platform capture/native replay is required before qualification.

Ordinary CI verifies the eight inputs before dependency setup, captures each
original eighteen-case cohort once, and passes its explicit result directory to
`cargo run --locked --example editor_sticky_tabs_contract -- --monotonic <directory>`.
The reader preflights every target/setup field and source/product/launch receipt
before native fixture writes. Source and every raw output are uploaded even on
failure. The normal no-argument example retains the historical Linux baseline;
`--verify-monotonic-source` checks source bytes without starting an editor.

The actual rejected Windows cohort and its eight historical source inputs are
preserved under `../vscode-reference/observations/1.95.0/sticky-settlement-deadline/951f2ad-win32`.
Its raw digests and 13,721 ms setup are checked by the offline settlement guard
under both timing limits. Full Windows admission also needs the real pinned
Windows product/executable bytes, verified in CI without relaxing identity. This does not prove
why Windows stalled, native UI parity, physical shortcuts or all tab semantics.
No local reference editor was launched for this change.
