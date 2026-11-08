"""Harness failure-path checks; no product or model success is simulated."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class RecordedLifecycleFailureTests(unittest.TestCase):
    def run_failure(self, with_credentials):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.db"
            with sqlite3.connect(source) as db:
                db.executescript("CREATE TABLE conversations(id TEXT);"
                    "CREATE TABLE runs(id TEXT,conversation_id TEXT,status TEXT);"
                    "INSERT INTO conversations VALUES('recorded');")
            original = None
            if with_credentials:
                vault = source.parent / ".nola-host-secrets" / (hashlib.sha256(os.fsencode(source.resolve())).hexdigest() + ".sqlite")
                vault.parent.mkdir(mode=0o700)
                with sqlite3.connect(vault) as db:
                    db.execute("CREATE TABLE fixture(value TEXT)")
                    db.execute("INSERT INTO fixture VALUES('test-only')")
                original = vault.read_bytes()
            command = [sys.executable, str(ROOT / "scripts/test_recorded_cli_lifecycle.py"),
                "--source-db", str(source), "--binary", str(source.parent / "missing-binary"),
                "--session", "recorded", "--project", directory]
            if not with_credentials:
                command += ["--exercise-model-exit"]
            result = subprocess.run(command, capture_output=True, text=True, timeout=10)
            self.assertNotEqual(result.returncode, 0)
            evidence = Path(result.stdout.splitlines()[0])
            try:
                report = json.loads((evidence / "result.json").read_text())
                self.assertEqual(report["status"], "failed")
                self.assertEqual(report["failure_type"], "FileNotFoundError")
                self.assertFalse((evidence / ".nola-host-secrets").exists())
                self.assertNotIn("real_model_exit_cancelled", report)
                if original is not None:
                    self.assertEqual(vault.read_bytes(), original)
            finally:
                shutil.rmtree(evidence)

    def test_binary_failure_after_copy_cleans_credentials_and_records_failure(self):
        self.run_failure(True)

    def test_missing_model_credentials_is_recorded_without_starting_cli(self):
        self.run_failure(False)


if __name__ == "__main__":
    unittest.main()
