# Rejected Windows sticky settlement capture

This is genuine failed qualification from GitHub CI run
[38066412649](https://github.com/vscli/vscli/actions/runs/38066412649), head
`951f2ad7a187ee6472598c0dc793c84a0c3906ea`, artifact `11674409574`.
All forty files from its sticky output directory are retained byte-for-byte,
including the empty failed native-comparison output. The eight exact historical
executable/harness inputs are separately retained in `sources/`; they are not the
new monotonic observer. `inventory.json` records the copied file sizes/digests.

The raw producer emitted eighteen cases, 65 target and 91 setup observations.
Setup operation zero for `close-editors-in-group-excludes-sticky` reports five
reads / 13,721 ms. The historical reader rejects this complete capture because
its settlement bound is 100..=3100 ms. That case's initial/target settlements are
124/133 ms. A correct target result cannot authorize over-budget setup evidence.

The producer checked quiet success before its deadline and used wall-clock time.
The new observer checks a monotonic deadline before accepting a result, without
widening the three-second budget or retrying gestures. This archive does not
identify the underlying Windows filesystem/event-loop stall or clock change.
It remains rejected evidence, never a successful baseline or native parity claim.
