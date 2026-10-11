# Broad feature todo

This is the retained project backlog, updated 2026-10-11. Items describe remaining
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

Current usable milestone: public document saves and retained-document text
`workspace.applyEdit` now reach actual native workers, with shared mirrors/events
before success and grouped workspace Undo/Redo. Next edit backends include unopened
resources, file create/rename/delete, snippets, EOL changes, annotations and
notebooks; the remaining API order above is unchanged. See
[native workspace edits](NATIVE_WORKSPACE_EDITS.md). A milestone advances a named
workflow, not the overall compatibility percentage or constructor export count.

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
