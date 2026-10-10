# Native settings persistence

F1 → **View: Toggle Breadcrumbs** saves `breadcrumbs.enabled` in the active
native settings profile. An existing valid workspace root override selects
`.vscode/settings.json`; otherwise the write selects the configured user file.
An activated VS Code import supplies its native copy, so the original imported
settings and keybindings remain unchanged. An explicit `--settings FILE` selects
that file for both reading and writing.

The header responds immediately while the write runs in the background. A
failed or retired write removes that temporary presentation override and shows
a notice. Successful persistence forces a fresh background settings reload;
restart reads the persisted value. Ctrl+Shift+Period, or Cmd+Shift+Period on
macOS, also persists enabling Breadcrumbs through its original `toggleToOn`
command. Terminal delivery still depends on the negotiated keyboard protocol.

Writes replace only the existing scalar value span, or insert one root scalar.
All unrelated JSONC/JSON5 bytes, comments, order, Unicode, CRLF, extension values
and language blocks remain intact. Duplicate target keys, malformed input,
composite target values, oversized input/output and unsupported values are
refused. Existing Unix mode bits and read-only flags are preserved; read-only files and symlink
payloads are refused. Reads can still follow regular-file symlinks. ACLs,
extended attributes and complete filesystem metadata preservation are not
qualified by this implementation.

A winning language override refuses this root-only toggle with a notice naming
the block to edit. The pinned VS Code toggle writes global root configuration;
native Breadcrumbs additionally reads language overrides. Native persistence
writes explicit booleans, including the default `true`, rather than reproducing
VS Code's removal of a user override when restoring its default. Nested writes,
default removal, a settings form and a general settings-editing UI remain
outstanding. An `App` constructed without configuring settings retains its
existing session-only toggle behavior; normal CLI startup configures a profile.

One actual background write and one latest batch are retained. Same-target
uncommitted changes keep distinct keys and replace earlier values of the same
key; changed profile/path intents cannot inherit retired changes. Bounds are
16 scalar keys, 64 KiB serialized scalar values, 1 MiB input/output, 4 KiB paths,
128 document-model proofs and 512 KiB total model paths. A preparation expires
after six seconds without authorization. Worker capacity remains occupied until
temporary-file cleanup and lock release finish.

Before authorizing, the editor checks the profile, workspace, scope and retained
document identity/path/text epoch/save generation. Dirty visible, shared or
hidden settings buffers refuse the write. Worker-side native file identity
checks cover recovered path aliases and hard links; new or relocated file models
during preparation require a fresh toggle. Unresolvable historical model paths
require closing that model before retrying.

The worker writes and syncs an adjacent temporary file, then rechecks exact
baseline bytes, native file identity, permissions, parent identity, requested
parent aliases and the owned lock before atomic replacement. Missing files use
atomic no-clobber persistence. Preparation may create parent directories and a
stable `.vscli-write.lock` sidecar before authorization. The advisory lock
coordinates cooperating writers; an uncooperative process can still race the
last check and rename. Cleanup after an externally renamed parent is best
effort. Settings writes fence disk-watcher publications, and settings reloads
retain one actual worker while invalidating earlier results.

Implementation and test sources are available in `src/settings_write.rs`,
`src/settings_writer.rs`, `src/app/settings_persistence.rs`,
`tests/settings_persistence.rs` and `tests/settings_persistence_pty.py`.
The integrated candidate passes 657 ordinary Rust tests across 39 suites
(20 opt-in tests ignored), all 133 optional-host Node tests, 25 Python checks,
formatting and strict all-target Clippy. Its 54 added ordinary tests are included
in that total. The eight public settings journeys and six private App proofs are
subsets, alongside seven lossless patcher tests, 22 Unix worker tests, three
scope-selection tests, four loader tests and four watcher regressions.

All three new debug terminal reports passed across four isolated sessions with
Node absent from PATH and native language services disabled. They qualify exact
JSONC persistence/restart, original enhanced Ctrl+Shift+Period enabling,
workspace selection, source/settings UnicodeCRLF Save/Undo/Redo and terminal
restoration. Three existing Breadcrumbs terminal journeys and the actual clangd
Breadcrumbs workflow also passed, alongside the existing 35 terminal workflows.
All three new terminal reports also passed with the optimized executable.
Fresh platform CI qualification remains underway. Windows native file-ID FFI and reparse-point
behavior require the upcoming Windows checks; capability-dependent symlink tests
report a skip when Windows does not grant symlink creation privileges.

The first Windows run exposed existing-target replacement failures while the
baseline file handle remained open. The repair keeps that identity proof live:
Windows replacement uses Rust 1.99 `fs::rename`, whose
[implementation](https://github.com/rust-lang/rust/blob/1.99.0/library/std/src/sys/fs/windows.rs#L1248)
supports replacing open destinations through POSIX rename semantics. This avoids
the direct legacy `MoveFileExW` call in
[tempfile's Windows persistence path](https://github.com/Stebalien/tempfile/blob/v3.27.0/src/file/imp/windows.rs).
Microsoft documents that [POSIX replacement keeps existing open handles usable](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_file_rename_information).
The temporary marker is cleared before replacement, and failed renames retain
automatic temporary cleanup. Missing files retain atomic no-clobber behavior.
Regression tests assert exact JSONC/CRLF bytes, distinct replacement identity,
continued access through the old reader, and success/error cleanup. The repair
awaits fresh Windows CI qualification; unsupported replacement semantics fail
without releasing the baseline identity proof early.

The repaired head passed Windows and Linux CI. Its macOS Rust/reference checks
passed, but the dirty-settings PTY expected only the immediate refusal wording.
An aliased macOS temporary parent correctly reached the later native-identity
guard, which displayed `Settings write refused` and retained every file byte.
The PTY now accepts either explicit refusal route while keeping the exact
unchanged header, source/settings bytes and Undo/Redo assertions. Fresh complete
platform qualification remains pending for this oracle correction.

This feature does not implement
autosave, format-on-save, save-time code actions or full settings parity.

```sh
cargo test --locked --lib settings
cargo test --locked --test settings_persistence
python3 tests/settings_persistence_pty.py target/debug/vscli
```
