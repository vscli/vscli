# Save-worker fixture diagnostics

The native save worker's test-only status includes a bounded progress trace from
the actual worker thread. Production builds contain no trace storage, recording
or extra control messages. Persistence targets, channels, authorization,
publication and cleanup decisions remain unchanged.

This instrumentation follows the Windows failure in
[CI run 38063013251](https://github.com/vscli/vscli/actions/runs/38063013251),
at transfer head `279242496e8fc94c4a82f5b7465711babe4015a3`.
`canceled_close_discard_save_as_and_reopen_preserve_disk_and_clamp_cursor`
exceeded its original five-second fixture wait during `untitled SaveAs
close-save`. Its status reported `phase=committing`, an unfinished thread, no
terminal reply and 1,015 ms remaining on the six-second authorization deadline.
That UI phase follows sending authorization; it does not establish worker-side
receipt or entry into the missing-target no-clobber operation. Neither an
expired authorization nor a specific Windows syscall cause was established.

Each actual worker owns a private 64-slot atomic trace. Elapsed timestamps and
stage codes publish through Release/Acquire readiness flags; a reserved but
incomplete slot is omitted. Overflow retains the first 64 records and marks
truncation. Recording does not allocate, lock a mutex or perform I/O. Formatting
the fixture status is bounded by those records. Scoped thread-local forwarding
lets existing synchronous persistence calls record without changing their
production signatures or giving unrelated settings-worker threads authority.

Authorization receipt, destination recheck, no-clobber/replacement API calls,
temporary cleanup, lock release, work outcome and terminal send have separate
records. A bracket end means return or unwind, not success. The existing work
outcome records distinguish commit, rejection, expiration and error. A
`worker_scope_exit` record is scoped diagnostic cleanup; only actual thread
completion and join release worker capacity. The no-clobber bracket cannot
distinguish tempfile's internal Windows syscalls or scheduler delays.

Four trace tests cover capacity, incomplete publication, actual-thread isolation
and scope restoration through unwind. Two real gated-worker tests cover
commit/cleanup/send/thread-exit separation and concurrent external target
creation. They assert exact Unicode/CRLF bytes, save generation, unchanged text
epoch and retained Redo. Gates control fixture execution; trace records grant
no save authority. The original navigation and saving helpers, five-second
fixture wait, six-second authorization and target assertions remain unchanged.

```sh
cargo test --locked --lib save_worker::diagnostics
cargo test --locked --lib save_worker::progress_tests
cargo test --locked --lib save_worker::tests
cargo test --locked --lib app::navigation::tests::canceled_close_discard_save_as_and_reopen_preserve_disk_and_clamp_cursor
```

Local qualification on the exact transfer base `2792424` plus this instrumentation
passes all six new tests, the original navigation Save As fixture and all 1,058
locked all-target Rust tests across 51 reports with four test threads; 20 opt-in
tests remain ignored. Formatting and strict locked all-target Clippy pass under
the pinned Rust toolchain. Clippy required replacing the deprecated atomic
`fetch_update` name with `try_update`; capacity and publication tests pass with
that change. The original navigation and saving helper bytes are unchanged.
No editor input behavior changes, so this diagnostic increment adds no PTY
workflow. The preceding transfer feature has its separately recorded PTY evidence.

Local and fresh Windows qualification are separate gates. Passing the isolated
fixture does not diagnose the historical full-workload failure. A runtime fix
requires actual progress/error evidence; this change only improves the evidence
available from existing failures.
