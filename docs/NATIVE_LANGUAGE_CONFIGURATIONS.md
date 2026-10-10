# Native installed language configurations

Installed packages can contribute native editing data through
`contributes.languages[].configuration`. Loading this data does not start Node,
activate an extension, execute its entry point, or grant permission to run code.
The native editor stays usable when Node and language servers are unavailable.

Declarative data is enabled by default for installed packages. An explicit
workspace enable/disable preference overrides the global preference; an explicit
global preference overrides the default. Code execution keeps its separate,
default-disabled grant. The installed-extension picker distinguishes native data
publication from code activation. Uninstalled packages do not supply native data,
even when their immutable files remain available to a running compatibility host.

## Supported data and commands

The initial loader supports single-scalar `autoClosingPairs`, `surroundingPairs`
and `brackets`, a bounded `autoCloseBefore` character set, and line/block comment
delimiters. Conditional pairs support `notIn: ["string", "comment"]` using the
existing native lexical proof where that profile is supported. Imported comments
and pair tables do not install a tokenizer or make unknown context trustworthy.
Guarded automatic pairing falls back conservatively when context is unproved.

Existing editor settings still control automatic pairing, surrounding, generated
closer deletion/overtyping, and indentation. A declared following-character set
does not prevent pairing at a physical line ending. Generated ownership remains
view-specific and requires a current configuration; pasted or manually loaded
delimiters acquire no generated ownership.

Configured comments use the original commands: Toggle Line Comment,
`editor.action.addCommentLine`, `editor.action.removeCommentLine`, and Toggle
Block Comment. Linux defaults include Ctrl+K Ctrl+C, Ctrl+K Ctrl+U and
Ctrl+Shift+A for block comments; the other keyboard profiles retain their own
bindings. Comment edits stage replacements and selection endpoints before
mutation, with one Undo transaction. Their inspection is bounded independently
of catalog loading.

## Installed JSON composition

The loader accepts bounded JSONC. Installed empty pair arrays, an empty comments
object, and an empty `autoCloseBefore` string are omitted, matching the pinned
installed-file parser. Omitted fields retain an earlier field or native fallback.
A valid partial comments object replaces the whole earlier comments field: a
line-only object does not retain an earlier block delimiter.

Malformed pair entries are skipped individually with warnings; supported valid
peers remain. If all entries are malformed, the field is omitted. Native-valid
but unsupported forms, such as multi-character automatic pairs or an unsupported
conditional guard, produce warnings. Supported peers remain available; when no
supported peer survives such an unsupported table, native handling of that table
is disabled conservatively. This fallback is an explicit compatibility boundary.
Empty typed configuration tables in the Rust API can express disabling; that is
distinct from an empty installed JSON array.

Packages compose in ascending package-ID order, then manifest contribution order,
with later present fields winning. Collisions produce warnings. This deterministic
native order is a documented policy; arbitrary VS Code extension registration
timing is not reproduced. Stable identity includes package owner, version,
installed generation path, archive hash, configuration path and content hash,
plus a digest of ordered contributing sources and effective fields.

Reloading identical sources preserves ownership. Changing source or effective
configuration retires generated marks in every shared view. Update, rollback,
disable, uninstall and language-changing Save As cannot revive retired ownership
through Undo/Redo. Native document identity, unsaved content and text history
remain independent of configuration publication.

## Work and filesystem bounds

Catalog loading runs on the existing bounded background activation worker;
rendering and typing read immutable catalogs and perform no configuration file
reads. Results require current workspace, generation and control proofs before
publication. Stale results cannot restore an earlier preference or source.
An unsuccessful current refresh clears native bindings to ordinary fallback.

Loading inspects at most 128 installed packages and 256 language contributions,
64 KiB per file and 4 MiB of actual reads across the catalog, including repeated
references and failed parses. JSON nesting is limited to 64 levels. Each pair
table is limited to 64 entries. Following-character sets allow at most 64 Unicode
scalars/256 UTF-8 bytes; comment delimiters allow at most 256 bytes each. The
catalog retains at most 512 bounded warnings. Optional typing decisions retain
the existing [native work and integrity bounds](SMART_TYPING.md).

Configuration paths must remain within the installed package and resolve to
ordinary files. Absolute paths, parent traversal, escaping symlinks, directories
and special files are rejected. Unix opens use nonblocking/no-follow flags and
verify the opened file, avoiding FIFO blocking. These checks do not establish
protection against all hostile concurrent filesystem mutations or a latency
guarantee.

## Evidence and remaining qualification

Catalog and native App tests cover default declarative loading without a code
grant, preference precedence, confined/bounded reads, source composition,
background publication, source retirement, and Unicode/CRLF persistence and
history. Local qualification passed 533 ordinary Rust tests across 35 suites; 18 opt-in
integration tests remained ignored. Formatting and strict all-target Clippy
checks passed. Four comparator integrity tests reject malformed source sets before
any fixture writes. `tests/native_configuration_pty.py` passed five terminal sessions
(four reports), covering original terminal commands and physical input with an
empty executable PATH and a missing Node executable. All six required checks
passed for the reviewed head of [PR #58](https://github.com/vscli/vscli/pull/58),
including fresh pinned-editor captures and native comparison on Linux, macOS and
Windows in [CI run 38018064477](https://github.com/vscli/vscli/actions/runs/38018064477).
The merged source includes short macOS reference-profile paths and exact-byte
fixture checkout on Windows. The optimized build also passes
these five sessions and the seven smart-typing terminal workflows. The
[published benchmark](PERFORMANCE.md#native-language-configuration-baseline-2026-10-10)
has mixed ordinary-typing results and establishes no overall speed ranking.

The separate pinned VS Code 1.95.0 installed-configuration observer has captured
52 cases and 208 snapshots across ten isolated fixture profiles. Local native
differential comparison passed the 50 C++ cases and 200 snapshots; two named
unknown-language cases and eight snapshots remain raw-reference evidence only.
An additional comparison passed 21 workflows and 84 snapshots using the unchanged
bundled C++ declaration. The observer and comparison retain source, configuration
and capture provenance. Named fixture behavior does not establish whole-package
compatibility or full language parity.

New file/language associations, extension lexical grammars, runtime JavaScript
`setLanguageConfiguration`, multi-character automatic delimiters, imported
indentation/on-Enter regular expressions, word patterns, folding configuration
and broader language behavior remain outstanding. Bundled C++/JSON advanced
indentation remains the separate [existing contract](ADVANCED_INDENTATION.md).
