# VSCLI compatibility contract

## Public promise

Compatibility must describe observed behavior against a pinned reference version. An extension being downloadable, installable, or activatable does not establish that its workflows work.

Publish three independent records: supported VS Code API behavior, verified extension workflows, and tested terminal/input combinations. Also record package provenance and use rights separately from technical compatibility. Most statuses in this design remain targets. The first command/edit host experiment and named Sort Lines workflow are recorded in [extension evidence](EXTENSIONS.md); that workflow does not establish broader API or package compatibility.

Local VSIX installation is implemented through native CLI and editor management. The installed list reports manifest runtime shape separately from the experimental command host's actual API support. Immutable package generations, atomic registry updates, a previous-generation rollback, provenance digests and malicious-archive integrity tests are recorded in [extension installation](EXTENSIONS.md). Registry download, package signatures, engine/native-ABI qualification and broad package compatibility remain outstanding. The native installed picker now uses the pinned `workbench.view.extensions` Ctrl+Shift+X / Cmd+Shift+X rule and context; this adds one rule per platform beyond earlier captured inventories. Its input path has a Linux PTY check and cross-profile dispatcher tests, not universal terminal qualification.

Exact VS Code keybindings are a firm user requirement on qualified terminal configurations. Extension compatibility remains progressive. Do not advertise “all VS Code extensions” or “all shortcuts on every terminal”; qualify the reference version, platform profile, keyboard layout, terminal, and any multiplexer configuration used for an exact-keybinding claim.

Native rope navigation uses Unicode 17 extended grapheme boundaries, with two
upstream chunk-boundary fixes backported to unicode-segmentation 1.13.3. The
bundled Unicode corpus passes forward/backward and boundary-query checks with
scalar-sized chunks; document and PTY tests cover movement, edits, undo, and
CRLF persistence on long lines. These are native correctness checks, not a
differential claim that every VS Code cursor or display-width behavior matches.
See [dependency provenance](../vendor/README.md) and [current limits](USAGE.md).

Native configuration migration now previews compatibility notices and atomically
activates versioned copies of user settings, keybindings, snippets and the selected
extension color theme. Native theme loading maps a bounded workbench-color subset
and approximates syntax foreground categories. Copying a field does not implement
it; broader TextMate/semantic/font-style/workbench fidelity remains incomplete.
See [implementation, test evidence and limits](IMPORT_AND_THEMES.md).

Recent-file navigation now provides a persistent native MRU picker and session-local
reopening of closed file-backed editors. Workspace switching, restored session
layouts and full VS Code history ordering remain unimplemented; see
[recent-file behavior and integrity evidence](USAGE.md#recent-files-and-reopening-closed-editors).


Native LSP Quick Fix/Refactor now supports bounded action discovery, lazy edit
resolution, and validated edits across synchronized open buffers. C++ qualification
includes a real clangd 23.1.1 missing-semicolon fix on Linux; broader server/refactor
compatibility remains unqualified. See [supported edits and integrity boundaries](USAGE.md#native-code-actions-and-quick-fixes).

Native explicitly invoked LSP parameter hints now show the server-selected signature
and active parameter in a themed nonmodal panel; automatic triggers and overload
navigation remain pending. See [behavior and evidence](USAGE.md#parameter-hints).

Native LSP document/workspace symbol search supports bounded hierarchical and flat
responses, UTF-16 navigation, dirty shared buffers and asynchronous existing-file
loads. Range-less WorkspaceSymbol resolve and an outline panel remain unsupported.
See [symbol-navigation behavior and evidence](USAGE.md#native-document-and-workspace-symbols).

## Feature disposition

| VS Code feature family | Planned terminal behavior | Scope |
| --- | --- | --- |
| Files, tabs, splits, selections, undo, search | Native workbench and document transactions | Core product |
| Command palette and quick open | Native lists with asynchronous filtering | First usable release |
| Platform keybindings and chords | Preserve exact defaults, contexts, chords, arguments, and user overrides | Firm requirement; qualify terminal configuration and track command implementation separately |
| Settings, snippets, profiles, multi-root workspaces | Versioned import with supported-field report | Staged support |
| Language intelligence | Direct LSP and compatible extension providers | Curated languages first |
| Syntax grammars and themes | Tree-sitter baseline; separate TextMate compatibility route | Approximate appearance unless tested |
| Git and SCM providers | Native change lists, diffs, actions | Built-in Git before generic SCM extensions |
| Tasks, debugger, test explorer | Native output, controls, trees, and results | Later IDE milestone |
| Quick picks, input boxes, tree views, status items | Native equivalents | Priority extension UI surface |
| Decorations, code lenses, inlay hints | Cell-based text and styles | No pixel-positioning equivalence |
| Integrated terminals | PTY-backed terminal panel | Separate emulation milestone |
| Markdown previews | Native rendered text; optional image capabilities later | Browser CSS/JS behavior excluded |
| Notebooks | Later cell model with text outputs and kernel integration | Rich renderers require separate work |
| Webviews and graphical custom editors | Explicit unsupported status or dedicated terminal adapter | Arbitrary HTML/JS equivalence excluded |
| Remote SSH and containers | Run editor remotely first; own agent later | No dependency on proprietary remote services |
| Authentication and secrets | Provider contracts and OS credential storage | Provider/service-specific validation |
| AI chat, inline completions, agent tools | Optional native provider surfaces after core maturity | No blanket compatibility claim for vendor extensions |
| Account sync, collaboration, browser tooling | Independent integrations if justified | Deferred; not v1 gates |

The presence of one native alternative does not mean the original extension works. A native Markdown preview and an arbitrary webview extension are different compatibility claims.

## Editor reload behavior

Native external reloads preserve each view's cursors and selections in the
common unchanged prefix/suffix, including through undo/redo. Tests cover two
views, secondary selections, Unicode/CRLF saves, and typing after a real watched
reload. The current single-replacement mapping is not a qualified match for
VS Code's handling of multiple disjoint external changes.

## Keyboard compatibility

Use the exact keybinding rules from a pinned VS Code reference, including command IDs, arguments, chords, platform overrides, context expressions, user overrides, and removal rules. VS Code resolves matching rules from the bottom upward; context and ordering are part of behavior, not incidental syntax. [Keyboard rules](https://code.visualstudio.com/docs/configure/keybindings).

Maintain a reproducible export of resolved default bindings for Linux, macOS, and Windows, recording reference version, keyboard layout, built-in extensions, and relevant settings. Preserve source provenance and notices. Track extension-contributed bindings separately. A manually selected list of popular shortcuts does not satisfy this requirement.

Parse `when` expressions into an AST and implement the documented operators in stages, including negation, comparisons, regular expressions, and membership where supported. Unknown syntax produces a diagnostic; an unknown context key follows the pinned reference behavior. Reuse the context engine for menus and command enablement. [When clause contexts](https://code.visualstudio.com/api/references/when-clause-contexts).

The current parser bounds each expression to 8 KiB, 1,024 tokens and 64 nested groups or negations. Native tests cover mixed grouping/negation depth, flat-token and byte limits, and preservation of active user/extension rules after rejection. Supported operators remain the documented subset; these resource limits are not evidence of complete resolver parity.

Select the matching operating-system profile by default and allow an explicit profile override, including when running through SSH. The intended keyboard profile belongs to the user at the terminal and must not silently change just because the remote host runs a different OS. Layer imported user bindings over the selected defaults:

| Profile | Behavior |
| --- | --- |
| Linux defaults | Exact bindings and behavior from the pinned Linux reference |
| macOS defaults | Exact bindings, preserving Command versus Control, from the pinned macOS reference |
| Windows defaults | Exact bindings and behavior from the pinned Windows reference |
| User overrides | Import `keybindings.json` without changing combinations, arguments, contexts, or removal rules |

Do not ship a substitute terminal keymap as the default. Additional bindings are an explicit user customization, not evidence that an unavailable original shortcut works.

Enhanced keyboard reporting is negotiated at startup and reevaluated after resume. A compatibility inspector shows the received key, matching rule, active context, and command result. For example, legacy terminals may encode Ctrl+I and Tab identically; no rule engine can distinguish them after that information has been lost. [Keyboard protocol background](https://sw.kovidgoyal.net/kitty/keyboard-protocol/).

Provide terminal and multiplexer configuration recipes that release conflicts or forward the original combinations with unambiguous encoding. Handle terminal line discipline so software flow control and signal processing do not consume intended editor keys. Use a guided key test to identify missing events: the editor cannot infer which swallowed physical key was pressed from an absent event. Configuration changes should be scoped, reviewable, and reversible. If an OS binding consumes a required combination, identify the needed OS change; do not silently remap it.

Test Ctrl+C with and without a selection, Ctrl+K chords, Ctrl+P, Ctrl+Shift+P, F1, F5, shifted navigation, Alt/Meta, international layouts, paste, and IME text. Editor input and focused terminal-panel input require different routing, with a documented escape command to return focus to the editor.

Keep actions discoverable through the command palette, but palette access is not a substitute for an exact shortcut. OS-reserved shortcuts, physical scan-code mappings, and system-wide hotkeys need explicit platform qualification. macOS Command-key parity depends on terminal configuration and event delivery; silently replacing Command with Control fails the requirement.

Verify three layers independently: physical combination reaches the application distinctly; the resolver selects the same command and arguments in the same context; the command has the same observable effect. Test collisions and inactive contexts as well as successful dispatch. Preserve editor-versus-terminal focus behavior, including interruption and copy behavior. A matching command name with different selection or undo effects does not pass.

Early releases may have incomplete command implementations. Keep their original bindings reserved and show unsupported commands explicitly; do not repurpose those combinations. Publish separate input, resolver, and command-behavior coverage over the complete baseline inventory. An exact-parity claim requires all applicable baseline cases to pass for the declared environment; any excluded cases must be explicit limitations rather than silently removed from the denominator.

### Current reference harness

The [pinned reference harness](../tests/vscode-reference/README.md) exports the actual VS Code 1.95.0 default rule inventory, built-in extension metadata, and observed keyboard layout on each CI platform. It compares all exported rules with the native defaults and runs 25 shared configuration observations against both APIs. The fixture does not establish physical input, resolver, command-effect, or full extension parity; the three keyboard verification layers above remain required.

[Recorded inventories and provenance](../tests/vscode-reference/baselines/1.95.0/provenance.json) come from the same [successful three-platform CI run](https://github.com/vscli/vscli/actions/runs/37831323012). All observed layouts were US; macOS used arm64, Linux and Windows x64. The 25 configuration observations matched on all three platforms.

| Platform | Reference rules | Native rules | All fields match | Key/command/args match, context differs | Only command ID matches | No native-default command ID |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Linux | 984 | 114 | 31 | 77 | 14 | 862 |
| macOS | 1,078 | 121 | 31 | 78 | 26 | 943 |
| Windows | 993 | 112 | 30 | 76 | 18 | 869 |

The table uses the [updated comparison summaries](../tests/vscode-reference/baselines/1.95.0/snippet-variables/provenance.json)
from [CI run 37859573471](https://github.com/vscli/vscli/actions/runs/37859573471),
including the four new snippet bindings per profile. These supplement the earlier
full inventory capture; they remain structural comparisons.

The four comparison categories partition the reference rules, including duplicates. These are structural categories, not a percentage of supported features. Context equivalence, rule ordering effects, input delivery, and command behavior remain unmeasured by this inventory comparison. The reference is VS Code 1.95.0, not the latest release.

## Configuration and migration

The native snippet template/document engine is compared against 34 pinned
reference cases (96 text/selection observations), including linked editing,
nested traversal, transforms, and undo/redo. Literal user-command insertion and conditional Tab/Shift+Tab/Escape navigation
are exposed through terminal keybindings. Catalogs, choice UI, LSP completion,
and the extension insertion API remain unfinished.
An additional 34 insertion traces (118 observations) cover multiline
indentation/EOL conversion and API-versus-command cursor ordering, matching on
Linux/macOS/Windows in [CI run 37857315175](https://github.com/vscli/vscli/actions/runs/37857315175). The current native regex subset is not full ECMAScript. See the
[snippet evidence and outstanding integration](SNIPPETS.md); this does not
establish general snippet or keybinding parity. Thirty additional reference cases (59 observations) cover native variables and
cancellation, matching on all three platforms in CI run 37859573471. Native/PTY tests cover command routing,
user overrides, shared editing, persistence, and rejecting stale clipboard replies.

Provide a read-only migration preview before writing the project's own configuration. Parse JSON with comments, preserve the user's source files, and show imported settings, approximations, unsupported settings, extension requirements, and unavailable shortcuts.

Keep familiar names for supported settings. Define precedence for defaults, user/profile settings, workspace settings, folder settings, and language overrides against a pinned VS Code reference. Preserve unknown fields when editing existing configuration. Initial support should cover fonts only as an explanatory unsupported setting, while indentation, wrapping, autosave, file excludes, search excludes, formatting, and keybindings have meaningful terminal behavior.

Support a documented subset of `.code-workspace`, `.vscode/settings.json`, `tasks.json`, `launch.json`, and snippet files. Configuration providers, command-variable substitution, problem matchers, snippet transformations, and extension-defined settings each need their own tests. Importing a file format is not executing every possible configuration in it.

Use dry-run migration output as a contributor-friendly issue generator. Users should be able to export a report without uploading their source code, home path, credentials, or workspace secrets.

## Extension execution strategy

The implemented experimental host now runs up to eight explicitly selected CommonJS packages in one shared process. Document objects, configuration and versioned native transactions are shared; command and edit requests retain package ownership. Adding/removing a package restarts the selected cohort. Deterministic VSCLI command/keybinding precedence, epoch-safe startup, bounded registries and worker retirement are described in [session behavior](EXTENSIONS.md). Dependencies, automatic activation, safe individual hot unload, and conflict-resolution differential qualification against VS Code 1.95.0 remain outstanding. The [native Quick Pick/Input Box subset](EXTENSIONS.md#native-quick-pick-and-input-box) supports bounded single selection, basic text input and cancellation. Its synthetic native/PTY tests and unchanged Lorem Ipsum 1.3.1 command workflow establish those named behaviors; complete Quick Input differential parity, password/validation, multi-select and live variants remain unqualified or unsupported.

A TypeScript shim supplies the `vscode` module inside a maintained Node.js runtime and connects to native services through a versioned broker protocol. It handles activation, disposables, events, cancellation, commands, provider registrations, settings, storage, workspace resources, and native view models in stages. Pin a stable VS Code API baseline; do not pretend to implement the latest API by changing a version string.

VS Code distinguishes Node and browser extension hosts. Initial executable-extension support should cover the Node `main` entry point and declarative packages. Browser-only extensions need an independently evaluated worker/runtime environment and remain unsupported initially. [Web extensions](https://code.visualstudio.com/api/extension-guides/web-extensions).

The difficult part is synchronous API behavior. Calls such as reading document text cannot wait for arbitrary asynchronous RPC without changing the extension contract. Maintain versioned document and selection mirrors in the Node host. Deliver each text delta before the corresponding change event; synchronous reads during callbacks must see the correct mirror version. Native state remains authoritative. Resynchronize after gaps or restart, and attach expected versions to requested mutations.

Use protocol handles for providers, documents, and UI objects; preserve identity and disposal semantics for their declared lifetimes. Implement event ordering, rejection behavior, cancellation, and built-in command semantics with contract tests. A method with the right name and return type can still be incompatible.

Extension dependencies may expose arbitrary in-memory APIs to each other. Begin with compatible dependency groups in the same host. Per-extension isolation would require additional semantics and may break these APIs. Native Node modules also require an OS, architecture, and runtime ABI match; a supported JavaScript entry point does not guarantee that bundled binaries load.

## Reuse versus independent shim

Make this a Phase 0 decision with a two-week initial timebox inside the broader feasibility phase:

| Experiment | Evidence required |
| --- | --- |
| Extract a pinned Code OSS extension host | Identify its main-thread service dependencies, build footprint, protocol coupling, license obligations, and cost of rebasing one upstream update |
| Implement a bounded independent shim | Run the same small fixture suite and two real permissively usable extensions; measure event fidelity and missing APIs |

Prefer the independent shim for a deliberately bounded API surface unless host extraction materially reduces ongoing maintenance. Reuse carefully selected source where licensing permits and it improves correctness. The upstream private extension-host RPC is not the project's stable public protocol. Code OSS source is MIT licensed, which is distinct from distribution and marketplace terms. [Code OSS license](https://github.com/microsoft/vscode/blob/main/LICENSE.txt).

Study Theia's compatibility reporting as an existing alternative-editor approach. Its documentation notes that VS Code API support and actual extension compatibility need to be checked; VSIX installation alone is insufficient. [Theia extension documentation](https://theia-ide.org/docs/user_install_vscode_extensions/).

If neither approach passes the behavior gate, ship the native IDE with direct LSP and native plugins while keeping VS Code extension support explicitly experimental. Do not let an uncertain compatibility project prevent useful releases.

## Order of API implementation

1. **Foundation:** URI, positions/ranges, disposables, events, cancellation, extension context, command registration, activation and configuration.
2. **Documents:** open/close/change/save, selections, edit builders, workspace edits, filesystem providers, watchers, and storage.
3. **Language providers:** diagnostics, completion, hover, definitions, references, formatting, code actions, and rename.
4. **Native UI contributions:** quick picks, inputs, notifications, progress, output channels, tree views, menus, and status items.
5. **IDE integrations:** tasks, SCM, debug providers, terminals, tests, authentication, and secrets.
6. **Specialized work:** notebooks, advanced decoration behavior, and selected terminal adapters for graphical extensions.

Prioritize work by the workflows it unlocks. One missing API can block a useful extension; counting implemented methods alone rewards the wrong work. Proposed APIs and private product APIs require explicit opt-in experiments and are not covered by the initial stable contract. [Proposed API policy](https://code.visualstudio.com/api/advanced-topics/using-proposed-api).

## Syntax and theme compatibility

Tree-sitter support does not load TextMate grammars automatically, and mapping a theme's colors does not reproduce its scope semantics. Native bundled C and C++ grammars now cover preprocessor directives, templates, multiline raw strings and Unicode comments without a code extension or language server. Header/module suffixes follow the named C/C++ contributions in the [pinned 1.95.0 manifest](https://github.com/microsoft/vscode/blob/1.95.0/extensions/cpp/package.json); this does not qualify every file association or CUDA. Grammar work retains the 2 MiB document budget, background cancellation and stale-revision rejection. Native query/worker tests and a real PTY cover theme-category rendering and CRLF edit/undo/save; exact TextMate/semantic appearance remains unqualified. Track language configuration, snippets, TextMate grammar loading, grammar injections, semantic tokens, and theme scope matching separately.

Start with native syntax and an approximate theme importer. Evaluate a compatible TextMate tokenizer in the optional host or a suitable native implementation for languages supplied only by extensions. Tokenization results carry document versions and never gate input. Limit pathological regex/tokenization work. Publish the distinction between approximate visual mapping and tested grammar compatibility.

Native typing now retains provisional colors for unchanged text through bounded byte-edit mapping while awaiting a current grammar result. Inserted/replaced bytes never reuse stale absolute token offsets; syntax changes can temporarily leave unchanged text with its previous classification. Deterministic tests hold the worker across multicursor Unicode/CRLF edits, undo/redo, failures and cancellation; a PTY oracle observes individual painted multiline C++ token cells, including transient repaint colors. This establishes continuity for those workflows, not semantic-token fidelity or uninterrupted colors after the documented history/mapping budgets are exceeded.

## Distribution and package policy

Use Open VSX and author-distributed VSIX files where their licenses permit use. Microsoft states that alternative products may not access the Visual Studio Marketplace and describes restrictions on Microsoft/affiliate extensions acquired there. The project must not depend on impersonating VS Code to acquire restricted packages. [Microsoft FAQ](https://code.visualstudio.com/docs/supporting/faq).

Open VSX is an alternative extension registry; presence there does not establish complete ecosystem coverage, runtime compatibility, or permission to redistribute every package. Track each package's source, version, license, target platform, hash, and verification status. [Eclipse Open VSX FAQ](https://www.eclipse.org/legal/open-vsx-registry-faq/).

Use bounded archive extraction, path-traversal protection, dependency-cycle checks, atomic install directories, and rollback. Pin the extension and dependency versions used in verification. Separate technical failure, unavailable package, license restriction, and account/service requirement in the UI and reports.

Make the native build and complete first-party source available without a proprietary service. Third-party extensions and remote AI services can have different licenses; describe them as optional dependencies rather than implying they become open source through the host.

## Verification and release reporting

Use five extension statuses: verified, partial, experimental, unsupported, and not tested. Only verified means the named workflows pass for the recorded environment. Avoid a single percentage that hides untested or excluded packages.

Each result records:

| Field | Purpose |
| --- | --- |
| Extension identity, version, hash, source | Reproduce exactly what ran |
| VSCLI commit, API baseline, Node runtime | Establish implementation and runtime versions |
| OS, architecture, terminal and multiplexer | Identify platform limitations |
| Workflow IDs and expected outputs | Distinguish activation from useful behavior |
| Passed, failed, skipped, unsupported cases | Keep the denominator visible |
| Missing API, command, service, or UI capability | Make the next contribution actionable |
| License/service prerequisite and test date | Distinguish access from technical support |

Build a fixed initial corpus of approximately 20 legally usable packages across commands, snippets, themes, formatting, language services, trees, Git, tasks, and debugging. Include at least one deliberately unsupported webview case. Select and pin actual packages during Phase 0; none are predeclared compatible here.

Run synthetic extension fixtures against both the pinned reference editor and VSCLI, comparing document versions, selections, event traces, command effects, and errors. Use real extension workflows as a separate layer. Static inspection of manifests and API references is useful for triage but misses dynamic behavior and bundled dependencies.

On each release, publish a generated API report and the named workflow results. Upgrade the reference baseline on a deliberate cadence, such as quarterly at first, and keep a working stable baseline while the new one is evaluated. Unsupported APIs should fail clearly rather than return plausible empty values that make an extension appear functional.

## Empty workbench

Starting without file arguments and closing the last editor leave a true zero-document workbench. The native welcome view displays a terminal logo and keyboard actions; it does not own a hidden untitled buffer. File-specific commands require an open editor, while workspace commands and the optional extension host remain available. Dirty close confirmation and crash-recovered documents take precedence over the empty welcome view. Unit tests cover command guards, context keys, resize/rendering and save/close integrity; Unix PTY coverage exercises startup, explicit creation, dirty close, last-tab close and restarting with empty recovery state. This is a terminal welcome view, not VS Code's browser-backed walkthrough or start-page extension API.

### Native clean-session metadata

Opt-in `--restore-session` and `vscli.session.restore` reopen clean disk files and
VSCLI's current global-tab/four-equal-group layout. Recovery buffers and explicit
CLI files remain authoritative; no editor text is stored in this metadata. The
implementation restores each tab's last view and each visible group's view, with
bounded background reads, per-instance leases, atomic publication and retryable
all-or-nothing restore. `--no-session` disables this separate persistence mechanism.
See [usage and limits](USAGE.md#clean-file-session-restoration) for qualification.
This is a native convenience feature, not a claim of VS Code hot-exit, independent
per-group tabs, terminal persistence, workspace transitions, or extension-state parity.
