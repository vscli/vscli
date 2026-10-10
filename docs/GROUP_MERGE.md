# Native group merge transaction

The pure Groups engine can prepare and publish a Close Group merge into the
most recently active other group. The sole group remains a proof-checked no-op.
It preserves document identities and reuses existing destination tab identities;
all incoming previews commit and unrelated destination previews survive.
App command registration, joint Layout/document publication and terminal
qualification remain pending in this foundation.

Preparation captures original source order, active editor, focus, modes and
membership identities. It stages the complete bounded destination union and
source removal before checking the original lineage and live proofs again at
commit. Stale focus/order/mode changes, inverse changes and divergent clones
cannot publish. Capacity, checked-counter, allocation and late Layout refusal
leave the live group state intact. No model text or disk is read or edited.

Default right insertion follows the pinned indexed-open rules: incoming ordinary
duplicates can move outside the sticky prefix and become ordinary; inactive new
sticky rows can insert within the existing prefix. These details differ from
single-tab transfer. Native global membership history remains an explicit policy,
with upstream editor-service history qualification still outstanding. Limits are
four groups, 128 tabs per group and 512 total memberships.

Twelve local integrity cases pass, including independent expected orders,
complete deduplication at capacity, sticky-boundary transitions, preview modes,
original active/MRU targeting, stale and foreign proofs, counter exhaustion,
reservation refusal and dropped stages. These tests qualify the Groups API.
The original workbench.action.closeGroup workflow still needs all affected
historical document-view leases and a merge-aware App projection before use;
its empty-group shortcut context is not available in the current native engine.
