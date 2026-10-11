# Extension URI behavior

`vscode.Uri` now uses the pinned stable VS Code 1.141.0 URI runtime algorithms,
including their POSIX and Windows path helpers. It implements `parse`, `file`,
`from`, `joinPath`, nullable `with` changes, `fsPath`, encoded/minimal strings,
JSON caches and unchanged-object identity. This replaces the WHATWG URL adapter.
Relative `Uri.file` inputs are literal paths, not resolved against the process
working directory. Dot segments, percent escapes, opaque schemes, Unicode
authorities and Windows drive/UNC handling follow the source algorithms.

The self-contained CommonJS module runs only in the optional extension host;
no JavaScript runtime is needed by native Rust editing. Only `Uri` is exported
through the public facade. The isolated platform factory supports tests.
Trusted-data `revive` and structural `isUri` are value helpers, not native
resource permissions. Native owner, document-version, coordinate and canonical
path checks remain authoritative.

[Source receipts](../extension-host/uri.provenance.json) record immutable URLs,
sizes and hashes. [The generator](../scripts/generate_extension_uri.py) uses
hash-checked, explicit type-only erasure and dependency extraction, not a
general TypeScript compiler. It removes four `revive` overload declarations and
retains the implementation. Microsoft and Joyent MIT notices remain inline.
With a checkout of the receipt's exact upstream commit:

```sh
python3 scripts/generate_extension_uri.py --upstream /path/to/vscode --output /tmp/uri.cjs
cmp extension-host/uri.cjs /tmp/uri.cjs
```

The module reproduces byte-for-byte with SHA256
`b3dd37a5711e13aa4cc4bc4546ca73d56a040259bde4e81f6b5a440c11cf5523`.
Syntax checking, 18 meaningful URI tests and the full 223-test host suite pass.
The native extension-related test selection passes 133 tests; the dirty-buffer
filesystem PTY workflow passes with the embedded URI module. Windows algorithm
branches are tested locally; actual Windows/macOS terminals and upstream
runtime comparisons remain separate qualification. Full API parity is pending.

Primary sources:

- [URI implementation](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/base/common/uri.ts)
- [Platform path implementation](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vs/base/common/path.ts)
