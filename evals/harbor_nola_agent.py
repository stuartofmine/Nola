#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Harbor custom-agent adapter for a prebuilt Linux Nola CLI binary."""

from __future__ import annotations

import json
import shlex
import tempfile
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

from harbor.agents.base import BaseAgent
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext
from evals.nola_task_acceptance import contract as acceptance_contract


def _load_config(path: Path) -> dict[str, Any]:
    """Parse Nola's JSON/JSONC config without copying unrelated settings."""
    source = path.read_text(encoding="utf-8")
    try:
        return json.loads(source)
    except json.JSONDecodeError:
        # Nola accepts JSONC. Strip comments with a small string-aware scanner;
        # URLs and comment-like text inside quoted credentials remain intact.
        output: list[str] = []
        index = 0
        in_string = False
        escaped = False
        while index < len(source):
            char = source[index]
            following = source[index + 1] if index + 1 < len(source) else ""
            if in_string:
                output.append(char)
                if escaped:
                    escaped = False
                elif char == "\\":
                    escaped = True
                elif char == '"':
                    in_string = False
                index += 1
            elif char == '"':
                in_string = True
                output.append(char)
                index += 1
            elif char == "/" and following == "/":
                index += 2
                while index < len(source) and source[index] not in "\r\n":
                    index += 1
            elif char == "/" and following == "*":
                end = source.find("*/", index + 2)
                if end < 0:
                    raise ValueError(f"unterminated JSONC comment in {path}")
                index = end + 2
            else:
                output.append(char)
                index += 1
        uncommented = "".join(output)
        output = []
        index = 0
        in_string = False
        escaped = False
        while index < len(uncommented):
            char = uncommented[index]
            if in_string:
                output.append(char)
                if escaped:
                    escaped = False
                elif char == "\\":
                    escaped = True
                elif char == '"':
                    in_string = False
                index += 1
                continue
            if char == '"':
                in_string = True
            if char == ",":
                following = index + 1
                while following < len(uncommented) and uncommented[following].isspace():
                    following += 1
                if following < len(uncommented) and uncommented[following] in "}]":
                    index += 1
                    continue
            output.append(char)
            index += 1
        return json.loads("".join(output))


def _parse_expected_artifacts(
    raw: str | dict[str, object],
) -> dict[str, list[str]]:
    payload = json.loads(raw) if isinstance(raw, str) else raw
    if not isinstance(payload, dict):
        raise ValueError("expected_artifacts_by_task_json must encode an object")
    result: dict[str, list[str]] = {}
    for task_name, references in payload.items():
        if not isinstance(task_name, str) or not task_name or not isinstance(references, list):
            raise ValueError("expected artifact map must contain task-name string-list pairs")
        normalized: list[str] = []
        for reference in references:
            if not isinstance(reference, str) or not reference.strip():
                raise ValueError(f"task {task_name!r} has an invalid artifact reference")
            reference = reference.strip()
            if reference not in normalized:
                normalized.append(reference)
        result[task_name] = normalized
    return result


def _task_name_from_session_id(session_id: str) -> str | None:
    """Recover Harbor's task identity without inspecting task instructions."""
    value = session_id.removesuffix("__agent")
    task_name, separator, trial_suffix = value.rpartition("__")
    if not separator or not task_name or not trial_suffix:
        return None
    return task_name


class NolaHarborAgent(BaseAgent):
    """Upload Nola into each Harbor task and run the native CLI tool loop."""

    def __init__(
        self,
        *args: Any,
        binary_path: str,
        config_path: str,
        timeout_seconds: str | float = 600,
        max_iterations: str | int = 64,
        tool_set: str = "auto",
        expected_artifacts_by_task_json: str | dict[str, object] = "{}",
        shell_purpose_contract: str = "required",
        task_acceptance_by_task_json: str | dict[str, object] = "{}",
        **kwargs: Any,
    ) -> None:
        super().__init__(*args, **kwargs)
        self.binary_path = Path(binary_path).expanduser().resolve()
        self.config_path = Path(config_path).expanduser().resolve()
        self.timeout_seconds = float(timeout_seconds)
        self.max_iterations = int(max_iterations)
        if self.timeout_seconds <= 0:
            raise ValueError("timeout_seconds must be positive")
        if self.max_iterations < 0:
            raise ValueError("max_iterations cannot be negative")
        self.tool_set = str(tool_set).strip().lower()
        if shell_purpose_contract not in {"required", "optional"}:
            raise ValueError("shell_purpose_contract must be required or optional")
        self.shell_purpose_contract = shell_purpose_contract
        acceptance = (json.loads(task_acceptance_by_task_json)
                      if isinstance(task_acceptance_by_task_json, str) else task_acceptance_by_task_json)
        if not isinstance(acceptance, dict) or any(not isinstance(key, str) or not key for key in acceptance):
            raise ValueError("task_acceptance_by_task_json must be a task-name map")
        self.task_acceptance_by_task = {key: acceptance_contract(value) for key, value in acceptance.items()}
        if self.tool_set not in {"auto", "all", "write"}:
            raise ValueError("tool_set must be one of: auto, all, write")
        self.expected_artifacts_by_task = _parse_expected_artifacts(
            expected_artifacts_by_task_json
        )
        if not self.binary_path.is_file():
            raise FileNotFoundError(f"Nola Linux binary is missing: {self.binary_path}")
        if not self.config_path.is_file():
            raise FileNotFoundError(f"Nola config is missing: {self.config_path}")
        source_config = _load_config(self.config_path)
        llm = source_config.get("llm") or {}
        selected = llm.get("default_model") or {}
        connection = next((item for item in llm.get("connections", [])
                           if item.get("id") == selected.get("connection")), None)
        if connection is None or not any(item.get("id") == selected.get("model")
                                         for item in connection.get("models", [])):
            raise ValueError("Nola config requires a declared default_model and connection")
        self.model_name = str(selected["model"])
        self._init_model_info()
        self.endpoint_host = urlparse(str(connection["endpoint"])).hostname or "unknown"
        self.network_policy = str(
            source_config.get("network_policy") or "allow"
        ).lower()
        if self.network_policy not in {"allow", "ask", "deny"}:
            raise ValueError(f"unsupported network_policy: {self.network_policy}")
        self.minimal_config = {
            "llm": dict(llm),
            "network_policy": self.network_policy,
            "max_iterations": self.max_iterations,
            # Host-owned scope, identical for every task; not inferred from
            # task wording, root identity, or a container-detection heuristic.
            "instructions": [
                "Execution scope: this is an isolated, disposable task environment. "
                "Installing task-required dependencies inside this environment is authorized, "
                "including its system packages when the effective user permits it. "
                "Prefer project-local isolation where practical. This does not authorize "
                "host changes, privileged escape, access to verifier internals, or bypassing "
                "network/permission restrictions. Dependency discovery, installation and "
                "functional verification consume the same task time budget. Verify the "
                "required capability after installation and retry the blocked operation.",
                "Performance validation: derive representative unseen local inputs from public "
                "requirements; separate workload populations rather than mixing favorable and "
                "unfavorable cases. Time baseline and candidate with symmetric warmup, repetition "
                "and setup boundaries, using fresh independent objects for cold-input claims. "
                "Report per-case ratios and the requested aggregation, including weak cases and "
                "variance. A local self-test is not an independent official-gate pass. Never "
                "inspect hidden cases or tune against verifier feedback during the scored run."
            ],
        }
        # Preserve the evaluated model contract (vision, sampling and output
        # budgets), without importing unrelated user permissions or services.
        if "model_capabilities" in source_config:
            self.minimal_config["model_capabilities"] = source_config["model_capabilities"]

    @staticmethod
    def name() -> str:
        return "nola"

    def version(self) -> str:
        return "architecture-v2"

    async def setup(self, environment: BaseEnvironment) -> None:
        prepared = await environment.exec(
            command="command -v python3 >/dev/null && mkdir -p /opt/nola /tmp/nola-home/.nola",
            user="root",
        )
        if prepared.return_code != 0:
            raise RuntimeError("Nola evaluation requires Python 3 for its process supervisor")
        # CodeMemory requires a real repository HEAD. This is a pristine task
        # snapshot, not a mount of the developer's repository or verifier.
        baseline = await environment.exec(
            command=("((command -v git >/dev/null && command -v ps >/dev/null) || "
                     "(if command -v apt-get >/dev/null; then apt-get update -qq && "
                     "apt-get install -y -qq --no-install-recommends git procps; "
                     "elif command -v dnf >/dev/null; then dnf install -y git procps-ng; "
                     "else echo 'git and ps are required for the Nola runtime' >&2; exit 1; fi)) && "
                     "(git -C /app rev-parse --verify HEAD >/dev/null 2>&1 || "
                     "(git -C /app init -q && git -C /app add . && "
                     "git -C /app -c user.name=Nola-Evaluation "
                     "-c user.email=evaluation@localhost commit -q --allow-empty -m 'Task input baseline'))"),
            user="root",
        )
        if baseline.return_code != 0:
            raise RuntimeError("Nola evaluation could not establish the task source baseline")
        await environment.upload_file(self.binary_path, "/opt/nola/nola")
        await environment.upload_file(Path(__file__).with_name("nola_task_acceptance.py"), "/opt/nola/acceptance.py")
        await environment.upload_file(
            Path(__file__).with_name('nola_agent_supervisor.py'),
            '/opt/nola/supervisor.py',
        )
        await environment.upload_file(
            Path(__file__).with_name('export_nola_evidence.py'),
            '/opt/nola/export_evidence.py',
        )
        with tempfile.NamedTemporaryFile(
            "w", encoding="utf-8", delete=False
        ) as config_file:
            json.dump(self.minimal_config, config_file)
            config_path = Path(config_file.name)
        config_path.chmod(0o600)
        try:
            await environment.upload_file(
                config_path, "/tmp/nola-home/.nola/config.json"
            )
        finally:
            config_path.unlink(missing_ok=True)
        user = shlex.quote(str(environment.default_user or "root"))
        await environment.exec(
            command=(
                "chmod 755 /opt/nola/nola && "
                + (
                    "chmod 600 /tmp/nola-home/.nola/config.json && "
                    f"chown -R {user} /tmp/nola-home"
                )
            ),
            user="root",
        )

        applied = await environment.exec(
            command=("HOME=/tmp/nola-home NOLA_DB_PATH=/tmp/nola-eval.db "
                     "NOLA_BENCHMARK_TOKEN=relay-only NOLA_PERMISSION_PROFILE=allow_all "
                     "/opt/nola/nola --project /app model apply-config "
                     "/tmp/nola-home/.nola/config.json"),
        )
        if applied.return_code != 0:
            raise RuntimeError("Nola model configuration could not be applied")

    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        task_name = _task_name_from_session_id(str(self.session_id))
        acceptance = getattr(self, "task_acceptance_by_task", {}).get(task_name or "")
        purpose_contract = getattr(self, "shell_purpose_contract", "required")
        agent_command = ("/opt/nola/nola --project /app --json --lang en --yes "
                         f"--network-policy {self.network_policy} "
                         f"--force-tool-set {shlex.quote(self.tool_set)}")
        if acceptance is not None:
            with tempfile.NamedTemporaryFile("w", encoding="utf-8", delete=False) as check_file:
                json.dump(acceptance, check_file)
                check_path = Path(check_file.name)
            try:
                await environment.upload_file(check_path, "/opt/nola/public-acceptance.json")
            finally:
                check_path.unlink(missing_ok=True)
            agent_command = ("python3 /opt/nola/acceptance.py --contract /opt/nola/public-acceptance.json "
                "--report /logs/agent/task-acceptance.json "
                f"--seconds {self.timeout_seconds:g} -- {agent_command} --session nola-eval-acceptance")
        requested_artifacts = self.expected_artifacts_by_task.get(task_name or "", [])
        # Verifier transport metadata is not a user mutation contract. A
        # missing path may be generated by a collection hook (e.g. a diff),
        # and a directory may contain inputs. Only the original instruction
        # supplies task requirements to Nola; Harbor owns artifact collection.
        context.metadata = {
            **(context.metadata or {}),
            "nola_invoked": True,
            "nola_finished": False,
            "nola_event_log": "/logs/agent/nola.ndjson.gz",
            "nola_native_code_memory_available": True,
            "nola_endpoint_host": self.endpoint_host,
            "nola_configured_model": self.model_name,
            "nola_permission_profile": "allow_all",
            "nola_requested_artifacts": requested_artifacts,
            "nola_shell_purpose_contract": purpose_contract,
            "nola_task_acceptance": "configured_public_check" if acceptance else "not_configured",
            "nola_acceptance_report": "/logs/agent/task-acceptance.json" if acceptance else None,
        }
        with tempfile.NamedTemporaryFile("w", encoding="utf-8", delete=False) as prompt:
            prompt.write(instruction)
            prompt_path = Path(prompt.name)
        try:
            await environment.upload_file(prompt_path, "/tmp/nola-eval-instruction.txt")
        finally:
            prompt_path.unlink(missing_ok=True)
        try:
            try:
                result = await environment.exec(
                    command=(
                        "HOME=/tmp/nola-home "
                        + (
                            "NOLA_DB_PATH=/tmp/nola-eval.db "
                            "NOLA_BENCHMARK_TOKEN=relay-only NOLA_PERMISSION_PROFILE=allow_all "
                            "NOLA_SKIP_ONBOARDING=1 "
                            "NOLA_EVAL_REQUEST_AUDIT_DIR=/logs/agent/model-requests "
                            f"NOLA_SHELL_PURPOSE_CONTRACT={purpose_contract} "
                            # Bound the process inside the container, not the
                            # Docker client's connection. Clean up the owned
                            # group even if its leader exits before children.
                            f"python3 /opt/nola/supervisor.py --seconds {self.timeout_seconds:g} -- "
                            f"{agent_command}"
                            " "
                            "< /tmp/nola-eval-instruction.txt "
                            "> /logs/agent/nola.ndjson 2>&1"
                        )
                    ),
                    timeout_sec=self.timeout_seconds + 15,
                )
            except BaseException:
                context.metadata = {
                    **(context.metadata or {}),
                    # Transport loss does not prove the agent never ran.
                    "nola_execution_observed": None,
                }
                raise
        finally:
            evidence_exported = False
            try:
                exported = await environment.exec(
                    command=('python3 /opt/nola/export_evidence.py /tmp/nola-eval.db '
                             '/logs/agent/nola-evidence.json /logs/agent/nola.ndjson '
                             '/tmp/nola-home/.nola/config.json')
                )
                evidence_exported = exported.return_code == 0
            except Exception:
                # Diagnostic export failure must not hide execution failure or
                # prevent credential cleanup and the independent verifier.
                pass
            context.metadata = {
                **(context.metadata or {}),
                'nola_evidence_exported': evidence_exported,
                'nola_session_retention': 'isolated_persistent',
            }
            await environment.exec(
                command=(
                    "if [ -f /logs/agent/nola.ndjson ]; then "
                    "gzip -f /logs/agent/nola.ndjson; fi; "
                    "rm -f /tmp/nola-home/.nola/config.json "
                    "/tmp/nola-eval-instruction.txt"
                )
            )
        context.metadata = {
            **(context.metadata or {}),
            "nola_exit_code": result.return_code if acceptance is None else None,
            "nola_harness_exit_code": result.return_code,
            "nola_finished": True,
            "nola_execution_observed": True,
            # A zero process exit only says that the CLI shut down cleanly. It
            # is not evidence that the requested artifact or behavior was
            # completed; the canonical run state and verifier own that claim.
            "nola_process_succeeded": result.return_code == 0 if acceptance is None else None,
            "nola_timed_out": result.return_code == 124,
            "nola_forced_termination": result.return_code == 137,
            "nola_event_log": "/logs/agent/nola.ndjson.gz",
        }
        # Terminal-Bench scores the resulting filesystem. A bounded or failed
        # agent can still leave a valid or partially valid artifact, so retain
        # its exact exit status above and always let the official verifier make
        # the task-level decision.
