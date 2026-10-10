# Native historical view merge lease

Document::prepare_historical_view_merge stages an exclusive view change without
activating historical views or editing text/history. App must separately prove
live group memberships and prepare the complete Layout/Groups transaction before
publishing all disjoint leases. Document::retained_view_state supplies an exact
borrowed view lookup; missing groups never return unrelated current geometry.

CopySource copies the exact source public selection and viewport with fresh
private session ownership. KeepTarget preserves the complete existing target,
including private generated pairs, snippet session and clocks. Origin creates
fresh native default state only when an exact target view is absent. An inactive
target membership alone does not prove a retained view. Historical upstream
editor-memento restoration remains a separate compatibility requirement.

Publishing retires the source view. An unrelated current view remains intact;
when the source is internally current, its chosen destination installs directly
without activation or changing text Undo/Redo. Save snapshot identity, text epoch,
revision, baseline, path and save generation remain unchanged. Copy requires an
exact source. Missing-source KeepTarget/Origin requires a nonzero unrelated
current view; ambiguous neutral state refuses without inferring provenance.

Visited cohorts admit 10,000 selections and valid scalar endpoints. Copied
secondary storage and new map room reserve before publication. Newly prepared
payload is bounded to 512 KiB per lease; KeepTarget stages no copied private
state. App still needs its own aggregate resource bound. Dropped/refused leases
preserve logical state; preparatory map capacity may grow. Standard small Arc
allocations follow the existing native allocation conventions.

Thirteen local integrity cases pass. They cover reversed Unicode/CRLF cohorts,
real multicursor Undo/Redo, kept target pair/snippet controls, retired owner
rejection, exact versus missing/neutral views, checked-clock refusal, selection
bounds, dropped stages and a second-model refusal leaving the first unpublished.
One authored oracle was corrected to account for both real multicursor edits;
production semantics were unchanged. Captured save-receipt metadata remains
valid, but actual App SaveWorker/terminal journeys are pending integration.
