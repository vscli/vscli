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

Use `--single-line` to put the entire ASCII fixture on one line without line
breaks, retaining the same exact byte count and readiness marker. The JSON
records `fixture_layout` as `single_line` or `multiple_lines`. This measures
opening and typing near the start of a long plain-text line; it does not measure
navigation or drawing far to the right, Unicode clusters, or syntax fallback.
CI also checks this screen oracle with a 1 MiB single-line fixture, without a
timing threshold.

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

The opening limit, blocking saves and startup recovery, large document operations, and extension/LSP limits remain separate work. Periodic recovery now streams shared-rope snapshots on a dedicated worker with one outstanding request. Stalled-write and shutdown-order tests verify its scheduling and integrity, but the measurements above disabled recovery and establish no latency or memory claim for that change. The next experiment measures recovery-enabled typing. Raising the size limit alone still would not establish a qualified large-file mode.

## Typing during periodic recovery: 2026-10-08

[Raw observations](benchmarks/2026-10-08-background-recovery.json) compare release
builds from `07c9fea` (before) and native change `4655e4b` (after), using harness
commit `7580c9c`. The same i9-13900H Linux host ran ten fresh launches and 500 key
samples per build at each size. Every trial enabled recovery in a fresh temporary
directory, waited one second after readiness, then inserted 50 characters with a
100 ms terminal-pumping interval after each preceding observed update. Builds and
tests did not run during measurement; unrelated host activity and CPU frequency
were not controlled. All 40 trials succeeded. Each journal contained the first
30 measured insertions when inspected after the typing workload.

| File size | Before key median ms | After key median ms | Before key p99 ms | After key p99 ms | Before maximum ms | After maximum ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 10 MiB | 0.939 | 0.866 | 458.92 | 1.275 | 524.47 | 1.456 |
| 32 MiB | 0.901 | 0.899 | 1,432.48 | 1.318 | 1,556.51 | 1.545 |

| File size | Before startup median ms | After startup median ms | Before sampled RSS MiB | After sampled RSS MiB |
| --- | ---: | ---: | ---: | ---: |
| 10 MiB | 20.47 | 19.54 | 33.41 | 20.30 |
| 32 MiB | 45.19 | 45.73 | 45.87 | 45.60 |

The previous implementation's recovery work periodically stalled the input
thread. The change combines shared snapshots, buffered JSON streaming, and a
dedicated writer; this experiment does not attribute the improvement to one
component independently. The large reduction is in the tail: median typing
latency at 32 MiB was effectively unchanged. Sampled memory after typing is not
peak memory during serialization and shows no material reduction at 32 MiB.

These are one-machine observations with the existing PTY/oracle boundaries, not
an overall editor ranking or physical-display latency. The interval follows each
completed key, so stalls extend the previous build's total trial duration; this
does not simulate fixed-rate queued input. Journal validation establishes that
recovery happened during the workload, not the exact time of each write. Slow
storage, shutdown waits, save latency, highlighting, LSP/extension contention,
and larger files remain separate qualification work.

```sh
python3 scripts/bench_editor.py --compare-vscli /path/to/before/vscli \
  --sizes 10485760 33554432 --trials 10 --keys 50 --key-interval-ms 100 \
  --idle-seconds 1 --recovery --output target/benchmarks/background-recovery.json
```


## Long-line viewport and navigation changes: 2026-10-08

The renderer previously copied a whole line, scanned it for the ASCII fast path,
and copied it again to find its end, even when the cursor and viewport were at
the beginning. Native commit `44c15a7` streams rope navigation and column lookup,
uses cheap line bounds, and limits plain-text/ready-grammar rendering to the
viewport prefix with query lookahead. The preceding dependency commit `ef81391`
backports two upstream Unicode chunk-boundary fixes, retaining Unicode 17 tables.
Unicode corpus tests, real rope tests, and a long-line PTY edit/undo/save workflow
cover correctness separately from the ASCII performance measurements.

[Single-line raw observations](benchmarks/2026-10-08-long-lines.json) compare
main `80e3446` with these changes using harness `389997a`. Each entire file is
one ASCII line; typing starts at its beginning. The same i9-13900H Linux host
ran ten fresh launches and 400 key samples per build at each size, with shuffled
build order, a one-second idle sample, and recovery disabled. All 60 trials
succeeded. No builds or tests ran during measurement; CPU frequency, scheduling,
and unrelated host activity were not controlled. Executable hashes and every
sample are retained in the report.

| File size | Before key median ms | After key median ms | Before key p99 ms | After key p99 ms | Before startup median ms | After startup median ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 MiB | 2.764 | 0.580 | 4.254 | 0.965 | 13.18 | 8.33 |
| 10 MiB | 21.924 | 0.485 | 27.588 | 0.932 | 42.42 | 19.59 |
| 32 MiB | 65.275 | 0.451 | 99.311 | 0.883 | 111.55 | 45.62 |

The 32 MiB typing median is about 145 times lower in this particular workload.
This removes observed file-size scaling from typing at the start of these plain
text lines; it does not establish constant-time navigation throughout a line or
an overall editor ranking. Sampled 32 MiB process-tree RSS was 45.69 MiB before
and 45.48 MiB after, so this experiment does not demonstrate a substantial memory
reduction. Whole-line temporary allocations can disappear before RSS sampling;
peak memory is not measured.

Far-right navigation still scans prefixes, fallback syntax coloring still
processes whole lines, and an unusually large grapheme cluster still needs its
full context. Synchronous open/save and the 32 MiB opening cap remain. This
experiment does not qualify Unicode timing, syntax workloads, physical terminal
latency, larger files, or editing under language-server/extension contention.

Reproduce with the saved pre-change release binary:

```sh
python3 scripts/bench_editor.py --compare-vscli /path/to/80e3446/vscli \
  --single-line --sizes 1048576 10485760 33554432 --trials 10 --keys 40 \
  --idle-seconds 1 --output target/benchmarks/long-lines.json
```


The initial [ordinary multi-line check](benchmarks/2026-10-08-long-lines-multiline-initial.json)
showed small latency increases. [Native phase measurements](benchmarks/2026-10-08-long-lines-native.json)
reproduced a roughly 13 µs rendering increase in both ASCII and Unicode fixtures;
CPU sampling identified line-slice handling as one added cost. Commit `fd12d48`
trims short borrowed lines directly, excludes empty selections from painting,
and hoists per-document language/highlight lookups out of the row loop. The
refined build's native render median was 200.51 µs versus 218.43 µs for ASCII,
and 276.97 µs versus 295.12 µs for Unicode in the paired runs. Each run has 4,000
samples; ordinary timings were collected separately from CPU sampling.

A [final ordinary multi-line check](benchmarks/2026-10-08-long-lines-multiline-final.json)
compares `80e3446` with `fd12d48`, again with ten launches and 400 keys per size,
shuffled order, recovery disabled, and zero failed trials:

| File size | Before key median ms | After key median ms | Before key p95 ms | After key p95 ms |
| --- | ---: | ---: | ---: | ---: |
| 10 KiB | 0.761 | 0.690 | 1.149 | 1.049 |
| 1 MiB | 0.754 | 0.790 | 1.014 | 1.034 |
| 10 MiB | 0.738 | 0.756 | 0.961 | 0.978 |

These small mixed PTY differences do not establish a general typing-speed
improvement for ordinary files. Both the initial observations and the final
check are retained; the native component timings do not replace the executable
results or qualify interactive performance under full IDE workloads.


[Final single-line measurements](benchmarks/2026-10-08-long-lines-final.json)
repeat the same long-line workload with `fd12d48`. All 60 trials passed:

| File size | Before key median ms | After key median ms | Before key p99 ms | After key p99 ms | Before startup median ms | After startup median ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 MiB | 2.791 | 0.570 | 4.476 | 0.912 | 12.99 | 8.65 |
| 10 MiB | 22.373 | 0.583 | 37.483 | 0.965 | 42.56 | 20.15 |
| 32 MiB | 69.318 | 0.473 | 100.138 | 0.846 | 128.96 | 44.92 |

The large single-line improvement remains after the short-line refinement.
All workload and measurement limitations above still apply.

## Resolved-completion core baseline: 2026-10-10

[Raw samples and provenance](benchmarks/2026-10-10-intellisense-baseline.json)
compare optimized IntelliSense source `cedc66e` against the prior main `7828303`
on the same Linux i9-13900H machine. Both use `cargo build --release --locked`.
Five fresh processes per editor/fixture produce 200 keystroke samples per row;
order is interleaved, with no concurrent local builds or tests.

| Fixture | Build | Startup median ms | Key median ms | Key p95 ms | Key p99 ms | RSS median MiB |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 10 KiB | Prior main | 6.051 | 0.307 | 0.664 | 0.925 | 11.71 |
| 10 KiB | IntelliSense | 6.741 | 0.323 | 0.763 | 1.034 | 12.23 |
| 1 MiB | Prior main | 9.105 | 0.308 | 0.685 | 0.929 | 12.89 |
| 1 MiB | IntelliSense | 7.220 | 0.306 | 0.682 | 0.927 | 13.27 |

All 20 trials completed without failures. The 10 KiB candidate records higher
startup/key medians; the 1 MiB key median is similar. Five startup samples and
uncontrolled host scheduling cannot establish a sustained regression or speedup.
The 0.3-second CPU sample reported zero ticks and does not qualify sustained idle
CPU. These ASCII, warm-cache, no-LSP/no-extension measurements cover the core;
completion load is separately exercised by the unchanged four-second 1,200-key
save oracle and real-server/package integrity workflows. Full IDE contention,
physical input-to-display and fastest-editor claims remain unqualified.

## Smart typing and idle hint work: 2026-10-10

The [initial observations](benchmarks/2026-10-10-smart-typing-initial.json)
compare smart-typing/signature source `123defd` with saved prior main `ccea48b`
(PR #55). The [final observations](benchmarks/2026-10-10-smart-typing-final.json)
use clean source `a0efe29`, including `db6fd73`: an idle signature controller skips
extra key resolution, and typing without an eligible signature source skips
selection snapshots and settings clones. These are two separately recorded
experiments; the initial results remain available.

Both experiments use release binaries on the same Linux i9-13900H machine,
interleaved fresh processes, five launches and 200 serial keys per build/fixture.
No local builds, tests or reference-editor runs execute during either measurement.
The raw records contain executable and fixture hashes; the saved comparison
binary has SHA-256 `cdf5375e57a5e69537237a5083ba21180f82346f64096d55403992f8e556a545`.
The final native executable was built from the same runtime source before its
subsequent test-only commit. These warm-cache ASCII plain-text workloads have
no LSP, extensions or recovery, and measure PTY-to-observed-cell latency including
the Python oracle. They do not exercise bracket rules or active hint providers.

| Run | Fixture | Build | Startup median ms | Key median ms | Key p95 ms | Key p99 ms | RSS median MiB |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| Initial | 10 KiB | Prior main | 6.234 | 0.305 | 0.686 | 0.895 | 12.08 |
| Initial | 10 KiB | Smart typing | 6.140 | 0.307 | 0.708 | 0.883 | 12.73 |
| Initial | 1 MiB | Prior main | 7.803 | 0.308 | 0.576 | 0.770 | 12.87 |
| Initial | 1 MiB | Smart typing | 8.282 | 0.320 | 0.766 | 0.960 | 13.78 |
| Final | 10 KiB | Prior main | 7.061 | 0.308 | 0.665 | 0.868 | 11.93 |
| Final | 10 KiB | Smart typing | 5.439 | 0.314 | 0.751 | 0.911 | 12.49 |
| Final | 1 MiB | Prior main | 7.136 | 0.323 | 0.716 | 1.009 | 13.23 |
| Final | 1 MiB | Smart typing | 6.873 | 0.317 | 0.716 | 0.970 | 13.32 |

All 20 trials in each experiment passed. The initial 1 MiB candidate had a higher
observed typing tail; the final 1 MiB p95 is similar to its interleaved comparison.
The final 10 KiB candidate still has a higher observed p95. Medians remain around
0.31–0.32 ms; neither experiment establishes a general speedup, sustained
regression, or a causal estimate for the fast paths. Startup and sampled RSS vary
between experiments too. Host scheduling and CPU frequency were uncontrolled;
five startup observations do not qualify startup tails. The 0.3-second idle
sample records zero CPU ticks and does not establish sustained idle behavior.

Reproduce with a saved PR #55 release executable:

```sh
python3 scripts/bench_editor.py --compare-vscli /path/to/pr55/vscli \
  --sizes 10240 1048576 --trials 5 --keys 40 --idle-seconds 0.3 \
  --output target/benchmarks/smart-typing.json
```

Active signature/completion contention, source highlighting, wider document sizes,
physical input-to-display latency and matched VS Code workloads remain separate
performance qualification. Functional native/extension/clangd PTYs qualify their
named interactions; they are not performance comparisons.

## Advanced indentation baseline: 2026-10-10

[Raw observations](benchmarks/2026-10-10-advanced-indentation.json) compare clean
advanced-indentation source `00a4718` with the saved PR #56 release executable
(merged main `04d01e1`). The candidate SHA-256 is
`fdcfbf3ab16b9ee6b88335b30607e42f16a3a380203b7660105db343cfb0ee0e`;
the comparison SHA-256 is
`6b9288a4ca05b8a8e675637872e8871e7d9f6beb03f29269cfaa31eeca017a7b`.
Both are optimized native builds on the same Linux i9-13900H host. Five fresh
launches per build/fixture yield 200 serial keys per row, with interleaved order
and no concurrent local builds, tests or reference-editor processes.

| Fixture | Build | Startup median ms | Key median ms | Key p95 ms | Key p99 ms | RSS median MiB |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 10 KiB | Prior main | 6.475 | 0.302 | 0.704 | 0.938 | 12.13 |
| 10 KiB | Advanced indentation | 6.251 | 0.298 | 0.575 | 0.884 | 12.36 |
| 1 MiB | Prior main | 5.113 | 0.161 | 0.398 | 0.624 | 13.22 |
| 1 MiB | Advanced indentation | 5.164 | 0.155 | 0.540 | 0.829 | 13.49 |

All 20 trials passed. The candidate's 10 KiB typing tail is lower, while its
1 MiB tail is higher; these mixed observations establish no general speedup or
sustained regression. Host scheduling and CPU frequency were uncontrolled.
The warm-cache ASCII plain-text workload exercises ordinary typing with no LSP,
extensions or recovery; it does not benchmark indentation predicates or lexical
scan exhaustion. PTY-to-observed-cell timings include the Python oracle and
exclude physical input and graphical terminal painting. Sampled RSS is not peak
memory; five startup samples do not qualify startup tails. The 0.3-second idle
sample recorded zero ticks and does not establish sustained idle CPU behavior.

Reproduce with a saved PR #56 release executable:

```sh
python3 scripts/bench_editor.py --compare-vscli /path/to/pr56/vscli \
  --sizes 10240 1048576 --trials 5 --keys 40 --idle-seconds 0.3 \
  --output target/benchmarks/advanced-indentation.json
```

Source-language typing, active IDE contention, matched VS Code workloads and
fastest-editor claims remain separate performance qualification.


## Native language configuration baseline: 2026-10-10

[Raw observations](benchmarks/2026-10-10-native-language-configurations.json)
compare the optimized native runtime from `ce6deaf` with the saved PR #57
executable (merged main `8955395`). Candidate SHA-256:
`d89313c42a0c06e0ca840bed2500a0bb3714c3e0c98b5775166dcfd4057fcb4f`;
comparison SHA-256:
`fdcfbf3ab16b9ee6b88335b30607e42f16a3a380203b7660105db343cfb0ee0e`.
Both ran on the same Linux i9-13900H host, with five fresh launches and 200 serial
keys per build/fixture. Order was interleaved, with no concurrent local builds,
tests or reference-editor processes. A subsequent comparator-only validation
fix does not change the measured runtime.

| Fixture | Build | Startup median ms | Key median ms | Key p95 ms | Key p99 ms | RSS median MiB |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 10 KiB | Prior main | 7.569 | 0.434 | 0.840 | 1.067 | 12.10 |
| 10 KiB | Native configurations | 7.850 | 0.621 | 0.806 | 1.182 | 12.36 |
| 1 MiB | Prior main | 5.040 | 0.159 | 0.540 | 0.704 | 13.21 |
| 1 MiB | Native configurations | 4.495 | 0.156 | 0.444 | 0.915 | 13.61 |

All 20 trials passed. The candidate has a higher 10 KiB key median and both
p99 observations, with a lower 1 MiB startup median and typing p95. These mixed
results establish no general speedup, fastest-editor ranking or sustained
regression. Host scheduling and CPU frequency were uncontrolled; five startup
samples do not qualify startup tails. Sampled RSS is not peak memory. The
0.3-second idle sample recorded zero ticks and does not establish sustained idle
CPU behavior.

This warm-cache ASCII plain-text workload uses isolated configuration/data
paths, with no LSP, installed extensions or recovery. It observes ordinary
native typing rather than configured pairing, comment commands or catalog
loading under contention. Timings include the Python cell oracle and exclude
physical input and graphical terminal painting. Source-language and matched
VS Code workload qualification remain separate work.

Reproduce with a saved PR #57 release executable:

```sh
python3 scripts/bench_editor.py --compare-vscli /path/to/pr57/vscli \
  --sizes 10240 1048576 --trials 5 --keys 40 --idle-seconds 0.3 \
  --output target/benchmarks/native-language-configurations.json
```

## Native navigation history baseline: 2026-10-10

The [ordinary-file run](benchmarks/2026-10-10-native-navigation-history.json) and
[single-line run](benchmarks/2026-10-10-native-navigation-history-single-line.json)
compare the optimized navigation candidate at `ac7a838` with the saved PR #58
native runtime. Executable SHA-256 is
`5d83baf92ff05263f7ee231965d511cf8273cb8636e36f22adf6e054c9cbc55b`
for the candidate and
`d89313c42a0c06e0ca840bed2500a0bb3714c3e0c98b5775166dcfd4057fcb4f`
for the prior runtime. Both reports preserve binary identities, fixture hashes,
individual trials and the exact invocation.

Five interleaved launches per executable and workload each observe 40 serial
keys in a 120×40 PTY on the same Linux/i9-13900H host. No local builds, test
workflows or reference editors ran concurrently. All 30 trials and 1,200 observed
keys passed. Each startup/RSS median has five samples; each key distribution has
200 samples.

| Fixture | Executable | Startup median ms | Key median ms | Key p95 ms | Key p99 ms | Sampled RSS MiB |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 10 KiB, multiple lines | Prior runtime | 5.710 | 0.303 | 0.702 | 0.935 | 12.35 |
| 10 KiB, multiple lines | Navigation candidate | 7.660 | 0.395 | 0.917 | 1.138 | 12.46 |
| 1 MiB, multiple lines | Prior runtime | 7.841 | 0.303 | 0.910 | 1.090 | 13.36 |
| 1 MiB, multiple lines | Navigation candidate | 8.803 | 0.395 | 0.938 | 1.141 | 13.92 |
| 1 MiB, single line | Prior runtime | 7.740 | 0.209 | 0.484 | 0.820 | 13.40 |
| 1 MiB, single line | Navigation candidate | 8.546 | 0.216 | 0.765 | 0.906 | 13.62 |

The candidate has higher measured startup/key medians and key tails in these
runs. These observations merit continued tracking; they establish no speedup,
fastest-editor ranking or causal/sustained regression. CPU frequency and host
scheduling remain uncontrolled, and five startup samples do not qualify startup
tails. The short 0.3-second idle samples recorded zero ticks and do not establish
sustained idle CPU behavior. Sampled RSS is not peak memory.

This is warm-cache ASCII plain-text typing with isolated configuration/data,
no language server, installed extensions or recovery. The single-line fixture
types at the beginning of a 1 MiB line. Native location capture uses Rope UTF-16
counters rather than flattening or scanning the line, but this experiment does
not prove all history operations have constant latency. Closed-file travel,
filesystem delays, actual language projects and sustained history contention
remain separate workloads. Timings include the Python cell oracle and exclude
physical input and graphical terminal painting.

Reproduce with a saved PR #58 native release executable:

```sh
python3 scripts/bench_editor.py --compare-vscli /path/to/pr58/vscli \
  --sizes 10240 1048576 --trials 5 --keys 40 --idle-seconds 0.3 \
  --output target/benchmarks/native-navigation-history.json
python3 scripts/bench_editor.py --compare-vscli /path/to/pr58/vscli \
  --single-line --sizes 1048576 --trials 5 --keys 40 --idle-seconds 0.3 \
  --output target/benchmarks/native-navigation-history-single-line.json
```


## Native Outline core baseline: 2026-10-10

The [ordinary-file run](benchmarks/2026-10-10-native-outline.json) and
[single-line run](benchmarks/2026-10-10-native-outline-single-line.json) compare
Outline source `d3b3736` with the preserved PR #59 native executable. Each row
has five interleaved launches and 200 serial key samples. All 30 trials and
1,200 keys passed. No build, test suite or reference editor ran concurrently.
The machine was the same Linux x86_64 i9-13900H host, with 20 logical CPUs and
120 × 40 terminal cells. CPU frequency, scheduling and thermal state were not
controlled.

| File / executable | Startup median ms | Key median ms | Key p95 ms | Key p99 ms | Sampled tree RSS median MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| 10 KiB multiline / PR #59 | 7.186 | 0.318 | 0.880 | 0.993 | 12.29 |
| 10 KiB multiline / Outline | 7.096 | 0.342 | 0.935 | 1.051 | 12.55 |
| 1 MiB multiline / PR #59 | 7.789 | 0.308 | 0.677 | 0.801 | 13.54 |
| 1 MiB multiline / Outline | 7.663 | 0.316 | 0.726 | 0.896 | 13.77 |
| 1 MiB single line / PR #59 | 8.145 | 0.212 | 0.545 | 0.782 | 13.52 |
| 1 MiB single line / Outline | 7.601 | 0.207 | 0.469 | 0.689 | 13.84 |

Candidate key tails were higher for both multiline workloads and lower for the
single-line workload. These mixed observations establish no overall speedup,
causal regression or editor ranking. Five startup observations cannot establish
startup tails. Sampled RSS is not peak memory; zero ticks in the 0.3-second idle
windows do not establish sustained idle CPU behavior.

This measures warm-cache ASCII plain-text editing with isolated configuration,
no language server, extensions or recovery, and Outline disabled by default.
It checks ordinary core editing after integration. It does **not** measure an
active Outline, provider contention, large symbol trees, project indexing or
physical terminal input-to-display latency. The Python cell oracle is included;
graphical terminal painting and physical input are excluded. Real clangd and
native/extension Outline correctness have separate tests, not latency claims.
Executable and fixture hashes remain in both raw reports.

Reproduce with a saved PR #59 release executable:

```sh
python3 scripts/bench_editor.py --compare-vscli /path/to/pr59/vscli \
  --sizes 10240 1048576 --trials 5 --keys 40 --idle-seconds 0.3 \
  --output target/benchmarks/native-outline.json
python3 scripts/bench_editor.py --compare-vscli /path/to/pr59/vscli \
  --single-line --sizes 1048576 --trials 5 --keys 40 --idle-seconds 0.3 \
  --output target/benchmarks/native-outline-single-line.json
```

## Native Breadcrumbs core baseline: 2026-10-10

The [ordinary-file run](benchmarks/2026-10-10-native-breadcrumbs.json) and
[single-line run](benchmarks/2026-10-10-native-breadcrumbs-single-line.json)
compare source `d677ebd` with the preserved PR #60 Outline executable. Each row
has five interleaved launches and 200 serial key samples: 30 successful trials
and 1,200 keys, with zero failures. No build, test suite or reference editor ran
concurrently. The Linux x86_64 i9-13900H host had 20 logical CPUs and a 120 × 40
terminal; CPU frequency, scheduling and thermal state were uncontrolled.

| File / executable | Startup median ms | Key median ms | Key p95 ms | Key p99 ms | Sampled tree RSS median MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| 10 KiB multiline / PR #60 | 4.455 | 0.268 | 0.606 | 0.909 | 12.50 |
| 10 KiB multiline / Breadcrumbs | 5.713 | 0.197 | 0.501 | 0.751 | 12.48 |
| 1 MiB multiline / PR #60 | 8.966 | 0.345 | 1.123 | 1.262 | 13.67 |
| 1 MiB multiline / Breadcrumbs | 7.365 | 0.319 | 0.737 | 0.933 | 13.65 |
| 1 MiB single line / PR #60 | 9.161 | 0.209 | 0.488 | 0.786 | 13.47 |
| 1 MiB single line / Breadcrumbs | 9.596 | 0.213 | 0.490 | 0.795 | 13.61 |

The candidate's observed multiline key medians and tails were lower, while its
single-line key measurements and two startup medians were higher. These mixed
observations establish no overall speedup, causal regression or editor ranking.
Five startup samples cannot establish startup tails; pooled key samples are not
independent launches. Sampled RSS is not peak memory. Short 0.3-second idle
windows are insufficient for sustained CPU claims.

This is warm-cache ASCII plain-text editing with isolated settings/data and no
language server, extensions or recovery. Breadcrumbs file labels are enabled by
default in the candidate, with no symbol provider; Outline remains disabled.
The baseline predates Breadcrumbs. The experiment measures ordinary core editing
and the new default file-trail presentation, rather than active symbol refresh,
provider contention, large projects or filesystem dropdowns. Timings include
the Python cell oracle and exclude physical input and graphical terminal painting.
Actual clangd and optional-host Breadcrumbs correctness have separate evidence.
Both reports retain binary and fixture hashes; the candidate executable SHA256
is `f970fc2d68ce9fe8115ef3ccf67088e5b831d3a62e8dc92fd737ed2ec974ec7d`.

Reproduce with a saved PR #60 native release executable:

```sh
python3 scripts/bench_editor.py --compare-vscli /path/to/pr60/vscli \
  --sizes 10240 1048576 --trials 5 --keys 40 --idle-seconds 0.3 \
  --output target/benchmarks/native-breadcrumbs.json
python3 scripts/bench_editor.py --compare-vscli /path/to/pr60/vscli \
  --single-line --sizes 1048576 --trials 5 --keys 40 --idle-seconds 0.3 \
  --output target/benchmarks/native-breadcrumbs-single-line.json
```
