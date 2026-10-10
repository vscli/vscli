# Native code-action request ownership

Discovery, lazy resolution and command execution share one actual native LSP
action lane. Closing the interactive picker cancels its exact request; it cannot
cancel another owner's work. Generic request/follow-up APIs cannot bypass the
lane. Replies and command-time workspace edits require their original current
interactive owner.

Advisory cancellation and the transport's 15-second timeout retire interest but
retain actual capacity. Only a valid matching JSON-RPC terminal response or
server retirement releases it. Wrong IDs, malformed responses and late replies
cannot mutate documents or reopen the picker. A settled late reply can make a
fresh action request available again.

Action payloads are bounded before copying or retention: 2 MiB of encoded JSON,
300 discovery entries, 8 KiB titles, 64 levels of nesting and 131,072 nodes.
Resolution cannot change immutable action identity fields. This lane is separate
from native formatting capacity and optional extension-host processes.

Six framed-peer tests cover discovery/resolve/execute occupancy, cancellation,
timeouts, malformed/wrong-ID replies, payload admission and generic API bypass.
Interactive action and native diagnostic integrations also pass locally. Full
extension/action compatibility and platform qualification remain separate from
these named transport contracts.

The original Ctrl+. terminal regression passes 32 user intents behind one held
actual callback. Its readiness barrier opens and closes the original command
palette after the input burst and checks the resulting caret. Cancellation is
asserted exactly once for the owned token; the old repeated-packet barrier is
incompatible with idempotent cancellation. The latest selected range, one actual
callback maximum, Unicode/CRLF bytes, explicit Save and Undo remain checked.
