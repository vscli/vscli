# Native snippets

The native template engine in `src/snippet.rs` stages expansion and Unicode
scalar placeholder ranges without mutating a document or starting a JavaScript
runtime. It parses numbered tab stops, linked defaults, nested placeholders,
choices, escapes, variables/defaults, and transform metadata. First nonempty
defaults populate matching numbered occurrences; zero is the final stop.

Native document sessions now support multiple insertion ranges, linked fields,
forward/backward traversal, transforms on leaving a field, cancellation, and
undo/redo. A separate user-command insertion entry point preserves cursor order
and adjusts nested template indentation; the API entry point follows the pinned
editor's fragment-level indentation and file-order insertion behavior. Both
normalize inserted line endings to the document. Snippet state belongs to its editor view; rendering another view does
not activate a copied session. Edits map the shared text and other views, and a
transform does not merge another view's edit into its undo transaction.

Literal `editor.action.insertSnippet` commands with `args.snippet` now insert
native templates through user keybindings. Tab/Shift+Tab traverse active fields;
Escape/Shift+Escape leave snippet mode while retaining the primary selection.
Leaving a field through cursor movement cancels the session permanently. The
original conditional bindings are layered before user overrides. An actual
PTY workflow covers linked Unicode typing, navigation, cancellation, CRLF saves,
and undo/redo; physical terminal/platform qualification is still incomplete.

Clipboard-dependent commands use one outstanding background read. A reply is
applied only while the document revision, selections, pane and editor focus still
match the request. A failed system clipboard read falls back to the editor's
internal clipboard, as its current paste path does. The existing external tools
cover Linux/macOS; Windows system clipboard integration remains incomplete.

This is a partial snippet feature. Calls without `args.snippet` report that catalog
selection is not implemented. User/workspace snippet files, name/language lookup,
choice presentation, extension contributions, nested session merging and
completion integration remain pending. LSP still advertises `snippetSupport:
false`; the optional extension API does not yet provide `TextEditor.insertSnippet`.
See [usage](USAGE.md#snippets) for a literal-template binding.

## Evidence

The pinned VS Code 1.95.0 harness captures 34 named snippet cases (33 API calls
and one user command). They cover initial text and scalar selections, with later observations
for linked typing, nested traversal, transforms on leaving a placeholder, and
undo/redo. The native document session is compared with every captured text and
selection observation, including zero-width neighboring fields, Unicode
transforms that change length, and undo while a nested field has been removed.
These calls exercise the native document API, not terminal key delivery or LSP
completion. They do not qualify every possible snippet or editing workflow.

Run the local expansion comparison after the reference harness:

```sh
cargo run --locked --example snippet_contract -- target/vscode-reference/result/snippets.json
```

The same comparison runs against the real pinned editor in Linux/macOS/Windows
CI. All 96 observations matched on all three platforms in
[CI run 37854940336](https://github.com/vscli/vscli/actions/runs/37854940336).
The [saved traces and provenance](../tests/vscode-reference/baselines/1.95.0/snippets/provenance.json)
record the source revision, fixture hashes, and platform results from that run. Unit tests
also verify nested occurrence ownership, multiple insertion points, split-view
identity and undo ownership, exact Unicode/CRLF saves, and rejection of
oversized/deep or exponentially amplified expansions before any document
mutation. A failed transform leaves text, selections and revision unchanged.

## Insertion context

An additional 34 cases (118 observations) compare multiline insertion with the
actual pinned editor. Cases cover spaces/tabs, indentation at each insertion
point, nested defaults and choices, Unicode, LF/CRLF conversion, primary and
secondary selection order, typing across fields, and undo/redo. API and user
command entry points are captured separately; the user command is now connected
to terminal dispatch, while the extension insertion API remains unfinished. The first Windows CI comparison exposed the platform default for a new empty
document: CRLF on Windows versus LF on Unix. Native documents now use that
default when the initial text has no line breaks. Offline fixtures preserve
the platform difference. All 118 observations matched on each platform in
[CI run 37857315175](https://github.com/vscli/vscli/actions/runs/37857315175).
The [insertion traces and provenance](../tests/vscode-reference/baselines/1.95.0/snippet-insertion/provenance.json)
record all three captures, source revision, and fixture hashes from that run.

The reference API and user command differ: API insertion sorts ranges by file
position and adjusts only fragment-level text, whereas the command preserves
selection order and adjusts nested text. Model EOL normalization can leave the
reference's placeholder offsets based on the unnormalized template, particularly
inside API defaults or choices. The native implementation preserves the observed
UTF-16 offset mapping, including the reference's CRLF boundary clamping. This is
an explicit pinned behavior, not a promise of ideal placeholder placement for
those templates. Offsets that split a surrogate pair cannot have complete
selection/edit equivalence in the native scalar-position model; further Unicode
qualification remains required.

Native integrity tests cover multiline CRLF saves, shared views, selection/undo
restoration, and rejecting indentation amplification before mutation. The
existing typing timeout still applies across Tab navigation; timing and all
possible undo grouping sequences are not qualified by these immediate traces.

## Outstanding compatibility

The native environment resolver supplies document/selection/cursor values,
file/workspace paths, a snapshot of local date/time, per-occurrence random values
and UUIDs, supplied clipboard text, and built-in comment tokens. Resolvers run
against each original insertion cursor before mutation. Thirty additional pinned
cases (59 observations) cover per-cursor context, selected-text indentation,
plain-text word lookup, ten language identifiers, cancellation, and leaving and
returning to a field. All 59 observations matched on Linux/macOS/Windows in
[CI run 37859573471](https://github.com/vscli/vscli/actions/runs/37859573471).
The [saved traces and provenance](../tests/vscode-reference/baselines/1.95.0/snippet-variables/provenance.json)
record the three captures and source/fixture hashes. A subsequent Linux run
exposed asynchronous reference language loading: shell comments were still
unavailable when the snippet ran. The harness now reads the pinned package's
language declarations and waits for a separate comment command to work in a
scratch document before collecting language-dependent traces. It fails on a
readiness timeout; it does not retry snippet results or install replacement
language configurations. File/path/date/random and
clipboard-spread behavior currently have native tests, not full differential
qualification. Language-specific/custom word patterns, extension language
configuration, localized names, overtyped selections, and OS clipboard adapters
remain separately qualified work. Native untitled file names also differ from
VS Code URI labels. API insertion and user snippets have separate
parsing entry points: unknown variables in the API fixture become empty or use
their default; user preprocessing turns unknown bare variables into numbered
editable placeholders. Broader whitespace/EOL qualification, nested snippet insertion,
choice presentation, partial edits crossing marker boundaries, and large
cursor sets still need implementation or broader qualification.

Transform execution currently uses Rust's bounded regex engine, not a complete
ECMAScript implementation. Lookaround/backreferences, sticky matching, UTF-16
regex semantics, invalid-pattern fallback, and complete format-string escaping
need further implementation and differential qualification. No general regex
or snippet parity claim follows from the captured cases.

Parsing is limited to 64 KiB of source, 64 nesting levels and 10,000 markers;
expansion is limited to 1 MiB and 10,000 occurrences. These limits bound native
work and prevent recursive defaults from exhausting memory. Completion/extension
integration must preserve atomic rejection, request revisions, shared document
identity, independent views, and grouped undo while exposing these limits.

The syntax and initial semantics were checked against the [official guide](https://code.visualstudio.com/docs/editing/userdefinedsnippets)
and the MIT-licensed [pinned parser](https://github.com/microsoft/vscode/blob/1.95.0/src/vs/editor/contrib/snippet/browser/snippetParser.ts).
Reference observations come from the actual editor, not a reimplementation of
its parser.
