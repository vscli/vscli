# Resource journal and ordered execution

The module is a worker-only filesystem backend, currently unintegrated with App
WorkspaceEdit publication. It never spawns a thread. The owning joined coordinator
supplies original preparation cancellation/deadline values, validates native caller
and model proofs, then explicitly admits `Executor::authorize` and filesystem work.
After admission, commit control must belong to the authorized worker, rather than
canceling already-authorized effects merely because an extension owner retired.

`Capture::prepare` preserves original operation indices, absent/null/value options,
raw binary contents versus absent/empty bytes, whole ordinary directory trees and
opaque dirty-model archives. Archive provenance/model authority belongs to the
caller. `ModelProof` includes native resource_epoch; legacy v1 archives missing it
remain inspectable with zero and explicit legacy provenance, and cannot authorize
new execution. Original preimages and unsaved archives remain intact throughout
execution and rollback.

`StagedJournal::persist` writes the original v1 closed manifest and bounded binary
payloads using guarded native persistence into a fresh private directory outside
all operand trees/aliases. Payload, journal-directory and journal-parent entries
are synced; original parent identity and requested alias remain checked. Incomplete
private staging artifacts are retained for inspection, never treated as applied
resource operations.

`Executor::authorize(staged, policy, execution_limits, control)` verifies retained
originals and persisted originals before writing the immutable v2 execution header.
It binds the original manifest digest and journal identity. `execute_authorized`
performs local create-file/create-folder, rename, directory/file copy, displacement
and delete through actual physical effects. Each contiguous CREATE batch creates
folders before files, with original order inside each partition; ignoreIfExists is
decided before that batch's effects. Surrounding kinds remain in original order.
Rename/copy batches freeze ignore decisions and validate member source/overwrite
preconditions before the first operand effect. Missing later sources reject the
batch without earlier operand effects. The caller must split phases at text/notebook boundaries: this filesystem-only
Operation vector does not encode those boundaries and cannot execute interleaved
App text publication. No global file sorting or final-state rewrite occurs.

Overwrite/delete first park existing trees in private journal slots, preserving
original native identities, binary bytes and permissions. Copy builds a bounded
private tree, then publishes it by guarded rename. Missing parents are created as
individual checkpointed effects. Linux effects use held parent handles/fds,
create-new and exclusive RENAME_NOREPLACE; namespace identities, byte hashes and
complete directory children are checked. World child-name transitions are computed
from owned effects; foreign observed descendants are never adopted into owned state.

Every operand effect has a create-only durable intent before mutation and an
acknowledged postimage afterward. Hash-chained v2 records retain operation/physical
indices, partial outcomes and rollback progress. Original v1 journals remain
NotStarted inspection-readable; an execution header requires ExecutionRecovery.
`ExecutionRecovery::load` validates all records and original payloads before
returning evidence. Unmatched forward or rollback intent is always Ambiguous,
including when paths resemble a preimage or the effect is visible. No missing
receipt implies no effects. `verify_current` and `into_rollback_authorized` refuse
foreign bytes, identities, children and parent ABA. No automatic retry or restore
occurs during loading; private same-user storage is not cryptographic authentication.

`rollback_authorized` reverses acknowledged PHYSICAL effects newest-first. This
restores dependent A-to-B-to-C chains and displaced/deleted originals, unlike the
pinned public Undo policy. It does not implement VS Code public Undo grouping,
source lost-destination behavior or App history. Delete maxSize suppression is
recorded separately; it never discards the internal recovery preimage.

Default capture limits remain 4,096 operations/nodes, 128 resources/archives,
32 MiB aggregate original payloads, 2 MiB manifest/path/name metadata and 8 MiB
archive metadata. Execution caps 16,384 physical steps, 16 MiB record retention,
32 MiB additional private parked/copied bytes and 32 MiB tracked live-tree bytes.
Worst-case receipt/terminal budgets and cumulative reverse-record leases are
reserved before effects; world and ancestor/name retention are bounded before
allocation/reads. A later refusal returns explicit applied-prefix evidence; failed
or interrupted post-effect acknowledgement remains Ambiguous.

Qualified backend boundaries remain explicit: Linux-only exclusive execution;
no cross-volume parking/rename/copy fallback; no OS trash; no custom URI providers;
no symlink/special/reparse/non-UTF8 payloads; no parent aliases inside manipulated
trees. Existing external parent aliases require the original mapping/identity.
Absent file-create contents need caller-selected encoded empty bytes (including
BOM); the backend never guesses model encoding. `policy.use_trash=true` refuses
unless each delete explicitly skips trash. Source folder options/raw presence are
retained; folder creates ignore raw file contents as upstream does. Application
readonly/encoding/save participants and dirty/shared model identity are caller
integration obligations; this backend does not publish models or advertise API
resource-edit support. Uncooperative source replacement races between identity
checks and a rename/unlink cannot be atomically eliminated; postimage failures are
explicit ambiguous evidence and never authorize blind recovery.

The committed foundation's 21 tests and two joined-worker tests previously
passed under parent qualification. Twenty-six new filesystem execution/recovery
regressions now pass under parent execution, including ordered chains with existing
batch sources, overwritten binary baselines, single-file/directory copy and delete,
CREATE scheduling, rename/copy preflight and frozen ignore decisions, partial
outcomes, cold rollback, foreign postimages, ambiguous crash prefixes, bounds and
legacy proof refusal. All 49 journal tests pass. These qualify this Linux backend,
not public WorkspaceEdit application or other platforms. The author launched no
Cargo/PTY/CI/VS Code runtime; the parent ran formatting, strict locked all-target
Clippy and behavior tests. Separate DTO integration and full-suite evidence live
in `docs/RESOURCE_EDITS_PLAN.md`.
