# Resource journal foundation

This module is deliberately unintegrated. `Capture::prepare` retains the original
ordered Create/Move/Delete operations, all option presence (`Presence::Absent`,
`Null`, `Value`), and absent versus empty versus arbitrary binary create contents.
It captures each existing canonical file/directory tree once, including displaced
and deleted disk preimages, while retaining strong source/parent identity handles.
`ModelArchive` separately retains caller-supplied unsaved text and opaque history;
its resource must belong to an operation endpoint or captured directory tree.
The caller remains responsible for verifying that archive proofs/text/history came
from the original retained models; this module does not inspect App model state.

`StagedJournal::persist` writes those originals to a fresh private directory outside
all operand trees and aliases. Each payload and the closed manifest use guarded
create-only native persistence; directory and parent creation entries are synced.
The original journal-parent handle and requested alias are checked before/after
creation and retained. Failure can leave an incomplete private staging directory;
no operand operation has occurred and no automatic cleanup is attempted.

`RecoveryAuthority::load` validates the complete manifest, exact operation operand
set, requested/canonical/ancestor mappings, tree graph, every bounded binary blob,
and all original live identity/byte proofs before returning any authority. A late
invalid record refuses the entire load. It refuses changed originals or journal
parents, including same-byte foreign replacements and parent ABA. Loading a live
owner's journal additionally compares its recovered capture to retained originals.
This is a private native storage contract with integrity checks, not cryptographic
authentication against an actor able to rewrite the entire private journal.

All methods must run on the owning bounded joined worker. `Control` borrows the
worker's original cancel flag and deadline. Defaults cap 4,096 operations/nodes,
128 resources/archives, 32 MiB aggregate binary/text/archive payloads, 2 MiB manifest
and retained path/name metadata, and 8 MiB archive metadata. Missing-parent prefixes
and directory names are charged before retention; limits cannot exceed defaults.

Payload symlinks, special nodes, Windows reparse points and non-UTF-8 paths/names
are explicitly refused. Existing parent aliases are allowed only with the unchanged
original mapping and retained identity. Existing parents are required for journal
creation; no operand parents are created. Directory durability and identity failures
are explicit refusals, rather than success with weaker protections.

There is no create/move/copy/delete execution, overwrite/ignore/recursive/folder/
trash behavior, postimage capture, partial execution receipt, rollback, public
Undo/Redo, or App publication in this foundation. Every successful object reports
`NotStarted`; options are retained for a later real ordered execution engine.
That engine needs durable authorized per-operation checkpoints, conflict-guarded
postimages, dependency-correct rollback distinct from public source Undo semantics,
and integration with retained dirty model/history proofs. The application must not
advertise resource edit support on the strength of this foundation.

Twenty-three focused data-integrity tests pass on Linux, including two actual
native-document/joined-worker storage controls. The parent coordinator executed
them; the source author launched no Cargo, PTY, CI or local VS Code runtime.
The full Rust suite passes 1,428 tests (1,079 library; 23 ignored), alongside
fresh build, formatting and strict locked all-target Clippy. This does not qualify the
unfinished operation engine, public API, destructive recovery or other platforms.
