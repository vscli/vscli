# CI evidence publication and background status

## Code-action fixture evidence

The code-action provider fixtures stage their bounded evidence records in exclusively created files in the destination directory, finish and close each record, then rename it over the final path. A reader that observes a final JSON path sees a complete record. Existing records are never truncated or removed before replacement. Write and rename errors propagate; owned temporary files are cleaned up without deleting another owner’s file.

This fixes the empty-record race seen in Linux CI run 38074990803 at main fb360539. It changes fixture evidence publication, not editor persistence. Original JSON payloads, command markers, document identities, parser assertions and deadlines remain unchanged. Atomic visibility is distinct from durability after a system failure.

Local qualification: all 22 Node code-action/publication tests pass, including 11 deterministic publication-boundary and error tests; native extension code-action integration tests pass (5 passed, 1 opt-in ignored). Formatting and strict locked all-target Clippy pass. Cross-platform qualification remains a fresh CI gate.

## Syntax cancellation and status ownership

Windows CI run 38075085037 exposed a canceled syntax setup overwriting a command-bearing save-action notice. Cancellation can follow visibility changes or an existing setup/parsing deadline; the log does not establish which cause occurred. The syntax engine now records that cancellation authority on the actual request, discards canceled success and error replies, and preserves the occupied slot until terminal acknowledgement. Deadline cancellation retains bounded retries; current noncanceled errors remain visible. No request queue, parsing deadline, save fixture or assertion was relaxed.

All three new regressions fail against the preceding implementation: visibility ABA rejects canceled success/error, canceled setup preserves the actual App status, and deadline cancellation stays quiet with bounded retries. Corrected local qualification passes all 14 syntax tests, the unchanged actual-save regression, all 1,027 locked all-target Rust tests across 51 reports (23 opt-in integrations ignored), formatting, strict Clippy and the terminal C++ color-stability journey. Independent source review found no material cancellation/slot/retry issues. Fresh platform CI remains required.

## Integration on the merged display foundation

After PR #71 merged, the same two repair implementations were rebased onto main 3cc75ab9. Syntax and fixture helper/test bytes are identical to preceding head 2aa40ca5. The combined source passes 1,087 locked all-target Rust tests/51 reports (23 opt-in integrations ignored), all 22 Node code-action/publication tests, formatting, strict Clippy, build and the native terminal color-stability journey. Fresh exact-head CI remains required.

The preceding CI run 38076975970 passed Linux, Windows, quality, commit style and real protocol checks. Its macOS Rust workload passed, but the upstream stable-save reference harness failed a warm API readiness assertion before target authorization. Exact artifact 11678477943 and its ten source blobs were independently verified. Formatter/configuration setup passed and the provider callback returned all four declared kinds; the capture did not retain the eventual public API result before its count assertion. Cancellation, filtering and timing remain unproven. This is distinct from the two repaired native failures; a separately source-bound diagnostic observer is needed. No original failure was reclassified as a success, no count/deadline was relaxed, and no local reference editor was launched.
