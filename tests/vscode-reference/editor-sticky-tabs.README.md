# Pinned sticky-tab reference

This standalone observer records unchanged VS Code 1.95.0, product commit
`912bb683695358a54ae0c670461738984cbb5b95`. A complete unattended Linux run
captures all 18 cases, 65 target snapshots and 91 setup observations. The full
39-file baseline preserves actual evidence and provenance. Six observer path/event
integrity tests pass. Native replay and App integration are qualified separately;
this actual capture alone does not establish native or cross-platform parity.
The existing eleven-case preview observer and its 86 snapshots remain separate
and unchanged.

The new corpus has 18 named cases, 65 target snapshots including each initial
state, and 91 separately recorded setup operations. Each case uses at most four
groups, eight total tab memberships, eight named files and eight target gestures.
Fixed public `openTextDocument` / `showTextDocument` setup records group, preview
mode and optional scalar selection. Original setup pin/focus commands establish
membership and MRU inputs. Setup is not a retry of a target result.

## Reproduce the reference

```sh
cd tests/vscode-reference
npm ci --ignore-scripts
cd ../..
node tests/vscode-reference/editor-sticky-tabs-run.cjs target/editor-sticky-tabs-reference
```

Linux requires a usable owned display; this runner does not start a display
server. It launches a fresh VS Code process/profile/workspace and empty external
extension directory per case. macOS uses a short `/tmp` session prefix. The
existing supervised process-tree cleanup is reused without modification. The
entire worker is bounded to 240 seconds; operation acknowledgement to five
seconds; public settlement to three seconds. An optional second argument names
one diagnostic case. Such a partial run cannot qualify the full cohort.

Every target executes its original command once. `vscode.open` has the exact
recorded preview option. There is no private group-ID argument, fabricated sticky
API option, preferred-result predicate or target retry. After acknowledgement,
settlement requires 100 ms of unchanged public state. An unexpected no-op is
preserved as an observation; an operation/setup failure preserves the available
partial trace and fails the run.

## Cases and policy

The first eight cases cover historical multi-pin order; first/middle/last
unstick and repetition; right insertion beyond the sticky prefix; preview
replacement; sticky-source splitting; and shared Unicode edits/Undo/Redo.
Five close cases cover same-group and other-group nonsticky MRU, no eligible
target, forced Close and ordinary Close under `never`. Two additional profiles
exercise `mouse` and `keyboard` policy through the original ordinary Close
command. The final three cases exercise sticky-excluding group/all-editor
batches and the clean nonsticky empty-group boundary.

The seven preview/group policies match the existing preview corpus: preview on,
Quick Open/code-navigation preview off, right insertion, recent-editor fallback,
close-empty-groups on and reveal-if-open off. The eighth per-case policy is
`workbench.editor.preventPinnedEditorClose`, explicitly set to
`keyboardAndMouse`, `keyboard`, `mouse` or `never`. Command inventory and actual
effective configuration are checked before setup/targets. Close targets require
every tab and named public document to be clean before execution. The observer
never saves, accepts a dirty dialog or discards unsaved text.

## Evidence and provenance

Snapshots preserve complete ordered public groups/tabs and active, dirty,
preview and `Tab.isPinned` flags; active/visible views; Unicode/CRLF text;
UTF-16/scalar selections; public document-object equality ordinals; versions/EOL;
and each named file's unchanged disk bytes. Actual tab/group/editor/document
events and each fixed setup observation are retained. Supplemental non-file
document-change/close events are retained separately with actual URI/scheme,
language, shared public document identity, dirty/version, exact changes and
readiness/setup/target phase. They are bounded to 256 events, 4 KiB URIs,
64 KiB change text per event and 512 KiB aggregate serialized bytes. Unknown
file events, non-file active/visible editors and non-text tabs still fail.
Public object/version
allocation is evidence, not a proposed native counter value.

Outputs include per-case evidence/provenance, the full target projection,
aggregate evidence and aggregate provenance. Provenance includes pinned product
identity and `product.json` digest, platform/architecture, SHA-256 of the actual
downloaded executable selected for launch, eight executable/harness input hashes,
case/settings hashes, all 144 case/file fixture hashes, exact trace/evidence
digests and per-case policy. Executable hashing streams a bounded regular file
before launch; it identifies the selected launcher, not every extension-host or
renderer binary. README prose is outside the executable-source hash inventory.
Failed boot/setup/operation evidence is diagnostic and is not a completed run.

The first failure and exact eight-file executable/harness source bundle are
preserved under `target/editor-sticky-tabs-first-reference`; the manifest also
hashes the original failure and log. The correction changes event scope rather
than a target outcome: no setup or target observation completed in that attempt.
Three pure recorder tests pass with
`node --test tests/vscode-reference/editor-sticky-tabs-suite.cjs`, without loading
the VS Code API or launching an editor. They cover exact supplemental recording,
strict file-event routing and atomic bounds rejection. A fresh complete capture
of the corrected sources remains required.

A future native consumer must verify the complete ordered eighteen-case cohort,
all inputs/hashes, setup and original gestures before fixture writes. It must
compare every declared supported frame and retain unexpected upstream behavior,
including no-ops and empty-group boundaries. There is no native sticky consumer
or implementation qualification in this preparation.

## Qualification boundary

Public `Tab.isPinned` denotes sticky membership; it is separate from preview
commitment. The proposed cases are based on the pinned
[group model](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/common/editor/editorGroupModel.ts),
[close and pin commands](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/browser/parts/editor/editorCommands.ts),
[batch actions](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/browser/parts/editor/editorActions.ts)
and [close-policy settings](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/browser/workbench.contribution.ts).
Those source facts do not substitute for running this observer.

Graphical click/middle-click and mouse policy are not directly simulated;
`mouse` is only an effective-policy input to an original keyboard-style command.
Explicit command-context targeting, dirty last-membership dialogs, save receipts,
stale asynchronous ownership, drag/move, session restoration and persistence of
sticky/preview modes remain outside this capture. It cannot establish full
VS Code editor-tab or terminal-interface parity.


The final unattended capture is retained locally at
`target/editor-sticky-tabs-qualified-reference`. Its owned display writes stderr
to a regular file so display diagnostics cannot block an undrained pipe. The
first failed event-scope attempt remains separate. A subsequent complete run
needed its blocked display pipe drained and is retained as diagnostic evidence,
not unattended qualification. Neither attempt replaces the final clean capture.
Both completed corrected runs have target trace SHA256
`e8aa4a1ed449ebbf2fd6b7e2910156efb1835cc8d88d19582b8ff2ee9d1175db`.
The final aggregate evidence SHA256 is
`d5960a48a67855eccff640a2e52208b0194b146a44b61be1e10adb3187386843`.

The selected Linux executable SHA256 is
`31a0d92e34790ab32a143d8e68a5886b5680a31e0683b17bdb3dcfeba1a1fd78`.
Its bounded streamed hash covers the selected launcher, not an entire VS Code
installation. Actual macOS and Windows captures remain required for platform
qualification.


## Canonical file identity refresh

The original baseline above remains unchanged. A separate complete unattended
Linux capture from the corrected observer is stored under
`baselines/1.95.0/editor-sticky-tabs-observer-corrected` and retained locally at
`target/editor-sticky-tabs-observer-corrected-reference`. It has all 18 cases,
65 target snapshots and 91 setup observations. Its target trace is byte-for-byte
identical to the original. The observer classifies every bounded file document
through the same strict canonical resource classifier used for tabs and views,
avoiding raw canonical-path string equality. Unknown files remain errors;
supplemental virtual events retain their existing bounds and scope. Three new
pure tests check Windows drive/root case, separators and containment, plus strict
POSIX inventory/case. These are path tests, not an actual Windows editor capture.

The corrected suite SHA256 is
`0821192a8135f054f054ec9eb766b50c5a27418b9fa68ffdc4ae770399f40a35`.
The other seven executable inputs and all case/target definitions are unchanged.
The full fresh native consumer uses this separate source-matched baseline;
original observations are not rewritten to match corrected source.
