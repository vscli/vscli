# Native snippets

The native template engine in `src/snippet.rs` stages expansion and Unicode
scalar placeholder ranges without mutating a document or starting a JavaScript
runtime. It parses numbered tab stops, linked defaults, nested placeholders,
choices, escapes, variables/defaults, and transform metadata. First nonempty
defaults populate matching numbered occurrences; zero is the final stop.

This is an implementation layer, not a completed editor feature. Interactive
sessions, Tab/Shift+Tab routing, choice UI, user/workspace snippet files,
extension contributions, and completion integration remain pending. LSP still
advertises `snippetSupport: false`. The optional extension API does not yet
provide native `insertSnippet`.

## Evidence

The pinned VS Code 1.95.0 harness captures 30 named `TextEditor.insertSnippet`
cases. They cover initial text and scalar selections, with later observations
for linked typing, nested traversal, transforms on leaving a placeholder, and
undo/redo. Native expansion currently compares **initial observations only**;
recording a later interaction is not evidence that VSCLI implements it.

Run the local expansion comparison after the reference harness:

```sh
cargo run --locked --example snippet_contract -- target/vscode-reference/result/snippets.json
```

The same comparison runs against the real pinned editor in Linux/macOS/Windows
CI. The committed Linux trace initially comes from a local isolated reference
run; cross-platform qualification is pending until those jobs pass. Unit tests
also verify nested occurrence ownership and rejection of oversized/deep or
exponentially amplified expansions before any document mutation.

## Outstanding compatibility

Variables are supplied by the caller; editor/file/selection/date/clipboard
resolvers are not connected yet. API insertion and user snippet commands need
separate contracts: unknown variables in the API fixture become empty or use
their default, whereas user snippet preprocessing has additional behavior.

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
