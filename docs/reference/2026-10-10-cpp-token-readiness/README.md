# C++ electric typing and token readiness

These are actual Linux observations from VS Code 1.95.0, product commit
`912bb683695358a54ae0c670461738984cbb5b95`. Each of five fresh processes starts
with matching global and `[cpp]` `editor.autoIndent` settings, covering `none`,
`keep`, `brackets`, `advanced` and `full`. Each captures six targets once.
Observer, fixture, trace, runner and worker SHA-256 hashes were checked before
archiving these exact bytes. Per-mode provenance also records the installed
language configuration, dependency/readiness hashes and independent witnesses.

| Target preparation | Observed result in every mode |
| --- | --- |
| Early natural target | Four spaces retained before typed `}` |
| Public `editor.action.forceRetokenize` | Closing brace aligned with opener |
| Model tab-size change 4→5→4 | Closing brace aligned with opener |
| Later natural target | Closing brace aligned with opener |

Every target has independent plain-Enter mode witnesses before and after it,
positive quote-pair and negative comment-quote readiness witnesses, and exact
Undo/Redo text and scalar-selection observations. Preparation checks preserve
text, version and selections. The target result never controls a retry.

The archived negative scratch probe does not independently establish grammar
readiness: cheap-token suppression can also produce a single quote. The observed
electric targets contain code without comments or strings, and their natural
versus prepared results remain recorded. The later native-comparison observer
strengthens both scratch probes with explicit unchanged retokenization.

The [pinned electric implementation](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/editor/common/cursor/cursorTypeEditOperations.ts#L388)
checks whether the target line is cheap to tokenize before forcing its tokens.
Together with the [token readiness implementation](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/editor/common/model/textModelTokens.ts#L131),
the observations support a token-readiness explanation. That explanation is an
inference; the tab-size arm also allows background work and does not isolate
cursor-configuration recreation. There is no observed `none`-specific policy.

These records qualify this timing distinction on one Linux installation. They
do not establish startup behavior on other platforms or native VSCLI parity.
Native comparison targets separately prepared, tokenized gestures; the natural
startup distinction stays outside that contract.

The archived scripts retain their original relative paths under
`target/advanced-indentation-modes-reference`. To replay, copy this directory's
scripts and `cases.json` there, install the locked dependencies under
`tests/vscode-reference`, provide an isolated display, and execute `node
target/advanced-indentation-modes-reference/run.cjs`. Each mode is supervised;
fresh captures replace files only in the ignored target directory. Preserve
archived outputs as evidence of this run.
