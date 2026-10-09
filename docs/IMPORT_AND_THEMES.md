# Importing VS Code configuration and color themes

VSCLI can preview and copy a VS Code **User directory** without modifying it.
The report separates copied data from implemented behavior. The native editor and
theme loader do not need Node or execute extension code.

```sh
# Read-only preview; use the User directory containing settings.json and snippets/.
vscli --import-vscode /path/to/Code/User
# Explicitly activate a versioned native profile copy.
vscli --import-vscode /path/to/Code/User --apply-import
# Point at another VS Code extensions directory for the selected theme.
vscli --import-vscode /path/to/Code/User --vscode-extensions /path/to/extensions --apply-import
```

`--config-dir /path/to/native-config` isolates the destination and subsequent
startup. Without it, the OS-native VSCLI configuration directory is used.
The preview prints JSON with source files, byte count and compatibility notices.
Settings, keybindings and direct `snippets/*.json` / `*.code-snippets` files are
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
automatically discovered by this directory scan. Extension executable code,
accounts, sync data and the entire VS Code profile system are not migrated.

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
