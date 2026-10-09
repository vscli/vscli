# Pinned VS Code differential reference

This harness runs the real VS Code **1.95.0**, commit
`912bb683695358a54ae0c670461738984cbb5b95`, in an isolated temporary profile,
single-folder workspace, and empty user-extension directory. Version and source
commit are asserted. The baseline is deliberately pinned, not the latest VS Code.
The official `@vscode/test-electron` downloader obtains the reference executable;
no editor binary is committed or distributed with VSCLI.

## Run

```sh
cargo build --locked
npm ci --ignore-scripts --no-audit --no-fund --prefix tests/vscode-reference
# Linux with Xvfb and xauth installed:
xvfb-run -a node tests/vscode-reference/run.cjs target/vscode-reference/result
# macOS or Windows (or Linux with an available display):
node tests/vscode-reference/run.cjs target/vscode-reference/result
node tests/vscode-reference/compare.cjs target/vscode-reference/result target/debug/vscli
node tests/vscode-reference/diagnostics-compare.cjs target/vscode-reference/result
# Use target/debug/vscli.exe on Windows.
node --test tests/vscode-reference/compare.test.cjs tests/vscode-reference/supervisor.test.cjs tests/vscode-reference/diagnostics-compare.test.cjs
```

The test extension contributes only fixture settings, with no keybindings. User
extensions, user settings, and the user's workspace are isolated. Built-in
extensions remain enabled and are inventoried with their declared licenses and
versions. Telemetry, updates, and extension auto-updates are disabled. The runner
removes its temporary profile/workspace after stopping the reference worker;
downloads remain in ignored `target/vscode-reference/cache`. The extension suite
has a 120-second timeout. A supervisor bounds the entire download/test worker to
five minutes; CI additionally bounds each complete platform job to 20 minutes.

The worker reports the awaited download/test result explicitly. The supervisor
then terminates its process group on POSIX or its process tree with `taskkill` on
Windows, including extraction children left behind by interrupted download
retries. Success requires a successful result and cleanup; a timeout, missing
result, interruption, or cleanup failure fails the run. This addresses a Linux
CI hang where the suite and VS Code exited successfully but a download extractor
kept the launcher alive. It does not treat a timeout as successful verification.

Process tests reproduce the interrupted-stream leak with a child and grandchild,
including processes which ignore graceful termination. They exercise success,
failure, malformed results, timeout, and premature worker exit on all platforms,
plus supervisor signal interruption on POSIX. Cleanup relies on descendants
remaining in the owned group/tree; detached descendants and arbitrary Windows
worker crashes with surviving children are not qualified by these tests.

## Evidence and boundaries

Diagnostic collection capture writes `diagnostics.json` and
`diagnostics-provenance.json`: 26 raw snapshots and 17 events from the executable,
including document edit/close retention. The shared optional-host comparison
checks 21 collection snapshots and 15 events. It excludes native document bridge
events and empty global URI resources retained by VS Code's main-thread marker
mirror; collection empty membership, reads and iteration remain compared.
The committed diagnostics baseline records an actual Linux run only. CI captures
and compares each platform independently; absent platform artifacts are not
inferred. `diagnostics-record.cjs` verifies the product commit, observer/trace
hashes and scoped counts before recording a new capture. These API observations
are separate from native Problems and unchanged-linter terminal qualification.

- `keybindings.jsonc`: the reference's default keybindings document, in its
  original resolver order, including its comment list of unbound commands.
- `inventory.json`: parsed rules (duplicates and order retained), reference
  version/commit, platform/architecture, locale, observed keyboard layout, and
  built-in extension inventory. The harness itself is excluded from that list.
- `keyboard.json`: the reference's observed layout and raw mapping, obtained
  through `workbench.action.inspectKeyMappingsJSON`.
- `configuration.json` and `configuration-vscli.json`: 25 observations from the
  same shared test procedure running against each API. Undefined remains
  distinguishable from null. Tests cover registered defaults, object/array
  merging, scope filtering, language overrides, held reads/inspection, masked
  setting changes, listener-time reads, and retained events.
- `comparison.json`: executable hash, configuration equality, and every
  reference binding's structural comparison with the native platform profile.

Configuration discrepancies fail the job. The fixture writer uses VS Code's
configuration API and VSCLI's internal settings synchronization separately;
passing these cases does **not** qualify configuration writes, disk watching,
native-to-host event timing, or the entire configuration API.

Binding categories are mutually exclusive: all rule fields match; key/command/
arguments match but context differs; only command ID matches; command ID is absent
from native defaults. Modifier order and `esc`/`escape` spelling are normalized;
context strings are compared textually. Argument presence is significant, so
absent arguments and explicit null differ. Repeated rules remain in the
denominator, and a native rule can be a candidate for multiple reference rules.

These counts are **not a compatibility percentage or a gate requiring complete
parity**. They do not test context equivalence, resolver priority, input delivery,
or command effects. An ID absent from the native binding table does not prove
that no code path implements it. Hosted CI layouts do not qualify physical keys,
terminals, multiplexers, international layouts, IME, or accessibility.

CI runs both the reference and comparison in each required Linux/macOS/Windows
test job and preserves the evidence as `vscode-reference-<runner>` artifacts for
30 days. Download all artifacts from a specific successful run before recording
a new checked-in baseline. Record its run URL and commit; do not mix platforms
from different runs.

## Recorded baseline

The additional snippet harness writes `snippets.json`: 34 insertion traces
with initial text/selections and selected later edit/navigation/undo observations.
`cargo run --locked --example snippet_contract -- <output>/snippets.json` compares
all captured text and selections with the native document/session engine. These
are document API calls, not terminal-key or completion qualification; see
[snippet implementation boundaries](../../docs/SNIPPETS.md). CI runs the same
comparison against each platform's fresh reference output. The
[snippet baseline](baselines/1.95.0/snippets/provenance.json) records 96 matching
observations per platform from successful CI run 37854940336, with source and
fixture hashes. It is separate from the earlier keybinding/configuration baseline.

The additional `snippet-insertion.json` trace covers 34 insertion-context cases
(118 observations): both API and user-command entry points, initial documents,
selections, indentation options, line endings, cursor order, and undo/redo.
Compare it with `cargo run --locked --example snippet_contract -- --insertion
<output>/snippet-insertion.json`. The same native runner is used by offline Rust
tests and live CI comparisons. The [insertion baseline](baselines/1.95.0/snippet-insertion/provenance.json)
preserves all three platforms from successful CI run 37857315175. Windows
retains its different default EOL for new empty documents.

The [1.95.0 baseline](baselines/1.95.0/provenance.json) preserves the parsed rule
inventories, observed keyboard maps, reference configuration traces, and comparison
summaries from [CI run 37831323012](https://github.com/vscli/vscli/actions/runs/37831323012).
The provenance file records the tested source revision and SHA-256 of each saved
file. All three platforms passed the same 25 observations. The Linux/macOS/Windows
inventories contain 984/1,078/993 rules respectively. The full raw JSONC and detailed
per-rule comparison are in the CI artifacts; the committed inventory preserves
all parsed binding fields and their order.

An offline comparison against the recorded Linux data can be run with:

```sh
node tests/vscode-reference/compare.cjs tests/vscode-reference/baselines/1.95.0/linux target/debug/vscli
```

The comparator writes `comparison.json` and `configuration-vscli.json` in its input
directory. Prefer copying a baseline directory under `target/` before comparing
to keep the checkout clean. This reuses saved observations; it does not replace
running the current shared fixture against the actual reference.

## Variable and cancellation traces

`snippet-variables.json` captures 30 user-command cases (59 observations), including
per-cursor values, selected-text indentation, word lookup, built-in comment
configuration, Escape cancellation, and leaving/reentering a field. Compare with
`cargo run --locked --example snippet_contract -- --variables <output>/snippet-variables.json`.
Native offline tests use the same runner. The [variable baseline](baselines/1.95.0/snippet-variables/provenance.json)
records all three matching platforms from CI run 37859573471; fresh comparisons
continue in CI. Nondeterministic date/random values,
file/URI labels, real clipboard services, and localized language configuration
are not qualified by these traces.

## Provenance

The default document and keyboard diagnostic come from the pinned upstream
[preferences service](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/workbench/services/preferences/browser/preferencesService.ts),
[keybinding service](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/workbench/services/keybinding/browser/keybindingService.ts),
and [inspection action](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/workbench/contrib/codeEditor/browser/inspectKeybindings.ts).
The preserved [upstream MIT notice](LICENSE.vscode.txt) accompanies derived
reference data. Built-in extension manifests supply their separate license
labels in each inventory. See the official
[extension testing guide](https://code.visualstudio.com/api/working-with-extensions/testing-extension)
for the reference runner.

Language-dependent snippet fixtures first probe the reference's comment command
in a separate scratch document. Expected comment tokens come from the pinned
editor's installed language declarations. This bounded readiness check avoids
observing an asynchronously unloaded language configuration. A timeout fails
the suite, and snippet outputs are never retried until they agree with VSCLI.
Run `node --test tests/vscode-reference/language-readiness.test.cjs` after
installing the harness dependencies to check delayed and missing registration.
