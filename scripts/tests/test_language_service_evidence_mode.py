"""Evidence mode must not launch a model or read the user's configuration."""
import contextlib
import io
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import test_real_cli_language_service as acceptance


class EvidenceModeTests(unittest.TestCase):
    def test_read_only_mode_does_not_require_binary_server_or_credentials(self):
        with patch.object(sys, "argv", ["acceptance", "--verify-evidence", "/tmp/sample"]), \
                patch.object(acceptance, "verify_evidence", return_value={"passed": True}) as verify, \
                patch.object(Path, "home", side_effect=AssertionError("No user config")), \
                patch.object(acceptance, "run_cli", side_effect=AssertionError("No execution")), \
                contextlib.redirect_stdout(io.StringIO()) as output:
            acceptance.main()
        verify.assert_called_once_with(Path("/tmp/sample").resolve())
        self.assertIn('"passed": true', output.getvalue())

    def test_evidence_failure_is_not_swallowed(self):
        with patch.object(sys, "argv", ["acceptance", "--verify-evidence", "/tmp/sample"]), \
                patch.object(acceptance, "verify_evidence", side_effect=AssertionError("bad evidence")):
            with self.assertRaisesRegex(AssertionError, "bad evidence"):
                acceptance.main()

    def test_new_sample_still_requires_execution_parameters(self):
        with patch.object(sys, "argv", ["acceptance"]), contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as exit_result:
                acceptance.main()
        self.assertEqual(exit_result.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
