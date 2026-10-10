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

Save and Save As now persist on a background native worker. You can keep typing
or switch editors while an authorized save finishes: the captured text reaches
disk, and newer edits stay dirty with their Undo/Redo intact. Save As retains the
editor that opened the prompt. A different existing destination is refused;
choose a new path. Closing while that editor's save is still pending waits for
the actual receipt, even if its disk bytes are already visible. It targets the
original pane and keeps it open if newer unsaved edits remain; Escape cancels
this deferred close while the save continues. Quit and Close All await pending native/settings
writes; Escape cancels deferred closing while saves continue. Watch the status
message for completion or failure before relying on disk bytes.

Autosave defaults to off. Native user or workspace settings support
`"files.autoSave": "afterDelay"` and `"files.autoSaveDelay": 1000` (milliseconds),
including language overrides such as `"[cpp]"`. The native delay range is
0–86,400,000 ms; the default is 1000 ms. Text edits and Undo/Redo restart the
delay, while cursor motion does not. Dirty named files, including hidden models,
qualify; untitled buffers require Save As. Malformed winning values, invalid
delays and unsupported focus/window modes disable automation with a settings
notice. Failed settings reloads pause autosave; repeated saves of the same failed
snapshot are suppressed until explicit retry or new qualifying evidence.

Opt-in native [format-on-save](FORMAT_ON_SAVE.md) and edit-only
[source actions on save](CODE_ACTIONS_ON_SAVE.md) run before background persistence.
Extension wait-until participants and Save All remain unsupported. Async saves and
after-delay autosave have passed the required Linux/macOS/Windows checks;
qualification of each new participant is recorded separately.
See [save ownership, settings, limits and qualification](SAVE_AUTOMATION_PLAN.md).

C/C++ and JSON/JSONC now have native [smart typing](SMART_TYPING.md): bracket/quote
pairing, surrounding selections, generated-close skipping/deletion and bracket-aware
Enter. Native [advanced indentation](ADVANCED_INDENTATION.md) distinguishes all five
`editor.autoIndent` modes, defaults to `full`, and adds bundled C++ unbraced-body
and JSON indentation rules plus guarded closing-bracket alignment. Paste remains
literal. Language overrides control pairing and indentation. Installed
[native language configurations](NATIVE_LANGUAGE_CONFIGURATIONS.md) now supply
bounded single-character pairs, surrounding selections, following-character
rules and comment delimiters without Node or a code-execution grant. Installed
data is enabled by default; explicit extension disable preferences apply.
Imported grammars, regular-expression indentation rules, other profiles and full
language-rule parity remain incomplete. Original Add/Remove Line Comment chords
are Ctrl+K Ctrl+C/U on Linux/Windows and Cmd+K Cmd+C/U on macOS; Toggle Block
Comment is Ctrl+Shift+A on Linux and Shift+Alt+A on macOS/Windows. Terminal delivery
still depends on the terminal's input protocol and shortcut interception.

Quick open indexes up to 100,000 workspace files in a background worker. It respects ignore rules and skips `.git`, `target`, `node_modules`, `.venv`, and `__pycache__`. The explorer shows the current directory; Right opens a directory or file (Enter also opens in Linux/Windows profiles), Left/Backspace goes to its parent, and Escape returns focus to the editor. macOS uses Enter for rename. Native file notifications refresh the index after a short debounce; Refresh Explorer is available when notifications are unavailable. See [external file changes](#external-file-changes) for qualification limits.

The command palette lists implemented actions. Background Tree-sitter highlighting covers C/C++, Rust, Python, JavaScript/JSX, TypeScript/TSX, and JSON, including multiline constructs. C/C++ use native bundled grammars without Node or a language server; C++ templates, preprocessor directives, multiline raw strings and Unicode comments map to native theme categories. Headers (`.h`, `.hpp` and related suffixes), module files and `.h.in`/`.hpp.in` templates select C++ highlighting. CUDA grammar, semantic tokens and exact TextMate scopes remain unsupported. Grammar work retains the 2 MiB document limit, cancellation and revision checks; larger files use existing fallback highlighting. Tabs switch using the profile's next/previous editor shortcuts. Multiple cursors support typing, deletion, indentation, comments, selection movement, clipboard operations, and undo. Up to four editor groups can share documents with independent cursors and scroll positions.

Explorer commands in F1 create files/folders, rename the selected item, move it to system trash, and refresh the index. F2 renames in Linux/Windows, while Enter renames in macOS. Delete moves the selected item to trash in Linux/Windows; Cmd+Backspace does so in macOS. Trash always asks for confirmation and never falls back to permanent deletion. Use the OS trash interface to restore items. Open buffers under a trashed path are retained as unsaved copies.

File operations run in a worker and refresh the explorer/index afterward. Save is refused while a file operation is pending, and file operations are refused until pending native saves settle. Create operations require an existing parent directory and refuse existing names. Rename preserves open unsaved buffers and refuses an existing destination; the initial portable rename implementation still has a check/rename race against concurrent filesystem writers. Native watching and undoable clean-buffer reload are implemented; dirty buffers retain their unsaved contents. See [external file changes](#external-file-changes) for limits.

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

F1 → **Keyboard Inspector** opens a themed reference keyboard. It shows the last received key, reported modifiers, readable shortcut/chord, protocol, selected platform profile, and command that would match the context captured before opening the inspector. Commands are previewed without executing or modifying text; Escape closes. At 104×30 the diagram includes function, navigation and arrow keys; 78×25 shows the main keyboard, and smaller terminals retain a compact event readout. Resizing preserves the captured event.

The diagram is a US reference layout, not physical keyboard detection. Highlighting identifies the last received event, including a reported release, rather than keys currently held. Combined modifier reports cannot identify left/right keys, international physical layout, or keys intercepted before delivery. Explicit side-specific modifier events can highlight their corresponding keycap. The normalized shortcut follows native keybinding rules (including legacy ambiguities); the reported-modifier chips only show flags actually received.

Exact physical key delivery depends on terminal configuration. VSCLI negotiates enhanced keyboard reporting when supported. A terminal may otherwise turn Ctrl+Shift+P into Ctrl+P or consume the combination entirely. The editor does not silently replace that binding. Use F1 → Keyboard Inspector, release the conflicting terminal binding, and retest. No terminal configuration is changed automatically. OS-global, international-layout, and multiplexer behavior still require real-device qualification.

## Navigation history

F1 → **Go: Back** or **Go: Forward** restores locations within this session.
The original shortcuts are Ctrl+Alt+- / Ctrl+Shift+- on Linux,
Alt+Left / Alt+Right on Windows, and Ctrl+- / Ctrl+Shift+- on macOS.
The terminal must deliver the combination; enhanced keyboard reporting can
distinguish shortcuts that legacy terminal input cannot represent.

History keeps up to 50 locations across native documents and editor panes.
Editor changes, explicit line jumps and large cursor movements record locations;
nearby ordinary movement coalesces. Travel preserves unsaved text, document
identity and Undo/Redo. Save As follows the model's current path, and closed
untitled models cannot be resurrected. Primary selections restore as forward
ranges; secondary cursors are not retained in history entries.

Closed file destinations load in the background. Shortcut availability reflects
the committed location; unsuccessful or stale loads cannot consume history.
See [recording rules, asynchronous bounds and qualification](NAVIGATION_HISTORY.md).
The native [Outline section](OUTLINE.md) merged after local and platform
qualification. [Breadcrumbs](BREADCRUMBS.md) merged after its platform checks;
folder dropdowns, filtering, reveal-aside
and complete navigation/UI parity remain incomplete.

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

Choice menus, nested snippet merging and extension API insertion are still unfinished.
Completion snippets use native linked fields; see [IntelliSense](COMPLETIONS.md). Unsupported regex
constructs fail explicitly. See [snippet evidence and limits](SNIPPETS.md).

## Multiple cursors and line commands

Disjoint selections retain their primary and secondary order through normalization, typing, and undo/redo. Overlapping ranges merge while retaining the earliest selection's direction.

Ctrl+D (Cmd+D on macOS) selects the word, then adds the next occurrence. Ctrl+Shift+L / Cmd+Shift+L selects every occurrence. Escape collapses to the primary cursor; Ctrl+U / Cmd+U undoes cursor changes. Alt-click adds a cursor. Shift+Alt+I places cursors at selected line ends.

Vertical cursor shortcuts differ by platform: Linux uses Shift+Alt+Up/Down, Windows uses Ctrl+Alt+Up/Down, and macOS uses Cmd+Alt+Up/Down. Line duplication uses Ctrl+Shift+Alt+Up/Down on Linux and Shift+Alt+Up/Down on Windows/macOS. Alt+Up/Down moves the selected line block. Ctrl+Enter / Ctrl+Shift+Enter inserts a line below/above (Cmd on macOS). Bracket navigation uses Ctrl+Shift+\ / Cmd+Shift+\.

Line move/copy commands preserve disjoint selections and undo together. Cursor creation is limited to 10,000 selections. Bracket matching is textual and does not yet exclude strings/comments. These are recorded parity gaps, not claims of complete VS Code editing semantics.

## Split editors

Ctrl+\ / Cmd+\ splits the selected editor group to the right. F1 → View: Split Editor Down splits that group into two stacked groups; existing branches retain their layout. Ctrl+1 through Ctrl+4 (Cmd on macOS) focus existing groups; current drawn tab/text targets focus their exact group membership. Each group has independent cursor/selection and scroll state, while edits, undo history, save state, and language synchronization belong to the shared document. Closing one of several views of a dirty document retains the buffer; closing its last view still asks about saving.

Each group now has its own ordered committed tabs. Opening a file adds it directly
right of the current tab in the current group, or focuses its existing membership
there. Splitting copies only the selected tab and its current view. Ordinary opens
in another group start a fresh view; revisiting a tab restores that group's cursor,
selection and scroll. Click a displayed tab to focus its exact membership.
Ctrl+PageDown/PageUp (Cmd+Alt+Right/Left on macOS) traverse tabs across groups;
F1 → View: Next/Previous Editor in Group wraps inside the selected group.
Ctrl+Shift+PageUp/PageDown moves the active tab left/right inside its current
group (macOS: Cmd+K, Cmd+Shift+Left/Right); F1 exposes View: Move Editor Left/Right.
An actual move commits a preview and pins/unpins it when crossing the sticky
prefix. Edge no-ops retain preview mode. Document identity, per-group views,
Undo/Redo and layout ratios remain retained. See [reorder scope and qualification](TAB_REORDERING.md).
Closing selects that group's most recently active surviving tab. Close Editors in
Group reviews last-owned dirty documents one at a time; Cancel preserves remaining
tabs, and a changed tab list retires the batch. A delayed Save→Close closes its
original tab even if it became inactive, and cannot close a reopened replacement.
See [editor-group bounds, restoration and remaining scope](EDITOR_GROUPS.md).


The current candidate supports up to four nested groups. F1 → View: Increase/Decrease
Current View Width changes the selected group by four terminal columns; the Height
commands use two rows, bounded by available space. View: Reset Editor Group Sizes
resets sibling ratios. These original command IDs have no invented resize shortcuts.
A palette resize may wait once for the next editor repaint. Mouse divider dragging,
left/up splits, presets, merge/maximize and full spatial focus remain unsupported.
The legacy View: Close Editor Group label runs Close Editors in Group; the distinct
upstream Close Group command is not implemented.

Very small editor areas project only the active group; the tree and shared models
remain retained. Whole-screen tiny mode and covered frames grant no stale source
hits. Recovery beyond group capacity shows its authoritative buffer for keyboard
editing/navigation with pointer targets disabled; Save still requires at most 128
retained models. The schema-3 candidate preserves clean nested topology, resized
ratios and sticky prefixes; older schema-1/2 slots remain readable without rewriting
on load. Local integrity and restart checks pass; platform checks remain separate.
See [layout implementation,
evidence and remaining qualification](EDITOR_LAYOUT.md). Undo position changes are
tracked across views; inactive viewport rows are not yet anchored to text across
line insertions.

## Workspace search

Ctrl+Shift+F / Cmd+Shift+F opens Find in Files. Alt+C toggles case sensitivity, Alt+W whole words, and Alt+R regular expressions. Enter starts a background search; arrows choose a result, Enter opens its selection, and Escape closes/cancels. Results use unsaved buffers for open files. A changed result is reported instead of selecting an outdated match.

Search respects ignore files and the same excluded directories as quick open. It scans at most 100,000 file paths, skips binary/non-UTF-8/oversized/unreadable files, and returns at most 5,000 matches. The result header reports skipped files and truncation. Search is line-based; multiline regex and workspace replacement are not implemented yet. Results are snapshots, not live subscriptions.

## Language servers

Opening a saved C/C++ file automatically starts installed `clangd`; opening a Rust
file starts installed `rust-analyzer`. Start with `vscli .`, then **Ctrl+O** or
**Ctrl+P** to open a file. No LSP flags or JavaScript runtime are needed. Empty and
untitled editors do not start a server, and nothing is downloaded automatically.

There is one active native server per window. Default C and C++ files share clangd.
Selecting a different language replaces the active server; selecting an unsupported
language, closing the last editor or disabling services retires it. Existing text,
undo history and shared views remain intact. Language pickers, hints and diagnostics
from the retired server are canceled. This first slice is not a concurrent
multi-language server pool or extension-provided language-server activation.

**Language: Restart Server**, **Language: Disable Services**, **Language: Enable
Services** and **Language: Server Status** are available in F1, without new default
shortcuts. Failed discovery/crashes report a useful notice and do not restart in a
loop; explicitly restart or change the effective server configuration to retry.
`--no-lsp` starts with automatic services disabled. Explicit `--lsp` remains an
authoritative manual override independent of the selected file and settings:

```sh
vscli .
vscli --no-lsp .
vscli --lsp rust-analyzer --lsp-language rust .
vscli --lsp clangd --lsp-language cpp --lsp-arg=--background-index=false src/main.cpp
```

User settings, including imported settings or `--settings FILE`, can configure a
language's executable, separate process arguments and enabled state:

```jsonc
{
  "[cpp]": {
    "vscli.languageServer.program": "clangd",
    "vscli.languageServer.args": ["--background-index=false", "-j=2"]
  },
  "[rust]": { "vscli.languageServer.enabled": false }
}
```

`vscli.languageServer.enabled` defaults to true and follows existing user/workspace
and language-block precedence. Workspace `.vscode/settings.json` program/args are
ignored with a notice unless **user** settings set
`"vscli.languageServer.allowWorkspaceConfiguration": true`; a repository cannot
turn that permission on itself. Workspace disabling is honored without this opt-in.
The existing task-specific trust dialog does not govern language servers. Programs
run directly, without a shell; relative explicit paths resolve against the workspace.
PATH lookup ignores empty/relative entries, examines at most 128 entries/64 KiB and
requires a regular executable. Program paths and individual args are limited to
4 KiB, with at most 32 args/16 KiB total. Settings reload applies effective changes.

Discovery, startup and process retirement use one bounded background worker;
rapid switches/retries retain its occupied slot. Stale startup offers cannot replace
a newer document/pane/focus or modal context. Initialization carries no document
edit: accepted servers synchronize the current buffers, while later language replies
retain document/revision/view checks. Normal shutdown waits at most five seconds
for this worker. On Unix, managed language servers use an isolated process group
for retirement, including ordinary descendants; Windows retirement owns the direct
child only. Neither behavior is a sandbox or a whole process-tree resource limit.

The server must already be installed. This implementation uses native stdio JSON-RPC with bounded transport queues, UTF-16 positions, document versions, and stale-response checks. Diagnostics appear as gutter markers and in Problems. Server failure leaves editing available. Incoming and outgoing serialized JSON frames are each capped at 16 MiB. Oversized document synchronization or tooling requests reject language-server work while retaining native buffers; the editor’s 32 MiB file-opening limit does not qualify every such file for LSP.

Successful-save notifications follow the server's static save capability. The
notification includes text only when `includeText` is true; disabled or omitted
object save options suppress it. Legacy numeric Full/Incremental synchronization
requests save without text, following the official language client's normalization.
The native snapshot API accepts the exact committed Rope, and the transport writer
serializes it off the input thread within its existing 64 MiB output budget.
Escaped output exceeding the 16 MiB frame limit reports a notification failure
while preserving the successful file save and protocol channel. Four real framed
subprocess tests cover capabilities, Unicode/CRLF snapshots, closed/unsynchronized
documents and overflow recovery. Background application saves, dynamic capability
registration and save participants remain separate work. See the
[LSP save contract](https://github.com/microsoft/language-server-protocol/blob/gh-pages/_specifications/lsp/3.17/textDocument/didSave.md)
and [official client normalization](https://github.com/microsoft/vscode-languageserver-node/blob/main/client/src/common/client.ts).

| Action | Linux | Windows | macOS |
| --- | --- | --- | --- |
| Completion | Ctrl+Space | Ctrl+Space | Ctrl+Space |
| Hover | Ctrl+K Ctrl+I | Ctrl+K Ctrl+I | Cmd+K Cmd+I |
| Definition | F12 | F12 | F12 |
| References | Shift+F12 | Shift+F12 | Shift+F12 |
| Format document | Ctrl+Shift+I | Shift+Alt+F | Shift+Alt+F |
| Rename symbol | F2 | F2 | F2 |
| Problems | Ctrl+Shift+M | Ctrl+Shift+M | Cmd+Shift+M |

Completion appears automatically after ordinary identifier typing or a server/provider trigger character, and Ctrl+Space requests it explicitly. The nonmodal caret popup uses Up/Down, Page Up/Down, Tab/Enter acceptance and Escape dismissal; snippet placeholder Tab retains precedence. Formatting is undoable. Rename currently accepts unversioned text edits, stages validation before changing any buffer, and leaves files unsaved for review; undo is per file. It refuses edits to other unsaved buffers. Versioned rename edits, file operations, completion follow-up commands, server provisioning, automatic restart, multiple simultaneous servers, and advanced capability negotiation remain incomplete. Native code-action support has its own stricter transaction rules below. The server is terminated when the editor exits; graceful shutdown is still pending. Diagnostics lacking server versions have weaker stale-result guarantees.

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

Save As and the first Save of a nonexistent file use atomic create-only persistence: if another process creates the destination after the initial check, Save fails and preserves that file and the unsaved buffer. Native integrity tests inject this exact race and verify Unicode/CRLF bytes, selections and both Undo/Redo branches. Replacement of an existing file still has a race between its final content check and rename. Hard-link identity, extended attributes, unusual filesystems, and power-loss durability are not guaranteed by this initial implementation. Recovery testing covers process failure; it does not prove durability across power loss.

Changed unsaved buffers are submitted for recovery roughly every two seconds when the recovery worker is available. Capturing a snapshot shares rope storage; a dedicated worker streams JSON, syncs the file, and replaces the journal. Only one snapshot can be outstanding. Edits made during a write remain eligible for the next snapshot; an older completion cannot acknowledge newer revisions. Write errors appear in the status message and are retried on a later interval. Snapshots retain the v1 format and contain the full unsaved text and saved baseline in the local state directory; the Unix recovery directory is restricted to the user. They are not encrypted. Each running editor holds a session lock so another instance skips its recovery file. After a crash, the next launch restores stale snapshots as editable tabs without writing their contents to the source files. Review and save them explicitly. Undo of a recovered buffer returns to its saved baseline, not to the entire previous session history.

The interval and write duration mean the most recent edits can be lost on abrupt termination. Normal close asks Save, Discard, or Cancel for dirty buffers. A clean exit waits for outstanding recovery I/O before removing its session file. `SIGTERM`/`SIGINT` on Unix wait for any older write, attempt a final snapshot of the latest buffers, and restore the terminal. Shutdown can therefore wait for slow storage. `SIGKILL` cannot restore terminal modes. Use your terminal's reset action or `reset` if necessary after a forced kill.

## Clipboard and current limits

On Linux, system clipboard integration uses `wl-copy`/`wl-paste` or `xclip` if available. macOS uses `pbcopy`/`pbpaste`. Without a helper, an internal clipboard remains available and bracketed terminal paste works. Clipboard helpers have bounded waits. Windows system clipboard integration is not implemented yet.

Files must be UTF-8, without NUL bytes, and at most 32 MiB when opened. Existing LF/CRLF bytes are preserved; newly inserted lines use the detected newline style. Very long lines, large replacements, startup recovery, and file saves can still pause this alpha. Periodic recovery I/O runs off the input thread; the performance report includes a recovery-enabled typing workload, while language-service/extension contention and the design document's latency budgets remain unqualified. Full bidirectional layout and terminal-independent emoji-width agreement are not implemented.

Not yet implemented: broad extension API compatibility, rich webviews, notebooks, full settings migration, or a remote agent. An optional experimental command extension host is available as described below. Running the executable inside an SSH session is supported in principle; the actual terminal/multiplexer combination must be tested.

Grammar highlighting uses a single background worker, document/revision checks, cancellation, and a 2 MiB source cap. During typing, unchanged text retains its previous grammar colors through a byte-offset mapping; inserted or replaced text receives fresh grammar colors when parsing completes. These temporary colors preserve geometry, but syntax-changing edits can make their old token classification provisional until the next parse. Other languages and larger files retain lightweight lexical colors. Embedded-language injection, semantic tokens, incremental parse-tree reuse, grammar folding and complete theme semantics remain incomplete. Native color-theme loading and its boundaries are documented in [Import and themes](IMPORT_AND_THEMES.md).

Color continuity uses at most 256 recent byte edits and 512 unchanged-text segments per visible document. If that history or segment budget is exceeded, the editor uses its lexical fallback until a fresh parse completes. Undo/redo use the same byte mapping and a monotonic edit generation, so an old reply cannot masquerade as a newer edit after undo. Failed parses retain available mapped colors rather than replacing them with an empty highlight result. Each revision receives at most three attempts, spaced at least one second apart after failure. Cold grammar/query setup has a separate five-second cancellation budget; parsing retains its 500 ms budget and 500,000-span limit. Cancellation retains the worker slot until completion, and grammar construction/parsing stay off the input/rendering thread. The bounded offset mapping runs during polling; rendering only performs lookups.

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

Use `--settings /path/to/settings.json` to select a user settings file for reading and native settings writes. By default, VSCLI reads `settings.json` beside its user keybindings file. An activated import supplies the native copy's user settings/keybindings/snippet directory unless explicit CLI paths override it. Workspace `.vscode/settings.json` is layered above user settings. Ctrl+, (Cmd+, on macOS) opens the user settings JSON file. Comments and trailing commas are accepted.

F1 → **View: Toggle Breadcrumbs** persists its root setting in the effective
user/workspace file, preserving JSONC comments and unrelated bytes. Dirty
settings buffers and winning language overrides refuse the write with a notice.
Background persistence and forced reloads keep filesystem work out of the
toggle's input handler. See [scope, bounds and evidence](SETTINGS_PERSISTENCE.md).

The current supported subset includes the native language-server controls above, `editor.tabSize` (1–16), `editor.insertSpaces`, and `editor.lineNumbers` (`on`, `off`, `relative`, `interval`). Indentation width applies to editing, cursor/mouse coordinates, rendering, and LSP formatting options. Language blocks such as `[python]` and `[javascript][typescript]` override general values; a single-language block takes priority over a multi-language block. Workspace values override user values within the same identifier group. Combined-language groups retain their first occurrence order across scopes; changing that order can change precedence. This follows the pinned [configuration model](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/platform/configuration/common/configurationModels.ts).

Settings reload in the background every two seconds. Malformed updates retain the previous configuration. Entries outside the native subset and invalid native values appear in F1 → Settings: Compatibility Report. A notice about an extension setting does not mean an enabled extension cannot read it. Native `off`/`afterDelay` autosave and opt-in native file formatting on explicit Save are implemented within their documented bounds. Automatic indentation detection, complete theme semantics, broader save participants, profiles, remote scopes, policies, and the broader settings catalog remain incomplete; importing those entries does not enable their behavior.

Enable `"editor.formatOnSave": true`, optionally inside `[cpp]` or another language block, to format an ordinary named visible model with its native language server before Save. `editor.formatOnSaveMode` supports `"file"` only. After-delay autosave skips formatting. Missing formatters, formatter errors, unsupported targets/options and the 1500 ms native save deadline fall back to saving current bytes with a notice. Escape cancels pending formatting; a still-running canceled callback retains actual formatter capacity until its reply or server retirement. Valid formatting is a separate Undo step; null or unchanged results add none. See [behavior, integrity checks and outstanding qualification](FORMAT_ON_SAVE.md).

Set `"editor.codeActionsOnSave": {"source.fixAll": "explicit", "source.organizeImports": "explicit"}`
to run eligible native edits before formatting on an explicit Save. Object entries
merge across settings scopes; language blocks apply. Legacy true means `explicit`
and false means `never`, excluding that kind and its descendants under enabled
ancestors. These executable interpretations preserve original JSONC and imported
settings files. After-delay autosave skips source actions. This first participant supports direct/lazy origin-only edits,
applies at most one mutating action per family, and skips command-bearing actions
in full with a notice. Each valid edit has its own Undo stage. See
[settings, cancellation, limits and qualification](CODE_ACTIONS_ON_SAVE.md).

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

## Native extension output, status and trees

An enabled extension can create read-only output channels, status items and
single-selection tree views. **F1 → Extensions: Output Channels** opens a
channel; arrow/Page keys scroll, End follows new output, and Escape closes the
panel. An extension's `show(true)` preserves focus; existing native prompts and
modals retain focus even for `show(false)`. Output is separate from documents,
saves and undo history, and can remain visible beside the integrated terminal.

**F1 → Extensions: Status Items** runs an item's command; visible command items
also respond to a mouse click in the status row. **F1 → Extensions: Tree Views**
opens a declared view. Right expands, Left collapses, Enter runs an item's
command (or expands), and Escape closes. Children load asynchronously on demand.
**R** retries failed child requests; provider change events refresh the whole
view. Native dialogs can open after a tree action. These discoverable commands
have no invented default shortcuts.

These surfaces also work without open editors and disappear when the host is
retired. Output history is bounded and oldest text is discarded; it is not
saved as a log file. Trees currently use a keyboard browser rather than VS Code's
full sidebar layout. See [surface scope, bounds and evidence](EXTENSIONS.md#native-output-status-and-tree-surfaces).

## Experimental extension commands

Install local extension packages with **F1 → Extensions: Install from VSIX**, or `vscli --install-extension ./package.vsix`. Use **Ctrl+Shift+X** (**Cmd+Shift+X** on macOS), or **F1 → Extensions: Show Installed Extensions**, to view compatibility descriptions, explicitly run a selected code package, restore its previous installation with **R**, or uninstall it with **Delete**. Installation alone never activates code. The picker also works from the empty welcome screen, Explorer and integrated terminal. Esc closes a pending loading view while its operation finishes; the result does not replace a newer prompt. CLI equivalents are `--list-extensions`, `--rollback-extension publisher.name` and `--uninstall-extension publisher.name`; `--extensions-dir` selects storage. Installed code packages can be launched with `vscli --extension publisher.name .`. Storage supports one previous generation, retains immutable files for running hosts, and does not yet collect old files. Installed packages are not automatically API compatible.

Use **F1 → Extensions: Search Open VSX** to find and install a compatible stable
package without command-line arguments. Enter submits a query, then installs the
selected displayed version. **/** opens search from the installed picker and
**U** checks for newer stable versions; **F1 → Extensions: Check for Updates** is
also available. Updates preserve rollback and leave running snapshots unchanged.
The native client works without Node. CLI alternatives are `--search-extensions`,
`--install-extension publisher.name`, `--check-extension-updates` and
`--update-extension publisher.name`. The default registry is Open VSX;
`--extension-registry` overrides it. See [registry scope](EXTENSION_REGISTRY.md).

`vscli --extension /absolute/path/to/unpacked-extension .` starts the extension's Node `main` entry in an optional process. This explicitly executes trusted extension code with your user permissions. Use `--extension-node /path/to/node` to select the runtime. Normal native editing does not require Node. Extensions receive imported user/workspace settings before activation and valid live updates through `workspace.onDidChangeConfiguration`; held configuration objects remain snapshots.

Registered commands appear in F1 with an `Extension:` prefix. Manifest keybindings retain their original combinations and platform overrides beneath user overrides/removals; unsupported context expressions are reported. Commands can also be assigned in user keybindings by their original IDs. Edits are version checked and undoable, and do not save files automatically. F1 → Extensions: Stop Host terminates the process while retaining native buffers. The initial host has substantial API and contribution limitations; read the [extension evidence and scope](EXTENSIONS.md) before using an extension.

Enabled command extensions can show a native single-selection Quick Pick or Input Box. Type to filter a pick or edit input, press Enter to accept, and Escape to cancel. Native prompts retain priority; stopping or replacing a host clears its prompts. Password, validation, multi-select and live Quick Input variants explicitly reject. See the [supported options, bounds and named workflow evidence](EXTENSIONS.md#native-quick-pick-and-input-box).


Extensions can read an existing file through `workspace.openTextDocument` without opening a tab, then display the same native document with `showTextDocument`. Hidden models retain identity, undo, file watching and dirty recovery. Global/workspace Memento state persists under the native configuration root, separately from imported profiles. A small explicit set of movement, selection and undo/redo commands can be delegated to the native editor. See [supported options, service bounds and qualification](EXTENSIONS.md#native-documents-commands-and-persistent-state); unsupported file schemes, preview tabs, save/task delegation and cloud state sync reject explicitly.

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


## Diagnostics and Problems

Native language servers and selected compatible extensions can report diagnostics
in the editor gutter. **Language: Problems** lists errors, warnings, information
and hints for open native models, including hidden and untitled documents. Enter
opens the selected document at the diagnostic position. A changed or closed
document invalidates that navigation row; reopen Problems to refresh it.

The optional host supports diagnostic collections and successful-save events.
Native editing remains usable without Node.js. Server and extension sources have
independent lifetimes, and bounded publication validation protects accepted state.
Version-qualified native results and mirrored extension notifications retain
document identity and text epochs, including across edit followed by Undo.
Unversioned LSP results and an extension's own delayed analysis have weaker
provenance. See [scope, limits and qualification](DIAGNOSTICS.md).

## Native code actions and quick fixes

With a ready language server or enabled extension action provider, **Ctrl+.** (**Cmd+.** on macOS) opens
**Language: Quick Fix**. **Ctrl+Shift+R** on all three platform profiles opens
**Language: Refactor**. The shortcuts and provider/editability context match the
pinned VS Code 1.95 inventories. The picker combines native and matching extension actions with source labels;
Enter applies the chosen action and Escape closes it. Disabled actions explain
why they cannot run. Servers can lazily resolve edits when an action is selected.
Diagnostic codes, related fields and opaque `data` are preserved in native action requests. Extension fixes also support original Diagnostic/action identities and lazy resolution. Unsupported extension command/resource rows explain their limits individually. See [bounds and qualification](CODE_ACTIONS.md).

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
timeout, further action requests and command callbacks remain fenced until its
exact terminal reply or language-server retirement; late replies release capacity
without authorizing edits. Closing the picker cancels only its own exact native
request. Unsolicited, expired or stale edits are rejected and acknowledged
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
no Node runtime is required. Quick Fix does not provision a language server; native automatic services above
start supported installed servers when a saved file is selected.

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
**Language: Parameter Hints** from a matching optional extension or native LSP.
Typing registered trigger characters also opens hints automatically, with a
120 ms debounce and registered retrigger/content-change contexts. The nonmodal
panel highlights the active parameter and displays inert plain-text documentation.
Up/Down cycles overloads locally; Alt+Up/Alt+Down remains available while completion
is open. Completion navigation and its first Escape take precedence. Escape or
Shift+Escape closes hints; typing and keyboard movement can retrigger active help.

`editor.parameterHints.enabled` and `.cycle` default to true and support language
overrides. Manual invocation remains available when automatic initial hints are
disabled. Save and unrelated retained-buffer edits preserve valid hints. Active
editor, settings, source and text-epoch checks reject stale replies. Invoking on
the empty welcome screen does not create a document.

See [parameter hint contracts, bounds and evidence](PARAMETER_HINTS.md) for the
independent actual-work lanes, original extension help-object revival, validation
of all overloads, UTF-16 labels and remaining qualification. Hints never edit/save.
Named native, synthetic extension and Unix PTY tests cover these workflows. The
opt-in real clangd checks can be run with:

```sh
cargo test --locked --test signature_help real_clangd -- --ignored
python3 tests/signature_hints_pty.py target/debug/vscli --real-clangd
```

This is native workflow evidence, not full signature-help parity with VS Code.

## Native document and workspace symbols

**Ctrl+Shift+O** (macOS **Cmd+Shift+O**) opens **Go to Symbol in Editor**.
**Ctrl+T** (macOS **Cmd+T**) opens **Go to Symbol in Workspace**. These defaults
and their contexts match the pinned VS Code 1.95.0 inventory. The LSP route requires a ready server advertising the relevant provider;
LSP document symbols additionally require a saved file handled by that server.
A matching selected extension can instead provide document symbols through the
same picker, including hierarchical symbols for untitled buffers.
Workspace symbol search also works from an empty welcome screen.

The native searchable picker displays names, kinds, containing symbols and file
locations. Document search filters one response locally; workspace queries wait
200 ms after input changes, cancel superseded requests and clear obsolete results.
Hierarchical `DocumentSymbol` uses `selectionRange`; flat `SymbolInformation` and
range-bearing `WorkspaceSymbol` use their location range. Range-less workspace
symbols requiring `workspaceSymbol/resolve`, non-file URIs, malformed ranges and
oversized responses produce explicit errors. Range-less resolve, symbol previews,
grouping modes and complete VS Code navigation behavior remain incomplete.
The separate [Outline section](OUTLINE.md) merged after its named local and
platform checks; [Breadcrumbs](BREADCRUMBS.md) also merged after all required checks.

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

## Outline

F1 → **Outline: Focus** enables the native tree in Explorer for this session.
There is no added default Outline shortcut. Up/Down select visible symbols,
Right/Left expand or collapse a node and traverse its children/parent, and Enter
reveals the identifier start as a collapsed caret. Escape returns to the editor
without navigation. **Outline: Collapse All**, **Expand All** and **Toggle Follow
Cursor** are also available in the palette. Follow Cursor starts enabled and
selects the deepest enclosing symbol; switching it off preserves manual tree
selection. Outline state is session-only.

A ready native server supplies the tree without Node. A matching selected
extension can provide document symbols through the optional host. Loading,
updating and unsupported trees cannot navigate; stale replies cannot revive after
edit/Undo. Focused native and extension publication checks, the named actual
clangd workflow, the narrow pinned provider/reveal comparison and three debug
terminal workflows passed locally. Optimized terminal workflows and all six
required platform checks also passed before merge. See
[bounds and scoped evidence](OUTLINE.md).

## Breadcrumbs

The active editor displays a native file trail and current enclosing symbols.
Use F1 → **Focus Breadcrumbs**, or Ctrl+Shift+Semicolon (Cmd+Shift+Semicolon on
macOS). Ctrl+Shift+Period (Cmd+Shift+Period) focuses the last crumb and opens its
symbol picker. Left/Right choose crumbs, Enter/Down opens symbol siblings, and
picker Enter reveals the identifier start. Escape returns to the editor. File
trails require no LSP or Node; symbol trails share Outline's guarded producer.
Folder/file dropdowns remain explicitly unsupported.

`breadcrumbs.enabled` defaults to `true`; `breadcrumbs.filePath` and
`breadcrumbs.symbolPath` accept `"on"`, `"off"` and `"last"`. **View: Toggle
Breadcrumbs** persists the configured profile's effective root setting through
[bounded native settings writes](SETTINGS_PERSISTENCE.md). Four pinned synthetic-provider cases
retain 31 snapshots and confirm three picker reveals; direct focus/reveal no-ops
remain raw qualification gaps. The narrow native comparison, five public
journeys, one actual clangd workflow, three new debug terminal workflows and the
existing 35 terminal checks passed locally. All three optimized workflows and
all six required platform checks passed before PR #61 merged. See
[scope and evidence](BREADCRUMBS.md); new settings-write qualification is underway.

## Clean-file session restoration

VSCLI records bounded clean-file layout metadata for each workspace under the native
configuration root. Reopening that layout is opt-in:

```sh
vscli --workspace ./project --restore-session
vscli --workspace ./project --no-session
```

`--restore-session` reopens the previous inactive instance's clean file-backed tabs,
ordered tabs in each group, active tab and group, nested split axes/weights, sticky
prefixes, per-group tab recency, and each membership's independent cursor/selections
and scroll positions,
including inactive historical tabs. Explicit
file arguments take precedence and suppress startup restoration. F1 → **File:
Restore Previous Clean Session** retries the previous session from an empty
workbench. This native command has no default shortcut. `--no-session` disables
session metadata reads and writes; dirty-buffer recovery and recent-file history
have independent controls. An ordinary empty launch still shows the welcome screen.

Files are read from their current disk contents; restoration never writes them.
Positions are line/character coordinates clamped to the current file, so edits made
outside VSCLI can change their meaning. Recovery runs first: existing buffers,
including independent dirty variants of the same path, retain their document
identity and selections. With recovered buffers present, clean restored files are
appended without replacing the recovered layout. Metadata contains paths and view
positions, never document contents, undo history, or snippet text. Unsaved and
untitled documents remain the responsibility of crash recovery and existing
Save/Discard dialogs. This feature does not implement VS Code hot exit.

A missing, unreadable, nonregular, malformed, or over-budget file rejects the whole
restore batch. No empty file is created; the previous session remains available
for retry. Interaction during loading cancels installation. After a failed or stale
restore, this instance leaves its saved layout protected until a successful retry
or an explicit close-all/last-editor close. Closing all editors intentionally
records an empty workbench. Save As uses the final saved path; cancelled Save As and
quit retain the live buffers. Missing-file and cancellation notices explain retry.

Storage is isolated by configuration root and canonical workspace directory, and
uses at most eight leased instance slots. Live instances are never evicted or used
as restore sources. At capacity, if reclaiming a slot would delete the only previous
saved session, session storage is unavailable for that instance and its previous
metadata remains intact. Import profile directories do not receive session state.
One worker handles metadata and file reads, with bounded/coalesced writes and atomic
publication. Failed writes preserve the previous complete snapshot. Shutdown waits
up to five seconds; a blocked filesystem operation may outlive that wait while its
worker retains the slot lease. Other instances cannot claim that live lease.

The schema-3 candidate stores a unique clean-file table, per-group membership views,
a bounded nested tree with integer weights and per-tab sticky flags. Capture omits
dirty/untitled memberships and prunes only their empty leaves on a clone; surviving
axes and weights remain. If every file is dirty, ordinary capture does not overwrite
the previous clean session with empty state. An explicit final close can do so.

Strict schema-1/2 slots remain readable using their original fields; injecting new
tree/sticky/preview fields into those schemas is rejected. Loading or failed migration
never rewrites the original metadata. New publication uses schema 3 after validating
and byte-counting the complete envelope before atomic replacement. A bounded old slot
can still be restored if its larger schema-3 encoding would exceed the write cap.
No live model/group/tab/split identifiers are trusted from disk; fresh restore stages
new identities, sticky prefixes, views and topology before publishing. Recovery keeps
its existing topology/views/modes; appending clean files never applies the saved tree
to authoritative retained buffers. Ratio-only changes retire an asynchronous restore
just as other guarded interactions do.

Limits: 32 distinct clean files, four groups, 32 saved tabs per group, 128 selections per view, 1 MiB metadata,
4 KiB per path, 32 MiB per file, and 128 MiB combined restored file data. Unsupported
state produces a notice rather than silent truncation. Recovery that exceeds the
available group-tab capacity retains all models in the legacy workbench with a
layout-unavailable notice and keyboard-editable active recovery view; source/tab
pointer targets are disabled there. Native Save still refuses more than 128 retained
models without dropping work or changing disk. Clean preview memberships restore
committed; transient preview modes are not persisted. Nested weights and sticky
prefixes are implemented in the schema-3 candidate, with fourteen new integrity tests
and clean nested-restart terminal checks passing locally. Terminal reconnection, extension state, recent-workspace
switching, dirty-content hot exit and full VS Code session parity remain outside this
slice. See [layout evidence and session boundary](EDITOR_LAYOUT.md).

Native integrity tests cover duplicate recovery paths, shared-view identity,
Unicode/CRLF edits after restart, Save/Discard/Cancel/Save As, close-all, missing-file
retry, stale requests, lease/config/workspace isolation, read budgets, FIFO rejection,
coalescing, shutdown deadlines, and injected failure before atomic publication.
`tests/session_restore_pty.py` exercises actual normal restarts, shared groups,
CLI-file precedence, disabled storage, missing-file retry, empty restart, and dirty
recovery collisions. Combined native tests and a terminal journey also qualify
restoration cancellation by a queued extension InputBox and a workspace-symbol
picker, safe explicit retry, and continued dialogs/symbols with `--no-session`.
A prompt opening invalidates restoration even if its owner closes it before the
file-loader reply arrives. Injected failure tests establish atomic old/new-file behavior;
they do not qualify arbitrary hardware power-loss or network-filesystem durability.

Remembered code activation is opt-in through native `vscli.extensions.enableGlobal` / `enableWorkspace` commands with an installed ID argument; matching disable commands revoke the corresponding scope. A workspace value overrides a global value. Explicit `--extension` and installed Enter run once without changing that state. See [execution grants, activation events and limits](EXTENSIONS.md#remembered-execution-enablement-and-supported-events) for a keybinding example. Installation and native editing require no Node runtime; eligible enabled code does. A command waiting for activation is canceled if its native document, selection, pane or focus changes; invoke it again after activation to use the current editor context.

In the installed extensions picker, **e/d** enable or disable the selected package globally; **E/D** (Shift+e/d) write a workspace override. The picker shows effective activation state; workspace values override global values. **Enter** runs the selected package once. Each dependency needs its own explicit grant.

Use `vscli --enable-extension publisher.name` to remember a global code grant without starting Node. `--disable-extension publisher.name --extension-scope workspace --workspace /path/to/project` writes a workspace override under the native user configuration root. Missing installed IDs and malformed existing state reject without replacing previous grants. These flags can be used without an interactive terminal.

## Native extension language providers

A running selected extension can supply **Language: Complete**, **Hover**, **Go to Definition**, **Find References**, **Format Document**, **Parameter Hints**, and **Go to Symbol in Editor**. Use the existing native commands and platform shortcuts (for example Ctrl+Space completion and Linux Ctrl+Shift+I formatting). The highest-scoring matching provider wins, with the newest registration breaking ties; if no extension matches, commands retain their native LSP route. Completion requests automatically after identifier typing or registered trigger characters; parameter hints use automatic triggers/retriggers and local overload navigation as described above.

Completion and formatting preserve the document's EOL convention, stage strict UTF-16 edits, and retain native save/undo. Definitions and references reuse dirty/shared buffers and load closed files asynchronously. Completion uses the nonmodal caret popup with Up/Down selection, Tab/Enter acceptance and Escape cancellation; native snippet placeholder Tab retains precedence. Symbols filter in the native searchable picker; parameter hints use the nonmodal native panel. Completion resolution and linked snippet fields with atomic import edits are supported. Completion commands, workspace symbol providers and provider aggregation remain unsupported. See [provider bounds, context guards and evidence](EXTENSION_PROVIDERS.md).

The empty workbench uses an original graphical VSCLI mark, clickable actions and recent files. Settings shows the active user JSON path; Ctrl+, opens it (Cmd+, on macOS), with workspace overrides in `.vscode/settings.json`. [Welcome rendering and terminal qualification](WELCOME.md) describes the high-resolution Kitty path and cell fallback.

## Automatic suggestions

A ready native language server or enabled active extension provider can supply suggestions while you type. The active matching extension provider takes precedence; without one, native LSP remains available. The popup stays at the active caret and input remains live. Up/Down select, Page Up/Down move eight rows, Tab or Enter accept, and Escape dismisses. Tab navigates native snippet placeholders first. User and imported conditional keybindings retain precedence and can use `suggestWidgetVisible`. Ctrl+Space requests suggestions explicitly.

Automatic requests use a 120 ms typing debounce and one latest queued editor context. Continued typing filters cached labels while a fresh reply is pending; the popup says “updating” and the old edits cannot be accepted. Tab then performs ordinary native indentation, and Enter performs its normal newline action. Cursor/focus changes, other commands, document edits, edit→Undo, source restart, or provider replacement invalidate prior replies. Native acceptance stages strict edits before one undoable transaction; files stay unsaved until you save.

Native settings, including user/workspace and language scopes, support `editor.quickSuggestions` (boolean, default `true`), `editor.quickSuggestionsDelay` (0–2000 milliseconds, default 120), `editor.suggestOnTriggerCharacters` (boolean, default `true`) and `editor.acceptSuggestionOnEnter` (`"on"` or `"off"`, default `"on"`). Context-specific quick-suggestion objects, `"smart"` Enter acceptance, provider aggregation, ghost text and AI inline suggestions remain unsupported. [Bounds and evidence](COMPATIBILITY.md) distinguish this subset from full VS Code IntelliSense behavior.

## Resolved completions and snippet fields

Selected suggestions can resolve additional documentation and import edits in the background.
Tab/Enter waits for the selected item to resolve; Escape cancels without editing.
Snippet completions select their first field; Tab/Shift+Tab traverse fields and repeated
fields update together. The insertion and its additional imports share one native Undo.
A Details panel shows inert documentation beside the list on wide editors, underneath
on medium layouts, and is omitted in small editor areas. Filtering ranks exact and
prefix matches, then token-boundary abbreviations and other subsequences.
See [completion contracts and limits](COMPLETIONS.md).


## Preview tabs and Keep Editor

Explorer left clicks open previews when `workbench.editor.enablePreview` is true
(the default). Accepted Ctrl+P results are committed by default; enable previews
there with `"workbench.editor.enablePreviewFromQuickOpen": true` in user or
workspace settings. These are root workbench policies; language overrides are
ignored with a compatibility notice. Explorer Enter/Right, Ctrl+O, CLI files,
history, symbols, settings and Untitled documents open committed.

Preview filenames are italic. Opening another eligible clean preview replaces the
previous one in that group. Editing or original Ctrl+K then Enter (Cmd+K then
Enter on macOS) keeps the tab permanently; F1 → **View: Keep Editor** does the
same. Undo to clean text preserves commitment. Keep Editor does not sticky-pin a
tab. Pending saves, Save As and close/file/settings operations retain protected
models; rejected capacity/ownership never drops unsaved work or Redo. Disabling
preview keeps existing tabs. Clean session restore commits historical previews.
See [behavior and qualification](EDITOR_GROUPS.md#preview-callers-keep-editor-and-retained-work)
for transient-mode, mouse and remaining layout limits.


## Sticky tabs and protected close (candidate)

F1 → **View: Pin Editor** keeps the current tab in its group's leading sticky
section. **View: Unpin Editor** returns it to the ordinary section. The original
Ctrl+K then Shift+Enter chord toggles those actions (Cmd+K then Shift+Enter on
macOS). Ctrl+K then Enter remains **Keep Editor**: it commits a preview without
making it sticky. Sticky tabs show **◆**; the modified **●** indicator remains
independent. Pinning belongs to one group membership, so another group showing
the same document can remain unpinned.

The root user/workspace setting `workbench.editor.preventPinnedEditorClose`
defaults to `"keyboardAndMouse"`. `"keyboard"` also protects original keyboard
Close; `"never"` and `"mouse"` allow it. A protected Close focuses a nonsticky
recent editor in the current group, then another group, and leaves every tab
open; if none exists it does nothing. F1 → **View: Close Pinned Editor** deliberately
closes the current tab through the normal dirty Save/Discard/Cancel flow.
Language overrides of this workbench policy are ignored with a notice.

Close All Editors and Close Editors in Group select nonsticky memberships.
Already accepted saves retain their exact destination and publish their receipt
before any eligible close; later unrelated tabs are not swept into a batch.
Session schema 3 restores clean historical tabs committed and preserves each group's
saved sticky prefix; older schemas restore nonsticky. Dirty/untitled recovery remains
independent. Mouse close gestures and graphical sticky-row layouts remain outside
this candidate. Schema-3 integrity and session checks pass locally; platform
qualification remains pending. See [scope and evidence](STICKY_TABS.md).
