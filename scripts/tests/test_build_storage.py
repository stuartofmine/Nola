# SPDX-License-Identifier: Apache-2.0
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("nola_build_storage", ROOT / "scripts/maintain_build_storage.py")
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)


class BuildStorageTests(unittest.TestCase):
    def test_only_owned_integration_executables_are_candidates(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            profile = root / "target/debug"
            deps = profile / "deps"
            deps.mkdir(parents=True)
            metadata = {"workspace_members": ["fixture"], "packages": [{"id":"fixture", "name":"fixture",
                "manifest_path":str(root / "Cargo.toml"), "targets":[{"name":"smoke","kind":["test"]}]}]}
            for index in range(3):
                digest = f'{index:016x}'
                runner = deps / ("smoke-" + digest)
                runner.write_bytes(b'\x7fELF' + b'fixture')
                runner.chmod(0o755)
                os.utime(runner, ns=(index + 1, index + 1))
                fingerprint = profile / ".fingerprint" / ("fixture-" + digest)
                fingerprint.mkdir(parents=True)
                (fingerprint / "test-integration-test-smoke").write_text("owned")
            (deps / "smoke-ffffffffffffffff").write_bytes(b'\x7fELF')  # no fingerprint
            (deps / "application.sqlite").write_bytes(b'precious')
            plan = module.candidates(root, profile, metadata, keep=1)
            self.assertEqual(len(plan), 2)
            for item in plan:
                module.remove_runner(item)
            self.assertTrue((deps / "smoke-0000000000000002").exists())
            self.assertTrue((deps / "smoke-ffffffffffffffff").exists())
            self.assertEqual((deps / "application.sqlite").read_bytes(), b'precious')

    def test_replacement_after_preview_is_not_removed(self):
        with tempfile.TemporaryDirectory() as directory:
            file = Path(directory) / "runner"
            file.write_bytes(b'original')
            info = file.stat()
            candidate = module.Runner(file, "fixture", info.st_size, info.st_mtime_ns, info.st_dev, info.st_ino)
            file.unlink()
            file.symlink_to(Path(directory) / "database")
            with self.assertRaises(RuntimeError):
                module.remove_runner(candidate)
            self.assertTrue(file.is_symlink())

    @unittest.skipUnless(os.name == 'posix', 'Cargo-compatible Unix locking')
    def test_real_cargo_lock_and_pruned_runner_rebuild_preserve_library_cache(self):
        with tempfile.TemporaryDirectory(prefix="nola-cache-fixture-") as directory:
            root = Path(directory)
            (root / "src").mkdir()
            (root / "tests").mkdir()
            (root / "Cargo.toml").write_text('[package]\nname="cache_fixture"\nversion="0.0.0"\nedition="2021"\n[workspace]\n')
            (root / "src/lib.rs").write_text('pub fn answer()->u32 { 42 }')
            (root / "tests/smoke.rs").write_text('#[test] fn smoke(){assert_eq!(cache_fixture::answer(),42)}')
            environment = {**os.environ, 'CARGO_TARGET_DIR':str(root / "target"), 'RUSTC_WRAPPER':'', 'CARGO_INCREMENTAL':'0'}
            command = ['cargo','test','--manifest-path',str(root / "Cargo.toml"),'--test','smoke','--no-run','--message-format=json']
            first = subprocess.run(command, env=environment, text=True, capture_output=True, check=True)
            events = [json.loads(line) for line in first.stdout.splitlines()]
            executable = Path(next(item['executable'] for item in events if item.get('executable')))
            profile = root / "target/debug"
            with module.cargo_lock(profile):
                blocked = subprocess.Popen(command, env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                try:
                    time.sleep(0.4)
                    self.assertIsNone(blocked.poll(), 'Cargo must honor the cleanup lock')
                finally:
                    blocked.terminate()
                    blocked.communicate(timeout=10)
                executable.unlink()
            rebuilt = subprocess.run(command, env=environment, text=True, capture_output=True, check=True)
            artifacts = [json.loads(line) for line in rebuilt.stdout.splitlines() if json.loads(line).get('reason')=='compiler-artifact']
            self.assertTrue(next(item for item in artifacts if item['target']['kind']==['lib'])['fresh'])
            self.assertFalse(next(item for item in artifacts if item.get('executable'))['fresh'])
            self.assertTrue(executable.exists())


if __name__ == '__main__':
    unittest.main()
