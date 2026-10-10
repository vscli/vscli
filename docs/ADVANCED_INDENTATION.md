# Native advanced indentation

The document engine distinguishes all five `editor.autoIndent` modes, defaulting
to `full`. The existing user/workspace/language settings precedence applies.
C/C++ and JSON/JSONC use bundled native rules; editing needs no Node, syntax
worker reply, or language server.

Enter retains no extra indentation in `none` and retains current indentation in
`keep`. Bracket-aware Enter starts in `brackets`. C++ `advanced` and `full` also
outdent an unbraced body after an immediately preceding `if`, `else`, `for` or
`while` header. The rule is textual: comment bodies can match, empty/braced/nested
`if` bodies can exclude it, and an intervening blank physical line stops it.
The native predicate preserves the bundled rule's ECMAScript whitespace and
ASCII word-boundary behavior. This is a specific bundled rule, not a C++ parser.

JSON `full` applies bundled increase/decrease rules and indentation inheritance.
For example, Enter before an existing `}` after a property can outdent its new
line; `advanced` retains the property's indentation. Existing whitespace before
the tail is consumed as part of the same staged transaction.

Typing a code closing bracket as the first content on a line can align it with
an opening bracket on an earlier line. This electric behavior is independent
of the Enter indentation mode. It requires a single cursor for C++; JSON `full`
also supports a complete matching multi-cursor cohort through its indentation
rules. A mixed JSON cohort takes the observed whole-gesture literal fallback.
Generated-closer overtyping wins before alignment; pasted/manual closers do not
acquire generated ownership.

Visual indentation follows tab stops and emits configured tabs/spaces. For
example, width six with tab size four outdents to width four. Ordinary Enter moves
the caret into the new line; `lineBreakInsert` keeps its original position.
Physical typing can coalesce a following electric edit with Enter. Explicit
ungrouped character insertion preserves separate Undo transactions.

## Integrity and work bounds

One gesture stages every replacement and final selection before modifying text.
Expanded ranges must pass overlap and size validation. Rejection preserves
text, epoch, history and generated ownership; shared document identity survives
successful edits, Save and Undo/Redo. Interior CRLF positions receive conservative
handling rather than indexing beyond the line's content.

Current Rope-derived lexical proofs share the existing epoch/profile-guarded
checkpoint cache. A gesture's indentation session has a 64 KiB scan budget that
also charges line reevaluation, at most 128 cached lines and 4,096 inspected
bracket tokens. Prefix scanning and repeated multi-cursor rule evaluation share
the budget. Unproved optional decisions fall back to ordinary insertion/base
indentation. Ordinary indentation reservation can reject an oversized gesture
before mutation. Existing 10,000-cursor, 4 MiB replacement and 32 MiB document
limits still apply. These bounds do not establish a latency guarantee.

## Evidence and qualification

The separate [reference observer](../tests/vscode-reference/README.md) captures
actual pinned VS Code 1.95.0 text/scalar-selection traces in five fresh processes,
one per fixed startup indentation mode. It records exact effective settings,
independent Enter and lexical scratch witnesses, and unchanged retokenization
before each typing/line-break gesture. Undo/Redo is observed uninterrupted.
Native comparison checks every recorded step, case identity and ordering.
The committed Linux capture contains 132 cases and 530 matching snapshots,
134 unchanged target preparations, 20 prepared positive/negative scratch proofs
and 131 single-gesture Undo/Redo witnesses.

This is a prepared-token contract. The [30-target natural-versus-prepared capture](reference/2026-10-10-cpp-token-readiness/README.md)
records a real startup distinction: early natural C++ electric typing can retain
indentation while prepared or later typing aligns it. The native engine uses
fresh bounded Rope proofs; reproducing that startup timing behavior is outside
the comparison. The archived scratch-readiness limitation is documented too.

Document integrity tests, settings/physical-Enter journeys and
`tests/advanced_indentation_pty.py` qualify Unicode/CRLF persistence, visual tabs,
generated-closer precedence, native operation with an empty PATH, and exact
Undo/Redo. The full native suite passes 496 tests (18 opt-in tests remain ignored),
alongside formatting and strict all-target Clippy. All ten new terminal sessions
pass, as do the existing 35 terminal smoke workflows, seven smart-typing terminal
workflows and 18-case typing reference comparison. [PR #57](https://github.com/vscli/vscli/pull/57)
is merged after all six required checks passed, including fresh prepared-token
comparisons on Linux, macOS and Windows. Physical terminal workflows run on Unix;
the Windows document comparison does not qualify Windows terminal delivery.

The fresh optimized build also passes all ten new terminal sessions and seven
existing smart-typing workflows. An interleaved [plain-text performance check](PERFORMANCE.md#advanced-indentation-baseline-2026-10-10)
preserves all 20 successful trials and mixed latency results; it establishes
neither an overall speed ranking nor indentation-rule performance.

Extension language configurations, other language profiles, arbitrary Enter
actions/regular expressions, broader C++/JSON syntax, multi-character pairs and
all VS Code typing behavior remain outstanding. Source inclusion, a successful
named workflow and full language parity remain separate claims.
