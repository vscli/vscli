#!/usr/bin/env python3
"""Guard complete scope and evidence in the live API progress report."""
from collections import Counter
import json
import hashlib
from pathlib import Path
import subprocess
import tempfile
import unittest

import extension_api_progress as tracker


class ProgressTests(unittest.TestCase):
    def test_changed_source_cannot_inherit_previous_test_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            subprocess.run(["git", "init", "--quiet", str(root)], check=True)
            source = root / "input"
            source.write_bytes(b"tested source")
            digest = hashlib.sha256(b"input\0" + hashlib.sha256(b"tested source").digest()).hexdigest()
            (root / "receipt.json").write_text(json.dumps({"source_cohort": {
                "roots": ["input"], "files": 1, "sha256": digest,
            }}))
            self.assertTrue(tracker.receipt_is_current(root, "receipt.json"))
            source.write_bytes(b"new untested source")
            self.assertFalse(tracker.receipt_is_current(root, "receipt.json"))
            source.unlink()
            self.assertFalse(tracker.receipt_is_current(root, "receipt.json"))

    def test_all_pinned_rows_overloads_type_only_and_namespaces_survive(self):
        report = tracker.progress()
        inventory = json.loads((tracker.REPO / tracker.INVENTORY).read_text())
        expected = Counter(("declaration", row["id"], None) for row in inventory["declarations"])
        expected.update(("member", row["id"], row["overload"]) for row in inventory["members"])
        expected.update(("ambient", "global." + row["name"], None)
                        for row in inventory["ambient_globals"])
        actual = Counter((row["kind"], row["id"], row["overload"]) for row in report["rows"])
        self.assertEqual(actual, expected)
        self.assertEqual(len(report["namespaces"]), 17)  # shared types + 16 namespaces
        self.assertEqual(sum(group["rows"] for group in report["namespaces"]), sum(expected.values()))
        self.assertFalse(any(row["qualified"] for row in report["rows"]))
        self.assertTrue(any(row["status"] == "unassessed" for row in report["rows"]))

    def test_narrowed_scope_unknown_api_and_missing_evidence_refuse_reporting(self):
        original = json.loads((tracker.REPO / tracker.LEDGER).read_text())
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            # Keep authentic immutable inputs and original evidence, replacing
            # only the ledger under test. This never mutates the repository.
            for relative in {tracker.INVENTORY,
                             "docs/extension-api/1.141.0/upstream/vscode.d.ts"}:
                destination = root / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.symlink_to(tracker.REPO / relative)
            evidence = {proof["file"] for contract in original["contracts"]
                        for criterion in contract["criteria"]
                        for proof in criterion.get("evidence", [])}
            for relative in evidence:
                destination = root / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.symlink_to(tracker.REPO / relative)
            for mutation in ("namespace", "scope", "api", "evidence"):
                with self.subTest(mutation=mutation):
                    ledger = json.loads(json.dumps(original))
                    if mutation == "namespace":
                        ledger["namespaces"].pop()
                    elif mutation == "scope":
                        ledger["scope"] = "text-only"
                    elif mutation == "api":
                        ledger["contracts"][0]["api_ids"].append("vscode.unknownApi")
                    else:
                        ledger["contracts"][0]["criteria"][0]["evidence"][0]["needle"] = "MISSING EVIDENCE"
                    (root / tracker.LEDGER).write_text(json.dumps(ledger))
                    with self.assertRaises(ValueError):
                        tracker.progress(root)


if __name__ == "__main__":
    unittest.main()
