# SPDX-License-Identifier: Apache-2.0
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

from evals.nola_task_acceptance import contract, invoke, run


class TaskAcceptanceTests(unittest.TestCase):
    def checker(self, root, code="raise SystemExit(0)"):
        return dict(argv=[sys.executable, "-c", code], cwd=str(root), timeout_seconds=1, public=True)

    def test_contract_is_explicit_public_and_not_model_selected(self):
        value = self.checker(Path("/tmp"))
        self.assertEqual(contract(value), value)
        for changed in [dict(value, public=False), dict(value, timeout_seconds=float("nan")),
                        dict(value, argv="echo ok"), dict(value, cwd="relative"),
                        dict(value, inferred_from_instruction=True)]:
            with self.assertRaises(ValueError):
                contract(changed)

    def test_process_success_does_not_replace_acceptance_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report = root / "report.json"
            responses = [dict(status="exited", exit_code=0, output="", output_truncated=False),
                         dict(status="exited", exit_code=1, output="assertion failed", output_truncated=False),
                         dict(status="exited", exit_code=0, output="", output_truncated=False),
                         dict(status="exited", exit_code=0, output="", output_truncated=False)]
            with patch("evals.nola_task_acceptance.invoke", side_effect=responses) as execution:
                self.assertEqual(run(["agent"], "original task", self.checker(root), 10, report), 0)
            value = json.loads(report.read_text())
            self.assertEqual(len(value["attempts"]), 2)
            self.assertEqual(value["attempts"][0]["public_check"]["exit_code"], 1)
            self.assertIn("assertion failed", execution.call_args_list[2].kwargs["prompt"])
            self.assertEqual(value["acceptance"], "passed_configured_check")
            self.assertEqual(value["canonical_run_result"], "not_modified")
            self.assertEqual(value["official_benchmark_result"], "not_evaluated_here")

    def test_unavailable_check_is_not_passed_and_does_not_spin(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            checker = self.checker(root)
            checker["argv"] = [str(root / "missing")]
            report = root / "report.json"
            code = run([sys.executable, "-c", "pass"], "task", checker, 2, report)
            self.assertEqual(code, 1)
            self.assertEqual(json.loads(report.read_text())["acceptance"], "unavailable")

    def test_real_process_retry_uses_failed_public_check_feedback(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result = root / "answer"
            agent = [sys.executable, "-c",
                     "import sys; from pathlib import Path; text=sys.stdin.read(); "
                     f"Path({str(result)!r}).write_text('fixed' if 'did not pass' in text else 'wrong')"]
            checker = self.checker(root, f"from pathlib import Path; assert Path({str(result)!r}).read_text() == 'fixed'")
            report = root / "report.json"
            self.assertEqual(run(agent, "original task", checker, 3, report), 0)
            attempts = json.loads(report.read_text())["attempts"]
            self.assertEqual(len(attempts), 2)
            self.assertNotEqual(attempts[0]["public_check"]["exit_code"], 0)
            self.assertEqual(attempts[1]["public_check"]["exit_code"], 0)

    def test_retries_share_one_deadline_instead_of_restarting_budget(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report = root / "report.json"
            success = dict(status="exited", exit_code=0, output="", output_truncated=False)
            failure = dict(success, exit_code=1)
            with patch("evals.nola_task_acceptance.time.monotonic", side_effect=[0, 0, 1, 2, 5, 6, 11]), \
                 patch("evals.nola_task_acceptance.invoke", side_effect=[success, failure, success]) as execution:
                self.assertEqual(run(["agent"], "task", self.checker(root), 10, report), 124)
            self.assertEqual(execution.call_args_list[0].kwargs["seconds"], 9)
            self.assertEqual(execution.call_args_list[2].kwargs["seconds"], 4)
            self.assertEqual(json.loads(report.read_text())["acceptance"], "deadline_exhausted")

    def test_checker_output_is_bounded_and_exit_status_is_not_text_inferred(self):
        result = invoke([sys.executable, "-c", "print('x'*100000); print('Traceback: failure')"],
                        seconds=2, capture=True)
        self.assertEqual(result["exit_code"], 0)
        self.assertTrue(result["output_truncated"])
        self.assertLessEqual(len(result["output"].encode()), 8192)
        self.assertIn("Traceback", result["output"])

    def test_timeout_and_nonzero_agent_cannot_become_acceptance(self):
        result = invoke([sys.executable, "-c", "import time; time.sleep(30)"], seconds=.05, capture=True)
        self.assertEqual(result["status"], "timeout")
        self.assertIsNone(result["exit_code"])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report = root / "report.json"
            code = run([sys.executable, "-c", "raise SystemExit(2)"], "task", self.checker(root), 2, report)
            self.assertEqual(code, 2)
            self.assertNotIn("public_check", json.loads(report.read_text())["attempts"][0])


if __name__ == "__main__":
    unittest.main()
