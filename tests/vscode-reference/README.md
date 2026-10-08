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
# Use target/debug/vscli.exe on Windows.
node --test tests/vscode-reference/compare.test.cjs
```

The test extension contributes only fixture settings, with no keybindings. User
extensions, user settings, and the user's workspace are isolated. Built-in
extensions remain enabled and are inventoried with their declared licenses and
versions. Telemetry, updates, and extension auto-updates are disabled. The runner
removes its temporary profile/workspace afterward; downloads remain in ignored
`target/vscode-reference/cache`. The extension suite has a 120-second timeout;
CI additionally bounds each complete platform job to 20 minutes.

## Evidence and boundaries

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
