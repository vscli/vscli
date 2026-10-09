# VSCLI delivery and open source plan

The current priority order and audited gaps are in [the parity and user experience plan](PARITY_PLAN.md). The milestone dates, Phase 0 program and initial backlog below are the original planning assumptions, not current delivery estimates or completion claims.

## Release strategy

Deliver a useful native editor before broad extension compatibility. Prove extension feasibility early, then grow native workflows and compatibility together. The first successful release lets someone complete real development tasks without extensive configuration; it does not need to reproduce the entire VS Code product.

Dates below assume focused full-time work and an experienced systems/editor engineer. They are planning ranges with substantial uncertainty, not commitments. Part-time work, unfamiliar editor internals, or unsupported dependencies can extend them significantly. Reestimate after feasibility experiments and each release gate.

## Milestones and exit criteria

| Stage | Deliverable | Exit gate | Solo cumulative planning range |
| --- | --- | --- | --- |
| M0 Feasibility | Editing/rendering prototype, input probe, extension spike, benchmarks | Major architecture risks demonstrated or explicitly narrowed | 4–6 weeks |
| M1 Editing alpha | Files, tabs/splits, ordinary selections, undo, search, palette, safe save/recovery | Maintainer uses it daily for basic editing; file integrity and recovery suite passes | 3–6 months |
| M2 Useful IDE alpha | Curated LSP setup, completion/rename/diagnostics, syntax, snippets, workspace search, Git diff/status | Closed test group completes defined code-change workflows | 6–12 months |
| M3 Extension and IDE beta | Bounded VS Code API, verified extension corpus, task runner, integrated terminal, first debugger | Compatibility report passes its declared gates; hostile/failed tools cannot corrupt buffers | 12–24+ months |
| M4 Stable v1 | Hardened supported workflows, migration, packaging, accessibility and platform qualification | Release checklist and upgrade/rollback/recovery tests pass | Reestimate after M2; potentially 18–36+ months solo |
| Beyond v1 | Broader extensions, tests, notebooks, advanced remote agent, optional AI providers | Separate proposals, maintainers, budgets, and measured demand | Multi-year program |

Stable v1 means a stable, explicit scope. Universal parity has no credible completion date and is excluded from the release definition.

A funded team of 3–5 experienced engineers could provisionally target a useful alpha in 4–8 months and a scoped stable release in 12–24 months. Validate that estimate after M0. Parallel work helps language tools, compatibility, and platform testing, but document semantics and protocol ownership remain shared dependencies. This is a staffing scenario, not a proposal to add all contributors immediately.

## Phase 0 experiments

### Editing and rendering

Build a minimal buffer that opens a file, accepts ordinary text and selections, supports undo, scrolls, and renders visible lines. Include Unicode, long lines, and repeated edits. Compare the new-core approach with a thin reuse experiment where practical.

Record launch time, input-to-frame latency, memory, and terminal bytes for empty, 100 KB, 10 MB, and pathological single-line fixtures. Repeat with background CPU and output load. Require no full-document copying or layout on an ordinary keystroke. Investigate missed targets before adding more UI.

### Keyboard feasibility

Build a small input inspector using the intended terminal backend and export the complete default binding inventory from pinned VS Code platform profiles. Begin with collision-prone and common shortcuts across one enhanced terminal, one conservative terminal, and tmux over SSH, then extend coverage across the inventory. Capture OS, keyboard layout, terminal versions, and configuration. Test Ctrl+I/Tab, Ctrl+M/Enter, Escape/Alt ambiguity, Ctrl+Shift combinations, Command/Control distinctions, chords, paste, focus, and resize.

Feasibility gate: demonstrate original VS Code combinations for critical workflows on at least one declared configuration and a concrete configuration/integration path for remaining input conflicts. Alternative shortcuts do not satisfy this gate. Keep configurations with unresolved conflicts unqualified for exact compatibility. Publish input-delivery, resolver, and command-behavior results separately; the prototype is not evidence that the full inventory already passes.

### Extension host feasibility

Allocate an initial two-week timebox to comparing upstream host extraction and an independent shim. Use the same synthetic fixtures and two real, legally usable extensions: one command/edit extension and one language/provider-oriented extension.

Demonstrate activation, a command, document synchronization, synchronous reads inside change callbacks, selections, versioned edits, a quick pick, configuration, cancellation, and host restart. Record unsupported APIs and source/build maintenance cost. Keep a deliberate unsupported webview fixture.

Gate: choose a host strategy based on working behavior. If neither passes, downgrade extension support to experimental and retain the native IDE roadmap. Do not widen the API indefinitely to make the experiment look successful.

### Data safety and dependency feasibility

Prototype saved-file replacement and recovery after forced termination. Exercise permission failures, disk-full behavior, external modifications, CRLF, Unicode, and symlinks. Inventory dependencies, platform support, package licenses, and the scope of any copied code.

Gate: document ownership, encoding conversion, save semantics, terminal lifecycle, and dependency obligations have concrete evidence. End M0 with an architecture decision record and revised schedule, not a large partially wired framework.

## First 30 working days

| Days | Work | Reviewable result |
| --- | --- | --- |
| 1–3 | Record assumptions, select reference versions and fixtures, establish minimal workspace and CI | Reproducible build, benchmark procedure, baseline decisions |
| 4–8 | Buffer/view separation, text transactions, grapheme navigation, basic rendering | A file can be edited and undone with latency measurements |
| 9–11 | Save/recovery prototype, external-change detection, failure injection | Demonstrated recovery boundaries and failure cases |
| 12–14 | Keyboard protocol probe, baseline inventory, and terminal matrix | Original shortcut traces, configuration recipes, and explicit unresolved conflicts |
| 15–24 | Host extraction versus shim experiments, contract fixtures | Evidence for one compatibility strategy and its missing surfaces |
| 25–27 | Real language-server workflow, syntax and viewport stress tests | Completion/diagnostics demonstrator with background-load measurements |
| 28–30 | Reuse decision, dependency/license review, scope adjustment | Accepted M1 architecture and prioritized backlog |

This is approximately six full-time working weeks. It is a feasibility program, not a public production-release promise. If experiments finish early, invest remaining time in failure cases rather than expanding scope.

## Initial implementation backlog

These issue-sized units follow the dependency order. Each should include a demonstrable behavior and avoid speculative framework work.

1. **Headless document core:** open UTF-8 text, versioned insert/delete, selection mapping, and undo/redo without a terminal.
2. **Coordinate conversion:** byte, scalar, UTF-16, and grapheme handling with boundary and randomized cases.
3. **Terminal lifecycle:** raw mode, alternate screen, bounded input parsing, resize, suspend/resume, and restoration.
4. **Editor viewport:** visible-line layout, wrapping, tabs, wide characters, incremental updates, and long-line limits.
5. **Command/context registry:** named actions shared by keys, palette, menus, and API calls.
6. **Keybinding resolver:** platform defaults, precedence, arguments, chords, `when` evaluation, and conflict inspector.
7. **Files and recovery:** safe saving, dirty state, reload/compare, crash journal, and recovery UI.
8. **Navigation/workbench:** tabs, splits, explorer, quick open, workspace search, and focus movement.
9. **Language service broker:** process lifecycle, LSP negotiation, synchronization, diagnostics, and one curated language.
10. **Editing language features:** completion, snippets, formatting, definitions, rename, and stale-result handling.
11. **Compatibility protocol and host:** document mirrors, event contracts, native quick picks, and deterministic fixtures.
12. **Packaging and diagnostics:** release builds, safe configuration migration, doctor command, and portable bug reports.

Multi-cursor editing belongs in the document model early even if its complete UI comes later. It is expensive to retrofit once selection and undo semantics are entrenched.

## Definition of the first useful release

A developer can install the editor, open a repository, find a file, navigate to a symbol, change code with familiar selection and undo behavior, see diagnostics, complete a symbol, format, rename safely, search across files, inspect a diff, and recover unsaved work after a crash.

Choose three language workflows during M0 based on actual early users. A reasonable provisional set is Rust, TypeScript/JavaScript, and Python through independently usable tools. Provisioning and server licenses must be checked before bundling anything. Do not claim equivalence to their entire VS Code extensions.

The release must work with the optional Node compatibility host disabled. It must also demonstrate that a busy or crashed host does not stop ordinary native editing. Publish the difference between core functionality and extension-provided functionality in onboarding.

## Validation strategy

| Layer | Meaningful tests |
| --- | --- |
| Document correctness | Random edit histories versus a simple reference model; undo round trips; selection mapping; Unicode and newline boundaries |
| Persistence | Crash at each journal/save stage; truncated records; disk full; permission failure; concurrent external modification |
| VS Code behavior | Differential fixtures for commands, contexts, selections, synchronous reads, events, and failures |
| UI and terminal | Cell-grid snapshots, PTY traces, focus traversal, resize, paste, suspend/resume, terminal restoration |
| Language/debug protocols | Replay fixtures plus real servers/adapters; delayed, malformed, canceled, and out-of-order replies |
| Integrated terminal | Resize, alternate screen, scrollback, child processes, nested TUIs, control sequences, and focus escape |
| Extension workflows | Pinned real packages, exact user actions, observable results, and documented limitations |
| Performance | Dedicated reference machine; latency distributions, total process memory, CPU, output bytes, and background contention |
| Distribution | Clean install, upgrade, rollback, extension disablement, offline core startup, and supported OS packages |

Fuzz parsers and state transitions where untrusted input could corrupt state: input escape sequences, RPC framing, document coordinate conversions, configuration parsers, and extension archives. Ordinary snapshot tests do not replace randomized editing and recovery tests.

Use hosted CI for correctness, formatting, compilation, and platform smoke tests. Run performance gates on a controlled runner to avoid interpreting noisy shared-host results as regressions. Start with recorded baselines, investigate meaningful sustained regressions, and tighten budgets once variance is understood.

The initial public platform matrix should distinguish supported, experimental, and untested configurations. Include Linux and macOS for alpha qualification, Windows/ConPTY before claiming Windows support, and tmux/SSH combinations as separate cells. Terminal emulators and multiplexers have versions and configurations; OS support alone is insufficient.

## v1 release gate

Before v1, require all of the following:

- Supported editing and recovery workflows pass the failure-injection suite, with no known reproducible data-loss issue in that scope.
- The published API baseline and named extension workflows pass; unsupported cases remain visible in the report.
- Exact original shortcuts pass input, context/dispatch, and effect tests for implemented workflows on qualified configurations. Preserve and report the full baseline inventory; a scoped v1 with unimplemented commands must not claim full VS Code keybinding parity. Full parity remains a project requirement beyond any explicitly scoped interim release.
- Performance targets are met or revised with published evidence, including total resource use with the supported extension profile enabled.
- A new user can install, complete the sample workflow, import supported settings, diagnose shortcut conflicts, and uninstall without manual state repair.
- Supported platforms pass their terminal and filesystem tests; unqualified platforms remain experimental.
- Accessibility review, release provenance, dependency inventory, upgrade/rollback tests, and maintainer ownership are complete.

Avoid using a raw extension count as the only release gate. Five useful, reliable workflows can create more value than dozens of extensions that merely activate.

## Open source stewardship

For original code, use a permissive license such as `MIT OR Apache-2.0`, subject to confirming obligations of reused files and dependencies. This is a proposed project choice, not an assertion that all upstream source can be relicensed. Keep third-party notices and provenance. Select a distinctive public name before launch and avoid implying Microsoft endorsement.

Start with a small maintainer team and public decision records. Use an RFC process for changes to document semantics, extension protocols, compatibility policy, and persistence formats. Routine features should not need an RFC. Use a lightweight contribution process, clear code ownership, a contributor conduct policy, and a private security-reporting route.

Keep the editor, native plugin protocol, compatibility host, test harness, documentation, and release tooling open. Avoid making basic functionality depend on a proprietary hosted service. Optional funding can come from sponsorship, grants, paid support, or independently optional hosted services; the product should remain useful offline.

Build contributor entry points around bounded work: one native widget, one command contract, one settings mapping, one terminal trace, one language recipe, one extension workflow. Provide a small sample plugin and a one-command fixture runner once the protocol exists. Prefer upstreaming general library fixes over maintaining silent forks.

Measure adoption through voluntary workflow reports, repeat users in the test group, successful migrations, recovery reliability, issue resolution, and contributor retention. Keep telemetry off by default; local performance diagnostics should be exportable after review. Stars and installation counts alone do not show that the editor is usable.

With a funded team, assign durable ownership for core/rendering, language/workbench services, extension compatibility, and platform/release quality. With one maintainer, move these through a shared sequence and keep feature intake narrow. Every new subsystem creates an ongoing maintenance obligation.

## Risk register

| Risk | Early signal | Response |
| --- | --- | --- |
| Compatibility work consumes the project | Host spikes require large unbounded service surfaces | Keep a bounded API contract and ship useful native workflows |
| Editing feels slow despite native code | Long-tail latency during layout, parsing, or extension load | Trace the input path, bound work, and remove blocking dependencies |
| Corrupted or lost edits | Recovery mismatches or stale edits in randomized tests | Block releases and fix document/persistence semantics first |
| Terminal inconsistency | Key collisions, width drift, multiplexer regressions | Capability negotiation, explicit matrix, and reachable command fallbacks |
| Required extensions cannot be distributed or used | Corpus depends on restricted packages or services | Use eligible alternatives and document the affected workflows |
| Extension/resource contention | Host memory or CPU overwhelms the machine | Lazy activation, resource reporting, bounded queues, disable/restart controls |
| Maintainer overload | Growing unowned subsystems and long review queues | Freeze new scope, recruit subsystem owners, fund maintenance |
| Weak differentiation | Users see only another configurable terminal editor | Focus on ordinary editing, migration, polished defaults, and verified compatibility |

The highest-value early asset is the combination of a reliable native document core and a public behavioral compatibility harness. The core creates immediate user value; the harness turns a large compatibility ambition into testable, contributor-sized work.
