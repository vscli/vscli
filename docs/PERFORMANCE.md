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

To measure typing while periodic recovery runs, use `--recovery --keys 50
--key-interval-ms 100`. This creates a fresh recovery directory for each VSCLI
trial, pumps terminal output between keys, and validates after timing that a
committed journal contains a prefix of the measured edits. The requested delay
is outside each latency sample and follows the previous key's observed update;
it is not a fixed-rate input schedule. Recovery mode requires at least 2.5 seconds
between the first and last key and supports only VSCLI before/after comparisons.
Missing or incorrect recovery evidence fails the trial. The report records the
interval, recovery mode, journal size, and number of inserted characters in the
observed snapshot. It does not require the final key to have reached the periodic
journal, measure shutdown, or establish peak memory. Hosted CI exercises this
workload too, without timing thresholds.

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

## Native input/render optimization: 2026-10-08

CPU sampling identified repeated grapheme segmentation, per-grapheme owned strings, and cell writing as substantial renderer costs. Key resolution also parsed contexts before checking whether a key could match. Commits `d49ca8b` and `ea690cb` filter unrelated keybindings before evaluating contexts, avoid owned strings for glyphs, and use direct cell writes for printable ASCII and tabs. Unicode glyphs retain Ratatui's width and cell-reset behavior; control-containing lines retain Unicode segmentation. Plain text no longer allocates a redundant color vector per line.

The resolver is compared against the previous algorithm over shipped platform bindings, chords, overrides, and context combinations. Renderer comparisons cover ASCII, tabs, combining marks, wide characters, emoji sequences, control characters, and styled cells. These preserve tested existing behavior; they do not establish full VS Code parity.

[Native phase observations](benchmarks/2026-10-08-native-phases.json), 4,000 samples per workload, with ordinary timing runs separate from CPU sampling:

| Text | Phase | Before median µs | After median µs |
| --- | --- | ---: | ---: |
| ascii | event | 25.49 | 3.58 |
| ascii | render | 395.95 | 219.87 |
| unicode | event | 25.53 | 3.57 |
| unicode | render | 370.77 | 295.42 |

[Interleaved executable observations](benchmarks/2026-10-08-render-change.json) compare the saved baseline binary with the optimized binary on the same machine. Each row has ten launches and 400 serial key samples per binary, with zero failed trials. No builds or tests ran during measurement. Names `vscli_comparison` and `vscli` in the raw report identify the baseline and optimized builds respectively; executable hashes are recorded.

| File size | Before key median ms | After key median ms | Before key p95 ms | After key p95 ms |
| --- | ---: | ---: | ---: | ---: |
| 10 KiB | 0.936 | 0.669 | 1.198 | 0.963 |
| 1 MiB | 0.914 | 0.783 | 1.313 | 1.072 |
| 10 MiB | 0.952 | 0.759 | 1.275 | 1.021 |

The 10 MiB workload's observed p95 decreased by about 20%. This is a same-machine improvement under the documented PTY boundary, not a new cross-editor ranking or a physical display measurement. It does not resolve the 32 MiB file limit, full IDE contention, or broader performance qualification gaps.

## Shared saved buffers and streamed I/O: 2026-10-08

Commit `2bc6abc` streams UTF-8 reads into ropes and shares the saved baseline through copy-on-write storage. Disk conflict checks and temporary-file writes use bounded buffers instead of whole-file byte/string copies. Commit `709d506` also compares periodic watcher reads in the worker, returning an unchanged result without constructing a replacement rope or comparing whole files on the input thread. Recovery keeps its existing on-disk format; synchronous recovery and saves remain latency risks.

The comparison executable was built from `7614258` before either change. Both experiments use the same i9-13900H Linux host, shuffled editor order, release executables, ASCII fixtures, ten launches and 400 key samples per editor/file size. No builds or tests ran during either measurement. Other host activity, CPU frequency and scheduling were not controlled. Every attempted trial succeeded; this does not qualify files above the existing 32 MiB opening limit.

The [first experiment](benchmarks/2026-10-08-disk-snapshots.json), after `2bc6abc` but before the watcher change, includes fresh Vim/Neovim comparisons with the original one-second idle interval. For the 10 MiB workload:

| Editor | Startup median ms | Key p95 ms | Tree RSS median MiB |
| --- | ---: | ---: | ---: |
| VSCLI before | 21.83 | 0.866 | 30.32 |
| VSCLI shared saved rope | 20.06 | 0.903 | 20.12 |
| Neovim 0.12.5 | 44.02 | 0.826 | 35.36 |
| Vim 9.2 | 22.61 | 0.380 | 21.63 |

The native change lowered sampled RSS by about 34%, while measured typing p95 was slightly higher. These are unequal-feature baseline configurations with the PTY/Python-oracle boundaries above; the table does not establish an overall editor ranking.

The [second experiment](benchmarks/2026-10-08-disk-watch.json), after `709d506`, extends the idle interval to three seconds so the editor's two-second periodic disk check runs before typing and memory sampling:

| File size | Before RSS MiB | After RSS MiB | Before startup ms | After startup ms | Before key p95 ms | After key p95 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 10 MiB | 30.32 | 20.14 | 20.34 | 19.29 | 1.011 | 0.930 |
| 32 MiB | 77.67 | 45.46 | 47.21 | 45.16 | 0.885 | 1.069 |

RSS decreased by about 41% at 32 MiB. Typing results are mixed: the 32 MiB p95 increased in this run, so no general typing-speed improvement is established. These observations do not measure peak memory during saving/recovery, long-line behavior, or full IDE contention. The three-second idle CPU sample remains too short and tick-quantized to establish the sustained idle budget.

Because the first 32 MiB run showed a possible latency regression, a [focused repeat](benchmarks/2026-10-08-disk-watch-repeat.json) used 20 launches and 800 key samples per binary, retaining the three-second idle interval and the same executables. It had zero failures. Before/after RSS was 77.66/45.49 MiB, startup median 49.74/45.60 ms, key median 0.709/0.705 ms, and key p95 0.966/0.937 ms. The initial latency regression did not recur, while the memory reduction remained consistent. Both runs are retained; this is evidence of variability, not proof of a general typing-speed improvement or a controlled significance test.

Reproduce the periodic-watch comparison with a saved pre-change binary:

```sh
python3 scripts/bench_editor.py --compare-vscli /path/to/previous/vscli \
  --sizes 10485760 33554432 --trials 10 --idle-seconds 3 \
  --output target/benchmarks/disk-watch.json
```

The opening limit, blocking saves and startup recovery, large document operations, and extension/LSP limits remain separate work. Periodic recovery now streams shared-rope snapshots on a dedicated worker with one outstanding request. Stalled-write and shutdown-order tests verify its scheduling and integrity, but the measurements above disabled recovery and establish no latency or memory claim for that change. Recovery-enabled contention and slow-storage shutdown need separate measurements. Raising the size limit alone still would not establish a qualified large-file mode.
