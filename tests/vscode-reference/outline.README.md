# Outline observer settlement

The named synthetic provider capture checks public document-symbol geometry and
original Outline/list command reveals against unchanged VS Code 1.95.0. The
native consumer still requires both confirmed collapsed identifier-start reveals;
this correction does not weaken that assertion or change native editor behavior.

Each target command executes exactly once. After acknowledgment, observation uses
an output-independent minimum of 1,000 ms and a full 100 ms quiet window for
public editor/document state and observed provider callback completion. The
existing 3,000 ms settlement deadline and 120-second worker supervision remain
unchanged. A no-op result can settle; the observer never waits for a preferred
caret or retries a target until an expected result appears.

Run the controlled-clock regressions without starting VS Code:

```sh
node --test tests/vscode-reference/outline-settlement.test.cjs
```

The regressions cover delayed 350–500 ms selection events, unchanged no-ops,
callback completion, quiet/deadline boundaries and exactly-once command dispatch.
Fresh capture remains a separate qualification step:

```sh
node tests/vscode-reference/outline-run.cjs target/outline-settlement-reference
cargo run --locked --example outline_contract -- target/outline-settlement-reference/outline.json
```

The earlier failed macOS artifact from run `38037611975`, job `114171228964`,
retains the final `list.select` no-op at `main()` rather than a `render` reveal.
Its source, trace and evidence hashes verified. The frozen Linux baseline and
failed raw artifact are unchanged; neither is rewritten to match this observer.
Fresh captures must carry this observer's actual source hashes before promotion.

This pacing cannot prove which Outline tree holds focus or that all private UI
work has finished. The exact macOS race remains unproven. Pinned
[Outline handling](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/contrib/outline/browser/outlinePane.ts)
performs asynchronous tree creation and delayed reveal, while
[list navigation](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/browser/actions/listCommands.ts)
can return command acknowledgment before its navigation completes. Unchanged
public editor state alone therefore cannot establish private tree/focus readiness.
The correction addresses that settlement gap; it is not a native runtime fix or
a claim of complete sidebar parity. One new Linux capture passed two cases/20
snapshots with both required reveals and no target retries. The fresh native
consumer and four integrity tests pass. Its genuine trio is recorded separately
under `baselines/1.95.0/outline/settlement-1000ms/`; the original baseline remains
unchanged. Fresh platform qualification remains pending.
