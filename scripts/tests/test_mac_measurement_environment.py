# SPDX-License-Identifier: Apache-2.0
import importlib.util
from pathlib import Path
import json
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("mac_measurement_environment",
    Path(__file__).resolve().parents[1] / "mac_measurement_environment.py")
measurement = importlib.util.module_from_spec(spec)
spec.loader.exec_module(measurement)


class MacMeasurementEnvironmentTests(unittest.TestCase):
    def test_disabled_removes_inherited_measurement_without_touching_secrets(self):
        target = {"EnvironmentVariables": {"NOLA_MEASUREMENT_FILE": "/user/file",
            "NOLA_MEASUREMENT_BUILD_DIGEST": "untrusted", "API_KEY": "private"}}
        measurement.configure(target, "/missing", "/missing", False)
        self.assertEqual(target["EnvironmentVariables"], {"API_KEY": "private"})

    def test_actual_artifact_set_and_private_isolated_output(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            binaries = root / "Test/NolaMac.app/Contents/MacOS"
            binaries.mkdir(parents=True)
            for name in ("NolaMac", "NolaMac.debug.dylib", "nola-service", "nola-agent-host"):
                (binaries / name).write_bytes(name.encode())
            target = {"TestHostPath": "__TESTROOT__/Test/NolaMac.app/Contents/MacOS/NolaMac"}
            measurement.configure(target, root / "test.xctestrun", root, True)
            manifest = json.loads((root / "measurement/build.json").read_text())
            self.assertEqual(len(manifest["members"]), 4)
            self.assertEqual(target["EnvironmentVariables"]["NOLA_MEASUREMENT_BUILD_DIGEST"],
                             manifest["build_digest"])
            events = Path(target["EnvironmentVariables"]["NOLA_MEASUREMENT_FILE"])
            self.assertEqual(events.stat().st_mode & 0o777, 0o600)
            self.assertEqual(events.read_bytes(), b"")
            # A changed service changes the label even if Host is unchanged.
            (binaries / "nola-service").write_bytes(b"changed")
            second = root / "second"
            second.mkdir()
            measurement.configure(target, root / "test.xctestrun", second, True)
            self.assertNotEqual(target["EnvironmentVariables"]["NOLA_MEASUREMENT_BUILD_DIGEST"],
                                manifest["build_digest"])

    def test_xcode_app_bundle_host_path(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            app = root / "Test/Nola.app"
            binaries = app / "Contents/MacOS"
            binaries.mkdir(parents=True)
            (binaries / "nola-agent-host").write_bytes(b"host")
            target = {"TestHostPath": "__TESTROOT__/Test/Nola.app"}
            measurement.configure(target, root / "test.xctestrun", root, True)
            self.assertTrue((root / "measurement/build.json").is_file())

    def test_missing_embedded_host_fails_closed(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            binary = root / "App.app/Contents/MacOS/App"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"app")
            with self.assertRaisesRegex(ValueError, "embedded Agent Host"):
                measurement.configure({"TestHostPath": str(binary)}, root / "run", root, True)
