# Extension workspace scope

The optional host exposes `workspace.getWorkspaceFolder`, `asRelativePath`,
`name`, `workspaceFile`, `workspaceFolders` and deprecated `rootPath` using a
shared workspace metadata service. The current native single-folder workspace
is published from the native root. Getter descriptors remain live; arrays are
fresh snapshots and unchanged folders retain identity. `workspaceFile` is
undefined for this single-folder backend; no configuration file is invented.

Folder lookup follows source URI segment boundaries and chooses the deepest
matching folder. Scheme/authority are insensitive, query/fragment do not affect
lookup, and filesystem capabilities control path case. Relative strings use
`Uri.file` semantics without cwd resolution. Outside inputs stay outside;
multi-root metadata calculations include folder names by default. Lookup is a
label/navigation calculation, not a filesystem authorization or symlink proof.
Native relative document requests use the privately retained native root;
extension mutation of a public folder URI cannot redirect that root.

Local evidence: 20 pure module tests, three facade tests and the complete
261-test host suite pass. The module tests include future metadata/trust
publication preparation; they do not establish native multi-root or trust
support. All 134 native extension-related tests pass. The real extension
filesystem PTY workflow checks folder identity, relative paths, workspace name
and configuration-file absence alongside dirty-buffer/undo/recovery behavior.
Formatting and strict Clippy pass.

Native multi-root mutation, `.code-workspace`, trust state and workspace change
events are pending. `updateWorkspaceFolders`, `isTrusted`,
`onDidGrantWorkspaceTrust` and `onDidChangeWorkspaceFolders` remain explicit
unsupported facade members. The internal metadata service does not pretend to
create native workspaces or grant security permissions.

Metadata publication is bounded to 128 folders, 8 KiB per URI, 4 KiB names and
2 MiB conservative prepared metadata. Public folder objects use frozen getters
with private name/index bindings; upstream runtime objects are mutable despite
readonly declarations. This adaptation, host Node path math, telemetry and
actual Windows/macOS behavior still need qualification before any full workspace
parity claim. Full extension API compatibility remains incomplete.

[Source receipts](../extension-host/workspace-scope.provenance.json) pin stable
1.141.0 at `2a59476c9bfcb90b3ddc372c36762471b7dfad1c`:

- [Workspace implementation](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/workbench/api/common/extHostWorkspace.ts)
- [URI resource calculations](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/base/common/resources.ts)
- [URI segment lookup](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/base/common/ternarySearchTree.ts)

Selected extensions can also use native `workspace.findFiles` and shared
`RelativePattern` values. See [file discovery](NATIVE_FILE_SEARCH.md) for default
exclusions, cancellation, bounds and qualification.
