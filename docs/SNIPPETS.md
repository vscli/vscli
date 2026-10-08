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

This is an implementation layer, not a completed editor feature. Terminal
Tab/Shift+Tab routing, choice UI, user/workspace snippet files,
extension contributions, and completion integration remain pending. LSP still
advertises `snippetSupport: false`. The optional extension API does not yet
provide native `insertSnippet`.

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
command entry points are captured separately; neither route is connected to
terminal commands yet. The first Windows CI comparison exposed the platform default for a new empty
document: CRLF on Windows versus LF on Unix. Native documents now use that
default when the initial text has no line breaks. Offline fixtures preserve
the platform difference; all three CI platforms rerun the comparison, with
final cross-platform provenance to be recorded after those jobs pass.

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

Variables are supplied by the caller; editor/file/selection/date/clipboard
resolvers are not connected yet. API insertion and user snippets have separate
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
