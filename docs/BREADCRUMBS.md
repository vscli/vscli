# Native Breadcrumbs

Breadcrumbs adds an active-editor file trail and an enclosing-symbol trail. File
segments work without a language server or JavaScript runtime. Hierarchical
symbols retain actual parents; flat SymbolInformation does not gain a fabricated
container hierarchy. Cursor movement reprojects the current tree without
requesting symbols again.

Use **Focus Breadcrumbs** or **Focus Breadcrumbs and Select** in F1. Original
shortcuts are Ctrl+Shift+Semicolon and Ctrl+Shift+Period on Linux/Windows,
Cmd+Shift+Semicolon and Cmd+Shift+Period on macOS. Focus selects the last crumb.
Left/Right move between crumbs; Enter or Down opens a symbol sibling/root picker.
Space or Ctrl+Enter (Cmd+Enter on macOS) reveals a focused symbol. In the picker,
Up/Down select and Enter reveals the identifier start as one collapsed caret.
Escape returns to the editor without editing. Original IDs include
`breadcrumbs.focus`, `breadcrumbs.focusAndSelect`, `breadcrumbs.focusNext`,
`breadcrumbs.focusPrevious`, `breadcrumbs.selectFocused`,
`breadcrumbs.revealFocused` and `breadcrumbs.selectEditor`.

Native `breadcrumbs.enabled` defaults to `true`; `breadcrumbs.filePath` and
`breadcrumbs.symbolPath` accept `"on"`, `"off"` and `"last"`, defaulting to
`"on"`. User, workspace and language overrides apply. **View: Toggle Breadcrumbs**
(`breadcrumbs.toggle`) now persists the effective user/workspace root setting
in the configured native profile. Dirty settings buffers and winning language
overrides refuse the write with a notice. The header updates while the bounded
background write runs; failure removes that temporary override. See
[settings persistence and its qualification boundaries](SETTINGS_PERSISTENCE.md).

Only saved local files have a file trail. Folder/file dropdowns, directory
navigation, reveal-aside, remote resources, broader symbol filtering/sorting and
complete desktop focus behavior remain outside this slice. Untitled documents
have no fake file path. Labels and path projection are bounded; rendering does
not read directories or request symbols.

Breadcrumbs and Outline consume one current document-symbol publication, with
one actual native/optional-host producer slot. Same-resource symbol labels stay
visible during refresh, but a stale or updating tree cannot supply actionable
symbol crumbs. Unmapped typing and paste while Breadcrumbs has focus leave
editor text unchanged; filtering is explicitly unsupported. Reveals validate document identity, text epoch,
source, publication, selections, workspace and pane before changing the active
selection cohort. Other views, dirty text, Undo and disk remain editor-owned.

## Evidence and outstanding qualification

Actual pinned VS Code 1.95.0, commit
`912bb683695358a54ae0c670461738984cbb5b95`, was observed in four fresh Linux
profiles using a named synthetic document-symbol provider. The warm capture
retains 31 editor snapshots, full selection vectors, callback identity, original
command inventories and exact source/product/artifact hashes. Registration
precedes opening the editor; the first automatic callback and a fixed 1500 ms
interval precede target commands. Each target executes once, without retrying
until a desired selection appears.

The capture confirms three collapsed picker reveals: hierarchical sibling
`reset`, outside-symbol root `main`, and flat-symbol `main`. The hierarchical
sibling journey also records Back/Forward. Direct focus/reveal commands remained
no-ops in this public command setup and are preserved as qualification gaps.
An earlier capture with registration after opening is retained separately;
its early picker no-ops are not presented as successful reveals.

The [recorded Linux trace](../tests/vscode-reference/baselines/1.95.0/breadcrumbs/linux.json),
[full evidence](../tests/vscode-reference/baselines/1.95.0/breadcrumbs/linux-evidence.json)
and [provenance](../tests/vscode-reference/baselines/1.95.0/breadcrumbs/linux-provenance.json)
retain those observations. The standalone observer has its own 120-second
supervisor and uses fresh short-path profiles, including `/tmp` on macOS.
It leaves the existing Outline observer and artifacts unchanged.

The native comparison is deliberately limited to four provider geometries and
those three confirmed picker reveals. VS Code exposes no public Breadcrumbs
trail, focus or picker getter, so the artifact does not establish whole desktop
UI parity or unchanged-extension compatibility. Local qualification passed
603 ordinary Rust tests across 38 suites (20
opt-in tests ignored), all 133 optional-host Node tests, 25 Python checks,
formatting and strict all-target Clippy. The five public native journeys, two
optional-host Breadcrumbs publication checks, and focused held-label, key and
focus integrity checks are subsets of those totals. The four reference integrity
tests also passed, with an accepted unmodified baseline before each malformed
artifact's intended guard.

The native comparison passed all four provider geometries and three confirmed
picker reveals. The named actual `/usr/bin/clangd` workflow passed separately
in 0.23 seconds. All three new debug terminal workflows passed with Node absent
from PATH, including one no-LSP file workflow. The existing 35 terminal workflows
also passed. All three Breadcrumbs terminal workflows also passed with the
optimized executable. The [published core measurements](PERFORMANCE.md#native-breadcrumbs-core-baseline-2026-10-10)
record 30 successful trials and mixed latency observations with file breadcrumbs
enabled and no symbol provider; they establish no fastest-editor ranking or
active-provider latency claim.

[PR #61](https://github.com/vscli/vscli/pull/61) merged after all six required
checks passed on reviewed head `a0b7e8c8f41497c18463ad43569c12f44a43fc3d`.
[The fresh CI run](https://github.com/vscli/vscli/actions/runs/38023849597)
qualified Linux, macOS and Windows tests and pinned provider comparisons, Unix
terminal workflows, quality, commit style and actual protocol servers. The
temporary-workspace fixtures now canonicalize their roots as CLI startup does,
covering macOS `/var` aliases and Windows extended-length prefixes without
relaxing the file-trail or data-integrity assertions.

For a new capture and its narrow native comparison:

```sh
node tests/vscode-reference/breadcrumbs-run.cjs target/breadcrumbs-fresh-reference
cargo run --locked --example breadcrumbs_contract target/breadcrumbs-fresh-reference/breadcrumbs.json
cargo test --locked --example breadcrumbs_contract
```

The native integrity and terminal workflows run separately:

```sh
cargo test --locked --test breadcrumbs
VSCLI_CLANGD=/usr/bin/clangd cargo test --locked --test breadcrumbs actual_clangd_breadcrumbs_enclosing_trail_and_sibling_reveal_preserve_dirty_work -- --ignored
python3 tests/breadcrumbs_pty.py target/debug/vscli
```

The actual clangd workflow is opt-in and uses a dirty Unicode/CRLF C++ buffer,
Breadcrumbs demand before Outline activation, sibling identifier-start reveal,
shared publication reuse and explicit save/Undo/disk checks. The named local
`/usr/bin/clangd` run passed; that single workflow does not qualify every server,
project or provider. Synthetic fixtures and the unchanged executable are
separate evidence scopes.
