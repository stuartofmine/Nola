#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Run Nola against the pinned LongMemEval or Terminal-Bench baseline."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import signal
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any

from check_external_benchmark_specialization import assert_no_benchmark_specialization


ROOT = Path(__file__).resolve().parents[1]
BENCHMARK_ROOT = ROOT / "evals" / "benchmarks"
DEFAULT_OUTPUT = ROOT / ".artifacts" / "evals" / "runs"
DEFAULT_TERMINAL_MAX_ITERATIONS = 64
DEFAULT_LONGMEM_ARCHITECTURE_MAX_ITERATIONS = 8
DEFAULT_LONGMEM_TRANSPORT_MAX_ATTEMPTS = 3
DEFAULT_LONGMEM_TRANSPORT_RETRY_BACKOFF_SECONDS = 1.0
TERMINAL_DOCKER_NETWORK_PROBE_IMAGE = (
    "alpine:3.23.4@sha256:"
    "5b10f432ef3da1b8d4c7eb6c487f2f5a8f096bc91145e68878dd4a5019afde11"
)
TERMINAL_DOCKER_NONE_PROBE_SCRIPT = (
    "set -eu; "
    "up_interfaces=\"$(ip -o link show up | awk -F': ' '{print $2}' | cut -d@ -f1)\"; "
    "[ \"$up_interfaces\" = lo ] || { "
    "echo \"unexpected UP interfaces: $up_interfaces\" >&2; exit 20; }; "
    "global_addresses=\"$(ip -o addr show scope global)\"; "
    "[ -z \"$global_addresses\" ] || { "
    "echo \"unexpected global address\" >&2; exit 21; }; "
    "default_v4=\"$(ip -4 route show default)\"; "
    "[ -z \"$default_v4\" ] || { "
    "echo \"unexpected IPv4 default route\" >&2; exit 22; }; "
    "default_v6=\"$(ip -6 route show default)\"; "
    "[ -z \"$default_v6\" ] || { "
    "echo \"unexpected IPv6 default route\" >&2; exit 23; }"
)


def run_with_boundary_interrupt(
    command: list[str], *, cwd: Path, env: dict[str, str]
) -> int:
    """Forward SIGINT to an isolated evaluator without killing its active CLI turn."""
    previous_sigint = signal.getsignal(signal.SIGINT)
    process: subprocess.Popen[Any] | None = None
    stop_requested = False
    forwarded = False
    announced = False

    def request_stop(_signum: int, _frame: Any) -> None:
        nonlocal announced, forwarded, stop_requested
        stop_requested = True
        if process is not None and process.poll() is None and not forwarded:
            process.send_signal(signal.SIGINT)
            forwarded = True
        if not announced:
            print(
                "interrupt received; waiting for the active benchmark turn to finish",
                file=sys.stderr,
                flush=True,
            )
            announced = True

    signal.signal(signal.SIGINT, request_stop)
    try:
        process = subprocess.Popen(command, cwd=cwd, env=env, start_new_session=True)
        if stop_requested and not forwarded:
            process.send_signal(signal.SIGINT)
            forwarded = True
        return process.wait()
    finally:
        signal.signal(signal.SIGINT, previous_sigint)


def load_manifest(name: str) -> dict[str, Any]:
    root = BENCHMARK_ROOT / name
    return json.loads((root / "benchmark.json").read_text(encoding="utf-8"))


def load_selection(path: Path, benchmark: str) -> dict[str, Any]:
    """Load an eval-only case selection without coupling product code to case IDs."""
    payload = json.loads(path.read_text(encoding="utf-8"))
    benchmarks = payload.get("benchmarks") if isinstance(payload, dict) else None
    selection = benchmarks.get(benchmark) if isinstance(benchmarks, dict) else None
    raw_cases = selection.get("cases") if isinstance(selection, dict) else None
    if not isinstance(raw_cases, list) or not raw_cases:
        raise ValueError(f"selection has no cases for {benchmark!r}: {path}")
    case_ids: list[str] = []
    for index, entry in enumerate(raw_cases):
        if isinstance(entry, str):
            case_id = entry
        elif isinstance(entry, dict):
            case_id = entry.get("id")
        else:
            case_id = None
        if not isinstance(case_id, str) or not case_id:
            raise ValueError(
                f"selection case {index + 1} for {benchmark!r} has no non-empty id"
            )
        if case_id in case_ids:
            raise ValueError(f"selection repeats case id {case_id!r} for {benchmark!r}")
        case_ids.append(case_id)
    required_mode = selection.get("required_mode")
    if required_mode is not None and (
        not isinstance(required_mode, str) or not required_mode
    ):
        raise ValueError(f"selection required_mode for {benchmark!r} must be a string")
    return {"case_ids": case_ids, "required_mode": required_mode}


def validate_case_ids(manifest: dict[str, Any], case_ids: list[str]) -> None:
    raw_cases = manifest.get("cases")
    if not isinstance(raw_cases, list):
        raise ValueError("benchmark manifest has no cases")
    known = {
        entry.get("id")
        for entry in raw_cases
        if isinstance(entry, dict) and isinstance(entry.get("id"), str)
    }
    unknown = sorted(set(case_ids) - known)
    if unknown:
        raise ValueError("unknown case ids: " + ", ".join(unknown))


def docker_capacity(docker_command: str = "docker") -> dict[str, int]:
    """Read capacity from the Docker daemon that Harbor will actually use."""
    try:
        completed = subprocess.run(
            [docker_command, "info", "--format", "{{json .}}"],
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as error:
        raise RuntimeError(
            f"cannot inspect Docker runtime capacity: {error}"
        ) from error
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or "unknown error"
        raise RuntimeError(f"cannot inspect Docker runtime capacity: {detail}")
    try:
        info = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("Docker returned invalid capacity metadata") from error
    cpus = info.get("NCPU") if isinstance(info, dict) else None
    memory_bytes = info.get("MemTotal") if isinstance(info, dict) else None
    if not isinstance(cpus, int) or cpus <= 0:
        raise RuntimeError("Docker did not report a positive CPU capacity")
    if not isinstance(memory_bytes, int) or memory_bytes <= 0:
        raise RuntimeError("Docker did not report a positive memory capacity")
    return {"cpus": cpus, "memory_mb": memory_bytes // (1024 * 1024)}


def docker_native_no_network_preflight(
    docker_command: str = "docker",
) -> dict[str, Any]:
    """Verify Docker's native isolated network namespace before a benchmark run."""
    result: dict[str, Any] = {
        "required": True,
        "mechanism": "docker-network-none",
        "image": TERMINAL_DOCKER_NETWORK_PROBE_IMAGE,
        "image_was_cached": False,
        "passed": False,
    }

    def invoke(arguments: list[str]) -> subprocess.CompletedProcess[str] | None:
        try:
            return subprocess.run(
                [docker_command, *arguments],
                check=False,
                capture_output=True,
                text=True,
                timeout=120,
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            result["error"] = str(error)
            return None

    inspected = invoke(["image", "inspect", TERMINAL_DOCKER_NETWORK_PROBE_IMAGE])
    if inspected is None:
        result["phase"] = "image-inspect"
        return result
    result["image_was_cached"] = inspected.returncode == 0
    if inspected.returncode != 0:
        pulled = invoke(["pull", TERMINAL_DOCKER_NETWORK_PROBE_IMAGE])
        if pulled is None or pulled.returncode != 0:
            result["phase"] = "image-pull"
            if pulled is not None:
                result["error"] = (
                    pulled.stderr.strip()
                    or pulled.stdout.strip()
                    or "Docker image pull failed"
                )
            return result

    probed = invoke(
        [
            "container",
            "run",
            "--rm",
            "--network",
            "none",
            TERMINAL_DOCKER_NETWORK_PROBE_IMAGE,
            "sh",
            "-c",
            TERMINAL_DOCKER_NONE_PROBE_SCRIPT,
        ]
    )
    if probed is None or probed.returncode != 0:
        result["phase"] = "native-no-network"
        if probed is not None:
            result["error"] = (
                probed.stderr.strip()
                or probed.stdout.strip()
                or "Docker native no-network probe failed"
            )
        return result
    result["phase"] = "complete"
    result["passed"] = True
    return result


def terminal_resource_preflight(
    manifest: dict[str, Any],
    task_root: Path,
    included_case_ids: list[str],
    capacity: dict[str, int],
) -> dict[str, Any]:
    """Validate declared task environments against local runtime capacity."""
    raw_cases = manifest.get("cases")
    if not isinstance(raw_cases, list):
        raise ValueError("terminal benchmark manifest has no cases")
    requested = set(included_case_ids)
    known = {
        case.get("id")
        for case in raw_cases
        if isinstance(case, dict) and isinstance(case.get("id"), str)
    }
    unknown = sorted(requested - known)
    if unknown:
        raise ValueError("unknown Terminal-Bench case ids: " + ", ".join(unknown))

    tasks: list[dict[str, Any]] = []
    violations: list[dict[str, Any]] = []
    for case in raw_cases:
        if not isinstance(case, dict) or not isinstance(case.get("id"), str):
            continue
        case_id = case["id"]
        if requested and case_id not in requested:
            continue
        source_path = case.get("source_path")
        if not isinstance(source_path, str) or not source_path:
            raise ValueError(f"manifest case {case_id!r} has no source_path")
        relative = Path(source_path)
        if relative.parts[:1] == ("tasks",):
            relative = Path(*relative.parts[1:])
        descriptor = (task_root / relative / "task.toml").resolve()
        if not descriptor.is_relative_to(task_root.resolve()):
            raise ValueError(f"manifest case {case_id!r} escapes the task root")
        with descriptor.open("rb") as stream:
            task_config = tomllib.load(stream)
        verifier_config = task_config.get("verifier")
        if not isinstance(verifier_config, dict):
            verifier_config = {}
        environments: list[dict[str, Any]] = []
        for stage, value in (
            ("agent", task_config.get("environment")),
            ("verifier", verifier_config.get("environment")),
        ):
            if not isinstance(value, dict):
                continue
            resources = {
                key: value.get(key)
                for key in ("cpus", "memory_mb", "storage_mb", "gpus")
                if isinstance(value.get(key), (int, float))
                and not isinstance(value.get(key), bool)
            }
            network_mode = value.get("network_mode")
            if network_mode is None and isinstance(value.get("allow_internet"), bool):
                network_mode = "public" if value["allow_internet"] else "no-network"
            environment = {"stage": stage, **resources}
            if isinstance(network_mode, str):
                environment["network_mode"] = network_mode
            environments.append(environment)
            for resource in ("cpus", "memory_mb"):
                amount = resources.get(resource)
                available = capacity.get(resource)
                if amount is not None and available is not None and amount > available:
                    violations.append(
                        {
                            "case_id": case_id,
                            "stage": stage,
                            "resource": resource,
                            "requested": amount,
                            "available": available,
                        }
                    )
        tasks.append(
            {
                "id": case_id,
                "verifier_environment_mode": verifier_config.get("environment_mode"),
                "environments": environments,
            }
        )
    requires_network_isolation = any(
        environment.get("network_mode") in {"no-network", "allowlist"}
        for task in tasks
        for environment in task["environments"]
    )
    return {
        "schema_version": 1,
        "runtime_capacity": capacity,
        "tasks": tasks,
        "requires_network_isolation": requires_network_isolation,
        "violations": violations,
        "passed": not violations,
    }


def terminal_native_no_network_incompatibilities(
    preflight: dict[str, Any],
) -> list[str]:
    """Return policies that the Docker-native provider cannot enforce safely."""
    incompatible: list[str] = []
    for task in preflight.get("tasks", []):
        if not isinstance(task, dict):
            continue
        for environment in task.get("environments", []):
            if not isinstance(environment, dict):
                continue
            mode = environment.get("network_mode")
            if mode not in {"no-network", "allowlist"}:
                continue
            if (
                environment.get("stage") != "verifier"
                or mode != "no-network"
                or task.get("verifier_environment_mode") != "separate"
            ):
                incompatible.append(
                    f"{task.get('id', '<unknown>')}:{environment.get('stage')}:{mode}"
                )
    return incompatible


def terminal_expected_artifacts(
    manifest: dict[str, Any], task_root: Path, included_case_ids: list[str]
) -> dict[str, list[str]]:
    """Load Harbor's structured artifact declarations for selected tasks."""
    requested = set(included_case_ids)
    result: dict[str, list[str]] = {}
    for case in manifest.get("cases", []):
        if not isinstance(case, dict) or not isinstance(case.get("id"), str):
            continue
        case_id = case["id"]
        if requested and case_id not in requested:
            continue
        source_path = case.get("source_path")
        if not isinstance(source_path, str) or not source_path:
            raise ValueError(f"manifest case {case_id!r} has no source_path")
        relative = Path(source_path)
        if relative.parts[:1] == ("tasks",):
            relative = Path(*relative.parts[1:])
        descriptor = (task_root / relative / "task.toml").resolve()
        if not descriptor.is_relative_to(task_root.resolve()):
            raise ValueError(f"manifest case {case_id!r} escapes the task root")
        with descriptor.open("rb") as stream:
            config = tomllib.load(stream)
        references: list[str] = []
        for artifact in config.get("artifacts", []):
            source = artifact if isinstance(artifact, str) else artifact.get("source") if isinstance(artifact, dict) else None
            if not isinstance(source, str) or not source.strip():
                raise ValueError(f"task {case_id!r} has an invalid artifact declaration")
            source = source.strip()
            if source not in references:
                references.append(source)
        result[case_id] = references
    unknown = sorted(requested - set(result))
    if unknown:
        raise ValueError("unknown Terminal-Bench case ids: " + ", ".join(unknown))
    return result


def run_longmemeval(args: argparse.Namespace) -> int:
    output = getattr(args, "run_directory", None) or (
        args.output / f"longmemeval-{run_stamp()}"
    )
    command = [
        sys.executable,
        str(BENCHMARK_ROOT / "longmemeval" / "run.py"),
        "--binary",
        str(args.binary),
        "--config",
        str(args.config),
        "--output",
        str(output),
        "--workers",
        str(args.concurrency),
        "--generation-timeout",
        str(args.timeout_seconds),
        "--mode",
        args.longmemeval_mode,
        "--architecture-max-iterations",
        str(args.architecture_max_iterations),
        "--transport-max-attempts",
        str(
            getattr(
                args,
                "longmemeval_transport_max_attempts",
                DEFAULT_LONGMEM_TRANSPORT_MAX_ATTEMPTS,
            )
        ),
        "--transport-retry-backoff-seconds",
        str(
            getattr(
                args,
                "longmemeval_transport_retry_backoff_seconds",
                DEFAULT_LONGMEM_TRANSPORT_RETRY_BACKOFF_SECONDS,
            )
        ),
    ]
    for case_id in args.case_id:
        command.extend(["--case", case_id])
    if args.judge_config:
        command.extend(["--judge-config", str(args.judge_config)])
    if args.judge_endpoint:
        command.extend(["--judge-endpoint", args.judge_endpoint])
    if args.judge_model:
        command.extend(["--judge-model", args.judge_model])
    if args.judge_api_key_env:
        command.extend(["--judge-api-key-env", args.judge_api_key_env])
    if args.dry_run:
        print(json.dumps({"benchmark": "longmemeval", "command": command}, indent=2))
        return 0
    env = os.environ.copy()
    env["PYTHONPATH"] = os.pathsep.join(
        part for part in (str(ROOT), env.get("PYTHONPATH", "")) if part
    )
    return run_with_boundary_interrupt(command, cwd=ROOT, env=env)


def run_terminal_bench(args: argparse.Namespace) -> int:
    manifest = load_manifest("terminal-bench")
    task_root = BENCHMARK_ROOT / "terminal-bench" / "tasks"
    output = getattr(args, "run_directory", None) or (
        args.output / f"terminal-bench-{run_stamp()}"
    )
    output.mkdir(parents=True, exist_ok=True)
    expected_artifacts = terminal_expected_artifacts(
        manifest,
        BENCHMARK_ROOT / "terminal-bench" / "tasks",
        args.case_id,
    )
    if not args.dry_run and args.binary.read_bytes()[:4] != b"\x7fELF":
        raise RuntimeError("Terminal-Bench requires a Linux ELF Nola binary")
    preflight = None
    if not args.dry_run and not args.skip_runtime_preflight:
        preflight = terminal_resource_preflight(
            manifest,
            task_root,
            args.case_id,
            docker_capacity(args.docker),
        )
        if preflight["passed"] and preflight["requires_network_isolation"]:
            incompatible = terminal_native_no_network_incompatibilities(preflight)
            native_probe = (
                {
                    "required": True,
                    "mechanism": "docker-network-none",
                    "passed": False,
                    "phase": "policy-shape",
                    "error": "unsupported network policies: " + ", ".join(incompatible),
                }
                if incompatible
                else docker_native_no_network_preflight(args.docker)
            )
            preflight["network_isolation"] = native_probe
            if native_probe["passed"]:
                native_probe["provider"] = (
                    "evals.harbor_native_no_network:"
                    "DockerNativeNoNetworkEnvironment"
                )
        (output / "preflight.json").write_text(
            json.dumps(preflight, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )
        if not preflight["passed"]:
            details = "; ".join(
                f"{item['case_id']} {item['stage']} {item['resource']} "
                f"requires {item['requested']}, runtime has {item['available']}"
                for item in preflight["violations"]
            )
            raise RuntimeError("Terminal-Bench resource preflight failed: " + details)
        network_isolation = preflight.get("network_isolation")
        if isinstance(network_isolation, dict) and not network_isolation.get("passed"):
            phase = network_isolation.get("phase", "unknown")
            detail = network_isolation.get("error", "unknown error")
            raise RuntimeError(
                "Terminal-Bench network-isolation preflight failed during "
                f"{phase}: {detail}"
            )
    command = [
        args.harbor,
        "run",
        "--path",
        str(task_root),
        "--env",
        "evals.harbor_native_no_network:DockerNativeNoNetworkEnvironment",
        "--agent",
        "evals.harbor_nola_agent:NolaHarborAgent",
        "--ak",
        f"binary_path={args.binary}",
        "--ak",
        f"config_path={args.config}",
        "--ak",
        f"timeout_seconds={args.timeout_seconds}",
        "--ak",
        f"max_iterations={args.max_iterations}",
        "--ak",
        f"tool_set={args.tool_set}",
        "--ak",
        f"shell_purpose_contract={getattr(args, 'shell_purpose_contract', 'required')}",
        "--ak",
        "expected_artifacts_by_task_json=" + json.dumps(
            expected_artifacts, ensure_ascii=False, separators=(",", ":")
        ),
        "--jobs-dir",
        str(output),
        "--n-concurrent",
        str(args.concurrency),
        "--yes",
    ]
    for case_id in args.case_id:
        command.extend(["--include-task-name", case_id])
    if getattr(args, "task_acceptance", None):
        # Explicit public-check configuration only; never derive this from a
        # benchmark verifier or artifact-collection declaration.
        sys.path.insert(0, str(ROOT))
        from evals.nola_task_acceptance import contract
        checks = json.loads(args.task_acceptance.read_text(encoding="utf-8"))
        if not isinstance(checks, dict):
            raise ValueError("task acceptance file must contain a task-name map")
        checks = {key: contract(value) for key, value in checks.items()}
        command.extend(["--ak", "task_acceptance_by_task_json=" + json.dumps(checks)])
    if args.dry_run:
        command.append("--print-config")
    metadata = {
        "schema_version": 1,
        "benchmark": "terminal-bench",
        "variant": manifest["variant"],
        "command": command,
        "resource_preflight": preflight,
        "expected_artifacts_by_task": expected_artifacts,
    }
    (output / "invocation.json").write_text(
        json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    env = os.environ.copy()
    env["PYTHONPATH"] = os.pathsep.join(
        part for part in (str(ROOT), env.get("PYTHONPATH", "")) if part
    )
    return subprocess.run(command, cwd=ROOT, env=env, check=False).returncode


def run_stamp() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def resolve_iteration_budgets(args: argparse.Namespace) -> None:
    """Validate benchmark-scoped iteration flags and install their defaults."""
    purpose = getattr(args, "shell_purpose_contract", None)
    if args.benchmark != "terminal-bench" and (purpose is not None or getattr(args, "task_acceptance", None)):
        raise SystemExit("--shell-purpose-contract and --task-acceptance apply only to Terminal-Bench")
    args.shell_purpose_contract = purpose or "required"
    terminal_value = args.max_iterations
    architecture_value = args.architecture_max_iterations
    transport_attempts = args.longmemeval_transport_max_attempts
    transport_backoff = args.longmemeval_transport_retry_backoff_seconds
    if terminal_value is not None and args.benchmark != "terminal-bench":
        raise SystemExit(
            "--max-iterations applies only to "
            "Terminal-Bench; use --architecture-max-iterations for "
            "LongMemEval memory-architecture runs"
        )
    if architecture_value is not None and (
        args.benchmark != "longmemeval"
        or args.longmemeval_mode != "memory-architecture"
    ):
        raise SystemExit(
            "--architecture-max-iterations applies only to LongMemEval "
            "--longmemeval-mode memory-architecture"
        )
    if (transport_attempts is not None or transport_backoff is not None) and (
        args.benchmark != "longmemeval"
        or args.longmemeval_mode != "memory-architecture"
    ):
        raise SystemExit(
            "--longmemeval-transport-* applies only to LongMemEval "
            "--longmemeval-mode memory-architecture"
        )
    args.max_iterations = (
        DEFAULT_TERMINAL_MAX_ITERATIONS
        if terminal_value is None
        else terminal_value
    )
    args.architecture_max_iterations = (
        DEFAULT_LONGMEM_ARCHITECTURE_MAX_ITERATIONS
        if architecture_value is None
        else architecture_value
    )
    args.longmemeval_transport_max_attempts = (
        DEFAULT_LONGMEM_TRANSPORT_MAX_ATTEMPTS
        if transport_attempts is None
        else transport_attempts
    )
    args.longmemeval_transport_retry_backoff_seconds = (
        DEFAULT_LONGMEM_TRANSPORT_RETRY_BACKOFF_SECONDS
        if transport_backoff is None
        else transport_backoff
    )
    if args.max_iterations < 0:
        raise SystemExit("--max-iterations cannot be negative")
    if args.architecture_max_iterations < 1:
        raise SystemExit("--architecture-max-iterations must be positive")
    if args.longmemeval_transport_max_attempts < 1:
        raise SystemExit("--longmemeval-transport-max-attempts must be positive")
    if args.longmemeval_transport_retry_backoff_seconds < 0:
        raise SystemExit(
            "--longmemeval-transport-retry-backoff-seconds cannot be negative"
        )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("benchmark", choices=("longmemeval", "terminal-bench"))
    parser.add_argument(
        "--binary", type=Path, default=ROOT / "target" / "release" / "nola"
    )
    parser.add_argument(
        "--config", type=Path, default=Path.home() / ".nola" / "config.json"
    )
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument(
        "--run-directory",
        type=Path,
        help="exact run directory; reuse it to resume retained per-case evidence",
    )
    parser.add_argument("--case-id", action="append", default=[])
    parser.add_argument(
        "--selection-file",
        type=Path,
        help="eval-only JSON campaign file containing case IDs for each benchmark",
    )
    parser.add_argument("--timeout-seconds", type=float, default=600)
    parser.add_argument(
        "--max-iterations",
        dest="max_iterations",
        type=int,
        default=None,
        help=(
            "Terminal-Bench Core model-iteration budget; zero is unbounded "
            f"(default: {DEFAULT_TERMINAL_MAX_ITERATIONS})"
        ),
    )
    parser.add_argument("--concurrency", type=int, default=1)
    parser.add_argument(
        "--longmemeval-mode",
        choices=("full-history", "memory-architecture"),
        default="full-history",
        help="LongMemEval execution path; memory-architecture persists each source session",
    )
    parser.add_argument(
        "--architecture-max-iterations",
        type=int,
        default=None,
        help=(
            "LongMemEval memory-architecture budget for each import and query "
            f"turn (default: {DEFAULT_LONGMEM_ARCHITECTURE_MAX_ITERATIONS})"
        ),
    )
    parser.add_argument(
        "--longmemeval-transport-max-attempts",
        type=int,
        default=None,
        help="bounded attempts for pre-response provider transport failures (default: 3)",
    )
    parser.add_argument(
        "--longmemeval-transport-retry-backoff-seconds",
        type=float,
        default=None,
        help="base linear backoff for LongMemEval provider transport retries (default: 1.0)",
    )
    parser.add_argument(
        "--tool-set",
        choices=("auto", "all", "write"),
        default="auto",
        help="CLI capability exposure policy for Terminal-Bench",
    )
    parser.add_argument("--harbor", default="harbor")
    parser.add_argument("--shell-purpose-contract", choices=("required", "optional"), default=None,
                        help="paired shell schema ablation; omission never grants validation credit")
    parser.add_argument("--task-acceptance", type=Path,
                        help="explicit task-name map of public argv/cwd/timeout_seconds/public checks; not hidden scoring")
    parser.add_argument("--docker", default="docker")
    parser.add_argument(
        "--skip-runtime-preflight",
        action="store_true",
        help="skip local Docker capacity and capability checks for managed runtimes",
    )
    parser.add_argument("--judge-config", type=Path)
    parser.add_argument("--judge-endpoint")
    parser.add_argument("--judge-model")
    parser.add_argument("--judge-api-key-env")
    parser.add_argument("--dry-run", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    resolve_iteration_budgets(args)
    subprocess.run(
        [sys.executable, str(ROOT / "scripts" / "validate_external_benchmarks.py")],
        cwd=ROOT,
        check=True,
    )
    assert_no_benchmark_specialization()
    args.binary = args.binary.expanduser().resolve()
    args.config = args.config.expanduser().resolve()
    if args.judge_config:
        args.judge_config = args.judge_config.expanduser().resolve()
    if args.selection_file:
        args.selection_file = args.selection_file.expanduser().resolve()
        if args.case_id:
            raise SystemExit("--selection-file and --case-id cannot be combined")
        try:
            selection = load_selection(args.selection_file, args.benchmark)
        except (OSError, UnicodeError, json.JSONDecodeError, ValueError) as error:
            raise SystemExit(f"invalid benchmark selection: {error}") from error
        args.case_id = selection["case_ids"]
        required_mode = selection["required_mode"]
        if (
            required_mode is not None
            and args.benchmark == "longmemeval"
            and args.longmemeval_mode != required_mode
        ):
            raise SystemExit(f"selection requires --longmemeval-mode {required_mode}")
        try:
            validate_case_ids(load_manifest(args.benchmark), args.case_id)
        except ValueError as error:
            raise SystemExit(f"invalid benchmark selection: {error}") from error
    args.output = args.output.expanduser().resolve()
    if args.run_directory:
        args.run_directory = args.run_directory.expanduser().resolve()
    if not args.binary.is_file():
        raise SystemExit(f"Nola binary is missing: {args.binary}")
    if not args.config.is_file():
        raise SystemExit(f"Nola config is missing: {args.config}")
    if args.benchmark == "longmemeval":
        return run_longmemeval(args)
    return run_terminal_bench(args)


if __name__ == "__main__":
    raise SystemExit(main())
