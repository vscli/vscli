# CI evidence publication and background status

## Code-action fixture evidence

The code-action provider fixtures stage their bounded evidence records in exclusively created files in the destination directory, finish and close each record, then rename it over the final path. A reader that observes a final JSON path sees a complete record. Existing records are never truncated or removed before replacement. Write and rename errors propagate; owned temporary files are cleaned up without deleting another owner’s file.

This fixes the empty-record race seen in Linux CI run 38074990803 at main fb360539. It changes fixture evidence publication, not editor persistence. Original JSON payloads, command markers, document identities, parser assertions and deadlines remain unchanged. Atomic visibility is distinct from durability after a system failure.

Local qualification: all 22 Node code-action/publication tests pass, including 11 deterministic publication-boundary and error tests; native extension code-action integration tests pass (5 passed, 1 opt-in ignored). Formatting and strict locked all-target Clippy pass. Cross-platform qualification remains a fresh CI gate.
