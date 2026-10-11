#!/usr/bin/env python3
"""Measure committed runtime name presence, never behavioral compatibility.

Copies committed host modules into a temporary directory, inspects the actual
facade without loading extensions or invoking API operations, and compares names
with the committed stable declaration inventory. Working-tree edits are excluded.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

REPO = Path(__file__).resolve().parents[1]
INVENTORY = "docs/extension-api/1.141.0/stable-api-inventory.json"
DECLARATION = "docs/extension-api/1.141.0/upstream/vscode.d.ts"
RUNTIME_KINDS = {
    "class_declaration", "enum_declaration", "function_signature",
    "internal_module", "lexical_declaration",
}
TYPE_ONLY_KINDS = {"interface_declaration", "type_alias_declaration"}
INSPECT = r"""
'use strict';
const {createApi} = require(process.argv[1] + '/extension-host/api.cjs');
const {createActivation} = require(process.argv[1] + '/extension-host/activation.cjs');
const runtime = createApi(async () => {
  throw new Error('Surface audit must never invoke API operations');
}, () => {}, {session: 1, languageProviders: true});
runtime.setActivation(createActivation([], {}));
const facade = runtime.forExtension('');
const names = new Set();
function collect(value, prefix) {
  for (const [name, descriptor] of Object.entries(Object.getOwnPropertyDescriptors(value))) {
    names.add(prefix + '.' + name);
    if (prefix === 'vscode' && descriptor.value && typeof descriptor.value === 'object') {
      collect(descriptor.value, prefix + '.' + name);
    }
  }
}
collect(facade, 'vscode');
// This namespace is a getter over the real empty activation registry. No
// selected package, provider callback, native document or API result is invented.
collect(facade.extensions, 'vscode.extensions');
process.stdout.write(JSON.stringify([...names].sort()));
"""


def git(*arguments):
    return subprocess.check_output(["git", *arguments], cwd=REPO)


def measure(revision, node):
    commit = git("rev-parse", "--verify", "--end-of-options", revision + "^{commit}").decode().strip()
    inventory = json.loads(git("show", commit + ":" + INVENTORY))
    declaration = git("show", commit + ":" + DECLARATION)
    if hashlib.sha256(declaration).hexdigest() != inventory["source_sha256"]:
        raise ValueError("Committed declaration and inventory source hash differ")
    entries = {}
    type_only_count = 0
    for entry in inventory["declarations"]:
        kind = entry["kind"]
        if kind in TYPE_ONLY_KINDS:
            type_only_count += 1
            continue
        if kind not in RUNTIME_KINDS:
            raise ValueError("Unknown declaration kind: " + kind)
        name = entry["parent"] + "." + entry["name"]
        previous = entries.get(name)
        if previous and previous["kind"] != kind:
            raise ValueError("Conflicting declaration kinds: " + name)
        entries[name] = entry

    with tempfile.TemporaryDirectory(prefix="vscli-api-surface-") as temporary:
        root = Path(temporary)
        files = git("ls-tree", "-r", "--name-only", commit, "extension-host").decode().splitlines()
        for name in files:
            if name.endswith(".cjs") and ".test." not in name:
                target = root / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(git("show", commit + ":" + name))
        result = subprocess.run(
            [node, "-e", INSPECT, str(root)], capture_output=True, text=True,
            check=True, timeout=30,
        )
        names = json.loads(result.stdout)
        if not isinstance(names, list) or any(not isinstance(name, str) for name in names):
            raise ValueError("Invalid facade introspection output")
        exposed = set(names)
    present = sorted(set(entries) & exposed)
    missing = sorted(set(entries) - exposed)
    by_kind = {}
    for kind in sorted(RUNTIME_KINDS):
        candidates = {name for name, entry in entries.items() if entry["kind"] == kind}
        by_kind[kind] = {"present": len(candidates & exposed), "total": len(candidates)}
    return {
        "contract": "vscli-committed-runtime-name-presence-v1",
        "shipped_commit": commit,
        "stable_declaration_sha256": inventory["source_sha256"],
        "metric": "Unique runtime declaration name presence; overloads collapsed, type-only declarations excluded",
        "qualification": "No behavioral compatibility qualification. Does not verify overloads, members, constructor behavior or native backends.",
        "working_tree_included": False,
        "extensions_loaded": False,
        "type_only_declarations_excluded": type_only_count,
        "present": len(present), "total": len(entries),
        "surface_percent": round(100 * len(present) / len(entries), 2),
        "by_kind": by_kind,
        "present_names": present, "missing_names": missing,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--revision", default="HEAD", help="Committed revision to inspect")
    parser.add_argument("--node", default="node", help="Node executable for isolated host introspection")
    parser.add_argument("--output", type=Path, help="Save the complete JSON report")
    args = parser.parse_args()
    report = measure(args.revision, args.node)
    if args.output:
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    summary = {key: value for key, value in report.items() if key not in {"present_names", "missing_names"}}
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
