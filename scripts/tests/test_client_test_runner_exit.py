# SPDX-License-Identifier: Apache-2.0
"""Check test runner status propagation without invoking Xcode or devices."""
from pathlib import Path
import subprocess
import unittest


class ClientTestRunnerExitTests(unittest.TestCase):
    def run_pipeline(self, build_status, formatter_status):
        script = (Path(__file__).resolve().parents[1] / "run_client_tests.sh").read_text()
        self.assertTrue(script.startswith("#!/bin/bash\n"))
        self.assertIn("set -euo pipefail", script)
        pipeline = script.split("# Pretty output via xcbeautify", 1)[1]
        pipeline = pipeline.split("\n", 1)[1]
        commands = (
            "set -euo pipefail\n"
            f"_run_xcodebuild() {{ echo fixture; return {build_status}; }}\n"
            f"xcbeautify() {{ /bin/cat; return {formatter_status}; }}\n"
            + pipeline
        )
        return subprocess.run(["/bin/bash", "-c", commands], capture_output=True).returncode

    def test_build_failure_is_not_hidden_by_successful_formatter(self):
        self.assertEqual(self.run_pipeline(65, 0), 65)

    def test_formatter_failure_is_not_success(self):
        self.assertEqual(self.run_pipeline(0, 7), 7)

    def test_success_requires_both_processes(self):
        self.assertEqual(self.run_pipeline(0, 0), 0)

    def test_simulator_signing_keeps_static_framework_target_policy(self):
        script = (Path(__file__).resolve().parents[1] / "run_client_tests.sh").read_text()
        branch = script.split('case "${SIMULATOR_SIGNING:-unsigned}" in', 1)[1].split('esac', 1)[0]
        for mode, expected in [('unsigned', 'CODE_SIGNING_ALLOWED=NO'), ('adhoc', 'CODE_SIGN_IDENTITY=-')]:
            result = subprocess.run(['/bin/bash', '-c',
                'case "$1" in' + branch + 'esac\nprintf "%s" "${IOS_TEST_SIGNING_FLAGS[@]}"',
                'fixture', mode], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0)
            self.assertEqual(result.stdout, expected)


if __name__ == "__main__":
    unittest.main()
