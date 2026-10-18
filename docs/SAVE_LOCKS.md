# Save coordination without project lock files

Ordinary saves, Save As, autosave, refactoring saves and native settings patches
use a process-local resource map. Preparation acquires the canonical target's
in-memory lock on the existing background worker. The lock stays held through
native authorization and commit/rejection, then releases when the guard drops.
Unrelated resources remain independent. Parent-directory aliases use the same
key; Windows/macOS resource comparison folds case. The registry retains only
active resources (up to 128), with the existing one-second bounded worker wait.
No lock file or lock directory is created.

This follows the [pinned VS Code disk provider's resource-lock map](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/platform/files/node/diskFileSystemProvider.ts).
It replaces vscli's persistent adjacent sidecar protocol. Other processes are
external writers, including other vscli instances: original baseline bytes,
file identity, permissions, parent identity, resource aliases and native model
proofs are still checked before replacement. Missing destinations retain atomic
no-clobber creation. The last check-to-rename race with another process remains;
in-memory locking does not promise exclusive cross-process ownership.

The existing atomic writer still stages and syncs a temporary file in the target
directory. Successful replacement renames it; cancellation and failure clean it
up. These transient files are separate from persistent lock bookkeeping.

Older `*.vscli-write.lock` files are ignored and left intact. The editor does not
scan projects or delete files based on a matching suffix. Once older vscli
instances are closed, users can remove their old sidecars; this implementation
does not recreate them. Recovery, session and extension-store locks under native
state/configuration directories have separate ownership and are unchanged.

Data-integrity controls cover same-resource exclusion, independent resources,
waiter wake-up, panic release, canonical aliases, same-process settings/document
arbitration, external writes/deletions/replacements, missing-target creation,
parent replacement, dirty model history, cancellation and shutdown cleanup.
Save/autosave/Save As and settings terminal journeys explicitly reject leftover
sidecars or staged files after settlement. These are local behavior checks, not
full upstream-runtime, platform or power-loss qualification.

The final local source passes 1,615 Rust tests across 67 reports (1,266 library;
23 ignored), required formatting, strict locked Clippy and a native build. Eleven
terminal journeys pass on that binary: seven save/autosave/Save As, three settings
and one public EOL workspace-edit/save regression. The unchanged host JavaScript
suite retains its prior 797-test evidence without rerunning it for this native
change. [Validation receipts](validation/process-local-save-locks.json) record
the source/binary digests, commands, results and remaining qualification.
