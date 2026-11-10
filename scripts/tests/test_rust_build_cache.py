# SPDX-License-Identifier: Apache-2.0
"""Test the Rust build launcher with owned fake executables, never real builds."""

import json
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest
import xml.etree.ElementTree as ET


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
HELPER = REPOSITORY_ROOT / "scripts/lib/rust_build_cache.sh"
BUILD_SCRIPTS = (
    "apps/cli/scripts/build.sh",
    "scripts/build_cli_runtime.sh",
    "scripts/build_mac_runtime.sh",
    "core/scripts/build_and_copy.sh",
    "core/scripts/build_android.sh",
    "core/scripts/build_xcframework.sh",
)
SHELLS = tuple(dict.fromkeys(
    executable for name in ("sh", "bash")
    if (executable := shutil.which(name)) is not None
))


class RustBuildCacheTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="nola-rust-cache-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.bin_directory = self.root / "fake bin"
        self.bin_directory.mkdir()
        self.cargo = self.bin_directory / "cargo executable"
        self.mbx = self.bin_directory / "mbx"
        self.log_path = self.root / "calls.jsonl"
        self.fixture_home = self.root / "home"
        self.fixture_home.mkdir()
        self.write_executable(self.cargo, "cargo")
        self.write_executable(self.mbx, "mbx")
        self.assertTrue(SHELLS, "A POSIX shell is required for launcher tests")

    def write_executable(self, path, kind):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            f"#!{sys.executable}\n"
            "import json, os, sys\n"
            f"kind = {kind!r}\n"
            "environment = {key: value for key, value in os.environ.items()\n"
            "               if key.startswith(('CARGO', 'MBX_'))\n"
            "               or key in ('RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER',\n"
            "                          'RUSTFLAGS', 'NOLA_TEST_VALUE', 'TMPDIR')}\n"
            "record = {'kind': kind, 'executable': sys.argv[0],\n"
            "          'argv': sys.argv[1:], 'env': environment, 'pid': os.getpid(),\n"
            "          'pgid': os.getpgid(0)}\n"
            "with open(os.environ['NOLA_TEST_LOG'], 'a', encoding='utf-8') as log:\n"
            "    log.write(json.dumps(record) + '\\n')\n"
            "if kind == 'mbx':\n"
            "    if 'FAKE_MBX_STATUS' in os.environ:\n"
            "        sys.exit(int(os.environ['FAKE_MBX_STATUS']))\n"
            "    cargo = os.environ['CARGO']\n"
            "    if os.environ.get('FAKE_CARGO_BLOCK') == '1':\n"
            "        import subprocess\n"
            "        sys.exit(subprocess.call([cargo] + sys.argv[1:]))\n"
            "    os.execv(cargo, [cargo] + sys.argv[1:])\n"
            "if os.environ.get('FAKE_CARGO_BLOCK') == '1':\n"
            "    import subprocess, time\n"
            "    grandchild = ('import json,os,time; '\n"
            "                  'record={\\\"kind\\\":\\\"grandchild\\\",'\n"
            "                  '\\\"pid\\\":os.getpid(),\\\"pgid\\\":os.getpgid(0)}; '\n"
            "                  'log=open(os.environ[\\\"NOLA_TEST_LOG\\\"],\\\"a\\\"); '\n"
            "                  'log.write(json.dumps(record)+\\\"\\\\n\\\"); log.close(); '\n"
            "                  'time.sleep(60)')\n"
            "    subprocess.Popen([sys.executable, '-c', grandchild])\n"
            "    time.sleep(60)\n"
            "sys.stdout.write(os.environ.get('FAKE_CARGO_STDOUT', ''))\n"
            "sys.stderr.write(os.environ.get('FAKE_CARGO_STDERR', ''))\n"
            "sys.exit(int(os.environ.get('FAKE_CARGO_STATUS', '0')))\n",
            encoding="utf-8",
        )
        path.chmod(0o755)

    def test_workspace_cleanup_is_preview_by_default_and_checkout_scoped(self):
        script = REPOSITORY_ROOT / "scripts/clean_rust_workspace.sh"
        for mode in ([], ["--dry-run"], ["--apply"]):
            self.log_path.unlink(missing_ok=True)
            env = dict(os.environ, CARGO=str(self.cargo), NOLA_TEST_LOG=str(self.log_path),
                       CARGO_TARGET_DIR=str(self.root / "unrelated target"))
            result = subprocess.run(["/bin/sh", str(script), *mode], env=env,
                                    cwd=self.root, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            record = json.loads(self.log_path.read_text())
            args = record["argv"]
            self.assertEqual(args[:5], ["clean", "--workspace", "--profile", "dev", "--frozen"])
            self.assertEqual(args[args.index("--target-dir") + 1], str(REPOSITORY_ROOT / "target"))
            self.assertEqual("--dry-run" in args, mode != ["--apply"])
            self.assertNotIn("--apply", args)
        self.log_path.unlink()
        rejected = subprocess.run(["/bin/sh", str(script), "--all"], env=env,
                                  capture_output=True)
        self.assertEqual(rejected.returncode, 2)
        self.assertFalse(self.log_path.exists())

    def run_launcher(self, shell, arguments=("build",), *, environment=None,
                     mode=None, execute=False, cargo=None, launcher=None):
        self.log_path.unlink(missing_ok=True)
        child_environment = {
            "PATH": str(self.bin_directory),
            "HOME": str(self.fixture_home),
            "NOLA_TEST_LOG": str(self.log_path),
        }
        if mode is not None:
            child_environment["NOLA_RUST_BUILD_CACHE"] = mode
        child_environment.update(environment or {})
        launch = launcher or ("nola_cargo_exec" if execute else "nola_cargo")
        program = (
            '. "$1"\n'
            'nola_rust_build_cache_init "$2"\n'
            'shift 2\n'
            'printf "backend=%s\\n" "$NOLA_RUST_CACHE_BACKEND"\n'
            f'{launch} "$@"\n'
            'printf "after-cargo\\n"\n'
        )
        command = [shell, "-eu", "-c", program, "nola-cache-contract",
                   str(HELPER), str(cargo or self.cargo), *arguments]
        with subprocess.Popen(command, env=child_environment, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE) as child:
            try:
                stdout, stderr = child.communicate(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()
                child.communicate()
                raise
            result = subprocess.CompletedProcess(command, child.returncode, stdout, stderr)
            pid = child.pid
        records = []
        if self.log_path.exists():
            records = [json.loads(line) for line in
                       self.log_path.read_text(encoding="utf-8").splitlines()]
        return result, records, pid

    def assert_backend(self, result, backend):
        self.assertIn(f"backend={backend}\n", result.stdout)

    def assert_launch(self, records, kinds, arguments):
        self.assertEqual([record["kind"] for record in records], kinds)
        for record in records:
            self.assertEqual(record["argv"], list(arguments))

    def test_mac_tests_cannot_replace_the_signed_debug_application(self):
        scheme = ET.parse(REPOSITORY_ROOT / "Nola.xcodeproj/xcshareddata/xcschemes/NolaMac.xcscheme").getroot()
        self.assertEqual(scheme.find("TestAction").attrib["buildConfiguration"], "Test")
        self.assertEqual(scheme.find("LaunchAction").attrib["buildConfiguration"], "Debug")
        runner = (REPOSITORY_ROOT / "scripts/test_mac_layers.sh").read_text()
        self.assertIn("-configuration Test", runner)

    def test_target_budget_warns_without_cleaning_or_changing_build_arguments(self):
        target = self.root / "target"
        target.mkdir()
        du = self.bin_directory / "du"
        du.write_text(f"#!{sys.executable}\nprint('26214400\\tfixture target')\n")
        du.chmod(0o755)
        for shell in SHELLS:
            for execute in (False, True):
                result, records, _ = self.run_launcher(shell, execute=execute,
                    environment={"CARGO_TARGET_DIR": str(target)})
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("exceeds 24 GiB", result.stderr)
                self.assert_launch(records, ["mbx", "cargo"], ("build",))
            result, records, _ = self.run_launcher(shell, ("metadata",),
                environment={"CARGO_TARGET_DIR": str(target)})
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertNotIn("exceeds 24 GiB", result.stderr)
            self.assert_launch(records, ["cargo"], ("metadata",))

    def test_default_auto_selects_available_mbx(self):
        for shell in SHELLS:
            with self.subTest(shell=shell):
                result, records, _ = self.run_launcher(shell)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assert_backend(result, "mbx")
                self.assert_launch(records, ["mbx", "cargo"], ("build",))
                self.assertIn("mbx", result.stderr.lower())
                self.assertEqual(records[0]["env"]["CARGO"], str(self.cargo))

    def test_mbx_does_not_learn_private_incremental_state_by_default(self):
        for shell in SHELLS:
            result, records, _ = self.run_launcher(shell)
            self.assertEqual(result.returncode, 0, result.stderr)
            for record in records:
                self.assertEqual(record["env"]["MBX_LEARNED_INCREMENTAL"], "false")
                self.assertEqual(record["env"]["MBX_EAGER_INCREMENTAL"], "false")

    def test_mbx_preserves_explicit_incremental_opt_in(self):
        for shell in SHELLS:
            result, records, _ = self.run_launcher(shell, environment={
                "MBX_LEARNED_INCREMENTAL": "true", "MBX_EAGER_INCREMENTAL": "true",
            })
            self.assertEqual(result.returncode, 0, result.stderr)
            for record in records:
                self.assertEqual(record["env"]["MBX_LEARNED_INCREMENTAL"], "true")
                self.assertEqual(record["env"]["MBX_EAGER_INCREMENTAL"], "true")

    def test_auto_without_mbx_falls_back_with_status(self):
        self.mbx.unlink()
        for shell in SHELLS:
            with self.subTest(shell=shell):
                result, records, _ = self.run_launcher(shell)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assert_backend(result, "cargo")
                self.assert_launch(records, ["cargo"], ("build",))
                self.assertIn("mbx", result.stderr.lower())
                self.assertIn("cargo", result.stderr.lower())

    def test_off_bypasses_mbx_and_preserves_existing_wrapper(self):
        for shell in SHELLS:
            with self.subTest(shell=shell):
                environment = {
                    "NOLA_MBX_BIN": str(self.root / "missing mbx"),
                    "RUSTC_WRAPPER": "my rustc wrapper",
                }
                result, records, _ = self.run_launcher(shell, mode="off",
                                                       environment=environment)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assert_backend(result, "cargo")
                self.assert_launch(records, ["cargo"], ("build",))
                self.assertEqual(records[0]["env"]["RUSTC_WRAPPER"], "my rustc wrapper")

    def test_strict_modes_fail_before_launch_without_mbx(self):
        self.mbx.unlink()
        for shell in SHELLS:
            for mode in ("mbx", "required"):
                with self.subTest(shell=shell, mode=mode):
                    result, records, _ = self.run_launcher(shell, mode=mode)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(records, [])
                    self.assertIn("mbx", result.stderr.lower())

    def test_strict_modes_select_available_mbx(self):
        for shell in SHELLS:
            for mode in ("mbx", "required"):
                with self.subTest(shell=shell, mode=mode):
                    result, records, _ = self.run_launcher(shell, mode=mode)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assert_backend(result, "mbx")
                    self.assert_launch(records, ["mbx", "cargo"], ("build",))

    def test_invalid_mode_fails_before_launch(self):
        for shell in SHELLS:
            with self.subTest(shell=shell):
                result, records, _ = self.run_launcher(shell, mode="invalid-mode")
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(records, [])
                self.assertIn("invalid-mode", result.stderr)

    def test_auto_preserves_explicit_wrappers_including_empty_values(self):
        for shell in SHELLS:
            for wrapper in ("RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"):
                for value in ("my existing wrapper", ""):
                    with self.subTest(shell=shell, wrapper=wrapper, value=value):
                        result, records, _ = self.run_launcher(
                            shell, environment={wrapper: value})
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assert_backend(result, "cargo")
                        self.assert_launch(records, ["cargo"], ("build",))
                        self.assertEqual(records[0]["env"][wrapper], value)
                        self.assertIn("wrapper", result.stderr.lower())

    def test_strict_modes_reject_explicit_wrappers_including_empty_values(self):
        for shell in SHELLS:
            for mode in ("mbx", "required"):
                for wrapper in ("RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"):
                    for value in ("my existing wrapper", ""):
                        with self.subTest(shell=shell, mode=mode, wrapper=wrapper,
                                          value=value):
                            result, records, _ = self.run_launcher(
                                shell, mode=mode, environment={wrapper: value})
                            self.assertNotEqual(result.returncode, 0)
                            self.assertEqual(records, [])
                            self.assertIn("wrapper", result.stderr.lower())

    def test_absolute_mbx_override_with_spaces_is_one_executable(self):
        override = self.root / "alternate tools" / "mbx cache executable"
        self.write_executable(override, "mbx")
        for shell in SHELLS:
            with self.subTest(shell=shell):
                result, records, _ = self.run_launcher(
                    shell, environment={"NOLA_MBX_BIN": str(override)})
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assert_launch(records, ["mbx", "cargo"], ("build",))
                self.assertEqual(records[0]["executable"], str(override))

    def test_path_mbx_override_with_spaces_is_one_executable(self):
        override = self.bin_directory / "cache mbx"
        self.write_executable(override, "mbx")
        for shell in SHELLS:
            with self.subTest(shell=shell):
                result, records, _ = self.run_launcher(
                    shell, environment={"NOLA_MBX_BIN": override.name})
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assert_launch(records, ["mbx", "cargo"], ("build",))
                self.assertEqual(records[0]["executable"], str(override))

    def test_unusable_override_falls_back_in_auto_and_fails_in_strict_modes(self):
        nonexecutable = self.root / "nonexecutable mbx"
        nonexecutable.write_text("not executable\n", encoding="utf-8")
        directory = self.root / "mbx directory"
        directory.mkdir()
        for shell in SHELLS:
            for override in (self.root / "missing mbx", nonexecutable, directory):
                for mode in ("auto", "mbx", "required"):
                    with self.subTest(shell=shell, override=override, mode=mode):
                        result, records, _ = self.run_launcher(
                            shell, mode=mode, environment={"NOLA_MBX_BIN": str(override)})
                        self.assertIn("mbx", result.stderr.lower())
                        if mode == "auto":
                            self.assertEqual(result.returncode, 0, result.stderr)
                            self.assert_backend(result, "cargo")
                            self.assert_launch(records, ["cargo"], ("build",))
                        else:
                            self.assertNotEqual(result.returncode, 0)
                            self.assertEqual(records, [])

    def test_mbx_override_is_never_evaluated_as_shell_text(self):
        sentinel = self.root / "must not be created"
        override = f"{self.mbx}; /usr/bin/touch {shlex.quote(str(sentinel))}"
        for shell in SHELLS:
            with self.subTest(shell=shell):
                result, records, _ = self.run_launcher(
                    shell, mode="required", environment={"NOLA_MBX_BIN": override})
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(records, [])
                self.assertFalse(sentinel.exists())

    def test_compilation_commands_use_mbx(self):
        for shell in SHELLS:
            for command in ("build", "test", "check", "rustc"):
                arguments = ("+nightly", command, "--target", "aarch64-apple-darwin")
                with self.subTest(shell=shell, command=command):
                    result, records, _ = self.run_launcher(shell, arguments)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assert_launch(records, ["mbx", "cargo"], arguments)

    def test_ndk_build_uses_mbx_with_original_arguments(self):
        arguments = ("+stable", "ndk", "-t", "arm64-v8a", "-P", "26", "build",
                     "--manifest-path", "manifest path/Cargo.toml", "--release")
        for shell in SHELLS:
            with self.subTest(shell=shell):
                result, records, _ = self.run_launcher(shell, arguments)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assert_launch(records, ["mbx", "cargo"], arguments)

    def test_noncompiling_commands_call_original_cargo(self):
        queries = (
            ("metadata", "--no-deps", "--format-version", "1"),
            ("+stable", "metadata", "--manifest-path", "manifest path/Cargo.toml"),
            ("clean", "--target", "aarch64-apple-darwin"),
            ("--version",), ("-V",), ("help", "build"), ("--help",), ("-h",),
            ("build", "--help"), ("ndk", "--version"),
            ("ndk", "-t", "arm64-v8a", "metadata"),
        )
        for shell in SHELLS:
            for arguments in queries:
                with self.subTest(shell=shell, arguments=arguments):
                    result, records, _ = self.run_launcher(shell, arguments)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assert_launch(records, ["cargo"], arguments)
                    self.assertEqual(records[0]["executable"], str(self.cargo))

    def test_argv_preserves_toolchain_empty_strings_spaces_and_shell_metacharacters(self):
        arguments = ("+nightly-2026-10-01", "build", "--target", "target with spaces",
                     "--manifest-path", "manifest with spaces/Cargo.toml", "--config",
                     'build.rustflags=["--cfg", "value = literal"]', "--", "",
                     "$(not-a-command)", "`not-a-command`", "semi;colon", "a*b")
        for shell in SHELLS:
            for mode, kinds in (("auto", ["mbx", "cargo"]), ("off", ["cargo"])):
                with self.subTest(shell=shell, mode=mode):
                    result, records, _ = self.run_launcher(shell, arguments, mode=mode)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assert_launch(records, kinds, arguments)

    def test_mbx_receives_resolved_cargo_and_preserves_build_environment(self):
        environment = {
            "CARGO": "unresolved-cargo-name",
            "CARGO_TARGET_DIR": str(self.root / "target directory"),
            "CARGO_INCREMENTAL": "0",
            "CARGO_BUILD_TARGET": "aarch64-linux-android",
            "MBX_CACHE_DIR": str(self.root / "mbx cache directory"),
            "MBX_LOG": "debug",
            "MBX_CUSTOM_VALUE": "spaces and ; $(literal)",
            "RUSTFLAGS": "--cfg nola_test",
            "NOLA_TEST_VALUE": "ordinary environment value",
        }
        for shell in SHELLS:
            with self.subTest(shell=shell):
                result, records, _ = self.run_launcher(shell, environment=environment)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assert_launch(records, ["mbx", "cargo"], ("build",))
                for record in records:
                    self.assertEqual(record["env"]["CARGO"], str(self.cargo))
                    for key, value in environment.items():
                        if key != "CARGO":
                            self.assertEqual(record["env"][key], value)
                    self.assertNotIn("RUSTC_WRAPPER", record["env"])
                    self.assertNotIn("RUSTC_WORKSPACE_WRAPPER", record["env"])

    def test_direct_cargo_preserves_existing_cargo_environment(self):
        for shell in SHELLS:
            with self.subTest(shell=shell):
                environment = {"CARGO": "keep caller value",
                               "CARGO_TARGET_DIR": str(self.root / "target directory"),
                               "MBX_LOG": "debug"}
                result, records, _ = self.run_launcher(shell, mode="off",
                                                       environment=environment)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assert_launch(records, ["cargo"], ("build",))
                for key, value in environment.items():
                    self.assertEqual(records[0]["env"][key], value)

    def test_long_tmpdir_is_shortened_only_for_mbx_compilation_child(self):
        temporary_root = "/tmp/" + "test-session-" * 8
        for shell in SHELLS:
            for execute in (False, True):
                with self.subTest(shell=shell, execute=execute):
                    result, records, pid = self.run_launcher(
                        shell, execute=execute, environment={"TMPDIR": temporary_root})
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assert_launch(records, ["mbx", "cargo"], ("build",))
                    self.assertEqual(records[0]["env"]["TMPDIR"], "/tmp")
                    self.assertEqual(records[1]["env"]["TMPDIR"], "/tmp")
                    if execute:
                        self.assertTrue(all(record["pid"] == pid for record in records))
                    else:
                        self.assertIn("after-cargo\n", result.stdout)

    def test_short_tmpdir_and_noncompiling_or_direct_cargo_keep_caller_tmpdir(self):
        for shell in SHELLS:
            for execute in (False, True):
                for mode, arguments, temporary_root, kinds in (
                    ("auto", ("build",), "/tmp/nola-short", ["mbx", "cargo"]),
                    ("auto", ("metadata",), "/tmp/" + "long-session-" * 8, ["cargo"]),
                    ("off", ("build",), "/tmp/" + "long-session-" * 8, ["cargo"]),
                ):
                    with self.subTest(shell=shell, execute=execute, mode=mode,
                                      arguments=arguments):
                        result, records, _ = self.run_launcher(
                            shell, arguments, mode=mode, execute=execute,
                            environment={"TMPDIR": temporary_root})
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assert_launch(records, kinds, arguments)
                        for record in records:
                            self.assertEqual(record["env"]["TMPDIR"], temporary_root)

    def test_synchronous_mbx_launch_does_not_change_parent_tmpdir(self):
        temporary_root = "/tmp/" + "owned-long-session-" * 8
        program = ('. "$1"\n'
                   'nola_rust_build_cache_init "$2"\n'
                   'nola_cargo build\n'
                   'printf "parent-tmp=%s\\n" "$TMPDIR"\n')
        for shell in SHELLS:
            with self.subTest(shell=shell):
                result = subprocess.run(
                    [shell, "-eu", "-c", program, "tmpdir-contract", str(HELPER), str(self.cargo)],
                    env={"PATH": str(self.bin_directory), "TMPDIR": temporary_root,
                         "HOME": str(self.fixture_home), "NOLA_TEST_LOG": str(self.log_path)},
                    capture_output=True, text=True, check=False, timeout=10)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn(f"parent-tmp={temporary_root}\n", result.stdout)

    @unittest.skipUnless(sys.platform == "darwin" and shutil.which("sandbox-exec"),
                         "Requires the macOS build-script sandbox")
    def test_runtime_copy_writes_only_exact_declared_outputs(self):
        fixture = self.root / "runtime fixture"
        for relative in ("apps/cli/src", "apps/cli/resources", "apps/cli/vendor", "core", "apps/macos/Resources"):
            (fixture / relative).mkdir(parents=True, exist_ok=True)
        for relative in ("Cargo.toml", "Cargo.lock", "apps/cli/build.rs"):
            (fixture / relative).write_text("fixture\n", encoding="utf-8")
        plist_name = "com.gaojc.Nola.background-scheduler.plist"
        (fixture / "apps/macos/Resources" / plist_name).write_text("fixture plist\n", encoding="utf-8")
        artifacts = fixture / "target/debug"
        artifacts.mkdir(parents=True)
        for name in ("nola-agent-host", "nola-service"):
            artifact = artifacts / name
            shutil.copyfile("/bin/echo", artifact)
            artifact.chmod(0o755)
            os.utime(artifact, (time.time() + 10, time.time() + 10))
        outputs = self.root / "declared runtime outputs"
        outputs.mkdir()
        declared = [outputs, *(outputs / name for name in ("nola-agent-host", "nola-service", plist_name))]
        declared.extend(outputs / (name + ".nola-staging")
                        for name in ("nola-agent-host", "nola-service"))
        # Match Xcode's literal-file output permissions: a declared directory
        # does not grant random staging-file siblings inside it.
        profile = '(version 1) (allow default)\n'
        profile += f'(deny file-write* (subpath {json.dumps(str(outputs))}))\n'
        for path in declared:
            profile += f'(allow file-read* file-write* (literal {json.dumps(str(path))}))\n'
        environment = dict(os.environ, CARGO=str(self.cargo), NOLA_RUST_BUILD_CACHE="off",
                           NOLA_REPOSITORY_ROOT=str(fixture), NOLA_RUNTIME_OUTPUT_DIR=str(outputs),
                           NOLA_BUILD_PROFILE="dev", NOLA_TEST_LOG=str(self.log_path))
        environment.pop("CARGO_TARGET_DIR", None)
        environment.pop("NOLA_AGENT_CARGO_TARGET_DIR", None)
        result = subprocess.run(
            [shutil.which("sandbox-exec"), "-p", profile, "/bin/bash",
             str(REPOSITORY_ROOT / "scripts/build_mac_runtime.sh")],
            env=environment, capture_output=True, text=True, check=False, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        records = [json.loads(line) for line in self.log_path.read_text().splitlines()]
        self.assertEqual([record["kind"] for record in records], ["cargo"])
        self.assertEqual(records[0]["env"]["CARGO_TARGET_DIR"], str(fixture / "target"))
        self.assertEqual(sorted(path.name for path in outputs.iterdir()),
                         sorted(("nola-agent-host", "nola-service", plist_name)))
        for name in ("nola-agent-host", "nola-service"):
            self.assertEqual((outputs / name).read_bytes(), (artifacts / name).read_bytes())
            self.assertEqual((outputs / name).stat().st_mode & 0o777, 0o755)
        self.assertEqual((outputs / plist_name).stat().st_mode & 0o777, 0o644)

    def test_mac_target_override_precedence_and_debugger_profile(self):
        fixture = self.root / "workspace"
        resources = fixture / "apps/macos/Resources"
        resources.mkdir(parents=True)
        (resources / "com.gaojc.Nola.background-scheduler.plist").write_text("fixture\n")
        general_target = self.root / "general target"
        agent_target = self.root / "agent target"
        for agent_override in (False, True):
            with self.subTest(agent_override=agent_override):
                self.log_path.unlink(missing_ok=True)
                target = agent_target if agent_override else general_target
                artifacts = target / "dev-debug"
                artifacts.mkdir(parents=True)
                for name in ("nola-agent-host", "nola-service"):
                    artifact = artifacts / name
                    shutil.copyfile("/bin/echo", artifact)
                    artifact.chmod(0o755)
                outputs = self.root / f"outputs-{agent_override}"
                environment = dict(os.environ, CARGO=str(self.cargo),
                                   CARGO_TARGET_DIR=str(general_target),
                                   NOLA_RUST_BUILD_CACHE="off",
                                   NOLA_REPOSITORY_ROOT=str(fixture),
                                   NOLA_RUNTIME_OUTPUT_DIR=str(outputs),
                                   NOLA_BUILD_PROFILE="dev-debug",
                                   NOLA_TEST_LOG=str(self.log_path))
                environment.pop("NOLA_AGENT_CARGO_TARGET_DIR", None)
                if agent_override:
                    environment["NOLA_AGENT_CARGO_TARGET_DIR"] = str(agent_target)
                result = subprocess.run(
                    [shutil.which("bash"), str(REPOSITORY_ROOT / "scripts/build_mac_runtime.sh")],
                    env=environment, capture_output=True, text=True, check=False, timeout=10)
                self.assertEqual(result.returncode, 0, result.stderr)
                records = [json.loads(line) for line in self.log_path.read_text().splitlines()]
                self.assertEqual(len(records), 1)
                self.assertEqual(records[0]["env"]["CARGO_TARGET_DIR"], str(target))
                arguments = records[0]["argv"]
                self.assertEqual(arguments[arguments.index("--profile") + 1], "dev-debug")
                for name in ("nola-agent-host", "nola-service"):
                    self.assertEqual((outputs / name).read_bytes(), (artifacts / name).read_bytes())

    def test_cargo_exit_status_and_output_are_preserved(self):
        for shell in SHELLS:
            for mode in ("auto", "off"):
                with self.subTest(shell=shell, mode=mode):
                    result, records, _ = self.run_launcher(
                        shell, mode=mode, environment={"FAKE_CARGO_STATUS": "47",
                        "FAKE_CARGO_STDOUT": "cargo stdout\n",
                        "FAKE_CARGO_STDERR": "cargo stderr\n"})
                    self.assertEqual(result.returncode, 47)
                    self.assertIn("cargo stdout\n", result.stdout)
                    self.assertIn("cargo stderr\n", result.stderr)
                    self.assertNotIn("after-cargo", result.stdout)
                    self.assertEqual(records[-1]["kind"], "cargo")

    def test_mbx_exit_status_is_preserved_without_retrying_cargo(self):
        for shell in SHELLS:
            with self.subTest(shell=shell):
                result, records, _ = self.run_launcher(
                    shell, environment={"FAKE_MBX_STATUS": "63"})
                self.assertEqual(result.returncode, 63)
                self.assert_launch(records, ["mbx"], ("build",))

    def test_synchronous_launch_returns_to_caller(self):
        for shell in SHELLS:
            for mode in ("auto", "off"):
                with self.subTest(shell=shell, mode=mode):
                    result, records, pid = self.run_launcher(shell, mode=mode)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn("after-cargo\n", result.stdout)
                    self.assertNotEqual(records[-1]["pid"], pid)

    def test_exec_launch_preserves_background_process_pid(self):
        for shell in SHELLS:
            for mode in ("auto", "off"):
                with self.subTest(shell=shell, mode=mode):
                    result, records, pid = self.run_launcher(shell, mode=mode, execute=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertNotIn("after-cargo", result.stdout)
                    self.assertTrue(records)
                    for record in records:
                        self.assertEqual(record["pid"], pid)

    def test_exec_launch_keeps_queries_on_original_cargo(self):
        for shell in SHELLS:
            with self.subTest(shell=shell):
                arguments = ("metadata", "--format-version", "1")
                result, records, pid = self.run_launcher(shell, arguments, execute=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assert_launch(records, ["cargo"], arguments)
                self.assertEqual(records[0]["pid"], pid)

    def test_owned_group_mismatch_refuses_to_start_cargo(self):
        # An ordinary child shares this test's existing group and cannot be
        # its leader. Exercise the real getpgrp API, not mocked ps output.
        for shell in SHELLS:
            with self.subTest(shell=shell):
                result, records, _ = self.run_launcher(
                    shell, launcher="nola_cargo_exec_owned_group")
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertEqual(records, [])
                self.assertIn("group", result.stderr.lower())

    def test_mac_runtime_cancellation_stops_grandchild_and_preserves_neighbor(self):
        self.assert_runtime_cancellation_stops_owned_group()

    @unittest.skipUnless(sys.platform == "darwin" and shutil.which("sandbox-exec"),
                         "Requires the macOS build-script sandbox")
    def test_mac_runtime_group_cancellation_with_privileged_ps_forbidden(self):
        profile = ('(version 1) (allow default)\n'
                   '(import "/System/Library/Sandbox/Profiles/bsd.sb")\n'
                   '(deny process-exec (literal "/bin/ps"))\n')
        forbidden = subprocess.run(
            [shutil.which("sandbox-exec"), "-p", profile, "/bin/bash", "-c",
             "/bin/ps -p $$ -o pgid="], capture_output=True, text=True, check=False)
        self.assertNotEqual(forbidden.returncode, 0, "Fixture must actually forbid ps")
        self.assert_runtime_cancellation_stops_owned_group(profile)

    def assert_runtime_cancellation_stops_owned_group(self, sandbox_profile=None):
        bash = shutil.which("bash")
        self.assertIsNotNone(bash)
        real_ps = shutil.which("ps")
        for utility in ("mkdir", "dirname"):
            executable = shutil.which(utility)
            self.assertIsNotNone(executable, utility)
            (self.bin_directory / utility).symlink_to(executable)
        fixture_helper = self.root / "scripts/lib/rust_build_cache.sh"
        fixture_helper.parent.mkdir(parents=True)
        shutil.copy2(HELPER, fixture_helper)
        environment = {
            "PATH": str(self.bin_directory), "HOME": str(self.fixture_home),
            "NOLA_TEST_LOG": str(self.log_path), "FAKE_CARGO_BLOCK": "1",
            "NOLA_REPOSITORY_ROOT": str(self.root), "CARGO": str(self.cargo),
            "NOLA_RUNTIME_OUTPUT_DIR": str(self.root / "runtime output"),
            "NOLA_BUILD_PROFILE": "dev", "NOLA_RUST_BUILD_CACHE": "required",
        }
        neighbor = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"],
                                    start_new_session=True, env=environment,
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        command = [bash, str(REPOSITORY_ROOT / "scripts/build_mac_runtime.sh")]
        if sandbox_profile is not None:
            command = [shutil.which("sandbox-exec"), "-p", sandbox_profile, *command]
        runtime = subprocess.Popen(command,
                                   start_new_session=True, env=environment, text=True,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        records = []
        try:
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                if self.log_path.exists():
                    records = [json.loads(line) for line in
                               self.log_path.read_text(encoding="utf-8").splitlines()]
                if any(record["kind"] == "grandchild" for record in records):
                    break
                if runtime.poll() is not None:
                    stdout, stderr = runtime.communicate()
                    self.fail(f"Mac runner exited before fake build started: {stdout}{stderr}")
                time.sleep(0.02)
            self.assertEqual([record["kind"] for record in records],
                             ["mbx", "cargo", "grandchild"])
            owner = records[0]["pid"]
            self.assertNotEqual(owner, runtime.pid)
            for record in records:
                self.assertEqual(record["pgid"], owner)
            self.assertEqual(len({record["pid"] for record in records}), 3,
                             "Cancellation must cover three live process generations")
            runtime.terminate()
            runtime.communicate(timeout=5)
            self.assertIsNone(neighbor.poll(), "Cancellation killed an unrelated process")
            for record in records:
                deadline = time.monotonic() + 3
                while time.monotonic() < deadline:
                    status = subprocess.run([real_ps, "-p", str(record["pid"]), "-o", "stat="],
                                            text=True, capture_output=True, check=False).stdout.strip()
                    if not status or status.startswith("Z"):
                        break
                    time.sleep(0.02)
                else:
                    self.fail(f"Fake {record['kind']} survived cancellation: PID {record['pid']}")
        finally:
            # All these groups were created by this test in private sessions.
            groups = {runtime.pid}
            groups.update(record["pgid"] for record in records
                          if record["pgid"] == record["pid"])
            for group in groups:
                try:
                    os.killpg(group, signal.SIGTERM)
                except ProcessLookupError:
                    pass
            if runtime.poll() is None:
                runtime.kill()
            runtime.communicate(timeout=5)
            neighbor.terminate()
            neighbor.wait(timeout=5)


    def test_android_cargo_check_preserves_unchanged_archive_but_copies_updates(self):
        # Use a tiny owned Core tree; never overwrite actual cross-build outputs.
        core = self.root / "core"
        (core / "scripts").mkdir(parents=True)
        (self.root / "scripts/lib").mkdir(parents=True)
        shutil.copyfile(REPOSITORY_ROOT / "core/scripts/build_android.sh", core / "scripts/build_android.sh")
        shutil.copyfile(HELPER, self.root / "scripts/lib/rust_build_cache.sh")
        ndk = self.root / "ndk"
        ndk.mkdir()
        archive = core / "target/aarch64-linux-android/release/libnola_mobile_bindings.a"
        archive.parent.mkdir(parents=True)
        archive.write_bytes(b"first archive")
        output = self.root / "native outputs"
        env = dict(os.environ, CARGO=str(self.cargo), NOLA_TEST_LOG=str(self.log_path),
                   NOLA_RUST_BUILD_CACHE="off", NOLA_ANDROID_ABIS="arm64-v8a", ANDROID_NDK_HOME=str(ndk))
        def build():
            result = subprocess.run(["bash", str(core / "scripts/build_android.sh"), str(output)],
                                    env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
        build()
        copied = output / "arm64-v8a/libnola_core.a"
        os.utime(copied, ns=(1_000_000_000, 1_000_000_000))
        build()
        self.assertEqual(copied.stat().st_mtime_ns, 1_000_000_000)
        archive.write_bytes(b"updated archive")
        build()
        self.assertEqual(copied.read_bytes(), b"updated archive")

class RustBuildCacheIntegrationTests(unittest.TestCase):
    def test_only_mobile_bindings_produce_native_abi_artifacts(self):
        import tomllib
        core = tomllib.loads((REPOSITORY_ROOT / "core/Cargo.toml").read_text())
        bindings = tomllib.loads((REPOSITORY_ROOT / "core/nola-mobile-bindings/Cargo.toml").read_text())
        self.assertEqual(core["lib"]["crate-type"], ["rlib"])
        self.assertEqual(set(bindings["lib"]["crate-type"]), {"rlib", "staticlib", "cdylib"})

    def test_profiles_bound_default_debug_storage_and_keep_opt_in_debugger(self):
        import tomllib

        manifest = tomllib.loads((REPOSITORY_ROOT / "Cargo.toml").read_text())
        for name in ("dev", "test", "release"):
            with self.subTest(profile=name):
                profile = manifest["profile"][name]
                self.assertEqual(profile["debug"], 0)
                self.assertEqual(profile["strip"], "debuginfo")
                self.assertEqual(profile["split-debuginfo"], "off")
                self.assertIs(profile["incremental"], False)
        debugger = manifest["profile"]["dev-debug"]
        self.assertEqual(debugger["inherits"], "dev")
        self.assertEqual(debugger["debug"], "full")
        self.assertEqual(debugger["strip"], "none")
        self.assertEqual(debugger["split-debuginfo"], "packed")
        self.assertIs(debugger["incremental"], False)

    def test_native_builds_share_target_and_cargo_owns_freshness(self):
        mac = (REPOSITORY_ROOT / "scripts/build_mac_runtime.sh").read_text()
        self.assertIn('${NOLA_AGENT_CARGO_TARGET_DIR:-${CARGO_TARGET_DIR:-${ROOT}/target}}', mac)
        android = (REPOSITORY_ROOT / "apps/android/app/build.gradle.kts").read_text()
        rust_task = android.split("val buildRustCore by", 1)[1].split("tasks.configureEach", 1)[0]
        self.assertIn("outputs.upToDateWhen { false }", rust_task)
        self.assertNotIn("inputs.files(fileTree", rust_task)
        self.assertNotIn("rust_runtime_needs_rebuild", mac)
        self.assertNotIn("-newer", mac)
        self.assertIn('"$ROOT/rust-toolchain.toml"', mac)
        runner = (REPOSITORY_ROOT / "scripts/run_real_mac_xctests.sh").read_text()
        self.assertNotIn("$ROOT/apps/cli/target", runner)
        self.assertIn("${CARGO_TARGET_DIR:-$ROOT/target}", runner)
        project = (REPOSITORY_ROOT / "shared/apple-core/NolaCore.xcodeproj/project.pbxproj").read_text()
        self.assertNotIn("target/mac-runtime", project)
        self.assertEqual(project.count('"$(SRCROOT)/../../rust-toolchain.toml",'), 2)

    def test_shared_cli_artifacts_use_one_workspace_manifest(self):
        # The CLI and native runtime share artifact names. Resolving features
        # through different manifests can replace one with a different graph,
        # independently of whether the compiler cache itself is correct.
        for relative_path in (
            "apps/cli/scripts/build.sh",
            "scripts/build_cli_runtime.sh",
            "scripts/build_mac_runtime.sh",
        ):
            with self.subTest(script=relative_path):
                source = (REPOSITORY_ROOT / relative_path).read_text(encoding="utf-8")
                self.assertNotIn('$ROOT/apps/cli/Cargo.toml', source)
                self.assertRegex(source, r'(?:ROOT|REPO_ROOT).*Cargo\.toml')
        # Both Xcode Rust phases must notice launcher-only edits even before
        # their first generated dependency file includes the helper.
        project = (REPOSITORY_ROOT / "shared/apple-core/NolaCore.xcodeproj/project.pbxproj").read_text(
            encoding="utf-8"
        )
        self.assertEqual(project.count('"$(SRCROOT)/../../scripts/lib/rust_build_cache.sh",'), 2)

    def test_build_scripts_source_and_initialize_shared_launcher(self):
        for relative_path in BUILD_SCRIPTS:
            with self.subTest(script=relative_path):
                source = (REPOSITORY_ROOT / relative_path).read_text(encoding="utf-8")
                commands = "\n".join(line for line in source.splitlines()
                                     if not line.lstrip().startswith("#"))
                commands = re.sub(r"\\\n\s*", " ", commands)
                invocation = r"(?m)^\s*(?:\w+=(?:\"[^\"]*\"|'[^']*'|\S+)\s+)*"
                self.assertRegex(commands, r"rust_build_cache\.sh")
                self.assertRegex(commands, r"(?m)^\s*(?:source|\.)\s+[^\n]*(?:rust_build_cache\.sh|NOLA_RUST_BUILD_CACHE_HELPER)")
                self.assertRegex(commands, r"(?m)^\s*nola_rust_build_cache_init\b")
                self.assertRegex(commands, r"\bnola_cargo(?:_exec(?:_owned_group)?)?\b")
                self.assertNotRegex(commands, invocation + r"cargo\s+(?:build|test|check|rustc)\b")
                self.assertNotRegex(commands, invocation + r"cargo\s+ndk\s+[^\n]*\bbuild\b")
                self.assertNotRegex(commands, r'(?m)^\s*"\$CARGO_BIN"\s+"\$\{CARGO_ARGS\[@\]\}"')
                if relative_path == "scripts/build_mac_runtime.sh":
                    self.assertRegex(commands, r"\bnola_cargo_exec(?:_owned_group)?\b")
                elif relative_path == "core/scripts/build_android.sh":
                    self.assertRegex(commands, r"\bnola_cargo\s+ndk\s+[^\n]*\bbuild\b")
                else:
                    self.assertRegex(commands, r"\bnola_cargo\s+build\b")


if __name__ == "__main__":
    unittest.main()
