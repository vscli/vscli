# Native Outline

Outline shows the active document's namespace, class, method and other symbols
in an Explorer section. Its tree and keyboard controls run in Rust. A ready
native language server can supply symbols without Node; a matching running
extension can supply them through the optional compatibility host.

The section starts disabled. Open the command palette with F1 and choose
**Outline: Focus** to enable and focus it for this session. These original
VS Code command IDs are implemented:

| Palette action | Command |
| --- | --- |
| Outline: Focus | `outline.focus` |
| Outline: Collapse All | `outline.collapse` |
| Outline: Expand All | `outline.expand` |
| Outline: Toggle Follow Cursor | `outline.followCursor` |

No default Outline shortcut is added. User bindings can invoke these commands.
While Outline has focus, Up/Down move between visible nodes, Right expands a
node or moves to its first child, and Left collapses a node or moves to its
parent. Home/End and PageUp/PageDown move within the visible tree. Enter reveals
the selected symbol and returns focus to the editor; Escape returns without
navigating. Tab moves focus to Explorer.

Ordinary reveal collapses the primary caret at `selectionRange.start`, rather
than selecting the identifier or whole declaration. It records an explicit
navigation-history jump when the line changes. Other views keep their own
selections. Revealing a symbol does not save, replace text or add an Undo entry.

Follow Cursor starts enabled. The deepest enclosing symbol is highlighted;
following it expands its ancestors and selects it in the tree. Turning following
off preserves the user's tree selection while the enclosing-symbol highlight
continues to track the editor. Escape leaves the section enabled. Hiding the
sidebar suspends its work. There is one current tree, with no cross-document
cache or persisted Outline state.

## Sources, states and bounds

A matching selected extension provider takes precedence; otherwise a ready native
server advertising document symbols is used. Native LSP requires a saved file
handled by that server. Extension providers can also handle untitled documents.
Hierarchical `DocumentSymbol` results keep their child relationships. Flat
`SymbolInformation` results become independent leaf targets.

Loading, updating, unsupported, empty and error states are explicit. A retained
old tree can remain visible while updating, but it is nonactionable. Enter cannot
reveal from a loading, stale or unsupported tree. An empty successful result is
Ready with no selected node; it does not manufacture a symbol or editor.

Updates debounce for 200 ms. Publication and reveal check document identity,
revision, monotonic text generation, saved generation, resource and exact source
ownership. Edit followed immediately by Undo cannot revive an old reply merely
because the text matches again. The native server and optional host each retain
one actual document-symbol callback lane and one latest intent; Outline shares
the native symbol lane with the symbol picker. Advisory cancellation does not
free actual callback capacity. Unknown timed-out work stays fenced until exact
settlement or process retirement, rather than starting overlapping work.

Responses are bounded to 512 symbols, 16 hierarchy levels, 4 KiB per display
field, 64 KiB cumulative labels and 2 MiB JSON. Invalid child containment,
redirected document-symbol resources, malformed ranges and split UTF-16
surrogate positions reject the result. These checks precede navigation.

## Evidence and qualification

Implementation is present on the Outline feature branch. Local qualification
passed the full Rust run with **583 ordinary tests across 37 suites**; 19 opt-in
tests remained ignored. Formatting and strict all-target Clippy passed. The
optional host's **133 Node tests** passed. These totals include focused subsets;
they are not additional tests to sum together.

The focused Outline evidence includes 14 Rust checks plus two extension
publication regressions for hierarchical file/untitled documents and source
A→B→A retirement. Three debug Unix PTY workflows passed with an empty executable
PATH and an unavailable Node runtime: tree collapse/expand/reveal plus navigation
and exact Unicode/CRLF save/Undo/Redo, pending Enter and edit/Undo stale fencing,
and unsupported-state Escape back to the existing editor. The named actual
`/usr/bin/clangd` test passed for nested C++ namespace/class/method geometry,
UTF-16 identifier-start reveal and dirty-buffer preservation.

The existing 35 terminal workflows also passed against the fresh debug binary.
Optimized terminal workflows, Outline performance measurements and fresh platform CI remain pending. Named
local successes do not establish every provider, project or terminal behavior.

An actual pinned VS Code **1.95.0**, commit
`912bb683695358a54ae0c670461738984cbb5b95`, capture observed two named synthetic
provider cases and 20 public editor snapshots in fresh isolated profiles. It
records provider setup, original command inventory, source/settings hashes and
full selections, and checks original disk bytes. Provider readiness is observed before the API
probe and target commands. Target commands run once, with output-independent
settlement and no expected-result retries.

The hierarchical API output preserves both ranges and children. The public
`vscode.executeDocumentSymbolProvider` command normalizes flat
`SymbolInformation` into leaf `DocumentSymbol` objects with empty detail, equal
range/selection ranges and empty children. Later original `list.select` commands
revealed the collapsed identifier start, including a UTF-16 position following
an emoji. These observations support that narrow provider/reveal contract.

The [recorded Linux trace](../tests/vscode-reference/baselines/1.95.0/outline/linux.json),
[full evidence](../tests/vscode-reference/baselines/1.95.0/outline/linux-evidence.json)
and [provenance](../tests/vscode-reference/baselines/1.95.0/outline/linux-provenance.json)
retain the complete observations. The native `outline_contract` example compares
provider geometry and the two confirmed main/render reveals. Flat container
metadata is excluded because the public API normalization loses it while native
LSP input retains it. Native comparison passed both geometry cases and both
confirmed collapsed reveals, with separate Unicode/CRLF save/Undo/document-ID
and disk checks. All four comparator integrity tests passed: the unmodified
baseline succeeds first, then wrong source, missing original command and invalid
UTF-16 geometry fail at their intended guards even when relevant artifact
digests are updated.

The initial Outline/list sequence did not navigate, even after an observed
provider callback. Its no-op snapshots remain in the raw trace. Public extension
APIs expose no Outline tree focus, selection or collapse getter, so this evidence
does not establish whole-sidebar trace equality, cold-start focus equivalence or
full Outline parity. The synthetic provider is not an unchanged extension-package
qualification. Actual clangd is a separate named workflow.

Breadcrumbs, symbol filtering/sorting menus, provider-group aggregation,
workbench decorations and full persisted view behavior remain outstanding.
The completed [navigation-history slice](NAVIGATION_HISTORY.md) was merged in
[PR #59](https://github.com/vscli/vscli/pull/59) after all six required platform
checks; that evidence does not automatically qualify Outline integration.
