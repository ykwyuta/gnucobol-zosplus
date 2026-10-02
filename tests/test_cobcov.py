"""Tests for the on-disk COBOL branch coverage contract."""

import json
import pathlib
import subprocess
import sys
import tempfile
import unittest


REPORTER = pathlib.Path(__file__).resolve().parents[1] / "bin" / "cobcov"


class CobcovReportTests(unittest.TestCase):
    def test_merges_runs_and_preserves_uncovered_outcome(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            build_id = "0123456789abcdef"
            outcomes = [{"id": i, "file": "main.cbl", "program": "MAIN",
                         "line": 7, "column": 0, "kind": "IF", "result": result}
                        for i, result in enumerate(("true", "false"))]
            (root / f"cobcov-{build_id}.json").write_text(
                json.dumps({"version": 1, "build_id": build_id,
                            "scope": "if", "outcomes": outcomes}), encoding="utf-8")
            for index, count in enumerate((2, 3)):
                (root / f"run-{index}.counts").write_text(
                    f"COBCOV1\n{build_id}\t0\t{count}\n", encoding="ascii")
            result = subprocess.run(
                [sys.executable, str(REPORTER), "report", str(root), str(root),
                 "--format", "json", "--fail-under-branches=60"],
                capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 1)
            report = json.loads(result.stdout)
            self.assertEqual((report["covered"], report["total"]), (1, 2))
            self.assertEqual([row["count"] for row in report["outcomes"]], [5, 0])
            lcov = subprocess.run(
                [sys.executable, str(REPORTER), "report", str(root), str(root),
                 "--format", "lcov"],
                capture_output=True, text=True, check=True)
            self.assertEqual(lcov.stdout,
                             "SF:main.cbl\nBRDA:7,0,0,5\nBRDA:7,0,1,0\n"
                             "BRF:2\nBRH:1\nend_of_record\n")

    def test_rejects_counter_for_another_build(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "cobcov-0123456789abcdef.json").write_text(
                json.dumps({"version": 1, "build_id": "0123456789abcdef",
                            "scope": "if", "outcomes": []}), encoding="utf-8")
            (root / "other.counts").write_text(
                "COBCOV1\nffffffffffffffff\t0\t1\n", encoding="ascii")
            result = subprocess.run(
                [sys.executable, str(REPORTER), "report", str(root), str(root)],
                capture_output=True, text=True, check=False)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("does not match map", result.stderr)


if __name__ == "__main__":
    unittest.main()
