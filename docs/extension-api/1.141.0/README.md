# Stable VS Code extension API audit

Scope: the complete public **VS Code 1.141.0** declaration, immutable official
commit [`2a59476c9bfcb90b3ddc372c36762471b7dfad1c`](https://github.com/microsoft/vscode/blob/2a59476c9bfcb90b3ddc372c36762471b7dfad1c/src/vscode-dts/vscode.d.ts).
Official release and stable update-service responses agreed on this pin on
2026-10-11. The complete MIT source license and exact declaration are preserved.
The static audit executes no JavaScript or native editor. Separate implementation
and runtime evidence is documented in [EXTENSION_API.md](../../EXTENSION_API.md).
No local VS Code instance was launched.

The inventory contains 776 declaration entries and 2,366 members, overloads and
anonymous nested type signatures: 16 namespaces, 122 classes, 302 interfaces,
64 enums, 20 aliases, 154 function signatures and 98 value declarations. Some
names repeat for overloads; these are counts of declaration entries, not features
or a compatibility percentage. The global `Thenable` helper is retained separately.
All named declaration nodes are checked against the parsed source traversal;
unknown declaration/member syntax causes generation to fail.

The optional proposed API scope is separately enumerated in
`proposed-scope.json`: 180 proposed declaration filenames/blob identities from
the same commit. Their contents and behavioral contracts are outside this stable
inventory. None has been silently classified as stable or implemented. The older
1.95 declaration and ID delta are retained for reference, not as the new scope.

## Files

- `stable-api-inventory.json`: complete stable declaration and signature inventory,
  source byte ranges, overload identity and source SHA.
- `coverage.json`: one fail-closed static status per inventory entry, with current
  host evidence. Names, fields and constructor availability do not qualify overloads.
- `host-static-inventory.json`: copied JavaScript source AST classes/facades/constants.
  This includes internal classes; it is not itself the public export inventory.
- `namespace-summary.json`, `limitations.json`, `implementation-order.json`: compact
  surface summary, source-confirmed restrictions and full-scope implementation order.
- `upstream/`: exact official source, source license, package version, older declaration.
- `current/`: exact copied host/native gateway/documentation bytes for this audit.
  The shared tree may have been dirty; copied SHA values are authoritative, not HEAD.
- `provenance.json`, `MANIFEST.json`: authoritative release/source/parser/copy receipts.

The facade currently exposes five namespaces (`window`, `workspace`, `commands`,
`extensions`, `languages`); eleven namespaces are absent. This is a surface fact,
not a conclusion that native features with similar names do not exist. Native
implementation, extension API routing and API/unchanged-package qualification
are distinct obligations. Existing host tests and docs were copied as evidence
inputs; this audit did not rerun their semantic tests.

Every coverage row has `qualified:false`: this audit grants no new behavioral
qualification. It does **not** mean that previously implemented/tested features
ceased to exist. `source_present_unqualified` identifies source presence;
`explicitly_unavailable` identifies exposed methods that reject; `type_only_contract`
keeps interfaces/options/callback shapes without inventing runtime constructors;
`dto_field_unqualified` does not assume writable properties are consumed. Absent
methods, namespaces and callback handling remain explicit. No `implemented` status
or guessed parity percentage is used.

## Reproduce and verify

Verification needs only Python's standard library:

```sh
python3 scripts/verify.py
python3 scripts/test_inventory.py
```

Regeneration optionally uses the pinned Python Tree-sitter packages in
`scripts/requirements.txt`. It does not execute JavaScript:

```sh
python3 -m pip install -r scripts/requirements.txt
python3 scripts/inventory.py
python3 scripts/coverage.py
```

Both generators accept `--root AUDIT_DIRECTORY` for an independently copied audit
cohort. They never copy mutable host input or overwrite the official source pin.
A refreshed cohort must record fresh snapshot provenance/manifests. Regeneration
of this frozen cohort is deterministic; changing inputs invalidates its manifest.
All nine pure integrity tests validate the unmodified positive baseline first,
then assert the intended missing/duplicate/unknown-status/source/qualification
failure. Runtime, unchanged-extension and terminal qualification remain separate.

This committed cohort lives in `docs/extension-api/1.141.0/`. It was refreshed
after the initial value/event, snippet and cancellation implementations. Earlier
agent snapshots remain archived separately; provenance records the original package
manifest and the implementation commit. The optional Python parser dependency is
used only for tooling. Native startup has no Python or JavaScript dependency.
Run the verification commands above from this directory.
