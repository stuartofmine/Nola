# SPDX-License-Identifier: Apache-2.0
"""Concrete emulator selection must not fall through to an attached phone."""
from pathlib import Path
import shlex
import subprocess
import unittest


class OwnedAndroidDeviceSelectionTests(unittest.TestCase):
    def invoke(self, body):
        library = Path(__file__).resolve().parents[1]/'lib/real_android_device.sh'
        script = 'source ' + shlex.quote(str(library)) + '\n' + body
        return subprocess.run(['bash', '-c', script], capture_output=True, text=True, timeout=5)

    def test_requested_emulator_wins_over_first_attached_phone(self):
        result = self.invoke('''
NOLA_ANDROID_REAL_SERIAL=emulator-5580
real_android_adb() { printf 'List of devices attached\nphone device\nemulator-5580 device\n'; }
real_android_device
''')
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), 'emulator-5580')

    def test_missing_owned_emulator_does_not_recover_server_or_start_another_device(self):
        result = self.invoke('''
NOLA_ANDROID_REAL_SERIAL=emulator-5580
real_android_adb() { printf 'List of devices attached\nphone device\n'; }
real_android_recover_adb() { echo UNEXPECTED_RECOVERY; }
real_android_require_device
''')
        self.assertEqual(result.returncode, 2)
        self.assertNotIn('UNEXPECTED_RECOVERY', result.stdout)
        self.assertIn('explicitly selected', result.stderr)
