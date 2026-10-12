# Broad feature todo

This is the retained project backlog, updated 2026-10-12. Items describe remaining
work across features that may already have partial implementations. They are not
a claim that any whole category is complete.

## Current priority: the entire extension API

Implement the full public stable VS Code extension API with matching observable
behavior: signatures, overloads, returned values, object identity, events,
cancellation, errors, disposal, persistence and document-version protection.
Inventory the complete pinned upstream declaration, including type-only contracts;
track each implementation and its tests. Version additions remain visible.
Proposed APIs are a separate tracked surface, not silently included in stable
compatibility claims. Installed packages, contributions, extension-host startup
and a few working providers do not constitute full API compatibility.

Work in this order:

1. API value types, URIs, events, cancellation, document/editor identity and lifetime.
2. Workspace folders, filesystem providers/watchers, documents, WorkspaceEdit,
   configuration, secrets, storage and activation/dependency behavior.
3. Commands, editor events, selections, decorations, messages, progress,
   quick input, output/status items, tree views and tab APIs.
4. All language provider registrations, resolution, cancellation and native
   presentations, including semantic tokens, CodeLens and inlay hints.
5. Terminal, task, debug, testing, SCM, authentication and environment APIs.
6. Notebooks, custom editors, webviews, remote/virtual workspaces and remaining
   contributed surfaces; record presentation differences explicitly.
7. Real extension workflows and cross-extension interactions for every subsystem.

Current usable milestone: public filesystem watchers and local file discovery
with `workspace.findFiles`,
public document saves and retained-document text
`workspace.applyEdit` now reach actual native workers, with shared mirrors/events
before success and grouped workspace Undo/Redo. Next edit backends include unopened
resources, file create/rename/delete, snippets, EOL changes, annotations and
notebooks; the remaining API order above is unchanged. See
[native workspace edits](NATIVE_WORKSPACE_EDITS.md) and
[file discovery](NATIVE_FILE_SEARCH.md) and [watchers](FILE_WATCHERS_API.md). A milestone advances a named
workflow, not the overall compatibility percentage or constructor export count.

Current delivered integration: [virtual text content providers](VIRTUAL_DOCUMENTS_API.md)
connect custom URI registration, open/show and change callbacks to retained
read-only native models. Public host/App and keyboard terminal workflows pass,
with readonly save/edit refusal and original dirty-file Undo/Redo preserved. Next
document backends include writable custom filesystem documents, encoding options
and scheme activation. Current workflow progress is tracked in
[the extension API workstream](EXTENSION_API.md#live-workflow-progress), separately
from the frozen declaration/export audit.

Current document integration also adds [API-created untitled resources](UNTITLED_DOCUMENTS_API.md):
initial content/language, associated destinations, native Save As and recoverable
identity. Initial Undo, fresh resource naming, public close/open/save ordering,
associated create-only saves and recovery pass local host/native tests and actual
terminal workflows. Encoding and full untitled presentation fidelity remain later work.

Current editor-state integration adds
[document language switching and editor selection setters/events](EDITOR_STATE_API.md).
Language-only changes retain text versions and history while retiring obsolete
provider/LSP lifetimes; selections preserve direction and native UTF-16 bounds.
The subsequent [editor inventory milestone](EDITOR_INVENTORY_API.md) adds independent
identities for shared panes, all visible editors, view columns, exact inactive-pane
selection routing and showTextDocument column/preserveFocus overloads.
[Editor presentation](EDITOR_PRESENTATION_API.md) adds shared indentation and
per-pane styles, measured UTF-16 visible ranges, scoped events and all four reveal
modes. [Editor decorations](EDITOR_DECORATIONS_API.md) adds per-view range/type
lifetimes, edit-aware UTF-16 markers and native colors/common styles.
[Public editor snippet insertion](EDITOR_SNIPPETS_API.md) adds bounded overloads,
options and an exact native view bridge; all 617 host and 1,279 Rust all-target
tests, four actual selected-host/App workflows and four new terminal journeys pass.
Rich decoration hover/attachments/CSS, snippet choice menus and command-driven nested-session
merge remain editor API work. Selection source kinds,
wrapping/folding range projection, preview tabs, sparse empty groups and broader
upstream option/reveal fidelity remain outstanding. The full stable API order
above remains active.

[Public text tabs and groups](EDITOR_TABS_API.md) now include all inventory/event/
close overloads, native Save/Discard/Cancel and untitled Save As, original caller
retirement, and a real empty welcome group. All 641 host and 1,304 Rust all-target
tests, five selected-host/App cases and 30 terminal journeys pass. Seven stable
TabInput value classes are exported; their diff/custom/webview/notebook/terminal
backends remain outstanding. [Navigation provider APIs](NAVIGATION_PROVIDERS_API.md)
now route all five navigation families with public queries and real native
location pickers. All 664 host and 1,316 Rust tests and four new terminal journeys
pass. That milestone routes eleven stable provider families. The navigation
ordering follow-up shares one host registry between public/native queries, with
cached candidate and competing-exclusive rules; all 676 host and 1,322 Rust tests
and six terminal journeys pass. Registration
alone will not count as delivered behavior.

[Global workspace symbols](WORKSPACE_SYMBOLS_API.md) now provide no-editor
public queries, exact original-object lazy resolution, hidden indexing and
native Ctrl+T navigation. All 694 host and 1,337 Rust all-target tests pass,
including seven actual selected-host/App cases; sixteen relevant terminal
journeys, the installed-clangd symbol test, build/fmt/strict Clippy pass.
[Rename/prepareRename](RENAME_PROVIDERS_API.md) now has public queries, full rich
query values and original native F2 with genuine background loading of unopened
text targets and grouped editing/history. All 714 host and 1,361 Rust tests pass,
with nineteen terminal journeys and installed-clangd rename; build/fmt/strict
Clippy pass. Thirteen of the 32 stable provider families are integrated, with
explicit behavioral limits. Next, default
multi-resource refactoring saves must finish through actual native save receipts;
resource operations, snippets, EOL, confirmation annotations and notebook targets
remain required backends. Single-resource refactoring now stays unsaved under
the default policy, matching the upstream multi-resource autosave condition.
Remaining provider families and mixed native/extension aggregation remain open.

Native Rust editing must stay usable without JavaScript. Optional compatibility
processes stay isolated. Unsupported calls must remain explicit until implemented;
placeholder methods and invented successful responses do not count as coverage.
Terminal presentation differences need their own evidence and cannot be described
as identical browser rendering. No local VS Code instances may be launched.

## Retained broad backlog

| Area | Remaining work |
| --- | --- |
| Editing/workbench | Wrapping and visible folding integration; tab/group movement; complete session restoration; drag interactions; richer snippet choices/nesting; remaining commands and exact contexts |
| Language intelligence | Concurrent servers; tool provisioning; semantic tokens; inlay hints; CodeLens; richer completion and unopened-file refactoring |
| Extensions | Entire API workstream above; broader tested real-extension corpus; native contribution fidelity |
| Projects/workspaces | Multi-root and `.code-workspace`; previewed cross-file replacement; live search; workspace switching; settings editor and profile import |
| Git | Inline changes; hunk actions; conflicts; blame; branches/remotes; stash and worktrees |
| Tasks/debug/testing | Task dependencies/problem matchers; broader `launch.json`; attach; watches; advanced breakpoints; test explorer/results/coverage |
| Terminal/remote | Shell integration; terminal splits/profiles/restoration; links/search; SSH, containers and WSL; reconnect and port forwarding |
| Visual/rich documents | Theme/TextMate/icon fidelity; embedded languages; Markdown; notebooks; images; webviews and custom editors |
| Accounts/optional services | Authentication; secrets; sync; collaboration; native AI and inline completion providers |
| Final qualification | Performance optimization and benchmarks; platform/accessibility checks; CI repair; packaging, upgrades and reviewed releases |

Features come first. CI investigation and repair are deferred to final
qualification. Relevant local behavior tests, data-integrity tests, PTY checks,
formatting and strict Clippy continue alongside implementation. Existing automatic
workflows are not disabled. Frozen wrapping and CI investigation packages remain
preserved for later integration; extension API work now takes priority.
