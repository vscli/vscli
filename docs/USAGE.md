# Running and testing VSCLI

VSCLI 0.1 is a usable native terminal text editor. It now includes native editing, workspace search, language tooling, terminals, tasks, and Git workflows. It is an alpha with a deliberately explicit feature boundary, not full VS Code compatibility.

## Launch

Build with a current stable Rust toolchain (validated here with Rust 1.99):

```sh
cargo build --release --locked
./target/release/vscli .
./target/release/vscli src/main.rs README.md
./target/release/vscli --workspace /path/to/project /path/to/project/file.rs
```

Optional installation into your Cargo bin directory:

```sh
cargo install --path . --locked
vscli .
```

An existing directory argument selects the workspace. A nonexistent file argument creates a buffer for that path, provided its parent directory exists. Creating a buffer does not write the file until Save. Without file arguments the editor opens the workspace with no document or untitled buffer. The native welcome screen shows the VSCLI logo and shortcuts for New File, Open File, Quick Open and the command palette. Closing the last editor returns to this screen. Ctrl+N (Cmd+N on macOS) explicitly creates an untitled buffer; typing or pasting into the empty welcome screen does not create one. Unsaved recovery buffers still reopen when present. Explorer, workspace search, Git, tasks and the terminal remain available without an open file.

Useful options:

| Option | Purpose |
| --- | --- |
| `--keymap linux`, `windows`, or `macos` | Select the keyboard profile, independently of the host OS |
| `--keybindings path/to/keybindings.json` | Import supported VS Code user rules |
| `--doctor` | Print environment, configuration paths, and feature diagnostics |
| `--list-keybindings` | Export this alpha's implemented default rule set as JSON |
| `--legacy-keys` | Skip enhanced keyboard negotiation for troubleshooting |
| `--no-mouse` | Keep mouse handling with the terminal |
| `--no-recovery` | Disable recovery snapshots and startup restoration |
| `--recovery-dir /path` | Override recovery storage, including for isolated testing |

## Implemented workflows

New documents and files without line breaks use CRLF on Windows and LF on Unix. Existing LF/CRLF content determines the document's line ending for subsequent insertion; opening does not rewrite file bytes. Mixed-line-ending selection and `files.eol` settings migration remain unqualified.

The editor supports UTF-8 file open/save and Save As, multiple tabs, ordinary text entry, grapheme-aware horizontal movement, visual-column vertical movement, keyboard/mouse selection, undo/redo, indentation, comments, line deletion, literal find, replace all, and go to line.

Quick open indexes up to 100,000 workspace files in a background worker. It respects ignore rules and skips `.git`, `target`, `node_modules`, `.venv`, and `__pycache__`. The explorer shows the current directory; Right opens a directory or file (Enter also opens in Linux/Windows profiles), Left/Backspace goes to its parent, and Escape returns focus to the editor. macOS uses Enter for rename. Native file notifications refresh the index after a short debounce; Refresh Explorer is available when notifications are unavailable. See [external file changes](#external-file-changes) for qualification limits.

The command palette lists implemented actions. Background Tree-sitter highlighting covers C/C++, Rust, Python, JavaScript/JSX, TypeScript/TSX, and JSON, including multiline constructs. C/C++ use native bundled grammars without Node or a language server; C++ templates, preprocessor directives, multiline raw strings and Unicode comments map to native theme categories. Headers (`.h`, `.hpp` and related suffixes), module files and `.h.in`/`.hpp.in` templates select C++ highlighting. CUDA grammar, semantic tokens and exact TextMate scopes remain unsupported. Grammar work retains the 2 MiB document limit, cancellation and revision checks; larger files use existing fallback highlighting. Tabs switch using the profile's next/previous editor shortcuts. Multiple cursors support typing, deletion, indentation, comments, selection movement, clipboard operations, and undo. Up to four editor groups can share documents with independent cursors and scroll positions.

Explorer commands in F1 create files/folders, rename the selected item, move it to system trash, and refresh the index. F2 renames in Linux/Windows, while Enter renames in macOS. Delete moves the selected item to trash in Linux/Windows; Cmd+Backspace does so in macOS. Trash always asks for confirmation and never falls back to permanent deletion. Use the OS trash interface to restore items. Open buffers under a trashed path are retained as unsaved copies.

File operations run in a worker and refresh the explorer/index afterward. Saving waits while an operation is pending. Create operations require an existing parent directory and refuse existing names. Rename preserves open unsaved buffers and refuses an existing destination; the initial portable rename implementation still has a check/rename race against concurrent filesystem writers. Native watching and undoable clean-buffer reload are implemented; dirty buffers retain their unsaved contents. See [external file changes](#external-file-changes) for limits.

## Keyboard shortcuts

Default combinations for implemented actions follow the [VS Code shortcut reference](https://code.visualstudio.com/docs/reference/default-keybindings). This is a manually implemented subset, not a complete exported upstream baseline or a claim of identical command semantics in every edge case. The broader exact-parity requirement remains in [Compatibility](COMPATIBILITY.md).

| Action | Linux / Windows | macOS |
| --- | --- | --- |
| Command palette | Ctrl+Shift+P or F1 | Cmd+Shift+P or F1 |
| Quick open | Ctrl+P | Cmd+P |
| Open file | Ctrl+O | Cmd+O |
| New buffer | Ctrl+N | Cmd+N |
| Save / Save As | Ctrl+S / Ctrl+Shift+S | Cmd+S / Cmd+Shift+S |
| Close buffer | Ctrl+W | Cmd+W |
| Close window | Ctrl+Shift+W | Cmd+Shift+W |
| Undo | Ctrl+Z | Cmd+Z |
| Redo | Ctrl+Y; also Ctrl+Shift+Z on Linux | Cmd+Shift+Z |
| Select all | Ctrl+A | Cmd+A |
| Copy / cut / paste | Ctrl+C / Ctrl+X / Ctrl+V | Cmd+C / Cmd+X / Cmd+V |
| Find | Ctrl+F | Cmd+F |
| Next / previous match | F3 / Shift+F3 | Cmd+G / Cmd+Shift+G |
| Open replacement workflow | Ctrl+H | Cmd+Alt+F |
| Go to line | Ctrl+G | Ctrl+G |
| Select current line | Ctrl+L | Cmd+L |
| Delete line | Ctrl+Shift+K | Cmd+Shift+K |
| Toggle line comment | Ctrl+/ | Cmd+/ |
| Add / remove line comment | Ctrl+K Ctrl+C / Ctrl+K Ctrl+U | Cmd+K Cmd+C / Cmd+K Cmd+U |
| Indent / outdent | Ctrl+] / Ctrl+[; Tab / Shift+Tab | Cmd+] / Cmd+[; Tab / Shift+Tab |
| Toggle explorer | Ctrl+B | Cmd+B |
| Focus explorer | Ctrl+Shift+E | Cmd+Shift+E |
| Next / previous tab | Ctrl+PageDown / Ctrl+PageUp | Cmd+Alt+Right / Cmd+Alt+Left |
| Keyboard shortcuts | Ctrl+K Ctrl+S | Cmd+K Cmd+S |
| Close all editors | Ctrl+K Ctrl+W | Cmd+K Cmd+W |

Shift plus navigation extends a selection. Ctrl+Left/Right moves by words on Linux/Windows; Alt+Left/Right does so in the macOS profile. Home toggles indentation/start of line; End moves to line end. Ctrl+Home/End moves to file boundaries on Linux/Windows; Cmd+Up/Down is available on macOS. Chords wait for the second key; Escape cancels.

Exact physical key delivery depends on terminal configuration. VSCLI negotiates enhanced keyboard reporting when supported. A terminal may otherwise turn Ctrl+Shift+P into Ctrl+P or consume the combination entirely. The editor does not silently replace that binding. Use F1 → Keyboard Inspector, release the conflicting terminal binding, and retest. No terminal configuration is changed automatically. OS-global, international-layout, and multiplexer behavior still require real-device qualification.

## Snippets

Literal templates work through `editor.action.insertSnippet` with `args.snippet`.
For example, save this **user override** in a keybindings file and load it with
`vscli --keybindings ./keybindings.json file.rs`:

```json
[
  {
    "key": "f6",
    "command": "editor.action.insertSnippet",
    "when": "editorTextFocus",
    "args": {"snippet": "fn ${1:name}(${2:args}) {\n\t$0\n}"}
  }
]
```

F6 then inserts that template. Tab and Shift+Tab move between active placeholders;
Escape or Shift+Escape ends the session while keeping the primary selection.
Linked fields edit together. Native variables include document/selection/cursor
values, file/workspace paths, dates, random values, clipboard text, and built-in
comment tokens. Multiline insertion follows the document's indentation and EOLs.
Clipboard reads run in the background and cancel insertion if the editor context
changes before the reply arrives.

F1 → **Insert Snippet** opens a searchable native picker. Type a name, prefix or
description, then press Enter; Escape cancels. Open or create an editor first;
invoking insertion on the welcome screen leaves it empty. Catalogs load in the
background when the command runs, and closing the target editor cancels pending
insertion. User files live in `snippets/` beside the active user
settings file (`--settings` can select a VS Code user settings file). Put
language-specific snippets in `<language>.json`, global snippets in
`*.code-snippets`, and project snippets in `.vscode/*.code-snippets`.
Comments and trailing commas, string/array bodies, prefix arrays, descriptions,
comma-separated scopes and no-prefix entries are supported. For example:

```jsonc
{
  "Log value": {
    "scope": "javascript,typescript",
    "prefix": ["log", "print"],
    "body": "console.log(${1:value});$0",
    "description": "Log a value"
  }
}
```

A binding can use `"args": {"name": "Log value", "langId": "javascript"}`
for direct insertion; omit `langId` to use the current document language.
Catalog edits are picked up on the next invocation. Replies and picker acceptance
are checked against the original document revision, cursor selections, view and
language so changed work is not overwritten. Malformed or oversized files show
catalog warnings while valid files remain available.

Installed VSIX packages also supply `contributes.snippets` to the same picker and
named lookup without Node or code activation. Install a local package with
**Extensions: Install from VSIX** or `--install-extension`; `--extensions-dir`
selects its store. Language contributions follow the document language (or
`langId`); global `.code-snippets` contributions use body scopes. Upgrades,
rollback and removal are reflected on the next invocation. Contributed paths
must remain inside the package. User, workspace and installed files share the
128-file, 4096-snippet and 16 MiB read limits; warnings leave valid catalogs usable.

Choice menus, nested snippet merging, extension API insertion and completion
snippets are still unfinished. Unsupported regex
constructs fail explicitly. See [snippet evidence and limits](SNIPPETS.md).

## Multiple cursors and line commands

Disjoint selections retain their primary and secondary order through normalization, typing, and undo/redo. Overlapping ranges merge while retaining the earliest selection's direction.

Ctrl+D (Cmd+D on macOS) selects the word, then adds the next occurrence. Ctrl+Shift+L / Cmd+Shift+L selects every occurrence. Escape collapses to the primary cursor; Ctrl+U / Cmd+U undoes cursor changes. Alt-click adds a cursor. Shift+Alt+I places cursors at selected line ends.

Vertical cursor shortcuts differ by platform: Linux uses Shift+Alt+Up/Down, Windows uses Ctrl+Alt+Up/Down, and macOS uses Cmd+Alt+Up/Down. Line duplication uses Ctrl+Shift+Alt+Up/Down on Linux and Shift+Alt+Up/Down on Windows/macOS. Alt+Up/Down moves the selected line block. Ctrl+Enter / Ctrl+Shift+Enter inserts a line below/above (Cmd on macOS). Bracket navigation uses Ctrl+Shift+\ / Cmd+Shift+\.

Line move/copy commands preserve disjoint selections and undo together. Cursor creation is limited to 10,000 selections. Bracket matching is textual and does not yet exclude strings/comments. These are recorded parity gaps, not claims of complete VS Code editing semantics.

## Split editors

Ctrl+\ / Cmd+\ splits the editor to the right. F1 → View: Split Editor Down creates a vertical layout. Ctrl+1 through Ctrl+4 (Cmd on macOS) focus existing groups; clicking an editor area focuses it. Each group has independent cursor/selection and scroll state, while edits, undo history, save state, and language synchronization belong to the shared document. Closing one of several views of a dirty document retains the buffer; closing its last view still asks about saving.

This first implementation supports up to four equal-sized groups in one horizontal or vertical layout. Nested/resizable groups, separate tab stacks per group, and restoring pane layout after restart remain incomplete. Undo position changes are tracked across views; inactive viewport rows are not yet anchored to text across line insertions.

## Workspace search

Ctrl+Shift+F / Cmd+Shift+F opens Find in Files. Alt+C toggles case sensitivity, Alt+W whole words, and Alt+R regular expressions. Enter starts a background search; arrows choose a result, Enter opens its selection, and Escape closes/cancels. Results use unsaved buffers for open files. A changed result is reported instead of selecting an outdated match.

Search respects ignore files and the same excluded directories as quick open. It scans at most 100,000 file paths, skips binary/non-UTF-8/oversized/unreadable files, and returns at most 5,000 matches. The result header reports skipped files and truncation. Search is line-based; multiline regex and workspace replacement are not implemented yet. Results are snapshots, not live subscriptions.

## Language servers

Start an explicitly selected language server; no project-supplied executable is launched automatically:

```sh
vscli --lsp rust-analyzer --lsp-language rust .
vscli --lsp clangd --lsp-language cpp src/main.cpp
# Repeat --lsp-arg for individual process arguments:
vscli --lsp clangd --lsp-language c --lsp-arg=--background-index=false example.c
```

The server must already be installed. This implementation uses native stdio JSON-RPC with bounded transport queues, UTF-16 positions, document versions, and stale-response checks. Diagnostics appear as gutter markers and in Problems. Server failure leaves editing available. Incoming and outgoing serialized JSON frames are each capped at 16 MiB. Oversized document synchronization or tooling requests reject language-server work while retaining native buffers; the editor’s 32 MiB file-opening limit does not qualify every such file for LSP.

| Action | Linux | Windows | macOS |
| --- | --- | --- | --- |
| Completion | Ctrl+Space | Ctrl+Space | Ctrl+Space |
| Hover | Ctrl+K Ctrl+I | Ctrl+K Ctrl+I | Cmd+K Cmd+I |
| Definition | F12 | F12 | F12 |
| References | Shift+F12 | Shift+F12 | Shift+F12 |
| Format document | Ctrl+Shift+I | Shift+Alt+F | Shift+Alt+F |
| Rename symbol | F2 | F2 | F2 |
| Problems | Ctrl+Shift+M | Ctrl+Shift+M | Cmd+Shift+M |

Completion uses arrows and Enter/Tab. Formatting is undoable. Rename currently accepts unversioned text edits, stages validation before changing any buffer, and leaves files unsaved for review; undo is per file. It refuses edits to other unsaved buffers. Versioned rename edits, file operations, completion snippets/resolve/follow-up commands, server provisioning, automatic restart, multiple simultaneous servers, and advanced capability negotiation remain incomplete. Native code-action support has its own stricter transaction rules below. The server is terminated when the editor exits; graceful shutdown is still pending. Diagnostics lacking server versions have weaker stale-result guarantees.

A deterministic subprocess fixture tests synchronization, completion, formatting, and rejection of a stale response. The local real-server integration test covers clangd diagnostics, hover, and formatting. This does not establish compatibility with every server.

## Integrated terminal and tasks

Ctrl+` toggles the terminal panel; Ctrl+Shift+` creates a terminal, and Ctrl+1 (Cmd+1 on macOS) focuses the editor. F1 exposes next/previous/kill terminal commands. There can be up to eight sessions. Hiding the panel leaves the process running. Shift+PageUp/PageDown or the mouse wheel scrolls its 5,000-line history.

Shells run through a native PTY; a VT parser renders colors, attributes, cursor movement, alternate screens, and bracketed paste. Input and output use bounded queues. Ctrl+C reaches the shell, while palette/quick-open/panel/focus shortcuts remain available to the workbench. This is a documented initial routing subset, not the complete VS Code `commandsToSkipShell` behavior. Shell selection uses `SHELL` (Unix) or `COMSPEC` (Windows). The terminal is not yet qualified for every full-screen program, mouse-reporting mode, keyboard enhancement, or Windows ConPTY workflow. Pasting more than 1 MiB is refused. Exiting VSCLI terminates its terminal sessions; persistence/reconnection is not implemented.

When the terminal writer is busy, accepted input is coalesced and retried in order without blocking the editor. The queue holds one batch plus up to 1 MiB pending, with at most one additional batch being written. If the pending byte budget is exhausted, the next input batch is refused with a status message; already accepted bytes remain queued. Short key bursts therefore do not depend on the writer being scheduled between keystrokes.

Tasks are read from `.vscode/tasks.json` version `2.0.0`. F1 → Tasks: Run Task opens the picker; Ctrl+Shift+B / Cmd+Shift+B selects the default build task. The first execution previews the exact prepared command and asks to trust workspace tasks for this session. No project task starts automatically. Output and input use the integrated terminal.

Process and POSIX shell tasks support string arguments, a working directory, environment overrides, OS task overrides, and common `${workspaceFolder}`, `${file}`, `${relativeFile}`, `${fileBasename}`, `${fileDirname}`, `${lineNumber}`, `${selectedText}`, and `${env:NAME}` variables. With a process task, each argument remains a separate argument. For shell tasks with `args`, each value is POSIX-quoted; a command with no args can be a pipeline. Shell overrides must use compatible quoting. Windows shell tasks remain unavailable; use process tasks.

Task dependencies, background readiness, extension providers, automatic task detection, inputs/command variables, and problem matchers are not implemented yet. Unsupported dependencies/variables/providers fail explicitly; a configured problem matcher produces an explicit notice while task output remains available. Tasks run saved filesystem contents; they do not automatically save dirty editor buffers.

## Git

Ctrl+Shift+G opens Source Control. Arrows select a file; Enter opens it, S stages it, U unstages it, D shows its working-tree diff, Shift+D shows its staged diff, R refreshes, and C enters a commit message. A commit applies to the current Git index, including entries staged outside VSCLI, and runs configured Git hooks. F1 → Git: History shows the latest 100 commits. Diff/history viewers support arrows, PageUp/PageDown, Home/End, and Escape.

Git operations run in a background worker using literal path arguments and NUL-delimited status records. No push, pull, reset of working-tree contents, or automatic repository initialization occurs. Staging refuses a selected file that has unsaved editor changes. Diffs and status describe saved files. Unstaging preserves working-tree contents, including in a repository without its first commit. The current backend requires the `git` executable and limits output to 8 MiB per stream and operations to 30 seconds.

Git status currently refreshes on request and after mutations. Hunk staging, conflict-resolution UI, branch management, remote operations, interactive signing/authentication, and advanced repository/submodule handling remain incomplete. Long-running commit hooks may exceed the current timeout; inspect repository state after any timed-out write operation.

## Custom bindings

The default configuration path is printed by `--doctor`. The file is optional. A supplied `--keybindings` path takes precedence. JSON comments and trailing commas are accepted. Settings, keybindings and task paths must resolve to regular files; directories, devices and FIFOs are rejected before opening. Symlinks to regular files remain supported. Each file has a 1 MiB read limit; oversized files are rejected without reading the remainder. A failed keybinding import preserves previously loaded rules. Configuration nesting is limited to 64 levels before parsing; deeper settings, bindings, tasks and snippet files produce a diagnostic.

Each `when` expression is limited to 8 KiB, 1,024 tokens and 64 nested groups or negations. These limits apply to user and extension rules before they become active. Rejected user imports retain the working keymap; copied-profile imports report skipped invalid rules individually. This bounds parsing work and recursion; it does not add unsupported context operators.

```jsonc
[
  // This explicit customization inserts text when the editor has focus.
  {
    "key": "ctrl+k ctrl+b",
    "command": "type",
    "args": { "text": "hello" },
    "when": "editorTextFocus && !inputFocus"
  }
]
```

Rules resolve from bottom to top. Chords, command arguments, empty commands, and `-command.id` removal rules are supported. Available contexts include `editorTextFocus`, `editorFocus`, `editorHasSelection`, `inputFocus`, `filesExplorerFocus`, `editorLangId`, `isLinux`, `isMac`, and `isWindows`. The expression subset supports `!`, `&&`, `||`, equality/inequality, and parentheses. Regex, membership, physical scan-code bindings, and the full VS Code context inventory are not yet supported; unsupported expression syntax causes an explicit import error. Unknown commands produce an explicit unavailable-command message.

Prompt text boxes currently handle their own editing keys, so custom keybinding dispatch inside those prompts is not yet supported. Imports are read at startup rather than live-reloaded.

## Saving and recovery

Opened files stream into a rope; the saved baseline shares unchanged rope storage with the editable buffer. Saving compares disk bytes against that baseline using bounded buffers, streams the current rope to a sibling temporary file, syncs that file, and replaces the destination. It does not allocate whole-file byte/string copies for comparison or saving. Existing Unix mode permissions are retained, and opening through a symlink resolves and saves the target. If external contents differ, Save refuses the overwrite; use Save As to preserve your edits or explicitly revert through the palette. Save As protects existing files rather than presenting an overwrite action in this alpha.

There is still a race between checking and replacing a file if another process writes at exactly that time. Hard-link identity, extended attributes, unusual filesystems, and power-loss durability are not guaranteed by this initial implementation. Recovery testing covers process failure; it does not prove durability across power loss.

Changed unsaved buffers are submitted for recovery roughly every two seconds when the recovery worker is available. Capturing a snapshot shares rope storage; a dedicated worker streams JSON, syncs the file, and replaces the journal. Only one snapshot can be outstanding. Edits made during a write remain eligible for the next snapshot; an older completion cannot acknowledge newer revisions. Write errors appear in the status message and are retried on a later interval. Snapshots retain the v1 format and contain the full unsaved text and saved baseline in the local state directory; the Unix recovery directory is restricted to the user. They are not encrypted. Each running editor holds a session lock so another instance skips its recovery file. After a crash, the next launch restores stale snapshots as editable tabs without writing their contents to the source files. Review and save them explicitly. Undo of a recovered buffer returns to its saved baseline, not to the entire previous session history.

The interval and write duration mean the most recent edits can be lost on abrupt termination. Normal close asks Save, Discard, or Cancel for dirty buffers. A clean exit waits for outstanding recovery I/O before removing its session file. `SIGTERM`/`SIGINT` on Unix wait for any older write, attempt a final snapshot of the latest buffers, and restore the terminal. Shutdown can therefore wait for slow storage. `SIGKILL` cannot restore terminal modes. Use your terminal's reset action or `reset` if necessary after a forced kill.

## Clipboard and current limits

On Linux, system clipboard integration uses `wl-copy`/`wl-paste` or `xclip` if available. macOS uses `pbcopy`/`pbpaste`. Without a helper, an internal clipboard remains available and bracketed terminal paste works. Clipboard helpers have bounded waits. Windows system clipboard integration is not implemented yet.

Files must be UTF-8, without NUL bytes, and at most 32 MiB when opened. Existing LF/CRLF bytes are preserved; newly inserted lines use the detected newline style. Very long lines, large replacements, startup recovery, and file saves can still pause this alpha. Periodic recovery I/O runs off the input thread; the performance report includes a recovery-enabled typing workload, while language-service/extension contention and the design document's latency budgets remain unqualified. Full bidirectional layout and terminal-independent emoji-width agreement are not implemented.

Not yet implemented: registry downloads, broad extension API compatibility, rich webviews, notebooks, full settings migration, or a remote agent. An optional experimental command extension host is available as described below. Running the executable inside an SSH session is supported in principle; the actual terminal/multiplexer combination must be tested.

Grammar highlighting uses a single background worker, document/revision checks, cancellation, and a 2 MiB source cap. Other languages and larger files retain lightweight lexical colors. Embedded-language injection, semantic tokens, incremental parse-tree reuse, grammar folding and complete theme semantics remain incomplete. Native color-theme loading and its boundaries are documented in [Import and themes](IMPORT_AND_THEMES.md). Highlight work exceeding the initial time/span budget is canceled and reported; editing remains available.

## Importing VS Code and choosing themes

`vscli --import-vscode /path/to/Code/User` previews a migration. Add
`--apply-import` to activate a versioned profile copy while retaining original files
and earlier native profiles. `--vscode-extensions` selects the source extension
directory for the active color theme; `--config-dir` selects the native destination.
Copied fields are not automatically supported: the JSON report names unsupported
settings, commands, shortcut expressions and theme features.

F1 → Preferences: Color Theme selects built-in or installed extension themes;
Preferences: Load Color Theme File loads JSON/JSONC directly. `--theme PATH`
overrides the saved selection. Themes are native data and do not start Node.
See [import guarantees, theme mappings and limits](IMPORT_AND_THEMES.md).

## Settings

Use `--settings /path/to/settings.json` to import user settings. By default, VSCLI reads `settings.json` beside its user keybindings file. An activated import supplies the user settings/keybindings/snippet directory unless explicit CLI paths override it. Workspace `.vscode/settings.json` is layered above user settings. Ctrl+, (Cmd+, on macOS) opens the user settings JSON file. Comments and trailing commas are accepted.

The current supported subset is `editor.tabSize` (1–16), `editor.insertSpaces`, and `editor.lineNumbers` (`on`, `off`, `relative`, `interval`). Indentation width applies to editing, cursor/mouse coordinates, rendering, and LSP formatting options. Language blocks such as `[python]` and `[javascript][typescript]` override general values; a single-language block takes priority over a multi-language block. Workspace values override user values within the same identifier group. Combined-language groups retain their first occurrence order across scopes; changing that order can change precedence. This follows the pinned [configuration model](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/platform/configuration/common/configurationModels.ts).

Settings reload in the background every two seconds. Malformed updates retain the previous configuration. Entries outside the native subset and invalid native values appear in F1 → Settings: Compatibility Report. A notice about an extension setting does not mean an enabled extension cannot read it. Automatic indentation detection, complete theme semantics, autosave, formatting-on-save, profiles, remote scopes, policies, and the broader settings catalog remain incomplete; importing those entries does not enable their behavior.

## Native debugging

Configure an installed stdio DAP adapter explicitly. For Python with `debugpy` installed in a chosen environment:

```sh
vscli program.py --debug-adapter /path/to/python --debug-arg=-m --debug-arg=debugpy.adapter
```

F9 toggles a line breakpoint in a saved file. F5 launches the active file (or `--debug-program PATH`) and continues paused execution. F10 steps over, F11 steps into, Shift+F11 steps out, and Shift+F5 stops. Repeat `--debug-program-arg` for program arguments. Save modified program and breakpoint files before launch.

Ctrl+Shift+D (Cmd+Shift+D on macOS) opens the debugger view. Tab switches stack, scopes, variables, and console. Enter selects a frame or expands a scope/object. `e` prompts for an expression evaluated in the selected frame; results appear in the console. The palette also exposes pause, evaluation, and console commands. On exit the client requests debuggee termination and gives the adapter a bounded cleanup interval before reaping it; adapters that ignore disconnect can leave their own descendants running.

Incoming and outgoing serialized DAP JSON frames are each capped at 16 MiB. Oversized adapter responses or debugger requests reject debugger work while retaining native buffers.

The real Python/debugpy integration test covers breakpoint, variables, step, evaluation, and continued exit. Deterministic tests cover reversed variable replies, rejected stepping, and disconnect. This is an initial DAP launch workflow: launch.json, attach, adapter-specific launch fields, watch persistence, thread selection, conditional/log breakpoints, source-reference downloads, and test-provider UI remain incomplete. Breakpoint positions are not yet tracked through source edits or persisted. Columns currently use scalar character positions. Adapters requiring reverse terminal requests are rejected explicitly.

## External file changes

Native filesystem notifications refresh the workspace index and explorer after a short debounce. Clean open files reload as one undoable edit while retaining shared document/view identity. Dirty files retain unsaved text and report a conflict; the save guard continues to reject overwriting changed disk content. Deleted open files become dirty retained buffers, so quit confirmation and crash recovery preserve their contents. Explicit Revert also retains view identity and can be undone.

Reload preserves cursor and selection positions within the common unchanged
prefix and suffix in every split view, shifting trailing positions by the edit's
character-count difference. Undo and redo retain these independent selections.
Reload currently maps one replacement between that prefix and suffix; unchanged
regions between multiple disjoint external edits are not independently mapped.

Disk reads run off the UI thread, are capped at 32 MiB per file, and are discarded if the buffer was edited, saved, closed, or renamed while reading. Open files are checked every two seconds as a fallback, including files outside the workspace. The worker compares disk bytes against the shared saved baseline with bounded buffers; unchanged files do not allocate replacement ropes or require a full comparison on the input thread. Changed files still load into a bounded rope before their version-checked reply is applied. Index notifications ignore common generated directories. If native watcher setup fails, use Refresh Explorer for index changes; the periodic open-file check continues. Network filesystem event behavior, very large directory trees, and non-Linux watcher backends still need qualification.

## Verification

Long-line movement and column lookup walk rope chunks without copying whole
lines. Plain-text rendering and rendering with ready grammar highlights copy
only the viewport prefix plus find-query lookahead. Unicode clusters, tabs,
selections, and CRLF boundaries retain their existing behavior. Horizontal
scrolling far into a line still scans its prefix, unusually large grapheme
clusters still need complete segmentation, and the fallback syntax lexer still
reads whole lines. This is not a qualified large-file mode: the 32 MiB opening
limit, blocking open/save paths, and 2 MiB grammar-highlighting limit remain.

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked
python3 tests/pty_smoke.py target/release/vscli
```

The automated PTY suite drives the real executable, including legacy and emulated enhanced input, Unicode paste, save, selection, undo, comment chords, Save As, resize, quick open, custom bindings, external-change protection, unsaved-close confirmation, terminal restoration, and SIGKILL recovery. It uses Python's standard library and temporary directories. Protocol emulation is not a substitute for testing a physical keyboard in every real terminal.

For a manual smoke test, open a disposable file, type text, save it, select/replace and undo, use Ctrl+P to open a second file, find text, and close with unsaved changes to exercise Save/Discard/Cancel. Use F1 → Keyboard Inspector for combinations your terminal consumes.

## Experimental extension commands

Install local extension packages with **F1 → Extensions: Install from VSIX**, or `vscli --install-extension ./package.vsix`. Use **Ctrl+Shift+X** (**Cmd+Shift+X** on macOS), or **F1 → Extensions: Show Installed Extensions**, to view compatibility descriptions, explicitly run a selected code package, restore its previous installation with **R**, or uninstall it with **Delete**. Installation alone never activates code. The picker also works from the empty welcome screen, Explorer and integrated terminal. Esc closes a pending loading view while its operation finishes; the result does not replace a newer prompt. CLI equivalents are `--list-extensions`, `--rollback-extension publisher.name` and `--uninstall-extension publisher.name`; `--extensions-dir` selects storage. Installed code packages can be launched with `vscli --extension publisher.name .`. Storage supports one previous generation, retains immutable files for running hosts, and does not yet download registry packages or collect old files. Installed packages are not automatically API compatible.

`vscli --extension /absolute/path/to/unpacked-extension .` starts the extension's Node `main` entry in an optional process. This explicitly executes trusted extension code with your user permissions. Use `--extension-node /path/to/node` to select the runtime. Normal native editing does not require Node. Extensions receive imported user/workspace settings before activation and valid live updates through `workspace.onDidChangeConfiguration`; held configuration objects remain snapshots.

Registered commands appear in F1 with an `Extension:` prefix. Manifest keybindings retain their original combinations and platform overrides beneath user overrides/removals; unsupported context expressions are reported. Commands can also be assigned in user keybindings by their original IDs. Edits are version checked and undoable, and do not save files automatically. F1 → Extensions: Stop Host terminates the process while retaining native buffers. The initial host has substantial API and contribution limitations; read the [extension evidence and scope](EXTENSIONS.md) before using an extension.

Enabled command extensions can show a native single-selection Quick Pick or Input Box. Type to filter a pick or edit input, press Enter to accept, and Escape to cancel. Native prompts retain priority; stopping or replacing a host clears its prompts. Password, validation, multi-select and live Quick Input variants explicitly reject. See the [supported options, bounds and named workflow evidence](EXTENSIONS.md#native-quick-pick-and-input-box).


## Recent files and reopening closed editors

**Ctrl+R** on Linux, Windows and macOS opens **Open Recent File**. The native picker
fuzzy-matches full paths, so files with the same name remain distinguishable. The
empty welcome screen shows up to five recent paths and shortcut hints from the
active keymap, including context, removal and shadowing of user overrides. While
the integrated terminal has focus these shortcuts are forwarded to the shell,
matching the [pinned default shell-routing list](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/workbench/contrib/terminal/common/terminal.ts); use F1 to invoke navigation there.
Recent entries cover files only; this
command does not switch workspaces or restore a window/session layout.

**Ctrl+Shift+T** (Linux/Windows) or **Cmd+Shift+T** (macOS) reopens the most recently
closed file-backed editor in this session. Accepted closes record its path and
primary display position; canceled closes record nothing. Discarded buffer text
and untitled editors are never resurrected. Save As records the saved destination.
A newly loaded file uses its current disk contents and a clamped cursor position;
missing or unreadable files remain retryable in history without creating a new
empty buffer. If the document is already open, its existing buffer, undo history
and shared identity are focused, even if the backing file has disappeared.

Recent-file metadata is stored under the native **configuration root** at
`state/recent-files.json`, including when an imported profile is active. It keeps
at most 100 absolute paths (4096 bytes per path; 512 KiB state-file limit), not file
contents. One background state worker coalesces updates and merges under a file
lock before an atomic replacement; older updates cannot overwrite newer timestamps.
Normal shutdown waits for queued state writes. Malformed or unwritable state is
reported and retained; new updates remain in memory and retry after another file
open/save or shutdown. Abrupt termination can lose pending history, and directory
metadata power-loss durability is not qualified. Other running instances' recent
lists refresh during their own state loads/writes, not through a live subscription.
The bounded closed-editor stack is session-local and is not persisted.

Recent/reopen disk loads use a separate single-outstanding background request;
changed editor/workspace context discards a late reply and retains retryable closed
history. Native tests cover persistence failures and concurrency, shared dirty
buffers, save/discard/cancel, Save As, bounds and stale replies. PTY tests exercise
close-to-welcome, reopen/edit/undo/save, missing-file retry and persisted recents
after restarting. These are native behavior checks, not full VS Code history or
session parity.


## Native code actions and quick fixes

With a configured language server, **Ctrl+.** (**Cmd+.** on macOS) opens
**Language: Quick Fix**. **Ctrl+Shift+R** on all three platform profiles opens
**Language: Refactor**. The shortcuts and provider/editability context match the
pinned VS Code 1.95 inventories. The picker lists server actions, preferred first;
Enter applies the chosen action and Escape closes it. Disabled actions explain
why they cannot run. Servers can lazily resolve edits when an action is selected.
Diagnostic codes, related fields and opaque `data` are preserved in action requests.

Supported text edits use either `changes` or versioned `documentChanges` across
already open, synchronized buffers. Every target identity, revision, server version,
UTF-16 range, overlap and size bound is validated before any buffer changes.
Other dirty buffers can participate when they still match the request snapshot.
The editor preserves shared buffer identity and leaves changes unsaved; Undo is per
file. Changed selections, views, documents or input context reject stale replies.
Files outside the synchronized open buffers must be opened before retrying.

Explicitly selected server commands can issue `workspace/applyEdit` while that
command is pending, and only through `documentChanges` with explicit numeric
versions for every target. Unversioned `changes` and null-version command callbacks
are rejected; direct selected code-action edits can still use those forms against
their own captured snapshot. LSP has no causal command identifier on `applyEdit`:
this boundary relies on the server attaching truthful document versions, and cannot
prove which completed command produced a late callback. Document versions increase
throughout a client session, including closing and reopening the same path. After an execute-command
timeout, further commands and their callbacks are disabled until the language server
restarts. Unsolicited, expired or stale edits are rejected and acknowledged
to the server. Each accepted `applyEdit` is a separate transaction across its own
targets; there is no whole-command rollback of earlier edits if a later callback
fails. Undo remains per file. Only one action/resolve/command chain runs at once. Combined actions
containing both an edit and a command are rejected before mutation. Resource
creation/rename/deletion, annotated edits, unknown text-edit forms and repeated
workspace targets are also rejected. Sequential edits from a command after its
original snapshot changes are not qualified; this is not a general server-command
or workspace refactoring implementation.

Limits are 128 intersecting diagnostics, 128 synchronized/edited buffers, 300
picker actions, 4096 text edits and 4 MiB total replacement text. Exceeding these
limits reports an error. Existing LSP edit validation also rejects resulting buffers
over the 32 MiB document limit before mutation. The native transport bounds messages and queues;
no Node runtime is required. This implementation does not provision or automatically
start a language server.

Deterministic protocol tests cover lazy resolve, selected commands, multi-file
atomic rejection, dirty/shared buffers, late replies, source-byte preservation and
undo. Unix PTY tests exercise real shortcuts, the picker, resolve and CRLF save/undo.
A separate real-server test passed locally with **clangd 23.1.1 on Linux** for a C++
missing-semicolon quick fix, including undo/redo and explicit save:

```sh
cargo test --locked --test code_actions real_clangd_cpp -- --ignored
```

This qualifies that C++ fix on that server, not all clangd refactors, all servers or
full VS Code language-feature parity. The pre-existing rename path retains its
separate limitations described above.

## Parameter hints

**Ctrl+Shift+Space** on Linux/Windows or **Cmd+Shift+Space** on macOS requests
**Language: Parameter Hints** from the configured LSP server. The original binding
requires editor focus and a server advertising signature help. The native themed
panel shows the server-selected signature, highlights its active parameter, and
renders signature/parameter documentation as plain text. **Escape** or
**Shift+Escape** dismisses it. The panel remains nonmodal: typing, movement, other
commands, prompts, selection/view changes or closing the editor dismiss it and
cancel any pending request. Late canceled replies cannot reopen it. Invoking the
command from an empty welcome screen does not create a document.

This first slice is explicitly invoked. Automatic triggers/retriggering while
arguments are typed, overload navigation, Markdown formatting and every server's
parameter-selection behavior remain unqualified. UTF-16 offset-pair and substring
parameter labels are supported; string labels use the pinned editor's ASCII word
boundaries so `int` matches the type in `print(value: int)`, with an empty highlight
when no match exists. This is source-informed by
[VS Code 1.95 parameter rendering](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/editor/contrib/parameterHints/browser/parameterHintsWidget.ts).
Out-of-range active indices fall back to zero.
The response accepts at most 32 signatures, 128 parameters per signature, 8 KiB per
signature label or selected documentation field, and 64 KiB total signature labels. The panel clips
long content to its available editor area. Malformed ranges or oversized data
report an error without touching the buffer. Hints never edit or save files.

Deterministic protocol/UI/PTY tests cover original shortcuts, active highlighting,
shared-document identity, cancellation, out-of-order replies, empty startup,
malformed data and CRLF save/undo after dismissal. A real clangd 23.1.1 C++ fixture
checks the second argument of a two-parameter function without changing file bytes:

```sh
cargo test --locked --test signature_help real_clangd -- --ignored
```

This is native workflow evidence, not full signature-help parity with VS Code.

## Native document and workspace symbols

**Ctrl+Shift+O** (macOS **Cmd+Shift+O**) opens **Go to Symbol in Editor**.
**Ctrl+T** (macOS **Cmd+T**) opens **Go to Symbol in Workspace**. These defaults
and their contexts match the pinned VS Code 1.95.0 inventory. Both commands require
an explicitly configured, ready LSP server advertising the relevant provider;
document symbols additionally require a saved file handled by that server.
Workspace symbol search also works from an empty welcome screen.

The native searchable picker displays names, kinds, containing symbols and file
locations. Document search filters one response locally; workspace queries wait
200 ms after input changes, cancel superseded requests and clear obsolete results.
Hierarchical `DocumentSymbol` uses `selectionRange`; flat `SymbolInformation` and
range-bearing `WorkspaceSymbol` use their location range. Range-less workspace
symbols requiring `workspaceSymbol/resolve`, non-file URIs, malformed ranges and
oversized responses produce explicit errors. Resolve, an outline panel, symbol
previews, grouping modes and complete VS Code navigation behavior are not implemented.

Before navigation, the picker checks the workspace, active buffer, all captured
open-buffer identities/revisions, selections, pane and focus. New input contexts,
changed buffers, canceled requests and stale file-load replies cannot move the
cursor or replace a newer picker. An already-open target retains its document,
shared views, unsaved text and undo history even when its backing file was deleted.
Successful navigation collapses the destination view to one cursor at the symbol;
other shared views retain their selections. Every accepted range is checked against the actual
buffer using UTF-16 positions, including rejection of split surrogate pairs.

Targets needing filesystem alias resolution use one bounded background loader.
It matches the resolved path to captured open buffers before reading file contents,
so a dirty buffer remains usable through a parent alias after its file is deleted.
Closed targets must be existing regular files within the existing 32 MiB open
limit; missing files never create buffers.
Cancellation retains the worker slot until it finishes. A server response cannot
guarantee the revision of an unopened file, so this path validates the supplied
range against freshly read text rather than claiming a server-version snapshot.
No symbol operation writes source files.

Limits are 128 open buffers, 512 returned symbols, 16 child levels, 4 KiB per
name/detail/container/path, 64 KiB cumulative display metadata and a 1 KiB query.
Excess results are rejected explicitly rather than silently truncated. Input and
rendering perform no symbol-file reads; no Node runtime is required.

Synthetic tests cover hierarchy, flat locations, malformed/unresolved responses,
Unicode offsets, dirty/shared/deleted targets, canceled and out-of-order replies,
native prompt preservation and bounded loader cancellation. A Unix PTY journey
covers both original shortcuts, search, async open and CRLF save/undo. An explicit
real-server test passed with **clangd 23.1.1 on Linux**, navigating C++ document and
workspace symbols without changing file bytes:

```sh
cargo test --locked --test symbol_navigation real_clangd -- --ignored
```

This evidence qualifies that fixture and server, not all workspace indexing,
server implementations or full symbol-navigation parity.
