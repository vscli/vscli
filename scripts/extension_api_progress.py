#!/usr/bin/env python3
"""Live behavior progress over EVERY pinned stable declaration/member row.

The frozen inventory/audit is input only. Unassessed means evidence has not yet
been reconciled here, not that the API is absent. A passed workflow criterion
never grants whole-API parity or closes an untested overload/type-only contract.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import subprocess

REPO = Path(__file__).resolve().parents[1]
INVENTORY = "docs/extension-api/1.141.0/stable-api-inventory.json"
LEDGER = "docs/extension-api/progress.json"


def receipt_is_current(root, receipt):
    cohort = json.loads((root / receipt).read_text())["source_cohort"]
    paths = subprocess.check_output([
        "git", "-C", str(root), "ls-files", "--cached", "--others", "--exclude-standard",
        "-z", "--", *cohort["roots"],
    ]).decode().split("\0")
    files = sorted({path for path in paths if path and (root / path).is_file()})
    digest = hashlib.sha256()
    for path in files:
        digest.update(path.encode() + b"\0" + hashlib.sha256((root / path).read_bytes()).digest())
    return len(files) == cohort["files"] and digest.hexdigest() == cohort["sha256"]


def progress(root=REPO):
    inventory = json.loads((root / INVENTORY).read_text())
    ledger = json.loads((root / LEDGER).read_text())
    declaration = root / "docs/extension-api/1.141.0/upstream/vscode.d.ts"
    if hashlib.sha256(declaration.read_bytes()).hexdigest() != inventory["source_sha256"]:
        raise ValueError("Pinned declaration differs from the stable inventory")
    namespaces = {entry["id"] for entry in inventory["declarations"]
                  if entry["kind"] == "internal_module"}
    if set(ledger["namespaces"]) != namespaces or ledger["scope"] != "entire-stable":
        raise ValueError("Progress scope must retain every stable namespace")
    rows = ([{"kind": "declaration", "id": entry["id"], "overload": None}
             for entry in inventory["declarations"]]
            + [{"kind": "member", "id": entry["id"], "overload": entry["overload"]}
               for entry in inventory["members"]])
    rows.extend({"kind": "ambient", "id": "global." + entry["name"], "overload": None}
                for entry in inventory["ambient_globals"])
    ids = {entry["id"] for entry in rows}
    contracts = {}
    for contract in ledger["contracts"]:
        name = contract["id"]
        if name in contracts or not contract["api_ids"] or not set(contract["api_ids"]) <= ids:
            raise ValueError("Duplicate contract or unknown stable API: " + name)
        if not contract["remaining"] or not contract["criteria"]:
            raise ValueError("Partial workflow needs criteria and remaining obligations: " + name)
        for criterion in contract["criteria"]:
            if criterion["state"] not in {"pending", "passed"}:
                raise ValueError("Unknown criterion state: " + name)
            if criterion["state"] == "passed" and not criterion.get("evidence"):
                raise ValueError("Passed criterion lacks evidence: " + name)
            for evidence in criterion.get("evidence", []):
                path = Path(evidence["file"])
                if path.is_absolute() or ".." in path.parts:
                    raise ValueError("Evidence must be a repository file")
                if evidence.get("needle", "") not in (root / path).read_text():
                    raise ValueError("Missing evidence: " + str(path))
        contracts[name] = contract
    receipts = {}
    for contract in contracts.values():
        receipt = contract.get("validation_receipt")
        if receipt:
            if receipt not in receipts:
                receipts[receipt] = receipt_is_current(root, receipt)
            contract["evidence_current"] = receipts[receipt]
    for row in rows:
        row["contracts"] = [name for name, contract in contracts.items()
                            if row["id"] in contract["api_ids"]]
        row["status"] = "partial-contracts-tracked" if row["contracts"] else "unassessed"
        # Closing a workflow is not closing a declaration, overload or type.
        row["qualified"] = False
    groups = []
    for namespace in ["vscode", *sorted(namespaces)]:
        def belongs(row):
            match = next((ns for ns in namespaces
                          if row["id"] == ns or row["id"].startswith(ns + ".")), "vscode")
            return match == namespace
        members = [row for row in rows if belongs(row)]
        groups.append({"namespace": namespace, "rows": len(members),
                       "status_counts": dict(Counter(row["status"] for row in members))})
    return {"scope": ledger["scope"], "inventory_source_sha256": inventory["source_sha256"],
            "declarations": len(inventory["declarations"]), "members": len(inventory["members"]),
            "ambient_contracts": len(inventory["ambient_globals"]),
            "qualified_rows": 0, "namespaces": groups, "contracts": list(contracts.values()),
            "next_batch": ledger["next_batch"], "rows": rows}


def render(report):
    lines = ["Scope: entire stable API; " + str(report["declarations"]) + " declarations, "
             + str(report["members"]) + " member/overload/type rows, 16 namespaces.",
             "Also retained: " + str(report["ambient_contracts"]) + " ambient contract(s).",
             "Workflow criteria are local evidence, not an API completion percentage."]
    for contract in report["contracts"]:
        criteria = contract["criteria"]
        passed = sum(item["state"] == "passed" for item in criteria)
        current = contract.get("evidence_current")
        binding = ("; not integrated" if not passed else
                   "; source matches receipt" if current is True else
                   "; historical evidence, source differs" if current is False else
                   "; historical milestone evidence")
        lines.append(f"{contract['id']}: {passed}/{len(criteria)} named criteria passed{binding}")
    lines.append("Next batch: " + report["next_batch"]["name"])
    lines.extend("  [ ] " + criterion for criterion in report["next_batch"]["criteria"])
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", action="store_true", help="Include every stable inventory row")
    args = parser.parse_args()
    report = progress()
    print(json.dumps(report, indent=2) if args.json else render(report))


if __name__ == "__main__":
    main()
