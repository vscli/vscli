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

## Remaining performance qualification

The next workloads must measure source highlighting and incremental parsing, 100 MiB files and very long lines, Unicode movement/multiple cursors, undo/paste/save, 100,000-file indexing and search, and editing under LSP/extension/task contention. Add matched VS Code and Helix measurements with versioned configurations, cold-cache procedures, real-terminal input-to-display latency, process-tree peak memory, and sustained CPU sampling. A single favorable component or empty-editor benchmark cannot answer which editor is fastest overall.

The separate [extension mirror measurement](EXTENSIONS.md#mirror-performance-measurement) remains a Node-component microbenchmark. Its speedup does not describe native typing latency or total editor speed.
