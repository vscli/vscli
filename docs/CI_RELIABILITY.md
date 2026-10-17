# CI evidence publication and background status

## Code-action fixture evidence

The code-action provider fixtures stage their bounded evidence records in exclusively created files in the destination directory, finish and close each record, then rename it over the final path. A reader that observes a final JSON path sees a complete record. Existing records are never truncated or removed before replacement. Write and rename errors propagate; owned temporary files are cleaned up without deleting another owner’s file.

This fixes the empty-record race seen in Linux CI run 38074990803 at main fb360539. It changes fixture evidence publication, not editor persistence. Original JSON payloads, command markers, document identities, parser assertions and deadlines remain unchanged. Atomic visibility is distinct from durability after a system failure.

Local qualification: all 22 Node code-action/publication tests pass, including 11 deterministic publication-boundary and error tests; native extension code-action integration tests pass (5 passed, 1 opt-in ignored). Formatting and strict locked all-target Clippy pass. Cross-platform qualification remains a fresh CI gate.

## Syntax cancellation and status ownership

Windows CI run 38075085037 exposed a canceled syntax setup overwriting a command-bearing save-action notice. Cancellation can follow visibility changes or an existing setup/parsing deadline; the log does not establish which cause occurred. The syntax engine now records that cancellation authority on the actual request, discards canceled success and error replies, and preserves the occupied slot until terminal acknowledgement. Deadline cancellation retains bounded retries; current noncanceled errors remain visible. No request queue, parsing deadline, save fixture or assertion was relaxed.

All three new regressions fail against the preceding implementation: visibility ABA rejects canceled success/error, canceled setup preserves the actual App status, and deadline cancellation stays quiet with bounded retries. Corrected local qualification passes all 14 syntax tests, the unchanged actual-save regression, all 1,027 locked all-target Rust tests across 51 reports (23 opt-in integrations ignored), formatting, strict Clippy and the terminal C++ color-stability journey. Independent source review found no material cancellation/slot/retry issues. Fresh platform CI remains required.

## Integration on the merged display foundation

After PR #71 merged, the same two repair implementations were rebased onto main 3cc75ab9. Syntax and fixture helper/test bytes are identical to preceding head 2aa40ca5. The combined source passes 1,087 locked all-target Rust tests/51 reports (23 opt-in integrations ignored), all 22 Node code-action/publication tests, formatting, strict Clippy, build and the native terminal color-stability journey. Fresh exact-head CI remains required.

The preceding CI run 38076975970 passed Linux, Windows, quality, commit style and real protocol checks. Its macOS Rust workload passed, but the upstream stable-save reference harness failed a warm API readiness assertion before target authorization. Exact artifact 11678477943 and its ten source blobs were independently verified. Formatter/configuration setup passed and the provider callback returned all four declared kinds; the capture did not retain the eventual public API result before its count assertion. Cancellation, filtering and timing remain unproven. This is distinct from the two repaired native failures; a separately source-bound diagnostic observer is needed. No original failure was reclassified as a success, no count/deadline was relaxed, and no local reference editor was launched.

## Migration release checks and canonical workspace identity

[CI run 39060987348](https://github.com/vscli/vscli/actions/runs/39060987348) at `9580f97` failed on Linux, macOS, Windows and the real
protocol-server job. Its Linux save-action fixture expired before the intended
post-action publication case; its macOS logs included `/var` versus `/private/var`
expectations and filesystem-invalid path/name fixtures. The Windows URI fixture
expected an uppercase drive despite upstream `URI.fsPath` normalization. A cached
upstream checkout failed its pinned revision check; capture staging also encountered
an existing artifact directory or an unstarted producer.

A local aliased `TMPDIR` reproduced three document-reference failures. Canonical
workspace identity at startup fixes repeated relative extension opens; native
navigation now owns clean hidden models until its pending alias resolution ends,
even after the API customer retires. Reopen tests drive the asynchronous native
lane explicitly. Platform fixtures compare canonical resource paths, allow wrapped
settings-path presentation, and respect native path/name representability. The
post-publication save test isolates its transport failure from the separately
qualified 1,500 ms production deadline; production deadlines are unchanged.

Pinned script-prepared upstream packages now use the runner's temporary directory,
separate from the Rust build cache. Trusted revision verification precedes fresh
capture setup. Existing raw/staged evidence is moved aside without replacement;
CI archives it outside the cached target directory. Staging/upload runs only after
capture setup succeeded, retaining partial producer evidence without adding an
unrelated missing-directory error when earlier steps fail. Existing strict source,
revision and complete-artifact consumer admission remains unchanged.

Local checks and exact source/binary identity are recorded in
[release-check repair receipts](validation/migration-release-checks.json). Actual
macOS/Windows and required exact-head GitHub results remain separate gates; local
alias coverage does not establish those platforms' release qualification.

At `417a83d`, the real protocol job and all three dedicated stable-save captures
pass. The macOS library run reaches 1,268 passing tests and one welcome assertion:
a long unbroken settings path is omitted by word wrapping. Welcome now wraps
literal paths by grapheme/cell width and explicitly elides the beginning beyond
two rows, keeping the settings filename visible. Three native welcome controls,
two terminal journeys, formatting and strict Clippy pass locally; see
[focused path-rendering evidence](validation/long-profile-path.json). This follow-up
does not qualify the separately failing Windows native path/URI boundary.

The Linux core terminal run at `417a83d` also exposed an overbroad quiet-startup
rule: an explicitly loaded or restored theme no longer announced its name.
Selected startup themes now append their notice, preserving the existing startup
or extension failure. Automatic fallback themes and background customization
refreshes stay quiet; manual choices retain their direct acknowledgement. The
12 native theme controls and all 35 core terminal scenarios pass locally.

Windows at `417a83d` passed Node controls but failed 51 native cases. Verbatim
filesystem paths were interpreted as URI authorities (`file://%3F/...`), and
native/public URI spellings differed for drive colons and reserved characters.
Package descriptors and the host workspace now use ordinary public filesystem
paths while native paths retain their original identity. Extension mirror URIs
follow the vendored upstream formatter; LSP wire spelling stays separate. A real
public URI/cache/WorkspaceEdit/save/Undo case, a direct upstream formatter oracle,
799 host tests, 1,651 full native regression tests and 38 terminal scenarios pass
locally. The receipt records the later non-allocating test assertion cleanup and
its focused final check separately. Windows-specific controls and fresh platform
CI remain required. See [native extension URI evidence](validation/native-extension-uris.json).


At `f142595`, Windows failures fall from 51 to 37 native cases; Linux and macOS
have one failing library case each. These are test counts, not API coverage.
Completed file-operation reference removal now uses the same public URI formatter
as the original model. Windows journal flushes reopen the retained object with
write access, compare its file identity, and still propagate flush errors; ordinary
inspection remains read-only. Permission restoration acquires its flush handle
before applying a read-only attribute. Microsoft documents the
[write-access requirement](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers)
and [retained-object reopening](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-reopenfile).

Fixtures now use native absolute resource paths, upstream-formatted public URIs,
platform-correct file URL decoding and a genuinely changed EOL on either OS.
Provider navigation waits for its actual file loader, retaining the original
identity/cursor assertions. The positive paused-receipt fixture extends its own
five-second acknowledgement timer so loaded CI does not turn identity checks into
a throughput test; production deadlines and independent timeout controls are unchanged.
A 164-character temporary root containing a space reproduces the welcome assertion:
including the explorer border between rows corrupts its reconstruction of a
correctly wrapped settings path. The assertion now reads only welcome content and
checks the intended two-row label and filename.

On this source, 1,652 native tests, 11 relevant host lifecycle controls, two document
terminal journeys, two welcome terminal journeys, formatting, strict Clippy and
build pass locally. The long-root welcome controls also pass. Actual macOS/Windows
qualification remains pending; see [platform repair receipts](validation/platform-repair.json).
