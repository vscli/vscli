# Native navigation history

Back and Forward restore locations within this editor session. They do not edit
text, save dirty files, or own document content and Undo history. The feature
works without JavaScript or a language server.

Use **Go: Back** and **Go: Forward** in the command palette, or the original
platform shortcuts:

| Keyboard profile | Back | Forward |
|---|---|---|
| Linux | Ctrl+Alt+- | Ctrl+Shift+- |
| Windows | Alt+Left | Alt+Right |
| macOS | Ctrl+- | Ctrl+Shift+- |

The terminal must deliver the shortcut. The Linux terminal tests use enhanced
CSI-u sequences for the original shortcuts. Binding availability uses
`canNavigateBack` and `canNavigateForward`; unavailable travel does not invent a
destination.

## Locations and recording

The bounded, session-only stack records native document identity, file resource,
editor pane and primary selection in UTF-16 coordinates. It stores at most 50
locations, with file-resource paths bounded to 4 KiB. It does not retain document
text or copy all secondary cursors into each history entry.

Editor switches and navigation commands record locations. Ordinary movement
within fewer than ten lines updates a nearby location; larger movement creates a
new entry. Explicit jumps create distinct locations when the line changes. A new
navigation after Back discards the former Forward branch. Back/Forward themselves
do not recursively append destinations. Nearby Jump to Bracket movement uses the
ordinary coalescing rule; it does not force a separate entry for each bracket.

Existing native models are authoritative. Returning to a dirty model reuses its
identity and unsaved text, including a retained model whose backing file was
deleted. Save As keeps history attached to the model's current path. If the
recorded pane remains available, travel restores that pane's primary selection;
other views of the shared model keep their selections. Secondary selections in
the destination view are cleared rather than reconstructed from old history.
A reversed primary selection restores as a forward range, retaining the selected
text with its cursor at the range end, as in the pinned workbench behavior.

Navigation does not bypass persistence guards. Ordinary Save still refuses a
backing file changed or removed outside the editor. Save As to a new resource
can deliberately preserve the retained buffer without overwriting external
changes or recreating the removed path implicitly.

Coordinates are clamped against the current document. A changed column inside a
UTF-16 surrogate pair floors to a valid scalar boundary. Locations are not live
range markers, so intervening edits can change which text an old location
identifies. Closing an untitled model removes its history targets; discarded
unsaved text cannot be resurrected from the stack.

## Asynchronous file travel

Closed file resources use the existing retained native file-loader slot. An
earlier cancellation keeps the slot occupied until its worker finishes. While a
file is loading, repeated history commands retain one latest destination. The
committed stack index changes only after successful navigation.

Keyboard availability currently follows the committed index, rather than the
pending destination. At a committed endpoint this can prevent a shortcut from
reversing pending travel until it settles, even when an explicitly invoked
history command could update the latest destination. Pending-travel key context
is an outstanding behavior boundary.

Publication checks the exact travel ticket, target entry and current editor
context. A stale load cannot change focus or install its document. Missing or
invalid targets report an error without committing travel or synthesizing an
empty file. Returning to an already retained model does not require reading its
backing file. Native document reads retain the existing ordinary-file and file
size checks. Background file-loader admission supports at most 128 retained
models, each with a file path of at most 4 KiB; it rejects over-budget snapshots
before starting a worker. These bounds do not promise a fixed filesystem latency.

## Evidence and remaining qualification

Five public App tests passed, exercising dirty Unicode/CRLF models and document IDs,
Back/Forward branch truncation, shared-pane primary selections, Save As followed
by deleted backing files, and discarded untitled targets. Four terminal workflows
passed using the original Linux shortcuts, ordinary large cursor movements and physical
Open/Save As commands, observing exact saved bytes and Undo/Redo with Node and
language servers unavailable.

Local qualification also passed the existing 35 terminal workflows and seven
smart-typing workflows against the fresh debug binary. The full Rust run passed
555 ordinary tests across 36 suites; 18 opt-in integration tests remained ignored.
The four new terminal workflows also passed against the optimized binary.
Fresh platform CI qualification remains pending.

A separate pinned VS Code 1.95.0 observer captured ten cases and 85 snapshots.
The capture retains source and settings provenance. Native differential comparison
passed all ten cases and 85 snapshots, and five comparator integrity tests passed.
Named workflows do not establish complete navigation parity.

History persistence across editor launches, separate edit/navigation stacks,
last-edit-location commands, full exclusion/settings behavior and every VS Code
navigation integration remain outstanding. Outline and breadcrumbs remain
separate features.
