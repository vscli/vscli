# Native smart typing

This slice adds generated bracket/quote pairs, surrounding selections, generated
closer overtyping/deletion, and language-aware Enter to the Rust document engine.
It works without Node, an extension host, syntax highlighting, or a language server.
C/C++ and JSON/JSONC have bundled, explicitly scoped editing profiles.

Ordinary Enter moves into the new indented line. The explicit `lineBreakInsert`
command inserts a line break while keeping the caret at the selection start,
matching the separately observed VS Code command. Existing CRLF files retain CRLF.
Paste, bulk document edits, and completion text remain literal. Punctuation in an
active linked snippet remains literal and linked; its Enter uses basic indentation.

## Settings

The existing user/workspace/language override precedence applies to these keys:

| Setting | Accepted values | Default |
| --- | --- | --- |
| `editor.autoClosingBrackets` / `editor.autoClosingQuotes` | `always`, `languageDefined`, `beforeWhitespace`, `never` | `languageDefined` |
| `editor.autoClosingDelete` / `editor.autoClosingOvertype` | `always`, `auto`, `never` | `auto` |
| `editor.autoSurround` | `languageDefined`, `quotes`, `brackets`, `never` | `languageDefined` |
| `editor.autoIndent` | `none`, `keep`, `brackets`, `advanced`, `full` | `full` |

The five modes are distinct. `none` adds no Enter indentation; `keep` retains the
current indentation; `brackets` adds bracket Enter rules. `advanced` also applies
the bundled C++ unbraced-control-body Enter rule; `full` adds JSON indentation
inheritance and closing-line rules. Indentation uses visual tab stops and the
configured spaces/tabs. Single-cursor electric closing alignment operates in
every mode, as separately observed in the pinned editor. JSON full indentation
can also adjust a complete multi-cursor cohort; mixed unsupported cohorts fall
back together. See [advanced indentation](ADVANCED_INDENTATION.md) for scope.

Unsupported language profiles retain ordinary literal typing/basic indentation;
bundled highlighting alone does not imply a smart-typing profile.

Generated-pair ownership distinguishes typed pairs from pasted/manual delimiters.
Undo/Redo, shared views, successful Save As, settings/profile transitions, and
touching delimiters preserve or retire that ownership explicitly. An effective
typing-settings change retires existing marks and breaks typing grouping; Undo
does not revive those retired marks. Failed validation or failed Save As leaves
the prior document and ownership intact.

The complete cursor/selection set is staged before mutation. Mixed selected/empty
sets and over-budget cases take a whole-gesture literal fallback. Adjacent
selections surround independently; surrounding a reversed selection produces the
observed forward selection, while Undo restores the original direction.

## Context and work bounds

Restricted quote/JSON rules use a fresh Rope-based lexical proof, independent of
asynchronous colors. C/C++ context includes strings, characters, comments,
continued line comments, and raw strings. Unrestricted C++ bracket rules can
operate without scanning a cold document. Each gesture scans at most 64 KiB for
context, with at most 10,000 probes; an unproved context falls back conservatively.
At most 512 checkpoints (bounded metadata of 64 KiB) and 4,096 live generated-pair
marks per view are retained. Replacement and document integrity limits still
apply. These are work/data bounds, not a universal typing-latency guarantee.

## Evidence and remaining scope

The actual pinned VS Code 1.95.0 executable produced 18 named C++/JSON typing
traces on Linux. The [reference harness](../tests/vscode-reference/README.md)
records its product commit, observer/case/trace hashes, and every observed text
and selection. The native `typing_contract` runner compares every recorded step;
fresh per-platform comparisons run in CI. These document/command observations
are separate from physical keyboard delivery.

`tests/smart_typing.rs` covers staged Unicode/CRLF multi-cursor edits, generated
versus manual delimiters, shared identity/history, option/profile transitions,
failed edits/persistence, and bounded large-document context. Unix
`tests/smart_typing_pty.py` exercises native terminal keys, save/Undo/Redo and
restored terminal modes with an empty executable PATH.

Installed [native language configurations](NATIVE_LANGUAGE_CONFIGURATIONS.md)
now contribute bounded single-character pairs, surrounding tables, following
characters and comment delimiters. Stable source identity guards generated
ownership across updates, disable, uninstall and shared views. These declarations
reuse existing lexical proof; they do not import extension grammars.

Rust/Python/JavaScript context, runtime configuration registration,
multi-character automatic delimiters, imported regular-expression rules,
additional language behavior and full VS Code typing parity remain outstanding.
The reference cases qualify their named gestures, not all editor settings or
language rules.
