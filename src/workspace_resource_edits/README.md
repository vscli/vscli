# Original file DTO preparation

This worker-only adapter connects `workspace_edit_stream` file rows to the native
resource backend. It performs no filesystem IO, model mutation, confirmation or
public API publication. Core editing and this adapter require no JavaScript runtime.

Call `prepare_groups(&stream, &original_control)` once before any phase effects.
It validates every file-content descriptor across the original DTO, including
later, non-create and skipped untitled-create rows. Missing data-transfer resolvers
and malformed binary descriptors refuse preparation before effects. File contents
plus scalar text payloads share the whole-stream 1 MiB raw limit; the stream's
separate 2 MiB wire/traversal limits apply first. Notebook payload/application
qualification remains separate and the controller must gate unsupported backends
for the entire request before starting an effect.

Returned groups retain original global entry indices, original group boundaries,
contiguous file-kind batches, annotations and raw option values. Unknown options
and nonnumeric maxSize remain available while their source-ignored physical
effects are normalized. Boolean options preserve absent/null/defined false before
truthiness. Standard padded base64 revives bounded exact binary bytes; absent
contents and an explicitly supplied zero-byte buffer remain distinct. Untitled
CREATE rows are explicitly skipped after descriptor revival. Their original rows
still contribute to stream-wide create-version suppression.

The coordinator must preserve `PreparedGroup.batches` when feeding the executor.
Do not flatten native operations after removing skipped rows: an all-skipped
CREATE batch can still separate two rename batches. Source file/text/notebook
phase boundaries similarly remain in the complete original stream. Translate
native receipt indices back through the retained original indices; they are not
interchangeable with global DTO positions. Retain original caller/model/settings
proofs and confirm annotations before native authorization. Dirty/shared model
publication, resource events, public Undo/Redo and autosave cohorts belong to that
coordinator, which is still unfinished.

Only unqualified local file resources have a native path backend. Custom schemes,
non-create untitled resources, query/fragment-qualified resources, NUL and
non-UTF8 paths refuse explicitly. Path decoding neither resolves aliases nor
reads missing parents; original filesystem proofs belong to the actual journal
worker. Absent create contents require captured empty-file encoding at executor
admission; preparation does not guess a BOM or grant trash/provider capabilities.

Ten adapter tests pass on Linux, including whole-stream refusal/budgets, source
option/untitled semantics and a real original DTO→journal→filesystem transaction
on the joined worker. Wrong authorization IDs refuse; postauthorization owner
retirement preserves actual binary effects and the final joined receipt. These
tests qualify the adapter/backend coupling, not the public extension gateway or
terminal/model lifecycle integration. The combined Rust suite passes 1,471 tests
(1,122 library; 23 ignored), global formatting and strict locked all-target Clippy.
