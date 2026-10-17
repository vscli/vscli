# Importing VS Code configuration and color themes

VSCLI can preview and copy a VS Code **User directory** without modifying it.
The report separates copied data from implemented behavior. The native editor and
theme loader do not need Node or execute extension code.

```sh
# Discover local installations and named profiles, or preview the default profile.
vscli --list-vscode-profiles
vscli --import-vscode
vscli --import-vscode --vscode-profile 'My Profile'
# Read-only preview; use the User directory containing settings.json and snippets/.
vscli --import-vscode /path/to/Code/User
# Explicitly activate a versioned native profile copy.
vscli --import-vscode /path/to/Code/User --apply-import
# Point at another VS Code extensions directory for the selected theme.
vscli --import-vscode /path/to/Code/User --vscode-extensions /path/to/extensions --apply-import
```

The welcome screen's **Import from VS Code** action and
**F1 → Preferences: Import from VS Code** provide native profile selection and
review. **P** chooses a custom User directory; **E** chooses its extension folder;
**B** chooses the source application and its built-in packages.
In review, **I** includes local packages and **A** explicitly allows source-enabled
code packages to activate. **Enter** applies. Configuration loads in the current
window without replacing open models or discarding unsaved edits. Escape cancels
preparation; a completed atomic activation is still reported if cancellation
arrives after commit. Workers retain their slot until they actually finish.

`--config-dir /path/to/native-config` isolates the destination and subsequent
startup. Without it, the OS-native VSCLI configuration directory is used.
The preview prints JSON with source files, byte count and compatibility notices.
Settings, keybindings, user tasks and direct `snippets/*.json` / `*.code-snippets` files are
copied byte-for-byte, including comments and unknown values. Invalid JSONC or
invalid top-level types fail before activation. Unsupported keybinding expressions
remain in the copied file and are skipped by the native imported-profile loader;
the report names them. Unknown command bindings are preserved but do not acquire
an implementation. Explicit `--keybindings` keeps the existing strict import
validation and takes precedence over the active profile. Skipped imported binding
notices remain available in **Settings: Compatibility Report**. If an automatically
loaded copied keybinding file becomes malformed or oversized, startup retains the
native defaults and reports the failure without rewriting the file.

The selected `workbench.colorTheme` is sought in VS Code extension manifests
under `~/.vscode/extensions` or `--vscode-extensions`. If found, its JSONC file and
relative include chain are copied into the copied profile. They remain usable even if
the source extension is later removed. Includes must stay inside the extension
package. Absolute includes and symlink aliases are reported instead of being copied
with broken references; relative include files retain their original bytes. Theme imports and automatic
imported-keybinding loads reject nonregular files before opening them. Missing, invalid or unsupported themes are reported; configuration import
can still succeed. Built-in themes shipped inside the VS Code application are not
automatically discovered by this directory scan. Registered named profiles and
per-resource inheritance flags are supported. User `tasks.json` is preserved;
native task execution still reads workspace `.vscode/tasks.json`. Accounts,
Settings Sync, complete profile management and extension storage migration remain
outstanding.

All copied files are prepared and synced in a temporary directory, then renamed
into `imports/<uuid>`. A single atomic `active-profile.json` replacement activates
them. Existing native files and previous imported profiles remain intact. Imported
profiles are versioned copies, not immutable archives: native settings and theme
preferences may edit the active copy while the original VS Code files stay unchanged. A failed
activation can leave an unreferenced completed profile copy; it does not intentionally
replace existing configuration files. Power-loss durability of directory metadata
has not been qualified. An invalid/missing active-profile target produces a notice
and startup falls back to the native configuration directory. Pointer files and
managed imports/profile directories may not be symlinks. Activation resolves its
destination to an absolute path and refuses destinations inside the original User
directory; imported theme paths remain valid when the working directory changes. `--settings` and
`--keybindings` override their respective imported files.

Limits: 128 snippet files, 4096 inspected directory entries, 1 MiB per configuration
file and 16 MiB of copied configuration/theme bytes. Theme discovery examines at
most 128 extension packages and 128 theme declarations per package. Selected theme
snapshots contain at most eight files and 4 MiB. Limits fail or report incomplete
coverage explicitly. Imported user snippets are available through **Insert Snippet**;
copying bytes alone does not provide completion snippets.

## Local extension packages and rollback

Add `--import-extensions` to copy registered local packages together with the
configuration. Add `--activate-imported-extensions` with `--apply-import` to grant
source-enabled code packages permission to activate. Without that grant,
installation does not start code. Source-disabled code packages remain ungranted.

```sh
vscli --import-vscode --import-extensions --apply-import
vscli --import-vscode --import-extensions --activate-imported-extensions --apply-import
vscli --rollback-import
```

The registered profile index supplies identity, version and paths. When it holds
multiple versions, migration selects the highest registered version for this
platform and reports the reconciliation. Original version metadata remains in
`extension-migration.json`. Source application engine-version selection and
cross-platform native binaries need further qualification. Unregistered folders
are reported rather than silently installed. No registry download is required.

Disabled identifiers are read from a private snapshot of the profile's SQLite
database and WAL. The importer queries extension disablement, not account or
secret records, and creates no SQLite sidecars in the original User directory.
The source package manifest must still match its preview. Packages are streamed
into independent native generations; executable modes and confined relative links
are retained. Special files and escaping/absolute links fail the import.

A profile's settings, packages, initial execution grants and rollback reference
are staged before its active pointer is atomically replaced. A failed or canceled
copy leaves the previous active profile intact. Native startup and CLI package
operations select the imported package store unless `--extensions-dir` overrides
it. Rollback selects the preceding profile without deleting either generation,
including modified imported settings. Existing native settings are restored when
rolling back the first import. Earlier copies made before rollback metadata was
implemented cannot automatically roll back.

Current local-copy bounds are 2,048 packages, 16 GiB total bytes, 2 GiB per file,
one million entries and 64 MiB of path metadata; directory nesting is bounded at
64 levels. State database/WAL snapshots each allow 256 MiB. These are explicit
errors, not truncation. Installation does not qualify a package's runtime API,
services, complete disabled declarative-contribution behavior or visual fidelity.
The selected code host admits up to 128 packages, 4,096 commands and 250,000
manifest JSON nodes within its 8 MiB aggregate manifest budget. Callback/service
queues keep separate bounds. Personal runtime workflows and remaining APIs are
still release gaps.

## Personal color customizations

Imported `workbench.colorCustomizations` now applies to the native palette below.
Global values are followed by matching theme blocks such as `[My Theme]`,
`[One][Two]` and `[*Dark*]`; wildcards work at the edges of theme names.
User/workspace objects merge before matching the active theme's settings ID.
Theme-specific values override global values, including workspace globals.

RGBA values blend against the final customized editor background. `"default"`
selects the native built-in light/dark default for the mapped color. Invalid
entries retain the theme color and produce a notice. These built-in defaults and
shared native surfaces are approximations, not the complete VS Code color registry.

Changes and removals reload in the background, and switching themes recalculates
customizations from the original theme. Stale appearance replies cannot replace
newer settings. Source theme/configuration files are never rewritten to apply
colors. F1 → Preferences: Color Theme Report lists unmapped surfaces.

The [migration release plan](MIGRATION_RELEASE_PLAN.md) retains full profile,
appearance, keybinding and extension migration requirements beyond this slice.

## Native color themes

Use **F1 → Preferences: Color Theme** for built-in dark/light palettes and installed
extension color themes. The picker refreshes installed contributions without
activating their extensions. **Preferences: Load Color Theme File** accepts a
VS Code JSON/JSONC theme path; `--theme /path/to/theme.json` overrides the saved
selection at startup. **Preferences: Color Theme Report** displays known limits
for the loaded file. Selections persist atomically in `theme-selection.json`
beside the active settings profile. Installed theme rows include their package and
relative path; that identity persists even when labels collide or a package changes
installation directories. Custom file paths are saved as absolute paths. A failed load leaves the current theme and
unsaved documents unchanged. Installed-package includes are confined to their
package; a broken extension registry reports a discovery failure while explicit
file and built-in theme selections remain usable. The priority is explicit `--theme`, saved native
selection, then `workbench.colorTheme`. Theme settings are resolved at startup;
use the picker to change themes during a session.

Workbench colors currently map as follows:

| VS Code color | Native surface |
| --- | --- |
| `editor.background`, `editor.foreground` | Editor background/default text |
| `editor.selectionBackground` | Selection and shared selected-item background |
| `editor.lineHighlightBackground` | Current editor line |
| `sideBar.background`, `editorWidget.background` | Shared panels/popups |
| `editorLineNumber.foreground` | Line numbers and muted UI text |
| `focusBorder`, `textLink.foreground` | Shared accent/focus color |

Hex RGB/RGBA colors are accepted; alpha blends against the editor background.
Relative `include` files are supported with cycle/read limits. Syntax foreground
rules map common TextMate selectors onto 14 native Tree-sitter/fallback categories.
This is approximate: TextMate grammar scope stacks, selector specificity, semantic
token colors, token font styles, `.tmTheme` files, icons, per-surface workbench
colors and live file watching are not fully implemented. Unmapped color keys,
font styles and semantic-token declarations produce notices instead of a claim of
full fidelity. Built-in fallback palettes fill unspecified colors.

The format follows the [official VS Code color-theme guide](https://code.visualstudio.com/api/extension-guides/color-theme)
and [color reference](https://code.visualstudio.com/api/references/theme-color).
Native unit/integration tests verify includes, alpha colors, mapped syntax cells,
compatibility notices, failed-load retention, profile activation, original bytes,
and selected-theme snapshots after source removal. PTY workflows exercise actual
imported settings/bindings, custom RGB output, theme selection, restart persistence
and failed-load behavior. These are native behavior tests, not differential proof
of VS Code theme parity.


## Source application and built-in packages

Local discovery also inspects the corresponding application without running it.
If several installations match, choose the intended executable, installation
folder or `resources/app` directory with **B** on the profile picker, or pass:

```sh
vscli --import-vscode --vscode-application /path/to/Code --import-extensions --apply-import
```

The preview counts registered user versions, source built-ins and selected IDs
separately. The combined inventory keeps the newest available version of a
duplicate ID and prefers a built-in on an equal version. Source engine validity
and native binaries still need qualification. A configuration-only import can
snapshot a selected built-in theme without starting its code.

Copying retains source enablement independently of execution consent. Native
snippets, themes and typing configurations skip source-disabled packages;
source-enabled data remains usable without granting its package's code access.
The installed metadata catalogue admits 2,048 packages within its existing byte
and JSON-node limits. Selected code sessions remain bounded at 128 packages with
separately bounded callback queues. An explicit grant cohort beyond its supported
size is refused before payload copying, preserving the active profile.

Local qualification covers a synthetic application and a private copy of all
184 selected IDs from the actual setup: 95 source built-ins yield 94 selected
built-in IDs alongside 90 selected user IDs. Versions, manifest hashes, enabled
state and configuration bytes match; native launch, installed listing and
Unicode/CRLF save/Undo work without Node. All 113 code packages remain ungranted.
This does not qualify their execution, all declarative contribution kinds,
workspace enablement overrides across every consumer, native language packs,
TextMate grammars, fonts/icons, arbitrary CSS or the entire stable API. See
[built-in migration receipts](validation/builtin-profile-migration.json).


An explicitly supplied unpacked theme directory still works for configuration-only
imports when no authoritative user extension index exists. Only its selected
contained JSON theme/include data is copied; its code remains unregistered.
