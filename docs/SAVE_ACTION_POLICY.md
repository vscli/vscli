# Native save-action configuration foundation

`Settings::save_code_actions(language, reason)` resolves the opt-in native
`editor.codeActionsOnSave` policy. This pure configuration API does not dispatch
providers, apply edits or persist files; save-participant registration is a
separate implementation step.

Object entries merge across settings layers and matching language scopes. Equal
composite language groups merge before single-language groups. Empty objects
inherit; arrays/scalars replace. Object policy prioritizes fix-all and legacy
arrays retain order after redundant descendants are removed. Kinds use dot
boundaries. The supported native projection is fix-all and organize-imports;
other requested source families produce compatibility notices.

The pinned runtime's false-versus-never distinction is retained: false disables
standalone enablement but does not exclude a descendant of an enabled ancestor;
never excludes the subtree. Explicit, always and true enable explicit-save work.
After-delay save resolves off for both object and array policy.

Admission bounds entries, kind text, layers, selectors and matching groups before
copying policy metadata. Malformed winning values fail closed rather than enabling
a lower policy. Five pure Rust cases passed locally, including the fourteen
observed settings matrices; an independent consumer also passed supported-kind
eligibility against all fourteen actual pinned Linux save results. Those checks
qualify configuration and filters, not action execution or full save parity.
