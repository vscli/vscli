# Importing VS Code configuration and color themes

VSCLI can preview and snapshot a VS Code **User directory** without modifying it.
The report separates copied data from implemented behavior. The native editor and
theme loader do not need Node or execute extension code.

```sh
# Read-only preview; use the User directory containing settings.json and snippets/.
vscli --import-vscode /path/to/Code/User
# Explicitly activate an immutable native snapshot.
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
validation and takes precedence over the active profile.

The selected `workbench.colorTheme` is sought in VS Code extension manifests
under `~/.vscode/extensions` or `--vscode-extensions`. If found, its JSONC file and
relative include chain are copied into the snapshot. They remain usable even if
the source extension is later removed. Includes must stay inside the extension
package. Missing, invalid or unsupported themes are reported; configuration import
can still succeed. Built-in themes shipped inside the VS Code application are not
automatically discovered by this directory scan. Extension executable code,
accounts, sync data and the entire VS Code profile system are not migrated.

All copied files are prepared and synced in a temporary directory, then renamed
into `imports/<uuid>`. A single atomic `active-profile.json` replacement activates
them. Existing native files and previous imported profiles remain intact. A failed
activation can leave an unreferenced completed snapshot; it does not intentionally
replace existing configuration files. Power-loss durability of directory metadata
has not been qualified. An invalid/missing active-profile target produces a notice
and startup falls back to the native configuration directory. `--settings` and
`--keybindings` override their respective imported files.

Limits: 128 snippet files, 4096 inspected directory entries, 1 MiB per configuration
file and 16 MiB of copied configuration/theme bytes. Theme discovery examines at
most 128 extension packages and 128 theme declarations per package. Selected theme
snapshots contain at most eight files and 4 MiB. Limits fail or report incomplete
coverage explicitly. User snippet catalog consumption requires the native snippet
catalog implementation; copying bytes alone does not provide completion snippets.

The snapshot layer preserves selected theme data. Native theme selection and rendering are documented with their UI integration.
