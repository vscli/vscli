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
  installation alone must not start code. The current eight-package selected
  code-host limit must not silently truncate a larger imported setup.
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

The personal cohort contains 80 code packages, 1,683 contributed commands, 169
keybindings and 13 packages requesting proposed APIs. These counts are concrete
requirements; they do not establish behavioral compatibility. Full source-disabled
contribution filtering, extension storage, exported `.code-profile` input,
portable/custom path inference, source engine-version resolution, platform-native
binaries and cross-platform UI/release qualification remain open.

## Next implementation batch

Remove the eight-package selected-host barrier while keeping callback/service
queues bounded. Completion requires a 90-package synthetic selected session with
more than 1,024 contributed commands, explicit rejection beyond the new package
and manifest-node budgets, unchanged eight-slot callback protections, actual
native command dispatch/edit/save/Undo, restart, full relevant Rust/host checks
and PTY evidence. Do not activate the personal cohort just to claim that it works;
report engine/proposed-API and missing-backend failures per package.

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
