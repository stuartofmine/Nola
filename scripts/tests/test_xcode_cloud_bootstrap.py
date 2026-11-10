# SPDX-License-Identifier: Apache-2.0
"""Exercise Cloud bootstrap in isolated fake toolchains without network/builds."""
import json
import os
import re
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class XcodeCloudBootstrapTests(unittest.TestCase):
    def test_clean_worker_installs_before_resolving_repository_toolchain(self):
        with tempfile.TemporaryDirectory(prefix="nola-cloud-clean-") as temporary:
            directory = Path(temporary)
            binary = directory / "fake-bin"
            binary.mkdir()
            cargo_home = directory / "cargo"
            log = directory / "installed"
            installer = directory / "installer.sh"
            installer.write_text(
                '#!/bin/sh\nset -eu\nmkdir -p "$CARGO_HOME/bin"\n'
                'for tool in rustup rustc cargo; do\n'
                ' cp "$NOLA_FAKE_TOOL" "$CARGO_HOME/bin/$tool"\n'
                'done\n'
            )
            fake_tool = directory / "tool"
            fake_tool.write_text(f'#!/bin/sh\nprintf "%s\\n" "$*" >> "{log}"\n')
            fake_tool.chmod(0o755)
            curl = binary / "curl"
            curl.write_text(
                '#!/bin/sh\nset -eu\n'
                'while [ "$#" -gt 0 ]; do\n'
                ' if [ "$1" = -o ]; then cp "$NOLA_FAKE_INSTALLER" "$2"; exit 0; fi\n'
                ' shift\ndone\nexit 90\n'
            )
            curl.chmod(0o755)
            result = subprocess.run(
                ["/bin/sh", str(ROOT / "ci_scripts/ci_post_clone.sh")],
                cwd=directory,
                env=dict(os.environ, CARGO_HOME=str(cargo_home),
                         PATH=f"{binary}:/usr/bin:/bin", NOLA_FAKE_TOOL=str(fake_tool),
                         NOLA_FAKE_INSTALLER=str(installer)),
                capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(log.read_text().splitlines(), [
                "show active-toolchain", "target add aarch64-apple-ios aarch64-apple-ios-sim",
                "--version", "--version",
            ])

    def test_preinstalled_toolchain_uses_repository_and_explicit_cargo_home(self):
        with tempfile.TemporaryDirectory(prefix="nola-cloud-test-") as temporary:
            directory = Path(temporary)
            cargo_home = directory / "cargo home"
            binary = cargo_home / "bin"
            binary.mkdir(parents=True)
            log = directory / "calls.jsonl"
            for tool in ("rustup", "rustc", "cargo", "curl"):
                executable = binary / tool
                executable.write_text(
                    f"#!{sys.executable}\n"
                    "import json, os, sys\n"
                    "from pathlib import Path\n"
                    f"with open({str(log)!r}, 'a') as output:\n"
                    " output.write(json.dumps([Path(sys.argv[0]).name, sys.argv[1:], os.getcwd()]) + '\\n')\n"
                    "if Path(sys.argv[0]).name == 'curl': sys.exit(91)\n"
                )
                executable.chmod(0o755)
            result = subprocess.run(
                ["/bin/sh", str(ROOT / "ci_scripts/ci_post_clone.sh")],
                cwd=directory,
                env=dict(os.environ, CARGO_HOME=str(cargo_home), PATH="/usr/bin:/bin"),
                capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            self.assertEqual([call[:2] for call in calls], [
                ["rustup", ["show", "active-toolchain"]],
                ["rustup", ["target", "add", "aarch64-apple-ios", "aarch64-apple-ios-sim"]],
                ["rustc", ["--version"]], ["cargo", ["--version"]],
            ])
            self.assertTrue(all(Path(call[2]).resolve() == ROOT.resolve() for call in calls))

    def test_bootstrap_is_executable_and_core_cargo_phase_is_not_sandboxed(self):
        self.assertTrue(os.access(ROOT / "ci_scripts/ci_post_clone.sh", os.X_OK))
        project = (ROOT / "shared/apple-core/NolaCore.xcodeproj/project.pbxproj").read_text()
        # Each Core target configuration must override the project sandbox:
        # Cargo writes registry/target caches and may fetch pinned dependencies.
        configurations = [settings for settings in re.findall(r"buildSettings = \{(.*?)\};", project, re.S)
                          if "PRODUCT_NAME = NolaCore;" in settings]
        self.assertEqual(len(configurations), 3)
        for settings in configurations:
            self.assertIn("ENABLE_USER_SCRIPT_SANDBOXING = NO;", settings)


if __name__ == "__main__":
    unittest.main()
