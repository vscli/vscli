# Release focus: bring your VS Code setup

The release objective is a seamless move from an existing VS Code setup to VSCLI:
settings, profiles, visual customizations, snippets, extensions and keybindings.
Complex configurations and interactions remain in scope. The entire stable
extension API remains required; this changes delivery priority, not its scope.
Native Rust editing must continue to work without JavaScript.

## What counts as completion

Import must lead to working editing workflows. Copying files, discovering an
extension, or retaining a setting does not establish that its behavior works.
Track each profile item as preserved, applied, workflow-tested, or unresolved,
with evidence and an explanation. Do not turn these into a guessed overall
compatibility percentage.

The first real acceptance case is the user's existing local VS Code setup. A
raw directory scan found 99 distinct IDs; the authoritative registered profile
index contains 93 version records resolving to 90 selected IDs. The existing
configuration preview produced 115 notices. These are inventory and report
counts, not measured compatibility. Personal configuration contents and package lists
stay outside the repository. Synthetic fixtures exercise the same mechanisms.
The personal profile's fourteen color overrides target cursor/overview-ruler
surfaces outside the current native palette. Native launch and byte-preserving
configuration/package copy have been checked. The 90 selected extensions' runtime
workflows and custom UI styling have not been qualified. The color-customization batch below does not close these gaps.

Release gates:

- [ ] Discover local installations and profiles across supported platforms,
  including alternate channels, portable/custom directories and exported
  `.code-profile` files. Offer a guided import from the welcome screen.
- [ ] Preserve original bytes, comments, unknown fields and source files. Preview
  the complete migration, apply a separate native copy, support retry and rollback,
  and preserve dirty editors when changing profiles.
- [ ] Apply settings precedence, language/workspace overrides, snippets, theme
  includes, theme-specific customizations and editor presentation. Compare visible
  results and interactions, not just parsed JSON.
- [ ] Reconcile every selected extension's exact version, platform, dependencies,
  enabled state and contributed settings/keybindings. Import existing local
  packages where possible. Activation must follow an explicit migration choice;
  installation alone must not start code. Selected code-host limits must not silently truncate a larger imported setup.
- [ ] Qualify the user's extension workflows and cross-extension interactions.
  Keep the complete stable API inventory open until its behavioral contracts are
  met. Evaluate upstream host/converter reuse before adding more custom bridges.
- [ ] Qualify commands, keybinding precedence, full context expressions, chords,
  extension contributions, terminal delivery and diagnostics for conflicts.
- [ ] Evaluate an optional compatibility presentation route for arbitrary webviews,
  custom editors and CSS-driven UI. Terminal colors/styles and terminal-controlled
  fonts cannot by themselves reproduce an arbitrary browser workbench. These
  features remain requirements; they are not silently discarded by import.
- [ ] Qualify installation, first launch, migration, daily editing, restart,
  recovery, update and rollback on Linux, macOS and Windows. Repair failing release
  checks as part of delivery. Publish only after reviewed version changes and the
  matching intentional tag.

Accounts, remote services and secrets need an explicit reconnection/storage
workflow; copying a configuration value must not be described as authenticated
service compatibility.

## First usable batch: imported workbench color customizations

Named acceptance criteria, separate from complete setup migration:

1. Imported global and theme-specific `workbench.colorCustomizations` reach actual
   native editor colors, using the original theme settings ID.
2. Combined selectors, edge wildcards and user/workspace object merging follow
   the named upstream precedence within the mapped native palette.
3. Transparent colors use the final background; `default` uses native built-in
   defaults. Invalid entries retain usable theme colors and report their keys.
4. Changing or removing customization settings and switching themes refreshes
   appearance without cumulative overrides or obsolete async publication.
5. Imported bindings, Unicode/CRLF edit/save/Undo, restart and original-source
   preservation work together without Node.
6. Focused native tests, a real terminal journey, the Rust regression suite,
   formatting and strict locked Clippy pass on the delivered source.

Local result: **6/6 batch criteria passed**, supported by ten new native tests,
1,625 Rust tests across 67 reports (1,276 library; 23 ignored), 35 existing terminal
scenarios and one new imported-appearance journey. Formatting and strict locked
Clippy pass. A temporary copy of the real profile also launches without Node and
preserves original settings/keybindings byte for byte; its extensions were not
activated. [Validation receipts](validation/imported-appearance.json) bind the
source/binary, commands and explicit qualification limits.

Only the existing native palette is covered in this batch. Separate surfaces,
TextMate/semantic token fidelity, fonts/icons/layout, custom CSS, full profile
import and extension migration remain open. Theme files and every other original
setting remain preserved.

## Guided profile and local-package migration

Local batch completion criteria (not full migration acceptance):

1. Discover default/registered named profiles and resource inheritance, with
   custom User/extension directory selection in native review and the CLI.
2. Preserve configuration bytes and reconcile registered package identity,
   selected version, platform metadata, dependencies and source disablement.
3. Snapshot local package files independently and publish configuration, package
   registry, initial execution grants and rollback reference together.
4. Keep code disabled unless explicitly granted; source-disabled code remains
   ungranted. Copy and native editing work without Node.
5. Apply/rollback in the native UI retains shared dirty documents and replaces
   imported shortcuts; canceled/failed work leaves the previous profile active.
6. Exercise native package activation only after an explicit grant, including
   edit/save/Undo and unchanged originals; pass integrity, regression and PTY checks.

Local result: **6/6 batch criteria passed**. The delivered cohort passes 1,642 Rust
tests across 67 reports (1,292 library; 23 ignored), 35 core terminal scenarios,
two migration journeys, the appearance journey and two welcome scenarios.
Formatting, strict locked Clippy and the native build pass. A private copy of
the actual profile contains all **90/90 selected packages**, with matching
selected versions/manifest hashes and byte-exact configuration; native launch,
package listing and Unicode/CRLF edit/save/Undo pass without Node. There are
**zero personal code execution grants**. See [validation receipts](validation/profile-migration.json).

The source application is VS Code 1.118.0 and contains 95 built-in packages.
A new atomic private profile copy contains all **184/184 selected IDs**: 94 selected
built-in IDs plus 90 user IDs, including 113 code packages and one source-disabled
package. All versions, manifest hashes and enabled state match; settings and keys
are byte-exact. Native launch, installed listing and Unicode/CRLF save/Undo pass
without Node, with zero personal code grants. The earlier 90-package generation
is retained. This validates copying and native integration, not those extensions'
workflows. See [built-in migration evidence](validation/builtin-profile-migration.json).

The personal user cohort contains 80 code packages, 1,683 contributed commands, 169
keybindings and 13 packages requesting proposed APIs. These counts are concrete
requirements; they do not establish behavioral compatibility. Source-disabled
native theme/snippet/typing data is now filtered. Other contribution consumers,
workspace override fidelity, extension storage, exported-profile fidelity,
custom path inference, source engine-version resolution, platform-native
binaries and cross-platform UI/release qualification remain open.

## Selected extension cohort capacity

The selected host now admits 128 packages and 4,096 commands within 8 MiB /
250,000 manifest nodes, while callback/service queues retain separate bounds.
Batch completion requires a 90-package synthetic selected session with
more than 1,024 contributed commands, explicit rejection beyond the new package
and manifest-node budgets, unchanged eight-slot callback protections, actual
native command dispatch/edit/save/Undo, restart, full relevant Rust/host checks
and PTY evidence. A synthetic large-cohort session provides this capacity evidence;
it does not establish personal extension compatibility. See
[cohort receipts](validation/extension-cohort.json). Do not activate the personal cohort just to claim that it works;
report engine/proposed-API and missing-backend failures per package.

Local result: **6/6 named cohort criteria pass**, with 1,647 Rust tests across 68
reports (1,295 library; 23 ignored), 798 host tests, one large-cohort terminal
journey, four activation journeys and five session journeys. Formatting and
strict locked Clippy pass. No personal code was activated.

## Next implementation batch

Finish exact-head platform CI qualification before release. Main CI now discovers
all host contract tests from the extension-host directory, instead of maintaining
six selected filename lists. Native Cargo tests use `--no-fail-fast` so one failed
test binary does not hide failures in later binaries; the job still fails. The
new context/complex-condition/default-label terminal journeys run on Unix CI.
Use each run's complete evidence to repair the next platform gap, and retain the
failed receipts. The observed associated-untitled Windows EOL fixture assumptions
have a focused repair; actual Windows results remain required.

Before expanding migration-specific compatibility shims, compare pinned upstream
host, configuration, theme and keybinding resolver reuse against the real profile.
Use that result to choose the execution/presentation route for its extension
cohort. Preserve bounded background work without using package or API limits as a
reason to reduce the migration scope.

## References

[VS Code profiles](https://code.visualstudio.com/docs/configure/profiles) describe
portable profile export/import. [Theme customization](https://code.visualstudio.com/docs/configure/themes)
and pinned [1.95.0 colorThemeData.ts](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/workbench/services/themes/common/colorThemeData.ts)
provide the customization precedence reference. Native presentation differences
and executed test evidence must remain distinct from upstream qualification.


## Source application migration checkpoint

**5/5 local batch criteria pass**: source application selection and version
reconciliation; atomic built-in/user snapshots and disabled metadata; an installed
catalogue larger than the selected-code bound; actual native data/application
picker/dirty-buffer/rollback integration; and required local regression checks.
There are 1,656 passing Rust tests across 69 reports (23 ignored), four relevant
migration/appearance terminal journeys and the private 184-package native journey,
plus formatting and strict Clippy. Entire stable API, personal code execution and
platform/release qualification remain outstanding. Windows journal failures and
macOS reference-witness evidence retain their own CI status and are not counted
as passed by this local checkpoint.

## Imported condition checkpoint

**4/4 local criteria pass**: pinned differential operator evidence (600 exact
evaluations), source-preserving rule diagnostics/priority/removals, actual native
resource and public extension context shortcuts with Unicode/CRLF save/Undo, and
the 1,661-test native regression/format/Clippy cohort. Two new terminal journeys
and the context-key regression journey pass. This adds membership, numeric
comparisons and a bounded regex subset; complete JavaScript regex semantics,
all workbench/config contexts, scoped resolution and physical key delivery remain
requirements. [Validation receipts](validation/native-when-expressions.json).

## Default manifest labels and installed theme capacity

**5/5 local criteria pass** for packaged default label normalization, native
catalogue coverage after 128 packages, translated source-theme/public metadata
integration, actual terminal editing/restart and required regression checks.
The 184-package private copy now discovers the translated built-in Light 2026
theme without Node. Its source inventory contains 121 default bundles and 19
placeholder theme labels; these counts do not qualify every label or color.
There are 1,668 passing Rust and 814 host tests. Preferred-language manifest
bundles, language packs, richer presentation, all code workflows and the entire
stable API remain required. [Evidence](validation/default-manifest-labels.json).

Delivery process checkpoint: all **814 host tests** pass through directory
discovery locally. The new terminal jobs have already passed locally; workflow
syntax parses. This changes failure collection and automatic test discovery,
without relaxing assertions or qualifying another platform.

CI discovery correction: run `node scripts/test_extension_host.cjs`. Local Node 26
accepted a directory argument, but all three Node 24 CI jobs at `7ffc243` rejected
it as a module path before running host tests. The driver discovers explicit
contract filenames and invokes the previously working `node --test <files>`
interface without shell globbing. Failed logs are retained; no test failure is
reclassified as success.

Portable application inference passes **4/4 local criteria**, including three
layout fixtures, explicit missing/ambiguous application selection and an actual
no-Node portable built-in/profile/shortcut/theme/save-Undo terminal journey.
The 1,670-test native cohort, formatting and strict Clippy pass. Actual other-platform
installations, exported profiles, full personal workflows and the whole stable API
remain open. [Evidence](validation/portable-profile.json).

Public editor EOL repair passes **4/4 local criteria**, with 1,671 passing native
tests, 816 host tests, four unchanged pinned Sort Lines controls and a real
Unicode/CRLF edit/save/Undo terminal journey. Complete failure collection exposed
all three Windows native failures in two binaries at `4e7017f`; a public/native
buffer mismatch was reproduced and corrected, while a five-second test watchdog
receives a bounded test-only repair. Fresh Windows results and the entire stable
API remain required. [Evidence](validation/editor-edit-eol.json).

## Exported profile integration

Batch completion criteria:

1. Decode the upstream resource wrappers while preserving configuration bytes,
   null/absent resources, unknown fields and the complete original export.
2. Reconcile requested local IDs, exact pinned versions, release channels, source
   disablement, platforms and dependencies; report every unresolved requirement
   and block package import before destination writes.
3. Integrate CLI and native review through the existing atomic copy/rollback
   pipeline, retaining shared dirty models and explicit execution grants.
4. Qualify native theme/shortcut/edit/save/Undo/restart and rollback without Node,
   plus ordinary directory, built-in and portable migration regression journeys.
5. Pass the relevant native regression suite, formatting and strict locked Clippy;
   retain failures and source/binary evidence separately from platform qualification.

The preceding `0221430` checkpoint passes complete main CI on Linux, macOS and
Windows, including reference and terminal checks:
[completed CI](https://github.com/vscli/vscli/actions/runs/39076446781).
Its optimized Linux archive is a packaging rehearsal, without a tag or publication.
New exported-profile changes require their own platform results. Full stable API,
personal extension workflows, storage/state and visual fidelity remain open.

Local result: **5/5 batch criteria pass**. The final source passes 1,681 Rust tests
across 73 reports (1,320 library; 23 ignored; four threads), formatting and strict
locked Clippy. All six scenarios across the exported, ordinary, built-in and
portable migration terminal scripts pass. The regression journey exposed a clipped
application-picker hint; product help now uses three visible lines and the unchanged
assertion passes. [Source/binary receipts](validation/exported-profile.json).

## Activation preflight and source identity

The `df785f1` main run passes Linux and Windows, while macOS reaches the new
migration journeys and exposes a clipped exported filename under its long temporary
root. Host/native phases pass on all three platforms. The failed observation is
retained; the source filename now has its own visible line, with the full location
retained separately. The same failure reproduces on Linux, and the unchanged
filename assertion now runs beneath a deliberately long parent directory.

A read-only preview of the actual 184-package source reports **30 code packages
requesting proposed APIs: 13 user packages and 17 built-ins**, all source-enabled.
The current host explicitly rejects those requests; none was activated. Migration
review now names these known blockers and bulk activation refuses them before
destination writes. Copy without grants remains available for the full selected
cohort. Source disablement is retained. Empty known-blocker lists do not establish
extension compatibility. The whole stable API, proposed APIs needed by this setup
and every personal workflow remain requirements.

Batch completion criteria: visible source identity under long paths; per-package
known blocker evidence without code execution; pre-publication rejection and
unchanged configuration/dirty buffers; full package copying without grants and
source-disabled behavior; relevant terminal, native, formatting and strict Clippy
checks on a recorded source/binary cohort.

Local result: **5/5 batch criteria pass**, with 1,684 Rust tests across 73 reports
(1,323 library; 23 ignored; four threads), eight terminal scenarios, formatting
and strict locked Clippy. Affected migration terminal journeys ran before the
full native suite. [Source/binary and failed observations](validation/migration-readiness.json).
Fresh complete repair-head platform results remain required.

The `50f60ad` migration repair now passes complete main CI and the dedicated stable
save reference on Linux, macOS and Windows. Its optimized Linux packaging rehearsal
passes 46 terminal scenarios, extracted long-path export and activation-preflight
journeys, archive checksums and embedded revision checks, without a tag or publication.
[Completed platform run](https://github.com/vscli/vscli/actions/runs/39080068658).
Entire stable API, personal extension workflows, richer presentation/storage and
other-platform release artifacts remain open requirements.

## Public editor Undo-stop integration

The `e7d1ef2` Position-replacement checkpoint passes complete Linux/macOS/Windows
CI. The next local Undo-stop batch passes **4/4 named criteria**, including all
four option pairs through actual native command dispatch and shared history,
empty-builder behavior, eleven terminal scenarios and original-byte protection.
There are **1,690 passing Rust tests** across 75 reports (1,327 library; 23 ignored)
and **822 passing host tests**; formatting and strict locked Clippy pass.
The first failing App case exposed an extra command-dispatch history boundary;
its repair is exercised through contributed F9. A prior read-only fixture now
matches upstream's successful no-op behavior while real edits and saves still
refuse. [Source and retained observations](validation/editor-undo-stops.json).
Fresh platform CI for this source, `setEndOfLine` ordering, entire stable API and
seamless personal extension/visual/storage migration remain required.

## Ordered editor EOL integration

The new EOL batch passes **5/5 local criteria**, with **1,701 Rust tests across
76 reports** (1,335 library; 23 ignored), **826 host tests**, four unchanged
Sort Lines controls and seventeen terminal scenarios. Public EOL and text
notifications precede success, default mixed edits retain separate Undo steps,
same-EOL requests preserve Redo, and concurrent plain editor edits cannot overtake
an earlier EOL preparation. Original native source/view/owner proofs and bounded
actual work remain required. [Evidence](validation/editor-set-eol.json).

The preceding `525edcc` main run passes Linux and Windows. Its macOS host/native
and editor Undo phases pass, then installed native configuration testing fails at
disable-picker readiness. The test now waits for the actual versioned package row
and controls, retaining all effect, disk/history and no-code-execution assertions;
four local native-configuration journeys pass. A background status-overwrite
explanation was an initial suspicion and was not established by source inspection.
Fresh exact-head platform CI, personal extension workflows and complete stable
API/visual/storage/release qualification remain open. No release is published.


## Native extension configuration writes

The current settings-write batch passes **5/5 local criteria**: values/removal and
local targets, JSONC/data integrity, original caller/profile/model proofs, actual
native reload/events and terminal behavior, and regression/style checks. The
source passes 1,712 native tests, 832 host tests, four unchanged real-extension
controls and fifteen terminal scenarios. Native language settings survive a
no-Node restart; dirty/shared document history remains intact.
[Source-bound evidence](validation/configuration-update.json) records the limits.
Whole schema/default/scope behavior, immediate clean settings-model reload,
multi-root/remote settings and upstream/platform qualification remain required.

The preceding EOL/picker head `6e61761` now has complete successful Linux, macOS
and Windows [main CI](https://github.com/vscli/vscli/actions/runs/39153168639) and
[dedicated CI](https://github.com/vscli/vscli/actions/runs/39153168576). This is
historical evidence for that head; the configuration-write source needs fresh CI.


Open settings-model integration passes **4/4 local criteria** on the recorded
1,716-test Rust / 832-test host cohort, with twelve terminal scenarios. A real
held pre-write reader cannot acknowledge a newer write; shared/hidden clean
settings refresh before public success, and late dirty edits retain native history
with an explicit already-persisted error. Hardlink resources remain independent.
[Receipts and remaining qualification](validation/configuration-model-refresh.json).


The asynchronous extension system-clipboard bridge passes **5/5 local criteria**
on a 1,719-test Rust / 837-test host cohort, with twelve terminal scenarios. Native
paste shares the same isolated helper data, Unicode/CRLF editing and history
remain correct, and native editing restarts without Node. Personal clipboard
contents were not read or changed. Actual OS/display-server and Windows backend
qualification remain open. [Evidence](validation/clipboard-api.json).

At `4d69f8d`, main CI passed Linux/Windows and failed a macOS save-policy assertion.
Its unchanged captured artifact proves an organize-imports request despite an
unapplied edit. The locally reproduced comparison is repaired without modifying
source captures or output/history oracles. Dedicated Windows capture failed during
its VS Code download before any target began; this is separate from native tests.
Fresh feature-head platform runs are required.


Native Windows clipboard access now has integrated worker-backed read/write and
snippet support, with explicit ownership/allocation guards. **4/5 local criteria**
pass; actual Windows linking/runtime is pending the isolated runner control. The
Linux cohort passes 1,719 Rust tests, 837 host tests and nine terminal scenarios.
The Linux inherited-pipe fixture uses a controlled descendant and explicit release.
The prior macOS library SIGABRT has no established cause; fresh CI captures fatal
output and crash reports. These are separate qualification gates.
[Exact scope/evidence](validation/windows-clipboard.json).


## Isolated Linux archive qualification at `8b6910c`

The extracted optimized Linux archive passes **68 terminal scenarios across 18
scripts**, including migration, appearance, native no-Node editing, snippets,
configuration writes and clipboard integration. Checksums, recorded source revision,
licenses and dependency notices, and an extracted no-Node doctor check pass.
Private synthetic clipboard helpers isolate every journey from personal data.
[Exact archive/binary receipts](validation/linux-release-8b6910c.json) retain the
preceding archive and confirm the existing user release binary is unchanged.
This is evidence for `8b6910c`, not later source or a published release.

The same head passes Linux main CI and dedicated reference CI on all three OSes.
macOS's library suite passes without repeating the preceding unexplained abort;
the watcher integration times out with a later create receipt visible instead of
the requested change receipt. Source inspection shows later unrelated callbacks
can overwrite that status; this is a supported race explanation, not a captured
interleaving. The fixture now pins an actual created resource and explicitly arms
change/delete phases, while still checking/counting all callbacks and preserving
native identity, history, disk and post-disposal silence assertions.

Windows compiles/links the native backend, passes the ordinary native suite, and
successfully writes/reads synthetic Unicode/CRLF clipboard data. Its isolated
control then fails because another read inside the same process succeeds while
the test holds the clipboard. A separately gated child process now holds the
clipboard for the contention assertion. Canceled-write, exact-boundary and empty
controls remain required; actual fresh Windows execution is the pending gate.

The repaired fixtures pass 1,679 standard Cargo-test cases (23 ignored), the
actual watcher terminal journey, formatting and strict all-target Clippy. The
preceding 1,719 all-target count includes additional example-target controls;
these are distinct commands/cohorts. [Test receipts](validation/platform-test-receipts.json)
retain the observed failures and fresh platform gates.


## Native masked extension input

The Input Box password batch passes **5/5 local criteria**: typed/captured intent,
native masked rendering with real editing, raw terminal secrecy and exact values,
original caller/document/history lifetime, and regression/documentation checks.
All 1,720 Rust all-target tests, 840 host tests and four prompt terminal journeys
pass; formatting and strict Clippy pass. The implementation extends the existing
native prompt controller and transports rather than adding another foundation.
[Source/binary evidence](validation/input-box-password.json) keeps full Quick Input,
fresh platform CI and personal authentication/visual/storage migration open.


At `79d76e8`, Windows again passes the ordinary native suite and initial OS
Unicode/CRLF write/read. Its independent null-window holder still allows a read,
so the proposed process-only repair did not satisfy the contention criterion.
The control now holds a real window and verifies the exact
`GetOpenClipboardWindow` receipt before publishing readiness. Nine local shared
clipboard controls, portable metadata checking including this test guard,
formatting and strict Clippy pass. Actual fresh Windows execution remains
required; the batch stays **4/5**, and no backend repair is inferred from a test
change. [Failure and source receipts](validation/windows-window-holder.json).


The extracted optimized Linux archive at `339e5a6` passes **72 terminal scenarios
across 19 scripts**, including the new masked-input/raw-output workflows. Checksum,
revision, notices and no-Node doctor checks pass. Earlier archives and the existing
user release binary remain intact. [Archive and binary receipts](validation/linux-release-339e5a6.json)
record this exact source, isolated clipboard data and pending platform/publication
gates. The uncommitted Window-progress implementation is not in this archive.
