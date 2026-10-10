# Native save-action configuration foundation

`Settings::save_code_actions(language, reason)` resolves the opt-in native
`editor.codeActionsOnSave` policy. This pure configuration API does not dispatch
providers, apply edits or persist files. The
[native save participant](CODE_ACTIONS_ON_SAVE.md) consumes this bounded plan.

Object entries merge across settings layers and matching language scopes. Equal
composite language groups merge before single-language groups. Empty objects
inherit; arrays/scalars replace. Object policy prioritizes fix-all and legacy
arrays retain order after redundant descendants are removed. Kinds use dot
boundaries. The supported native projection is fix-all and organize-imports;
other requested source families produce compatibility notices.

The executable policy interprets legacy true as `explicit` and false as `never`.
Both false and `never` exclude a kind and its dot-bounded descendants, including
under an enabled ancestor. This normalization happens after the existing raw
layer merge; it does not rewrite JSONC files, imported profiles, raw settings
layers or extension-visible values. `explicit`, `always` and true enable
explicit-save work. After-delay save resolves off for object and array policies.

The original fourteen-case raw settings/order observations remain historical
evidence. Current executable eligibility uses the separately admitted stable
configuration cohort from revision `133897055d5c524ed1fd0b51866edf95f1faffd8`,
run `38063145339`. Its genuine Linux/macOS/Windows artifacts retain fourteen
cases and 162 snapshots per platform. Stable readiness independently proves
canonical configuration and installed save participants before target gestures;
false-child/family observations match explicit-never eligibility. The earlier
false-versus-never observations are preserved without reclassifying their cause.

Admission bounds entries, kind text, layers, selectors and matching groups before
copying policy metadata. Malformed winning values fail closed rather than enabling
a lower policy. The historical foundation passed five pure Rust cases, including its fourteen
raw matrices. Current qualification passes nine policy tests, separating raw merge/order
checks from stable executable eligibility and checking layered boolean
equivalence, exclusions and read-only Unicode/CRLF JSONC. Seven debug native
terminal journeys pass on debug and optimized executables, including false-child
and false-family cases. See
[reference validation](SAVE_REFERENCE_VALIDATION.md) for exact native/metadata
evidence and outstanding platform qualification. These establish the supported
projection rather than full save parity.
