# SPDX-License-Identifier: Apache-2.0
"""Exercise the test runner guard with owned subprocesses, not real devices."""

import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest


path = Path(__file__).resolve().parents[1] / "run_mac_history_acceptance.py"
spec = importlib.util.spec_from_file_location("mac_history_runner", path)
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class MacHistoryRunnerTests(unittest.TestCase):
    def run_child(self, program, check):
        with tempfile.TemporaryDirectory(prefix="nola-runner-guard-test-") as temporary:
            log_path = Path(temporary) / "test.log"
            with log_path.open("wb") as log:
                child = subprocess.Popen([sys.executable, "-u", "-c", program],
                                         stdout=log, stderr=subprocess.STDOUT,
                                         start_new_session=True)
                try:
                    check(child, log_path)
                finally:
                    runner.stop_test_group(child)

    def test_remote_device_probe_terminates_owned_child(self):
        def check(child, log_path):
            started = time.monotonic()
            with self.assertRaises(runner.DeviceDiscoveryDetected):
                runner.wait_for_test_process(child, log_path, 5)
            self.assertIsNotNone(child.poll())
            self.assertLess(time.monotonic() - started, 3)
        self.run_child("import time; print('DTDKRemoteDeviceConnection: device is passcode protected'); time.sleep(30)", check)

    def test_marker_across_read_boundary_is_not_missed(self):
        def check(child, log_path):
            with self.assertRaises(runner.DeviceDiscoveryDetected):
                runner.wait_for_test_process(child, log_path, 5)
        self.run_child("import sys,time; sys.stdout.write('x' * 65525 + 'DTDKRemoteDeviceConnection'); sys.stdout.flush(); time.sleep(30)", check)

    def test_clean_child_exit_is_preserved(self):
        def check(child, log_path):
            self.assertEqual(runner.wait_for_test_process(child, log_path, 5), 0)
        self.run_child("print('native Mac tests completed')", check)

    def test_deadline_remains_bounded(self):
        def check(child, log_path):
            with self.assertRaises(subprocess.TimeoutExpired):
                runner.wait_for_test_process(child, log_path, 0.15)
        self.run_child("import time; time.sleep(30)", check)


if __name__ == "__main__":
    unittest.main()
