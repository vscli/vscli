# Native extension registry

The native Rust registry client implements bounded reads from the public
[Open VSX REST API](https://github.com/eclipse-openvsx/openvsx/wiki/Registry-API).
It uses HTTPS by default. A numeric-loopback HTTP registry can be selected for
local development and deterministic tests. HTTP downgrades from an HTTPS registry,
credentials in URLs and URL fragments are rejected. No network operation occurs
until a user requests registry work; JavaScript is unnecessary for the client.

Search reads at most 20 summaries and 1 MiB per metadata response. Because Open VSX
summaries can omit platform and license fields, the client resolves full metadata
for the displayed version before offering installation. It tries this machine's
target platform, then universal; missing compatible variants are omitted. Stable
versions only are selected. Runtime engine and native-module compatibility remain
separate from selecting platform metadata.

Downloads stream into a temporary file with a 128 MiB bound. When the registry
supplies a SHA-256 file, it must match the downloaded archive. The existing native
VSIX installer validates the archive and checks its manifest identity and version
against the selected metadata before atomically changing the installed registry.
Failures preserve the preceding installation and rollback generation. Provenance
records the archive digest and download URL. A matching checksum is an integrity
check against registry metadata, not publisher signature verification.

All reads have a 120-second operation deadline, five-second connection/resolution
limits, ten-second body waits, and at most five redirects per request. Cancellation
is checked between chunks and before installation begins. The native editor uses
its existing single extension-management worker slot; dismissing a loading view
hides that view while work finishes, and cannot release its slot or revive it over
a newer prompt. Dismissing an installation does not reverse a committed install.

Update checks inspect at most 128 installed packages, offering newer stable
semantic versions without downgrades. Unavailable/private packages and invalid
installed version strings produce per-package notices while other checks continue.
A request exceeding its operation deadline fails clearly. There is no automatic
update or automatic execution grant. Running extension hosts retain their immutable
selected package snapshots until explicitly restarted/reselected.

Four deterministic Rust HTTP/integrity tests cover platform fallback, summary
resolution, exact metadata versions, query encoding, byte/result/display limits,
unsupported targets/prereleases/URLs, cancellation, unavailable-package notices,
checksum/manifest mismatch, preserved registry bytes and rollback. These tests
run without external network access. Signature verification, engine/API/ABI
qualification, dependency downloading, authenticated registries, prerelease
selection, pagination, automatic updates and broad extension compatibility remain
outstanding.
