# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import importlib.util
import argparse
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, call, patch

scripts = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(scripts))
spec = importlib.util.spec_from_file_location(
    "run_external_benchmark", scripts / "run_external_benchmark.py"
)
runner = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(runner)


class TerminalResourcePreflightTests(unittest.TestCase):
    def run_network_probe_script(
        self, *, up_links: str, global_addresses: str = "",
        default_v4: str = "", default_v6: str = "",
    ) -> runner.subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as temporary:
            fake_ip = Path(temporary) / "ip"
            fake_ip.write_text(
                "#!/bin/sh\n"
                "case \"$*\" in\n"
                "  '-o link show up') printf '%s\\n' \"$PROBE_UP_LINKS\" ;;\n"
                "  '-o addr show scope global') printf '%s\\n' \"$PROBE_GLOBAL\" ;;\n"
                "  '-4 route show default') printf '%s\\n' \"$PROBE_DEFAULT_V4\" ;;\n"
                "  '-6 route show default') printf '%s\\n' \"$PROBE_DEFAULT_V6\" ;;\n"
                "  *) exit 64 ;;\n"
                "esac\n"
            )
            fake_ip.chmod(0o755)
            environment = {
                **os.environ,
                "PATH": f"{temporary}:/usr/bin:/bin",
                "PROBE_UP_LINKS": up_links,
                "PROBE_GLOBAL": global_addresses,
                "PROBE_DEFAULT_V4": default_v4,
                "PROBE_DEFAULT_V6": default_v6,
            }
            return runner.subprocess.run(
                ["sh", "-c", runner.TERMINAL_DOCKER_NONE_PROBE_SCRIPT],
                check=False,
                capture_output=True,
                text=True,
                env=environment,
            )

    def parse_iteration_args(self, *arguments: str) -> argparse.Namespace:
        with patch.object(sys, "argv", ["run_external_benchmark.py", *arguments]):
            return runner.parse_args()

    def test_iteration_budget_defaults_remain_benchmark_specific(self) -> None:
        terminal = self.parse_iteration_args("terminal-bench")
        runner.resolve_iteration_budgets(terminal)
        self.assertEqual(terminal.max_iterations, 64)

        memory = self.parse_iteration_args(
            "longmemeval", "--longmemeval-mode", "memory-architecture"
        )
        runner.resolve_iteration_budgets(memory)
        self.assertEqual(memory.architecture_max_iterations, 8)

    def test_public_acceptance_and_shell_ablation_cannot_be_silently_ignored(self):
        for flag, value in [("--shell-purpose-contract", "optional"), ("--task-acceptance", "public.json")]:
            args = self.parse_iteration_args("longmemeval", flag, value)
            with self.assertRaisesRegex(SystemExit, "only to Terminal-Bench"):
                runner.resolve_iteration_budgets(args)
        args = self.parse_iteration_args("terminal-bench", "--shell-purpose-contract", "optional",
                                         "--task-acceptance", "public.json")
        runner.resolve_iteration_budgets(args)
        self.assertEqual(args.shell_purpose_contract, "optional")
        self.assertEqual(args.task_acceptance, Path("public.json"))

    def test_terminal_iteration_budget_uses_the_current_option_only(self) -> None:
        args = self.parse_iteration_args("terminal-bench", "--max-iterations", "16")
        runner.resolve_iteration_budgets(args)
        self.assertEqual(args.max_iterations, 16)
        with self.assertRaises(SystemExit):
            self.parse_iteration_args("terminal-bench", "--terminal-max-iterations", "16")

    def test_long_memory_rejects_terminal_iteration_budget_with_hint(self) -> None:
        args = self.parse_iteration_args(
            "longmemeval",
            "--longmemeval-mode",
            "memory-architecture",
            "--max-iterations",
            "16",
        )
        with self.assertRaisesRegex(SystemExit, "use --architecture-max-iterations"):
            runner.resolve_iteration_budgets(args)

    def test_architecture_budget_rejects_modes_that_ignore_it(self) -> None:
        for arguments in (
            ("longmemeval", "--architecture-max-iterations", "16"),
            ("terminal-bench", "--architecture-max-iterations", "16"),
        ):
            args = self.parse_iteration_args(*arguments)
            with self.assertRaisesRegex(SystemExit, "memory-architecture"):
                runner.resolve_iteration_budgets(args)

    def test_loads_string_and_structured_artifact_declarations(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for task_id, declaration in (
                ("task-a", '["/app/alpha.out", "/app/alpha.out"]'),
                ("task-b", '[{ source = "/shared", service = "workspace" }]'),
            ):
                task = root / task_id
                task.mkdir()
                (task / "task.toml").write_text(
                    f'schema_version = "1.2"\nartifacts = {declaration}\n',
                    encoding="utf-8",
                )
            manifest = {
                "cases": [
                    {"id": "task-a", "source_path": "tasks/task-a"},
                    {"id": "task-b", "source_path": "tasks/task-b"},
                ]
            }

            self.assertEqual(
                runner.terminal_expected_artifacts(manifest, root, []),
                {"task-a": ["/app/alpha.out"], "task-b": ["/shared"]},
            )
            self.assertEqual(
                runner.terminal_expected_artifacts(manifest, root, ["task-b"]),
                {"task-b": ["/shared"]},
            )

    def test_loads_eval_only_case_selection_and_rejects_duplicates(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "selection.json"
            path.write_text(
                json.dumps(
                    {
                        "benchmarks": {
                            "longmemeval": {
                                "cases": [
                                    {"id": "memory-a", "coverage": "temporal"},
                                    "memory-b",
                                ]
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )
            self.assertEqual(
                runner.load_selection(path, "longmemeval"),
                {
                    "case_ids": ["memory-a", "memory-b"],
                    "required_mode": None,
                },
            )

            path.write_text(
                json.dumps(
                    {"benchmarks": {"longmemeval": {"cases": ["memory-a", "memory-a"]}}}
                ),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "repeats case id"):
                runner.load_selection(path, "longmemeval")

    def test_selected_case_ids_must_exist_in_the_pinned_manifest(self) -> None:
        manifest = {"cases": [{"id": "case-a"}, {"id": "case-b"}]}
        runner.validate_case_ids(manifest, ["case-b"])
        with self.assertRaisesRegex(ValueError, "unknown case ids: missing"):
            runner.validate_case_ids(manifest, ["missing"])

    def write_task(
        self,
        root: Path,
        task_id: str,
        *,
        agent_cpus: int,
        agent_memory_mb: int,
        verifier_cpus: int | None = None,
        verifier_allow_internet: bool | None = None,
    ) -> None:
        task = root / task_id
        task.mkdir(parents=True)
        verifier_values = []
        if verifier_cpus is not None:
            verifier_values.extend([f"cpus = {verifier_cpus}", "memory_mb = 1024"])
        if verifier_allow_internet is not None:
            verifier_values.append(
                f"allow_internet = {str(verifier_allow_internet).lower()}"
            )
        verifier = (
            "\n[verifier.environment]\n" + "\n".join(verifier_values) + "\n"
            if verifier_values
            else ""
        )
        (task / "task.toml").write_text(
            f"""schema_version = "1.2"
[environment]
cpus = {agent_cpus}
memory_mb = {agent_memory_mb}
{verifier}
""",
            encoding="utf-8",
        )

    def test_reports_each_stage_that_exceeds_runtime_capacity(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.write_task(
                root,
                "task-a",
                agent_cpus=3,
                agent_memory_mb=9000,
                verifier_cpus=9,
            )
            manifest = {"cases": [{"id": "task-a", "source_path": "tasks/task-a"}]}

            report = runner.terminal_resource_preflight(
                manifest, root, [], {"cpus": 8, "memory_mb": 8192}
            )

            self.assertFalse(report["passed"])
            self.assertEqual(
                {(item["stage"], item["resource"]) for item in report["violations"]},
                {("agent", "memory_mb"), ("verifier", "cpus")},
            )

    def test_filters_selected_tasks_and_rejects_unknown_ids(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.write_task(root, "task-a", agent_cpus=1, agent_memory_mb=1024)
            self.write_task(root, "task-b", agent_cpus=16, agent_memory_mb=1024)
            manifest = {
                "cases": [
                    {"id": "task-a", "source_path": "tasks/task-a"},
                    {"id": "task-b", "source_path": "tasks/task-b"},
                ]
            }

            report = runner.terminal_resource_preflight(
                manifest, root, ["task-a"], {"cpus": 8, "memory_mb": 8192}
            )

            self.assertTrue(report["passed"])
            self.assertEqual([task["id"] for task in report["tasks"]], ["task-a"])
            with self.assertRaisesRegex(ValueError, "unknown Terminal-Bench case ids"):
                runner.terminal_resource_preflight(
                    manifest, root, ["missing"], {"cpus": 8, "memory_mb": 8192}
                )

    def test_reports_when_a_selected_stage_requires_network_isolation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.write_task(
                root,
                "task-a",
                agent_cpus=1,
                agent_memory_mb=1024,
                verifier_allow_internet=False,
            )
            manifest = {"cases": [{"id": "task-a", "source_path": "tasks/task-a"}]}

            report = runner.terminal_resource_preflight(
                manifest, root, [], {"cpus": 8, "memory_mb": 8192}
            )

            self.assertTrue(report["requires_network_isolation"])
            self.assertEqual(
                report["tasks"][0]["environments"][1]["network_mode"],
                "no-network",
            )

    def test_native_network_provider_accepts_only_static_separate_verifiers(self) -> None:
        compatible = {
            "tasks": [
                {
                    "id": "task-a",
                    "verifier_environment_mode": "separate",
                    "environments": [
                        {"stage": "agent"},
                        {"stage": "verifier", "network_mode": "no-network"},
                    ],
                }
            ]
        }
        self.assertEqual(
            runner.terminal_native_no_network_incompatibilities(compatible), []
        )
        incompatible = {
            "tasks": [
                {
                    "id": "task-b",
                    "verifier_environment_mode": "shared",
                    "environments": [
                        {"stage": "agent", "network_mode": "no-network"},
                        {"stage": "verifier", "network_mode": "allowlist"},
                    ],
                }
            ]
        }
        self.assertEqual(
            runner.terminal_native_no_network_incompatibilities(incompatible),
            ["task-b:agent:no-network", "task-b:verifier:allowlist"],
        )

    @patch.object(runner.subprocess, "run")
    def test_native_no_network_probe_uses_cached_pinned_image(self, run) -> None:
        run.side_effect = [
            Mock(returncode=0, stdout="[]", stderr=""),
            Mock(returncode=0, stdout="", stderr=""),
        ]

        report = runner.docker_native_no_network_preflight("runtime")

        self.assertTrue(report["passed"])
        self.assertTrue(report["image_was_cached"])
        self.assertEqual(run.call_count, 2)
        self.assertEqual(
            run.call_args_list[1],
            call(
                [
                    "runtime",
                    "container",
                    "run",
                    "--rm",
                    "--network",
                    "none",
                    runner.TERMINAL_DOCKER_NETWORK_PROBE_IMAGE,
                    "sh",
                    "-c",
                    runner.TERMINAL_DOCKER_NONE_PROBE_SCRIPT,
                ],
                check=False,
                capture_output=True,
                text=True,
                timeout=120,
            ),
        )

    def test_native_no_network_script_accepts_loopback_as_only_up_interface(self) -> None:
        completed = self.run_network_probe_script(
            up_links="1: lo: <LOOPBACK,UP,LOWER_UP> mtu 65536 state UNKNOWN"
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_native_no_network_script_rejects_an_extra_up_interface(self) -> None:
        completed = self.run_network_probe_script(
            up_links=(
                "1: lo: <LOOPBACK,UP,LOWER_UP> mtu 65536 state UNKNOWN\n"
                "2: eth0@if3: <BROADCAST,UP,LOWER_UP> mtu 1500 state UP"
            )
        )
        self.assertEqual(completed.returncode, 20)
        self.assertIn("unexpected UP interfaces", completed.stderr)

    def test_native_no_network_script_rejects_a_global_address(self) -> None:
        completed = self.run_network_probe_script(
            up_links="1: lo: <LOOPBACK,UP,LOWER_UP> mtu 65536 state UNKNOWN",
            global_addresses="2: eth0 inet 172.18.0.2/16 scope global eth0",
        )
        self.assertEqual(completed.returncode, 21)
        self.assertIn("unexpected global address", completed.stderr)

    def test_native_no_network_script_rejects_default_v4_or_v6_route(self) -> None:
        fixtures = (
            ({"default_v4": "default via 172.18.0.1 dev eth0"}, 22, "IPv4"),
            ({"default_v6": "default via fe80::1 dev eth0"}, 23, "IPv6"),
        )
        for routes, expected_code, family in fixtures:
            with self.subTest(family=family):
                completed = self.run_network_probe_script(
                    up_links=(
                        "1: lo: <LOOPBACK,UP,LOWER_UP> mtu 65536 state UNKNOWN"
                    ),
                    **routes,
                )
                self.assertEqual(completed.returncode, expected_code)
                self.assertIn(f"unexpected {family} default route", completed.stderr)

    @patch.object(runner.subprocess, "run")
    def test_native_no_network_probe_pulls_missing_image_and_fails_closed(self, run) -> None:
        run.side_effect = [
            Mock(returncode=1, stdout="", stderr="missing"),
            Mock(returncode=0, stdout="pulled", stderr=""),
            Mock(returncode=1, stdout="", stderr="kernel unsupported"),
        ]

        report = runner.docker_native_no_network_preflight("runtime")

        self.assertFalse(report["passed"])
        self.assertFalse(report["image_was_cached"])
        self.assertEqual(report["phase"], "native-no-network")
        self.assertEqual(report["error"], "kernel unsupported")
        self.assertEqual(
            run.call_args_list[1].args[0],
            ["runtime", "pull", runner.TERMINAL_DOCKER_NETWORK_PROBE_IMAGE],
        )

    @patch.object(runner.subprocess, "run")
    @patch.object(runner, "docker_native_no_network_preflight")
    @patch.object(runner, "docker_capacity", return_value={"cpus": 8, "memory_mb": 8192})
    @patch.object(runner, "terminal_resource_preflight")
    @patch.object(runner, "load_manifest")
    def test_network_preflight_failure_prevents_harbor_run(
        self, load_manifest, resource_preflight, _capacity, network_preflight, run
    ) -> None:
        load_manifest.return_value = {"variant": "fixture", "cases": []}
        resource_preflight.return_value = {
            "passed": True,
            "violations": [],
            "requires_network_isolation": True,
        }
        network_preflight.return_value = {
            "passed": False,
            "phase": "image-pull",
            "error": "registry unavailable",
        }
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "nola"
            binary.write_bytes(b"\x7fELF")
            args = argparse.Namespace(
                output=root,
                run_directory=None,
                dry_run=False,
                binary=binary,
                config=root / "config.json",
                skip_runtime_preflight=False,
                case_id=[],
                docker="docker",
                harbor="harbor",
                timeout_seconds=60,
                max_iterations=8,
                tool_set="auto",
                concurrency=1,
            )

            with self.assertRaisesRegex(RuntimeError, "registry unavailable"):
                runner.run_terminal_bench(args)

            run.assert_not_called()
            report = json.loads(
                next(root.glob("terminal-bench-*/preflight.json")).read_text()
            )
            self.assertFalse(report["network_isolation"]["passed"])

    @patch.object(runner.subprocess, "run")
    @patch.object(runner, "docker_native_no_network_preflight")
    @patch.object(runner, "docker_capacity", return_value={"cpus": 8, "memory_mb": 8192})
    @patch.object(runner, "terminal_resource_preflight")
    @patch.object(runner, "load_manifest")
    def test_terminal_run_uses_native_isolated_environment_provider(
        self,
        load_manifest,
        resource_preflight,
        _capacity,
        network_preflight,
        run,
    ) -> None:
        load_manifest.return_value = {"variant": "fixture", "cases": []}
        resource_preflight.return_value = {
            "passed": True,
            "violations": [],
            "requires_network_isolation": True,
        }
        network_preflight.return_value = {"passed": True}
        run.return_value.returncode = 0
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "nola"
            binary.write_bytes(b"\x7fELF")
            args = argparse.Namespace(
                output=root,
                run_directory=None,
                dry_run=False,
                binary=binary,
                config=root / "config.json",
                skip_runtime_preflight=False,
                case_id=[],
                docker="docker",
                harbor="harbor",
                timeout_seconds=60,
                max_iterations=8,
                tool_set="auto",
                concurrency=1,
            )

            self.assertEqual(runner.run_terminal_bench(args), 0)

            command = run.call_args.args[0]
            self.assertEqual(
                command[command.index("--env") + 1],
                "evals.harbor_native_no_network:DockerNativeNoNetworkEnvironment",
            )

    @patch.object(runner.subprocess, "run")
    def test_reads_capacity_from_docker_daemon(self, run) -> None:
        run.return_value.returncode = 0
        run.return_value.stdout = json.dumps({"NCPU": 6, "MemTotal": 12 * 1024**3})
        run.return_value.stderr = ""

        self.assertEqual(
            runner.docker_capacity("runtime"), {"cpus": 6, "memory_mb": 12288}
        )
        run.assert_called_once_with(
            ["runtime", "info", "--format", "{{json .}}"],
            check=False,
            capture_output=True,
            text=True,
        )

    @patch.object(runner.subprocess, "run")
    @patch.object(runner, "run_stamp", return_value="fixed")
    @patch.object(runner, "load_manifest")
    def test_terminal_command_uses_native_runtime_contract(
        self, load_manifest, _run_stamp, subprocess_run
    ) -> None:
        load_manifest.return_value = {"variant": "fixture", "cases": []}
        subprocess_run.return_value.returncode = 0
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            args = argparse.Namespace(
                output=root,
                dry_run=True,
                binary=root / "nola",
                config=root / "config.json",
                skip_runtime_preflight=True,
                case_id=[],
                docker="docker",
                harbor="harbor",
                timeout_seconds=60,
                max_iterations=8,
                tool_set="auto",
                concurrency=1,
            )

            self.assertEqual(runner.run_terminal_bench(args), 0)
            invocation = json.loads(
                (root / "terminal-bench-fixed" / "invocation.json").read_text()
            )
            command = invocation["command"]
            self.assertFalse(
                any("code_memory_binary_path=" in argument for argument in command)
            )
            self.assertIn(f"binary_path={args.binary}", command)

    @patch.object(runner, "run_with_boundary_interrupt", return_value=0)
    def test_long_memory_exact_run_directory_is_resumable(self, managed_run) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            run_directory = root / "retained-run"
            args = argparse.Namespace(
                output=root,
                run_directory=run_directory,
                binary=root / "nola",
                config=root / "config.json",
                concurrency=1,
                timeout_seconds=60,
                longmemeval_mode="memory-architecture",
                architecture_max_iterations=8,
                case_id=["memory-a"],
                judge_config=None,
                judge_endpoint=None,
                judge_model=None,
                judge_api_key_env=None,
                dry_run=False,
            )

            self.assertEqual(runner.run_longmemeval(args), 0)
            command = managed_run.call_args.args[0]
            self.assertEqual(command[command.index("--output") + 1], str(run_directory))

    @patch.object(runner.signal, "signal")
    @patch.object(runner.subprocess, "Popen")
    def test_long_memory_interrupt_is_forwarded_to_isolated_evaluator(
        self, popen, install_signal
    ) -> None:
        process = popen.return_value
        process.poll.return_value = None

        def wait() -> int:
            handler = install_signal.call_args_list[0].args[1]
            handler(runner.signal.SIGINT, None)
            return 130

        process.wait.side_effect = wait
        result = runner.run_with_boundary_interrupt(
            ["python", "run.py"], cwd=Path("/tmp"), env={}
        )

        self.assertEqual(result, 130)
        popen.assert_called_once_with(
            ["python", "run.py"], cwd=Path("/tmp"), env={}, start_new_session=True
        )
        process.send_signal.assert_called_once_with(runner.signal.SIGINT)

    @patch.object(runner.subprocess, "run")
    @patch.object(runner, "load_manifest")
    def test_terminal_exact_run_directory_can_reuse_preflight_directory(
        self, load_manifest, subprocess_run
    ) -> None:
        load_manifest.return_value = {"variant": "fixture", "cases": []}
        subprocess_run.return_value.returncode = 0
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            run_directory = root / "retained-run"
            run_directory.mkdir()
            (run_directory / "preflight.json").write_text("{}\n")
            binary = root / "nola"
            binary.write_bytes(b"\x7fELF")
            args = argparse.Namespace(
                output=root,
                run_directory=run_directory,
                dry_run=False,
                binary=binary,
                config=root / "config.json",
                skip_runtime_preflight=True,
                case_id=[],
                docker="docker",
                harbor="harbor",
                timeout_seconds=60,
                max_iterations=8,
                tool_set="auto",
                concurrency=1,
            )

            self.assertEqual(runner.run_terminal_bench(args), 0)
            command = subprocess_run.call_args.args[0]
            self.assertEqual(command[command.index("--jobs-dir") + 1], str(run_directory))


if __name__ == "__main__":
    unittest.main()
