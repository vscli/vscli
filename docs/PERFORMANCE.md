# Performance evidence

VSCLI has no established claim to be the fastest editor. Native implementation and small binaries do not establish interactive latency, large-file performance, or performance with equivalent IDE features enabled.

## Reproduce the PTY benchmark

On Linux or macOS with Python 3.11 or later:

```sh
cargo build --release --locked
python3 scripts/bench_editor.py --output target/benchmarks/vscli.json
# Optional comparisons against explicitly selected installed binaries:
python3 scripts/bench_editor.py --nvim /path/to/nvim --vim /path/to/vim \
  --output target/benchmarks/comparison.json
# Compare a saved VSCLI release binary with the current build, interleaved:
python3 scripts/bench_editor.py --compare-vscli /path/to/previous/vscli \
  --output target/benchmarks/change.json
```

The default run opens deterministic 10 KiB, 1 MiB, and 10 MiB ASCII plain-text fixtures. Each editor gets five fresh processes and 40 serial character insertions per process, at 120 by 40 cells. Executable versions/hashes, source revision, environment, fixture hashes, commands, all samples, failures, median, p95, p99, and maxima are recorded in JSON. Nearest-rank percentiles on only five startup samples are effectively extremes, not reliable tail estimates. Use more trials before assessing a sustained regression.

The harness checks the initial document marker and each changed screen cell rather than assuming that any output acknowledges an input. Its limited screen model is shared with the PTY smoke tests and is qualified for these ASCII workloads. Startup query replies follow the [xterm control-sequence reference](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html), including cursor position, device status, and background color. Without replies, a terminal probe timeout can dominate an editor's reported startup. Unknown optional modes are reported as unsupported. This is not full terminal emulation or physical-key qualification.

Boundaries and limitations:

- Startup begins before process creation and ends when the document marker is observed. Files are freshly generated in cache; this is a **warm file-cache** experiment, not cold disk or language-service readiness.
- Input latency runs from writing a character to the PTY until the Python screen oracle observes the expected cell. It includes scheduling, rendering, pipe I/O, decoding, and oracle overhead. It excludes physical keyboard, graphical terminal, compositor, and display latency. It does not measure complete-frame presentation.
- VSCLI uses its native workbench and disables recovery, mouse capture, and enhanced keyboard negotiation. Vim/Neovim use insert mode with plugins, user configuration, swap, and undo files disabled. No editor runs language servers or extension workloads. Feature sets and rendering implementations still differ; these are baseline comparisons, not equivalent IDE configurations.
- Linux memory sums live process-tree RSS after readiness and typing. This includes Neovim's separate editor/UI processes and counts shared pages once per process. It is not PSS or a measured tree peak. Per-process high-water marks remain in raw records. macOS memory is currently unmeasured.
- Idle CPU uses process-tree CPU time over one second per trial. Tick quantization can report zero for a quiet process. It does not establish the 60-second idle budget in the architecture document.
- Editors run sequentially with reproducibly shuffled order. The harness does not pin CPUs, control turbo/thermal state, or isolate unrelated host activity. Run without concurrent builds or tests; use a dedicated machine for performance gates.
- Fixtures are disposable. Processes are killed after measurement; save, shutdown, recovery, search, highlighting, Unicode, pathological long lines, and concurrent background work are not covered by this run.

Hosted CI checks that the screen oracle works with the debug executable on Linux/macOS. **There is no hosted-CI timing threshold.** Record release-build baselines and investigate repeatable regressions on controlled hardware before setting gates.

## Recorded run: 2026-10-08

[Raw observations and executable/fixture hashes](benchmarks/2026-10-08-linux.json), captured with benchmark commit `3f870ce` and native code from `8a90288`. Host: Intel Core i9-13900H, Linux x86_64; release VSCLI 0.1.0, Neovim 0.12.5, Vim 9.2 (patches 1–1046). Each row has five process launches and, when successful, 200 serial key samples. No builds or test suites ran concurrently with this recorded run; other host activity, scheduling, and thermal state were not controlled.

| Editor | File size | Startup median ms | Key median ms | Key p95 ms | Key p99 ms | Tree RSS median MiB | Failed trials |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscli | 10 KiB | 8.12 | 0.973 | 1.288 | 1.394 | 8.62 | 0/5 |
| nvim | 10 KiB | 31.42 | 0.475 | 0.810 | 1.055 | 23.97 | 0/5 |
| vim | 10 KiB | 7.88 | 0.195 | 0.473 | 0.573 | 10.15 | 0/5 |
| vscli | 1 MiB | 8.74 | 0.960 | 1.339 | 1.452 | 10.70 | 0/5 |
| nvim | 1 MiB | 34.18 | 0.315 | 0.868 | 0.953 | 25.08 | 0/5 |
| vim | 1 MiB | 9.91 | 0.139 | 0.451 | 0.529 | 11.36 | 0/5 |
| vscli | 10 MiB | 22.11 | 0.918 | 1.157 | 1.283 | 30.15 | 0/5 |
| nvim | 10 MiB | 47.60 | 0.585 | 0.849 | 0.905 | 35.36 | 0/5 |
| vim | 10 MiB | 22.49 | 0.219 | 0.438 | 0.539 | 21.63 | 0/5 |
| vscli | 100 MiB | — | — | — | — | — | 5/5 |
| nvim | 100 MiB | 161.78 | 0.412 | 0.853 | 1.009 | 138.55 | 0/5 |
| vim | 100 MiB | 141.53 | 0.161 | 0.376 | 0.495 | 124.62 | 0/5 |

VSCLI was slower on the measured typing path in every file size it opened. Its startup and memory results were competitive in some cases, but neither establishes an overall performance lead. All five VSCLI 100 MiB attempts exited with an error: the alpha's `MAX_FILE_BYTES` limit is 32 MiB. This is a capability failure, not a missing sample to exclude from the comparison. A real large-file mode remains required work; raising the limit alone would not qualify its performance or reliability.

These are one-machine baseline observations with Python-oracle overhead, not isolated native handler timings or a statistically established editor ranking. VS Code, Helix, source highlighting, extensions, and full IDE workloads have not been compared.

## Remaining performance qualification

The next workloads must measure source highlighting and incremental parsing, 100 MiB files and very long lines, Unicode movement/multiple cursors, undo/paste/save, 100,000-file indexing and search, and editing under LSP/extension/task contention. Add matched VS Code and Helix measurements with versioned configurations, cold-cache procedures, real-terminal input-to-display latency, process-tree peak memory, and sustained CPU sampling. A single favorable component or empty-editor benchmark cannot answer which editor is fastest overall.

The separate [extension mirror measurement](EXTENSIONS.md#mirror-performance-measurement) remains a Node-component microbenchmark. Its speedup does not describe native typing latency or total editor speed.

## Native phase profiling

`cargo run --release --example profile_editor -- --iterations 4000` reports native input handling, background polling, and Ratatui rendering to an in-memory `TestBackend`. Add `--unicode` for combining marks, wide characters, emoji sequences, and tabs. Each sample inserts one character; undo restores the same document and viewport before the next sample. The first 100 iterations warm up and are excluded. The example verifies that all edits are undone at exit.

These phase timings exclude the PTY, OS event decoding, terminal output, and physical display. They help locate costs inside VSCLI and must not replace the executable benchmark. For Linux CPU sampling with symbols, build using `cargo rustc --release --locked --example profile_editor -- -C debuginfo=1 -C strip=none`, then run `perf record -g --call-graph dwarf -- target/release/examples/profile_editor --iterations 4000`. Collect ordinary timings separately from `perf` to avoid treating sampling overhead as the baseline.
