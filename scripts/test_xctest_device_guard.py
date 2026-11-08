#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

import importlib.util
import errno
from pathlib import Path
import os
import plistlib
import shlex
import subprocess
import sys
import tempfile
import unittest


MODULE_PATH = Path(__file__).with_name("xctest_device_guard.py")
SPEC = importlib.util.spec_from_file_location("xctest_device_guard", MODULE_PATH)
assert SPEC and SPEC.loader
guard = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(guard)


class XCTestDeviceGuardTests(unittest.TestCase):
    @staticmethod
    def write_device(path: Path, name: str) -> None:
        path.mkdir(parents=True)
        with (path / "device.plist").open("wb") as destination:
            plistlib.dump({"name": name}, destination)

    def test_cleanup_removes_only_new_registered_and_partial_clones(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            device_set = Path(directory)
            existing = "11111111-1111-1111-1111-111111111111"
            registered = "22222222-2222-2222-2222-222222222222"
            partial = "33333333-3333-3333-3333-333333333333"
            unrelated = "not-a-device"
            for device_id in (existing, registered, partial, unrelated):
                (device_set / device_id / "data").mkdir(parents=True)
            (device_set / existing / "device.plist").touch()
            (device_set / registered / "device.plist").touch()

            def delete_device(root: Path, device_id: str) -> bool:
                self.assertEqual(root, device_set)
                self.assertEqual(device_id, registered)
                import shutil
                shutil.rmtree(root / device_id)
                return True

            removed, retained = guard.cleanup_new_devices(
                device_set,
                {existing},
                delete_device,
            )

            self.assertEqual(removed, [registered, partial])
            self.assertEqual(retained, [])
            self.assertTrue((device_set / existing).is_dir())
            self.assertTrue((device_set / unrelated).is_dir())
            self.assertFalse((device_set / registered).exists())
            self.assertFalse((device_set / partial).exists())

    def test_failed_simctl_delete_retains_registered_clone(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            device_set = Path(directory)
            device_id = "AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA"
            (device_set / device_id / "data").mkdir(parents=True)
            (device_set / device_id / "device.plist").touch()

            removed, retained = guard.cleanup_new_devices(
                device_set,
                set(),
                lambda _root, _device_id: False,
            )

            self.assertEqual(removed, [])
            self.assertEqual(retained, [device_id])
            self.assertTrue((device_set / device_id).is_dir())

    def test_partial_clone_removal_retries_while_xctest_is_still_writing(self) -> None:
        calls = 0
        sleeps: list[float] = []

        def remover(_path: Path) -> None:
            nonlocal calls
            calls += 1
            if calls < 3:
                raise OSError(errno.ENOTEMPTY, "Directory not empty")

        removed = guard.remove_partial_device(
            Path("/XCTestDevices/partial"),
            remover=remover,
            sleeper=sleeps.append,
        )

        self.assertTrue(removed)
        self.assertEqual(calls, 3)
        self.assertEqual(sleeps, [0.1, 0.1])

    def test_partial_clone_removal_retains_device_after_bounded_retries(self) -> None:
        calls = 0

        def remover(_path: Path) -> None:
            nonlocal calls
            calls += 1
            raise OSError(errno.ENOTEMPTY, "Directory not empty")

        self.assertFalse(guard.remove_partial_device(
            Path("/XCTestDevices/partial"),
            attempts=3,
            delay_seconds=0,
            remover=remover,
            sleeper=lambda _: None,
        ))
        self.assertEqual(calls, 3)

    def test_prune_stale_devices_removes_only_old_clones_and_partial_devices(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            device_set = Path(directory)
            old_clone = "11111111-1111-1111-1111-111111111111"
            fresh_clone = "22222222-2222-2222-2222-222222222222"
            old_named_device = "33333333-3333-3333-3333-333333333333"
            old_partial = "44444444-4444-4444-4444-444444444444"
            now = 10_000.0
            old_time = now - 7_200

            self.write_device(device_set / old_clone, "Clone 2 of Nola iPhone 15")
            self.write_device(device_set / fresh_clone, "Clone 3 of Nola iPhone 15")
            self.write_device(device_set / old_named_device, "Persistent test device")
            (device_set / old_partial / "data").mkdir(parents=True)
            for device_id in (old_clone, old_named_device, old_partial):
                os.utime(device_set / device_id, (old_time, old_time))

            deleted: list[str] = []

            def delete_device(root: Path, device_id: str) -> bool:
                deleted.append(device_id)
                import shutil
                shutil.rmtree(root / device_id)
                return True

            removed, retained = guard.prune_stale_devices(
                device_set,
                minimum_age_seconds=3_600,
                now=now,
                delete_device=delete_device,
            )

            self.assertEqual(removed, [old_clone, old_partial])
            self.assertEqual(retained, [])
            self.assertEqual(deleted, [old_clone])
            self.assertTrue((device_set / fresh_clone).is_dir())
            self.assertTrue((device_set / old_named_device).is_dir())

    def test_prune_stale_devices_retains_clone_when_simctl_delete_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            device_set = Path(directory)
            device_id = "AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA"
            self.write_device(device_set / device_id, "Clone 2 of iPhone 17 Pro")
            os.utime(device_set / device_id, (1_000, 1_000))

            removed, retained = guard.prune_stale_devices(
                device_set,
                minimum_age_seconds=100,
                now=2_000,
                delete_device=lambda _root, _device_id: False,
            )

            self.assertEqual(removed, [])
            self.assertEqual(retained, [device_id])
            self.assertTrue((device_set / device_id).is_dir())


class XCTestDeviceGuardShellTests(unittest.TestCase):
    """Run only the guard shell with fake Apple binaries, never Xcode/devices."""

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="nola-guard-contract-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repository = Path(__file__).resolve().parents[1]
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.trace = self.root / "commands.log"
        self.trace.touch()
        self.devices = self.root / "XCTestDevices"
        self.devices.mkdir()
        self.original_tmp = self.root / "tmp"
        self.original_tmp.mkdir()
        self.lock = self.root / "owned.lock"
        self.environment = dict(os.environ)
        for key in tuple(self.environment):
            if key.startswith("NOLA_XCTEST_"):
                self.environment.pop(key)
        self.environment.update({
            "PATH": str(self.bin) + os.pathsep + os.environ.get("PATH", ""),
            "TMPDIR": str(self.original_tmp) + "/",
            "NOLA_GUARD_ORIGINAL_TMP": str(self.original_tmp) + "/",
            "NOLA_GUARD_COMMAND_TRACE": str(self.trace),
            "NOLA_XCTEST_GUARD_LOCK": str(self.lock),
            "NOLA_XCTEST_DEVICE_SET": str(self.devices),
            "NOLA_XCTEST_STALE_DEVICE_AGE_SECONDS": "1",
        })
        self.write_binary("uname", "#!/bin/sh\nprintf 'Darwin\\n'\n")
        self.write_binary("python3", "#!/bin/sh\n"
            'printf "python3 %s\\n" "$*" >> "$NOLA_GUARD_COMMAND_TRACE"\n'
            f"exec {shlex.quote(sys.executable)} \"$@\"\n")
        for name in ("xcrun", "xcodebuild", "simctl", "devicectl", "xcdevice", "xctrace", "osascript"):
            self.write_binary(name, "#!/bin/sh\n"
                'printf "%s %s\\n" "${0##*/}" "$*" >> "$NOLA_GUARD_COMMAND_TRACE"\n'
                "exit 0\n")

    def write_binary(self, name: str, source: str) -> None:
        path = self.bin / name
        path.write_text(source)
        path.chmod(0o755)

    def run_shell(self, body: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["/bin/sh", "-eu", "-c",
             '. "$1/scripts/lib/xcode_test_device_guard.sh"\n' + body,
             "nola-guard-contract", str(self.repository)],
            env=self.environment, capture_output=True, text=True, timeout=15,
        )

    def test_macos_keeps_lock_and_tmp_lifecycle_without_any_device_script(self) -> None:
        stale = self.devices / "11111111-1111-1111-1111-111111111111"
        XCTestDeviceGuardTests.write_device(stale, "Clone 2 of fixture")
        os.utime(stale, (1, 1))
        before = (stale / "device.plist").read_bytes()
        result = self.run_shell('''
nola_xctest_device_guard_begin "$1" macos
test -f "$NOLA_XCTEST_GUARD_LOCK/pid"
test -d "$NOLA_XCTEST_RUN_TMP"
test "$TMPDIR" != "$NOLA_GUARD_ORIGINAL_TMP"
guard_test_run_tmp=$NOLA_XCTEST_RUN_TMP
nola_xctest_device_guard_end "$1" macos
test ! -e "$guard_test_run_tmp"
test ! -e "$NOLA_XCTEST_GUARD_LOCK"
test "$TMPDIR" = "$NOLA_GUARD_ORIGINAL_TMP"
''')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.trace.read_text(), "")
        self.assertEqual((stale / "device.plist").read_bytes(), before)

    def test_macos_exit_trap_ignores_even_an_inherited_simulator_baseline(self) -> None:
        baseline = self.root / "foreign-baseline"
        baseline.write_text("must remain unchanged\n")
        self.environment["NOLA_XCTEST_GUARD_BASELINE"] = str(baseline)
        result = self.run_shell('''
nola_xctest_device_guard_begin "$1" macos
nola_xctest_device_guard_install_traps "$1" macos
exit 17
''')
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assertFalse(self.lock.exists())
        self.assertEqual(list(self.original_tmp.iterdir()), [])
        self.assertEqual(baseline.read_text(), "must remain unchanged\n")
        self.assertEqual(self.trace.read_text(), "")

    def test_ios_simulator_retains_scoped_clone_prune_snapshot_and_cleanup(self) -> None:
        stale_id = "11111111-1111-1111-1111-111111111111"
        fresh_id = "22222222-2222-2222-2222-222222222222"
        stale = self.devices / stale_id
        XCTestDeviceGuardTests.write_device(stale, "Clone 2 of fixture")
        os.utime(stale, (1, 1))
        result = self.run_shell(f'''
nola_xctest_device_guard_begin "$1" ios_simulator
mkdir "$NOLA_XCTEST_DEVICE_SET/{fresh_id}"
cp "$NOLA_XCTEST_DEVICE_SET/{stale_id}/device.plist" "$NOLA_XCTEST_DEVICE_SET/{fresh_id}/device.plist"
nola_xctest_device_guard_end "$1" ios_simulator
''')
        self.assertEqual(result.returncode, 0, result.stderr)
        commands = self.trace.read_text().splitlines()
        python_commands = [line for line in commands if line.startswith("python3 ")]
        self.assertEqual(len(python_commands), 3)
        for command, mode in zip(python_commands, ("prune-stale", "snapshot", "cleanup")):
            self.assertIn("xctest_device_guard.py " + mode, command)
            self.assertIn("--device-set " + str(self.devices), command)
        apple_commands = [line for line in commands if not line.startswith("python3 ")]
        self.assertEqual(apple_commands, [
            f"xcrun simctl --set {self.devices} shutdown {stale_id}",
            f"xcrun simctl --set {self.devices} delete {stale_id}",
            f"xcrun simctl --set {self.devices} shutdown {fresh_id}",
            f"xcrun simctl --set {self.devices} delete {fresh_id}",
        ])
        self.assertFalse(self.lock.exists())
        self.assertEqual(list(self.original_tmp.iterdir()), [])

    def test_missing_or_unknown_platform_is_rejected_before_acquiring_any_resource(self) -> None:
        for arguments in ('"$1"', '"$1" unknown'):
            for operation in ("begin", "end", "install_traps"):
                result = self.run_shell(f"nola_xctest_device_guard_{operation} {arguments}\n")
                self.assertEqual(result.returncode, 64, result.stderr)
                self.assertFalse(self.lock.exists())
                self.assertEqual(list(self.original_tmp.iterdir()), [])
                self.assertEqual(self.trace.read_text(), "")


if __name__ == "__main__":
    unittest.main()
