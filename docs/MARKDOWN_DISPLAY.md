# Native documentation display

The native documentation model retains Markdown versus plaintext metadata for completion documentation, signature and parameter documentation, and hover blocks. It projects Markdown to inert terminal text when a result is accepted, then paints that retained text. Plaintext stays literal. Source produced by MarkdownString.appendText displays spaces and escaped punctuation instead of literal `&nbsp;`/backslashes; fenced and inline code retains its literal content, including those sequences. Links and images contribute labels only, without actions, images or URLs.

This is a bounded terminal subset, with common escapes, six named entities, safe numeric references, basic emphasis/strikethrough, headings, quote markers, inline links/images and fenced spans. It conservatively keeps indented code literal. It supports neither the whole CommonMark grammar nor graphical Markdown styles, tables, references, full named entities, HTML rendering, trusted command links, image fetching, theme icons, alert UI or base-relative URI actions. Unsupported HTML/tag syntax remains inert text. Code tabs and terminal controls retain the existing terminal sanitization behavior. Code span bytes are preserved rather than applying CommonMark whitespace normalization; conservative indentation and simplistic emphasis/link grammar are explicit adaptations.

Source and output are bounded at 64 KiB, with stricter existing completion/signature source limits (32 KiB and 8 KiB per part). Hover admits at most 32 parts and 64 KiB in aggregate. Work and inline/link depth are capped; complex Markdown that exceeds the work budget rejects publication without mutating editor data. Completion's existing 2 MiB retained budget includes the new display string. Signature's source envelope stays 256 KiB and its new rendered strings have a separate 256 KiB aggregate limit.

`MarkdownString` now delegates through the source-derived private value class,
with upstream constructor defaults, mutable property references, exact
appendText escaping/spacing/CRLF handling, appendMarkdown coercion and code
fences longer than embedded line-leading backticks. The shared ES5 constructor
helper preserves legacy subclasses and private initialization. Public values
have no invented size cap; provider/signature/diagnostic boundaries keep their
independent byte budgets, trust/HTML restrictions and cheap string-length
preflight. [Source receipts](../extension-host/markdown-string.provenance.json)
pin Microsoft VS Code 1.141.0 at
`2a59476c9bfcb90b3ddc372c36762471b7dfad1c`; its MIT attribution is retained.

Local executed evidence: seventeen MarkdownString tests and all 349 host tests
pass. Thirteen projection/model/lifecycle regressions and all 826 native library
tests pass, with twenty language-server/completion/signature integration tests
(one separately ignored case is not qualification), formatting, strict locked
all-target Clippy and a native build. The actual optional-extension terminal
fixture passes for source-correct appendText/appendCodeblock in hover,
completion details and signature hints, literal plaintext/code, inert command
links, unchanged dirty Unicode/CRLF work, Save/Undo and terminal restoration.
The existing seven-provider terminal journey also passes. Source comparison
and local tests establish this subset; full upstream runtime/platform comparison
and the renderer features listed above remain outstanding. No local VS Code
instance or CI investigation was used.
