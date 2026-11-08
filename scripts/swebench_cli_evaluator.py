#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Run Nola CLI against SWE-bench Verified instances and collect predictions.

This script focuses on boundary testing for the current CLI agent:
1) load SWE-bench Verified instances,
2) run Nola CLI on each instance repo at base_commit,
3) capture produced patch + trajectory signals,
4) optionally call the official SWE-bench harness for unit-test evaluation.
"""

from __future__ import annotations

import argparse
import ast
import csv
import dataclasses
import json
import os
import shutil
import subprocess
import sys
import re
import time
import socket
from urllib.parse import urlparse
from functools import lru_cache
from pathlib import Path
from typing import Any


DEFAULT_TEST_BASH_PREFIXES = [
    "awk",
    "cat",
    "cd",
    "chmod",
    "cp",
    "echo",
    "env",
    "find",
    "git",
    "grep",
    "head",
    "ls",
    "mkdir",
    "mv",
    "pip",
    "pip3",
    "pwd",
    "python",
    "python3",
    "pytest",
    "tox",
    "nox",
    "make",
    "npm",
    "pnpm",
    "yarn",
    "node",
    "cargo",
    "go",
    "gradle",
    "./gradlew",
    "mvn",
    "poetry",
    "uv",
    "ruff",
    "bash",
    "sh",
    "source",
    "sed",
    "sort",
    "tail",
    "tee",
    "test",
    "touch",
    "uniq",
    "wc",
    "which",
    "xargs",
]

_LOCAL_TEST_MAX_INIT = 20
_LOCAL_TEST_MAX_REFINE = 10
_LOCAL_TEST_MAX_FAIL_PER_ATTEMPT = 10
_LOCAL_TEST_REGRESSION_TEST_LIMIT = 3
_LOCAL_TEST_ERROR_MARKERS = (
    "Traceback (most recent call last)",
    "FAILED",
    "ERROR",
    "E   ",
    "TypeError",
    "AssertionError",
    "ValueError",
)
_LOCAL_TEST_SOURCE_ERROR_MARKERS = (
    "indentationerror:",
    "syntaxerror:",
    "taberror:",
)

_LOCAL_TEST_INCONCLUSIVE_MARKERS = [
    "while loading conftest",
    "error while parsing the following warning configuration",
    "parsing the following warning configuration",
    "importerror while loading conftest",
    "file or directory not found",
    "no such file or directory",
    "does not exist",
    "you appear to be trying to import astropy from within a source checkout",
    "extension modules first",
    "pip install -e .",
    "module not found",
    "modulenotfounderror",
    "setuptools_scm",
    "error collecting",
    "py_compile.py",
    "cannot import name",
    "could not determine astropy package version",
    "command failed with exit code 4",
]
_LLM_REQUEST_ERROR_MARKERS = (
    "llm 请求失败",
    "llm request failed",
    "request failed: error trying to connect",
    "error trying to connect",
    "error sending request for url",
    "dns error",
    "could not resolve host",
    "nodename nor servname provided",
    "operation not permitted",
    "network is unreachable",
    "no route to host",
    "connection was refused",
    "connection refused",
    "connection reset by peer",
    "connection timed out",
    "timed out",
    "failed to connect",
    "permission denied",
    "operation is not permitted",
    "读取 llm 流失败",
    "stream idle timeout",
    "stream first event timeout",
)
_LLM_NETWORK_ENV_ERROR_MARKERS = (
    "operation not permitted",
    "execution environment blocked outbound network",
    "runtime cannot reach the api endpoint",
    "runtime cannot reach",
    "network is unreachable",
    "no route to host",
    "os error 1",
    "could not connect to",
    "couldn't connect",
    "permission denied",
)

_LLM_DNS_FALLBACK_ENV_NAMES = (
    "NOLA_LLM_DNS_FALLBACKS",
    "NOLA_CLI_DNS_FALLBACKS",
)
_DEFAULT_DNS_FALLBACKS = (
    ("api.deepseek.com", "119.188.44.230"),
)


def _fallback_tooling_model(model_name: str, endpoint: str) -> str | None:
    endpoint_lc = endpoint.lower()
    model_lc = model_name.lower()
    if "api.deepseek.com" in endpoint_lc and (
        "deepseek-v4" in model_lc or "deepseek-r1" in model_lc
    ):
        return "deepseek-chat"
    return None


_LOCAL_TEST_COLLECTION_ERROR_MARKERS = (
    "collected 0 items",
    "collected 0 tests",
    "collection failed",
    "errors during collection",
    "error collecting",
)
_LOCAL_TEST_PLACEHOLDER_FAILURES = {
    "<execution_error>",
    "<execution_inconclusive>",
    "<execution_timeout>",
    "<unsupported_local_test_format>",
}
_LOCAL_TEST_KEEP_PATCH_INCONCLUSIVE_TYPES = {
    "environment",
    "no_tests_collected",
    "collection_error",
    "test_name_mismatch",
    "timeout",
    "unsupported_local_test_format",
}
_FAILED_TEST_PATTERNS = [
    re.compile(r"(?im)^\s*\[[^]]+\]\s*(?:FAILED|ERROR)\s+(.+)$"),
    re.compile(r"(?im)^\s*(?:FAILED|ERROR)\s+(.+)$"),
    re.compile(r"(?im)^\s*(?:FAILED|ERROR)\s+([^\n\r]+?)(?:\s+-\s+[^-\n\r]+)?$"),
    re.compile(r"(?im)^([^\n\r:]+(?:\.[A-Za-z0-9_]+)*(?:::[A-Za-z0-9_]+)+\.[A-Za-z0-9_]+\[?[^\s\]]*\]?)"),
    re.compile(r"(?im)^(?:\w+[-\w]*\s+)?(?:(?:FAILED|ERROR):\s*)?(.+::.+)$"),
]
_FAILED_LINE_PREFIX_RE = re.compile(r"^\s*(?:\[[^]]+\]\s*)?(?:FAILED|ERROR)\s+")
_LOCAL_TEST_NOT_FOUND_RE = re.compile(
    r"(?im)^(?:error:|ERROR:)\s*not\s+found:\s*(.+?)(?:\s+in\s+any\s+of\b.*)?$"
)
_LOCAL_TEST_NOT_FOUND_SUMMARY_RE = re.compile(
    r'(?im)^.*(?:Couldn\'t|Could not)\s+find\s+(?:test|module|file)\s+(?:the\s+)?\"?([^\n\r\"]+?)\"?$'
)
_PYTEST_TEST_CACHE: dict[tuple[str, str], set[str]] = {}

_SWEBENCH_WRITE_TOOLS = {
    "write_file",
    "notebook_edit",
    "move_file",
    "delete_file",
    "replace_symbol_body",
    "edit_file",
    "edit_file_lines",
    "multi_edit_file",
    "replace_in_files",
    "rename_symbol",
    "apply_patch",
}
_SWEBENCH_READ_TOOLS = {
    "context_pack",
    "project_overview",
    "instruction_context",
    "skill_context",
    "agent_context",
    "command_context",
    "agent_config_context",
    "coding_conventions",
    "file_tree",
    "repo_map",
    "code_search",
    "workspace_symbols",
    "find_up",
    "find_files",
    "glob",
    "grep",
    "list_dir",
    "file_outline",
    "read_symbol",
    "read_file",
    "read_many_files",
    "notebook_read",
    "code_navigation",
    "impact_analysis",
    "call_graph",
    "related_files",
    "file_dependencies",
    "symbol_search",
    "implementation_search",
    "workspace_status",
    "git_diff",
    "git_history",
    "git_show",
    "git_blame",
    "changed_symbols",
}


def _classify_swe_tool(name: str) -> str:
    normalized = str(name or "").strip()
    if normalized in _SWEBENCH_WRITE_TOOLS:
        return "write"
    if normalized in _SWEBENCH_READ_TOOLS:
        return "read"
    return "other"


def _normalize_repo_path(instance_repo: Path, candidate: str) -> str:
    value = _normalize_test_name(candidate)
    if not value:
        return ""
    repo_root = str(instance_repo.resolve())
    if value.startswith(repo_root):
        relative = value[len(repo_root) :].lstrip("/\\")
        return relative
    return value


def _local_swebench_cache_root() -> Path:
    return Path(os.environ.get("HF_HOME", str(Path.home() / ".cache" / "huggingface"))).expanduser() / "datasets"


@lru_cache(maxsize=2)
def _find_local_swebench_arrow_path(dataset_name: str, split: str) -> Path | None:
    if split != "test":
        return None
    if dataset_name.lower() != "swe-bench/swe-bench_verified":
        return None

    cache_root = _local_swebench_cache_root()
    if not cache_root.is_dir():
        return None

    base_root = cache_root / "SWE-bench___parquet"
    if not base_root.is_dir():
        return None

    for arrow_path in sorted(base_root.rglob("parquet-test.arrow")):
        if "SWE-bench--SWE-bench_Verified" in str(arrow_path):
            return arrow_path
    return None


def _collect_module_test_ids(instance_repo: Path, module_path: str) -> set[str]:
    module_path = module_path.strip()
    if not module_path:
        return set()
    module_path_as_arg = module_path
    abs_module = Path(module_path)
    if not abs_module.is_absolute():
        abs_module = instance_repo / abs_module
    if not abs_module.exists():
        return set()

    key = (str(instance_repo.resolve()), str(abs_module.resolve()))
    if key in _PYTEST_TEST_CACHE:
        return _PYTEST_TEST_CACHE[key]

    proc = _run(
        [
            "python",
            "-m",
            "pytest",
            "-q",
            "--disable-warnings",
            "-c",
            "/dev/null",
            "--collect-only",
            module_path_as_arg,
        ],
        cwd=instance_repo,
        timeout_sec=120,
    )
    output = _coerce_text(proc.stdout) + "\n" + _coerce_text(proc.stderr)
    found: set[str] = set()
    for raw in output.splitlines():
        line = raw.strip()
        if not line.startswith("::"):
            continue
        candidate = line.lstrip(":").strip()
        if not candidate:
            continue
        full = f"{module_path_as_arg}::{candidate}"
        found.add(full)
    _PYTEST_TEST_CACHE[key] = found
    return found


def _extract_not_found_tests(test_output: str, instance_repo: Path) -> list[str]:
    found: list[str] = []
    seen: set[str] = set()
    for raw in test_output.splitlines():
        line = raw.strip()
        if not line:
            continue
        match = _LOCAL_TEST_NOT_FOUND_RE.match(line) or _LOCAL_TEST_NOT_FOUND_SUMMARY_RE.match(line)
        if not match:
            continue
        candidate = match.group(1).strip().rstrip(".")
        if "(" in candidate:
            candidate = candidate.split("(", 1)[0].strip()
        normalized = _normalize_repo_path(instance_repo, candidate)
        if normalized and normalized not in seen:
            seen.add(normalized)
            found.append(normalized)
    return found


def _resolve_not_found_tests(instance_repo: Path, test_ids: list[str]) -> list[str]:
    resolved: list[str] = []
    seen: set[str] = set()
    for raw in test_ids:
        normalized = _normalize_repo_path(instance_repo, _normalize_test_name(raw))
        if not normalized:
            continue
        if "::" not in normalized:
            if normalized not in seen:
                seen.add(normalized)
                resolved.append(normalized)
            continue
        if "[" not in normalized:
            if normalized not in seen:
                seen.add(normalized)
                resolved.append(normalized)
            continue
        module, _, suffix = normalized.partition("::")
        if not suffix:
            continue
        base_suffix = suffix.split("[", 1)[0]
        base_candidate = f"{module}::{base_suffix}"
        if base_candidate in seen:
            continue
        collected = _collect_module_test_ids(instance_repo, module)
        if base_candidate in collected:
            seen.add(base_candidate)
            resolved.append(base_candidate)
            continue
        for item in sorted(collected):
            if item.split("::", 1)[1].split("[", 1)[0] == base_suffix:
                if item not in seen:
                    seen.add(item)
                    resolved.append(item)
    return resolved


def _normalize_test_name(raw: str) -> str:
    line = _strip_ansi(str(raw)).strip()
    if not line:
        return ""
    line = line.replace("`", "").strip()
    if line in _LOCAL_TEST_PLACEHOLDER_FAILURES:
        return ""
    if line.startswith("::") and len(line) > 2:
        line = line[2:].strip()

    if line.startswith("(") and ")" in line:
        line = re.sub(r"^\([^)]*\)\s*", "", line).strip()

    if line.startswith("[") and "]" in line:
        line = re.sub(r"^\[[^]]+\]\s*", "", line).strip()

    status_prefix = re.compile(r"^(?:FAILED|ERROR)\s*:?\s*")
    line = status_prefix.sub("", line, count=1).strip()
    line = re.sub(r"^\[[0-9]+%\]\s*", "", line).strip()
    line = re.sub(r"^\[[0-9]+\]\s*", "", line).strip()

    # Trim common prefixes from stacked traces such as "  File ..., in function_name"
    line = re.sub(r"^\s*File\s+[^:]+:\d+:\s*in\s+(.+?)$", r"\1", line)
    line = re.sub(r"^\s*\w+\s+in\s+(.+)$", r"\1", line)

    if " in " in line and "::" in line:
        tail = line.rsplit(" in ", 1)[-1].strip()
        if "::" in tail:
            line = tail

    if " - " in line:
        candidate = line.split(" - ", 1)[0].strip()
        if candidate and "::" in candidate:
            line = candidate

    if line.startswith("FAILED ") or line.startswith("ERROR "):
        line = line.split(None, 1)[1] if " " in line else line

    line = re.sub(r"\s+\[[0-9]+\]$", "", line).strip()
    if "::" not in line and ".py" in line and ":" in line:
        line = line.split(":", 1)[0].strip()
    if "[" in line and line.endswith("]") and "::" not in line:
        line = line[: line.rfind("[")].strip()

    # Keep pytest-style node-id formats, drop non-test prose.
    if not ( "::" in line or line.endswith(".py") ):
        if " at " in line or " returned " in line:
            return ""
    return line.strip()


def _extract_patch_files(patch_text: str, max_files: int = 12) -> list[str]:
    files: list[str] = []
    seen: set[str] = set()
    for line in patch_text.splitlines():
        if not line.startswith("diff --git"):
            continue
        parts = line.split()
        if len(parts) < 3:
            continue
        lhs = parts[2]
        if not lhs.startswith("a/"):
            continue
        path = lhs[2:]
        if path in seen:
            continue
        seen.add(path)
        files.append(path)
        if len(files) >= max_files:
            break
    return files


def _coerce_patch_files(instance: dict[str, Any]) -> list[str]:
    collected: list[str] = []
    for key in ("patch", "test_patch"):
        raw = instance.get(key)
        if not raw:
            continue
        collected.extend(_extract_patch_files(_coerce_text(raw)))

    deduped: list[str] = []
    seen: set[str] = set()
    for path in collected:
        if path in seen:
            continue
        seen.add(path)
        deduped.append(path)
    return deduped


def _now_ts() -> str:
    return time.strftime("%Y%m%d-%H%M%S", time.localtime())


def _coerce_text(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    return str(value)


def _coerce_test_list(value: Any) -> list[str]:
    if value is None:
        return []
    if isinstance(value, list):
        return [str(item).strip() for item in value if str(item).strip()]
    if isinstance(value, tuple):
        return [str(item).strip() for item in value if str(item).strip()]
    if not isinstance(value, str):
        return []

    text = value.strip()
    if not text:
        return []

    parsed: Any = None
    if text.startswith("[") and text.endswith("]"):
        try:
            parsed = json.loads(text)
        except json.JSONDecodeError:
            try:
                parsed = ast.literal_eval(text)
            except (ValueError, SyntaxError):
                parsed = None
        if isinstance(parsed, list):
            return [str(item).strip() for item in parsed if str(item).strip()]

    return [part.strip() for part in text.split(",") if part.strip()]


def _strip_ansi(text: str) -> str:
    return re.sub(r"\x1b\[[0-9;]*m", "", text)


def _extract_failed_tests(test_output: str) -> list[str]:
    failed: list[str] = []
    seen: set[str] = set()
    for raw in test_output.splitlines():
        line = _strip_ansi(str(raw)).strip()
        if not _FAILED_LINE_PREFIX_RE.match(line):
            continue
        if _is_pytest_lookup_error_line(line):
            continue
        normalized = _normalize_test_name(line)
        if normalized and normalized not in seen:
            seen.add(normalized)
            failed.append(normalized)

    if not failed:
        for regex in _FAILED_TEST_PATTERNS:
            for match in regex.finditer(test_output):
                candidate = match.group(1 if match.groups() else 0)
                if _is_pytest_lookup_error_line(candidate):
                    continue
                normalized = _normalize_test_name(candidate)
                if normalized and normalized not in seen:
                    seen.add(normalized)
                    failed.append(normalized)
    return failed[:200]


def _is_pytest_lookup_error_line(line: str) -> bool:
    lower_line = _strip_ansi(str(line)).strip().lower()
    return (
        "not found:" in lower_line
        or "file or directory not found" in lower_line
        or "no name " in lower_line
        or "collected 0 " in lower_line
    )


def _extract_failure_snippet(test_output: str, max_lines: int = 60) -> str:
    if not test_output:
        return ""
    output = _strip_ansi(test_output)
    lines = output.splitlines()
    failing_indexes: list[int] = [
        idx
        for idx, line in enumerate(lines)
        if any(marker in line for marker in _LOCAL_TEST_ERROR_MARKERS)
        and line.strip() != ""
    ]
    if not failing_indexes:
        return _truncate_text(output, max_chars=max_lines * 120)

    start = max(0, failing_indexes[0] - 12)
    end = min(len(lines), failing_indexes[-1] + 1)
    snippet = "\n".join(lines[start : end + 1]).strip()
    if not snippet:
        return ""
    return _truncate_text(snippet, max_chars=max_lines * 120)


def _run(
    cmd: list[str],
    *,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
    input_text: str | None = None,
    timeout_sec: int | None = None,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=str(cwd) if cwd else None,
        env=env,
        input=input_text,
        text=True,
        encoding="utf-8",
        errors="replace",
        capture_output=True,
        timeout=timeout_sec,
        check=False,
    )


def _first_env(names: list[str]) -> str:
    for name in names:
        value = os.environ.get(name, "").strip()
        if value:
            return value
    return ""


def _normalize_chat_endpoint(raw: str) -> str:
    value = raw.strip().rstrip("/")
    if not value:
        return ""
    if value.endswith("/chat/completions"):
        return value
    return f"{value}/chat/completions"


def _normalize_network_policy(raw: str) -> str:
    value = raw.strip().lower()
    if value in {"allow", "ask", "deny"}:
        return value
    return "allow"


def _normalize_network_endpoint_host(endpoint: str) -> tuple[str, int]:
    parsed = urlparse(endpoint)
    scheme = parsed.scheme.lower()
    host = parsed.hostname
    if not host:
        raise ValueError("endpoint missing hostname")
    if parsed.port:
        return host, parsed.port
    return host, (443 if scheme == "https" else 80)


def _parse_dns_fallback_hosts() -> dict[str, list[str]]:
    raw = _first_env(list(_LLM_DNS_FALLBACK_ENV_NAMES))
    result: dict[str, list[str]] = {}
    if not raw:
        return result

    for entry in re.split(r"[;,]", raw):
        text = entry.strip()
        if not text or "=" not in text:
            continue
        host, fallback = text.split("=", 1)
        host = host.strip().lower()
        fallback = fallback.strip()
        if not host or not fallback:
            continue
        result.setdefault(host, []).append(fallback)

    return result


def _ensure_fallback_host(raw: str, host: str, fallback_host: str) -> bool:
    normalized_host = host.strip().lower()
    if not normalized_host or not fallback_host.strip():
        return False
    for entry in re.split(r"[;,]", raw):
        text = entry.strip()
        if not text or "=" not in text:
            continue
        entry_host, entry_addr = text.split("=", 1)
        if entry_host.strip().lower() == normalized_host and entry_addr.strip():
            return True
    return False


def _append_dns_fallbacks(env: dict[str, str], endpoint_host: str) -> None:
    if not endpoint_host:
        return
    endpoint_host = endpoint_host.lower()

    existing = env.get("NOLA_LLM_DNS_FALLBACKS", "").strip()
    fallback_specs: list[str] = []
    if existing:
        fallback_specs.append(existing)

    missing = []
    for target_host, target_addr in _DEFAULT_DNS_FALLBACKS:
        if target_host != endpoint_host:
            continue
        if _ensure_fallback_host(existing, target_host, target_addr):
            continue
        if target_host.lower() == endpoint_host:
            missing.append(f"{target_host}={target_addr}")

    if not missing:
        return

    for entry in missing:
        fallback_specs.append(entry)

    combined = ";".join([spec for spec in fallback_specs if spec.strip()])
    env["NOLA_LLM_DNS_FALLBACKS"] = combined
    env["NOLA_CLI_DNS_FALLBACKS"] = combined


def _normalize_network_probe_endpoint(endpoint: str) -> tuple[str, int]:
    parsed = urlparse(endpoint)
    scheme = parsed.scheme.lower()
    port = parsed.port
    host = parsed.hostname
    if not host:
        raise ValueError("endpoint missing hostname")
    if port:
        return host, port
    return host, (443 if scheme == "https" else 80)


def _iter_network_probe_addrs(endpoint_host: str, endpoint_port: int):
    seen: set[tuple[str, int]] = set()
    fallback_targets = _parse_dns_fallback_hosts().get(endpoint_host.lower(), [])

    for candidate in [endpoint_host, *fallback_targets]:
        try:
            infos = socket.getaddrinfo(candidate, endpoint_port, type=socket.SOCK_STREAM)
        except Exception:
            continue
        for info in infos:
            sockaddr = info[4]
            if not isinstance(sockaddr, tuple):
                continue
            if len(sockaddr) < 2:
                continue
            key = (str(sockaddr[0]), int(sockaddr[1]))
            if key in seen:
                continue
            seen.add(key)
            yield sockaddr


def _probe_network_endpoint(endpoint: str, timeout_sec: int = 3) -> tuple[bool, str]:
    host, port = _normalize_network_endpoint_host(endpoint)
    last_error: str = ""
    attempted = False
    for sockaddr in _iter_network_probe_addrs(host, port):
        attempted = True
        try:
            with socket.create_connection(sockaddr, timeout=timeout_sec) as connection:
                connection.shutdown(socket.SHUT_RDWR)
            return True, ""
        except Exception as exc:
            last_error = str(exc)

    if not attempted:
        return False, "no resolvable network addresses"
    if not last_error:
        return False, "connection failed"
    return False, last_error


def _probe_git_remote(repo: str, timeout_sec: int = 10) -> tuple[bool, str]:
    remote = f"https://github.com/{repo}.git"
    try:
        proc = _run(
            ["git", "ls-remote", remote, "HEAD"],
            timeout_sec=timeout_sec,
        )
    except subprocess.TimeoutExpired:
        return False, f"git ls-remote timed out after {timeout_sec}s for {remote}"
    except Exception as exc:
        return False, str(exc)
    if proc.returncode == 0:
        return True, ""
    error = (proc.stderr or proc.stdout or "").strip()
    return False, error or f"git ls-remote failed for {remote}"


def _ask_network_access(endpoint: str, error: str) -> bool:
    if not sys.stdin.isatty():
        print(
            f"[network] unreachable ({endpoint}): {error}\n"
            f"[network] non-interactive mode, cannot ask for confirmation"
        )
        return False
    print(f"[network] unreachable ({endpoint}): {error}")
    try:
        reply = input("Continue without guaranteed network access? [y/N]: ").strip().lower()
    except Exception:
        return False
    return reply in {"y", "yes"}


@dataclasses.dataclass
class InstanceRunResult:
    instance_id: str
    repo: str
    base_commit: str
    exit_code: int
    timeout: bool
    elapsed_sec: float
    done_reason: str
    cli_done_reason: str
    tool_use_count: int
    tool_error_count: int
    text_event_count: int
    tool_read_tool_count: int
    tool_write_tool_count: int
    tool_use_name_count: int
    tool_uses: list[str]
    patch_chars: int
    patch_nonempty: bool
    patch_path: str
    stdout_path: str
    stderr_path: str
    meta_path: str
    local_test_attempts: int
    local_tests_passed: bool
    local_test_failed_count: int
    local_tests_inconclusive: bool
    local_test_failure_type: str


def _load_dataset_instances(dataset_name: str, split: str, offset: int, limit: int) -> list[dict[str, Any]]:
    try:
        from datasets import Dataset as HFArrowDataset  # type: ignore
        from datasets import DownloadMode, load_dataset  # type: ignore
    except ImportError as exc:  # pragma: no cover - runtime guard
        raise RuntimeError(
            "Missing dependency: datasets. Install with `python -m pip install datasets`."
        ) from exc

    if dataset_name.lower() == "swe-bench/swe-bench_verified":
        cached_arrow = _find_local_swebench_arrow_path(dataset_name, split)
        if cached_arrow:
            try:
                dataset = HFArrowDataset.from_file(str(cached_arrow))
                if offset < 0:
                    raise ValueError("offset must be >= 0")
                end = min(len(dataset), offset + max(0, limit))
                if offset >= end:
                    return []
                return [dict(dataset[i]) for i in range(offset, end)]
            except Exception as exc:
                print(f"[dataset] failed loading SWE-bench local arrow cache: {exc}")

    load_kwargs: dict[str, Any] = {
        "cache_dir": str(_local_swebench_cache_root()),
        "split": split,
    }
    load_kwargs["download_mode"] = getattr(
        DownloadMode, "REUSE_DATASET_IF_EXISTS", None
    )
    if load_kwargs["download_mode"] is None:
        load_kwargs.pop("download_mode")

    dataset = load_dataset(dataset_name, **load_kwargs)

    if offset < 0:
        raise ValueError("offset must be >= 0")
    end = min(len(dataset), offset + max(0, limit))
    if offset >= end:
        return []
    return [dict(dataset[i]) for i in range(offset, end)]


def _filter_instance_subset(
    instances: list[dict[str, Any]], instance_ids: list[str] | None
) -> list[dict[str, Any]]:
    if not instance_ids:
        return instances
    wanted = {value.strip() for value in instance_ids if value.strip()}
    if not wanted:
        return instances
    return [row for row in instances if str(row.get("instance_id") or "").strip() in wanted]


def _find_existing_repo_cache(repo_key: str, repos_dir: Path) -> Path | None:
    direct = repos_dir / repo_key
    if direct.exists():
        return direct

    work_root = repos_dir.parent.parent
    if not work_root.exists():
        return None

    try:
        siblings = [path for path in work_root.iterdir() if path.is_dir()]
    except OSError:
        return None

    for sibling in siblings:
        if sibling == repos_dir.parent:
            continue
        candidate = sibling / "repos_cache" / repo_key
        if candidate.exists():
            return candidate

    return None


def _attach_repo_cache(source: Path, target: Path) -> None:
    if target.exists():
        return
    target.parent.mkdir(parents=True, exist_ok=True)
    try:
        target.symlink_to(source, target_is_directory=True)
        return
    except Exception:
        shutil.copytree(source, target)


def _ensure_repo_cache(repo: str, repos_dir: Path) -> Path:
    repos_dir.mkdir(parents=True, exist_ok=True)
    repo_key = repo.replace("/", "__")
    cache_path = repos_dir / repo_key
    remote = f"https://github.com/{repo}.git"
    if not cache_path.exists():
        shared_cache = _find_existing_repo_cache(repo_key, repos_dir)
        if shared_cache is not None:
            try:
                _attach_repo_cache(shared_cache, cache_path)
                print(f"[cache] reused existing repository cache from {shared_cache}")
            except Exception as exc:
                print(f"[cache] failed to reuse cached repository: {exc}")
        if not cache_path.exists():
            clone_cmds = [
                ["git", "clone", "--quiet", "--depth", "1", remote, str(cache_path)],
                ["git", "clone", "--quiet", remote, str(cache_path)],
            ]
            proc = subprocess.CompletedProcess(["git", "clone"], returncode=1, stdout="", stderr="")
            for cmd in clone_cmds:
                proc = _run(cmd)
                if proc.returncode == 0:
                    break
                if proc.stderr:
                    print(f"[cache] clone retry with {cmd[0]} {cmd[1]}... failed: {proc.stderr.strip()}")
            if proc.returncode != 0:
                raise RuntimeError(f"clone failed for {repo}: {proc.stderr.strip()}")
    else:
        proc = _run(["git", "-C", str(cache_path), "fetch", "--all", "--tags", "--prune", "--quiet"])
        if proc.returncode != 0:
            print(f"[cache] fetch warning for {repo}: {proc.stderr.strip()}")
        if proc.returncode != 0:
            print(f"[cache] keeping existing cache for {repo}")
    return cache_path


def _prepare_instance_checkout(repo_cache: Path, instance_dir: Path, base_commit: str) -> None:
    if instance_dir.exists():
        shutil.rmtree(instance_dir)
    instance_dir.parent.mkdir(parents=True, exist_ok=True)
    if not repo_cache.exists():
        raise RuntimeError(f"cache path missing: {repo_cache}")

    clone_attempts = [
        ["git", "clone", "--quiet", "--shared", str(repo_cache), str(instance_dir)],
        ["git", "clone", "--quiet", "--shared", "--no-local", str(repo_cache), str(instance_dir)],
    ]
    checkout_error = ""
    for attempt_index, clone_cmd in enumerate(clone_attempts):
        clone_proc = _run(clone_cmd)
        if clone_proc.returncode != 0:
            if attempt_index == len(clone_attempts) - 1:
                raise RuntimeError(f"clone instance repo failed: {clone_proc.stderr.strip()}")
            continue

        checkout_proc = _run(
            ["git", "-C", str(instance_dir), "-c", "advice.detachedHead=false", "checkout", "--force", base_commit]
        )
        if checkout_proc.returncode == 0:
            break
        checkout_error = checkout_proc.stderr.strip()
        shutil.rmtree(instance_dir, ignore_errors=True)
        if "unable to read tree" in checkout_error:
            unshallow_proc = _run(["git", "-C", str(repo_cache), "fetch", "--unshallow"])
            if unshallow_proc.returncode != 0:
                print(f"[cache] fetch --unshallow warning: {unshallow_proc.stderr.strip()}")
        else:
            raise RuntimeError(f"checkout {base_commit} failed: {checkout_error}")
    else:
        raise RuntimeError(f"checkout {base_commit} failed: {checkout_error}")

    clean_proc = _run(["git", "-C", str(instance_dir), "clean", "-fdx"])
    if clean_proc.returncode != 0:
        raise RuntimeError(f"git clean failed: {clean_proc.stderr.strip()}")


def _build_prompt(instance: dict[str, Any]) -> str:
    issue = str(instance.get("problem_statement") or "").strip()
    hints = str(instance.get("hints_text") or "").strip()
    requirements = str(instance.get("requirements") or "").strip()
    interface = str(instance.get("interface") or "").strip()
    fail_to_pass = _coerce_test_list(instance.get("FAIL_TO_PASS") or instance.get("fail_to_pass"))
    pass_to_pass = _coerce_test_list(instance.get("PASS_TO_PASS") or instance.get("pass_to_pass"))
    patch_targets = _coerce_patch_files(instance)
    fail_list = fail_to_pass
    pass_list = pass_to_pass
    target_tests = "\n".join(f"- {item}" for item in fail_list[:40]) if fail_list else "(not provided)"
    stable_tests = "\n".join(f"- {item}" for item in pass_list[:40]) if pass_list else "(not provided)"
    target_files = "\n".join(f"- {item}" for item in patch_targets[:12]) if patch_targets else "(not provided)"
    instance_id = str(instance.get("instance_id") or "")
    repo = str(instance.get("repo") or "")
    base_commit = str(instance.get("base_commit") or "")
    return (
        "You are a SWE-bench coding agent operating on this checked-out repository.\n"
        "Work in a strict four-stage loop: explore, plan, implement, verify.\n"
        f"Instance: {instance_id}\n"
        f"Repo: {repo}\n"
        f"Base commit: {base_commit}\n\n"
        "Objective:\n"
        "1. Read the issue and related files, then identify the exact root cause.\n"
        "2. Treat Structured requirements and Interface/API details as authoritative acceptance criteria when provided.\n"
        "3. Before edits, make a concise checklist of every required method/file/behavior from Structured requirements and Interface/API details.\n"
        "4. Implement all checklist items that affect the failing behavior; do not stop after fixing only the first visible test when other listed requirements remain unimplemented.\n"
        "5. Make only targeted code edits to fix behavior; avoid stylistic churn.\n"
        "6. Keep API and public behavior unchanged unless requested by the issue.\n"
        "7. After edits, run targeted failing tests first, then a small regression slice.\n"
        "8. If tests still fail, make one minimal follow-up change and re-run.\n"
        "9. Stop when the focused failing path is green and every structured requirement is either implemented or explicitly proven irrelevant.\n\n"
        "Issue:\n"
        f"{issue}\n"
        f"\nStructured requirements:\n{requirements or '(not provided)'}\n"
        f"\nInterface/API details:\n{interface or '(not provided)'}\n"
        f"\nHints:\n{hints or '(none)'}\n"
        "Reference changed files from dataset patch/test data:\n"
        f"{target_files}\n"
        "Target failing tests:\n"
        f"{target_tests}\n"
        "Likely stable tests:\n"
        f"{stable_tests}\n"
        "For each change, keep edits minimal and executable in one shot, with no markdown-only output.\n"
        "When uncertain, prefer to read only the minimum required files, apply one behavioral fix, then validate.\n"
        "Execution constraints:\n"
        "1) read only the minimal target files and immediate callers.\n"
        "2) read-only git commands such as git status, git diff, git show, and git log are allowed; do not use git commit, push, reset, clean, checkout, restore, fetch, pull, clone, or other commands that mutate state or contact remotes.\n"
        "3) FAIL_TO_PASS names may refer to hidden harness tests or test patch data that is not present in the checkout; do not spend turns searching for missing hidden fixtures.\n"
        "4) do not run repository setup, pip install, build_ext, full builds, or dependency installation before making the first code edit.\n"
        "5) if local validation reports missing compiled extensions or environment/setup/import errors, treat validation as inconclusive: do not repair site-packages, conda, build_ext, or global installs; keep the source-code patch and stop.\n"
        "6) after edits, run the listed FAIL_TO_PASS tests first when they exist locally; otherwise run the nearest focused local regression or a small Python reproducer.\n"
        "7) if tests still fail with a real assertion in project code, fix only the minimal path causing the first failure.\n"
        "8) avoid changing test files, public APIs, or unrelated behavior.\n"
        "9) do not mock, fake, stub, monkeypatch, hard-code special cases, or change tests to manufacture success; implement the real production behavior.\n"
        "10) do not inspect full large files or full large diffs repeatedly; use symbols, grep, and focused line ranges.\n"
        "Never use shell commands to format or auto-fix the entire repository.\n"
    )


def _truncate_text(text: str, max_chars: int = 6000) -> str:
    if len(text) <= max_chars:
        return text
    keep = max_chars // 2
    return (
        f"{text[:keep]}\n"
        f"... [truncated {len(text) - 2 * keep} chars] ...\n"
        f"{text[-keep:]}"
    )


def _build_refine_prompt(
    base_prompt: str,
    failed_tests: list[str],
    test_output: str,
    attempt: int,
    patch_path: Path | None = None,
    selected_tests: list[str] | None = None,
    test_output_snippet: str = "",
    require_write: bool = False,
) -> str:
    if failed_tests:
        failed_block = "\n".join(f"- {item}" for item in failed_tests)
    else:
        failed_block = "(none)"

    patch_snippet = ""
    if patch_path is not None and patch_path.exists():
        patch_raw = _coerce_text(patch_path.read_text(encoding="utf-8"))
        patch_snippet = _truncate_text(patch_raw, max_chars=3500)
    if not patch_snippet:
        patch_snippet = "(no patch captured yet)"

    return (
        base_prompt
        + "\n\nIteration Instructions:\n"
        + f"Attempt {attempt}: local tests still fail.\n"
        + (
            "1) Repair only what is needed for the exact failing tests above.\n"
            "2) Focus on the fewest files and smallest symbols possible.\n"
            "3) Keep prior behavioral fixes; do not introduce extra style-only edits.\n"
            "4) Prioritize the first listed failing test and run only that first if needed.\n"
            "5) Then run the remaining selected failures and 1-2 regression tests.\n"
            "6) If local tests are still failing, make one minimal follow-up change and re-run tests once.\n"
            "7) If output is noisy, extract the first actionable assertion or exception root and patch only that area.\n"
            "8) If the latest change likely regressed earlier behavior, prefer a focused revert of that behavior over broad edits.\n"
            "9) If recent output is an environment/setup/import/build_ext failure, do not try more setup commands; keep the best source-code patch and stop for harness evaluation.\n"
        )
        + (
            "\nIMPORTANT: this retry exists because the previous attempt had no write action.\n"
            "For this iteration, use normal read/search tools if the target is still unclear, then make one minimal source-code edit once the target is identified.\n"
            "Allowed write/edit tools: write_file, edit_file, edit_file_lines, replace_symbol_body,\n"
            "replace_in_files, multi_edit_file, rename_symbol, move_file, delete_file, apply_patch.\n"
            "Do not create scratch, exploration, notebook, note, tmp, marker, or placeholder files. Only edit files that are part of the actual repository fix.\n"
            if require_write
            else ""
        )
        + "\n"
        f"Failing tests:\n{failed_block}\n\n"
        "Run only these test targets this iteration:\n"
        + ( "\n".join(f"- {item}" for item in (selected_tests or [])) if (selected_tests or []) else "(none)" )
        + "\n\n"
        f"Recent failing output (trimmed):\n"
        "```\n"
        f"{_truncate_text(test_output_snippet or test_output, max_chars=6000)}\n"
        "```\n\n"
        "For the next model step, prioritize the first listed failing test first.\n"
        "Prefer to rerun only the first failing test unless the failure slice expands.\n"
        "Current patch (truncated):\n"
        f"{patch_snippet}\n\n"
    )


def _extract_fail_to_pass_tests(instance: dict[str, Any]) -> list[str]:
    return _coerce_test_list(instance.get("FAIL_TO_PASS") or instance.get("fail_to_pass"))


def _extract_pass_to_pass_tests(instance: dict[str, Any]) -> list[str]:
    return _coerce_test_list(instance.get("PASS_TO_PASS") or instance.get("pass_to_pass"))


def _merge_tests(*groups: list[str]) -> list[str]:
    merged: list[str] = []
    seen: set[str] = set()
    for group in groups:
        for item in group:
            normalized = _normalize_test_name(item)
            if not normalized or normalized in seen:
                continue
            seen.add(normalized)
            merged.append(normalized)
    return merged


def _is_local_test_inconclusive(output: str, return_code: int) -> tuple[bool, str]:
    if return_code == 0:
        return False, ""
    output_lower = output.lower()
    if any(marker in output_lower for marker in _LOCAL_TEST_SOURCE_ERROR_MARKERS):
        return False, "test_fail"
    if return_code == 5:
        return True, "no_tests_collected"
    if any(marker in output_lower for marker in _LOCAL_TEST_COLLECTION_ERROR_MARKERS):
        return True, "collection_error"
    for marker in _LOCAL_TEST_INCONCLUSIVE_MARKERS:
        if marker in output_lower:
            return True, "environment"
    if return_code in {2, 4, 5} and "traceback" in output_lower and "failed to import" in output_lower:
        return True, "environment"
    return False, "execution_error"


def _run_local_tests(
    instance_repo: Path, tests: list[str], timeout_sec: int
) -> tuple[bool, int, list[str], str, bool, str]:
    tests = [normalized for raw in tests if (normalized := _normalize_test_name(raw))]
    if not tests:
        return True, 0, [], "", False, ""
    pytest_tests = [test for test in tests if _looks_like_pytest_nodeid(test)]
    if len(pytest_tests) != len(tests):
        unsupported = [test for test in tests if test not in pytest_tests]
        output = (
            "Local pytest validation skipped: selected test identifiers are not pytest node ids.\n"
            "Use the official harness for this instance.\n"
            "Unsupported tests:\n"
            + "\n".join(f"- {test}" for test in unsupported[:50])
        )
        return False, 1, ["<unsupported_local_test_format>"], output, True, "unsupported_local_test_format"
    try:
        proc = _run(
            [
                "python",
                "-m",
                "pytest",
                "-q",
                "--disable-warnings",
                "-c",
                "/dev/null",
                f"--maxfail={max(min(len(tests), _LOCAL_TEST_MAX_FAIL_PER_ATTEMPT), 1)}",
                *pytest_tests,
            ],
            cwd=instance_repo,
            timeout_sec=timeout_sec,
        )
    except subprocess.TimeoutExpired as timeout_exc:
        output = _coerce_text(timeout_exc.stdout) + "\n" + _coerce_text(timeout_exc.stderr)
        return (
            False,
            1,
            ["<execution_timeout>"],
            output,
            True,
            "timeout",
        )

    output = _coerce_text(proc.stdout) + "\n" + _coerce_text(proc.stderr)
    failed = _extract_failed_tests(output)
    failed = [line.strip() for line in failed if line.strip()]
    if proc.returncode == 0:
        return True, 0, [], output, False, ""

    if not failed:
        output_lower = output.lower()
        not_found = _extract_not_found_tests(output, instance_repo)
        if not_found:
            return (
                False,
                1,
                ["<test_name_mismatch>"],
                output,
                True,
                "test_name_mismatch",
            )
        if "collected 0" in output_lower:
            return (
                False,
                1,
                ["<collection_no_match>"],
                output,
                True,
                "no_tests_collected",
            )
        inconclusive, failure_type = _is_local_test_inconclusive(output, proc.returncode)
        if inconclusive:
            return False, 1, ["<execution_inconclusive>"], output, True, failure_type
        return (
            False,
            1,
            ["<execution_error>"],
            output,
            False,
            "execution_error",
        )

    return False, len(failed), failed, output, False, "test_fail"


def _looks_like_pytest_nodeid(test: str) -> bool:
    normalized = test.strip()
    if not normalized:
        return False
    module = normalized.split("::", 1)[0]
    return module.endswith(".py")


def _local_test_candidates(
    fail_to_pass_tests: list[str],
    pass_to_pass_tests: list[str],
    prior_failed_tests: list[str],
    attempt: int,
) -> list[str]:
    fail_to_pass = _merge_tests(fail_to_pass_tests)
    pass_to_pass = _merge_tests(pass_to_pass_tests)
    prior_failed = [
        normalized
        for raw in (prior_failed_tests or [])
        if (normalized := _normalize_test_name(raw))
    ]
    if attempt <= 1:
        merged = list(fail_to_pass)
        if pass_to_pass:
            for item in pass_to_pass[:_LOCAL_TEST_REGRESSION_TEST_LIMIT]:
                if item not in merged:
                    merged.append(item)
        return merged[:_LOCAL_TEST_MAX_INIT]

    merged = list(prior_failed)
    merged_set = set(prior_failed)
    if prior_failed:
        merged = list(prior_failed)
    else:
        merged = fail_to_pass
        merged_set = set(fail_to_pass)

    for item in fail_to_pass:
        if item in merged_set:
            continue
        merged_set.add(item)
        merged.append(item)

    if pass_to_pass:
        regression_budget = _LOCAL_TEST_REGRESSION_TEST_LIMIT if len(merged) > 0 else _LOCAL_TEST_MAX_INIT
        for item in pass_to_pass[:regression_budget]:
            if item in merged_set:
                continue
            merged_set.add(item)
            merged.append(item)

    if not merged:
        return []
    return merged[:_LOCAL_TEST_MAX_REFINE]


def _collect_patch(repo_dir: Path) -> str:
    excluded = {
        "NOLA.md",
        "NOLA.local.md",
    }
    proc = _run(
        [
            "git",
            "-C",
            str(repo_dir),
            "diff",
            "--no-color",
            "--",
            ".",
            ":(exclude)NOLA.md",
            ":(exclude)NOLA.local.md",
            ":(exclude).nola",
        ]
    )
    if proc.returncode != 0:
        raise RuntimeError(f"git diff failed: {proc.stderr.strip()}")
    patch_parts = [_coerce_text(proc.stdout)]

    untracked_proc = _run(
        [
            "git",
            "-C",
            str(repo_dir),
            "ls-files",
            "--others",
            "--exclude-standard",
        ]
    )
    if untracked_proc.returncode != 0:
        raise RuntimeError(f"git ls-files failed: {untracked_proc.stderr.strip()}")

    for raw_path in _coerce_text(untracked_proc.stdout).splitlines():
        rel_path = raw_path.strip()
        if (
            not rel_path
            or rel_path in excluded
            or rel_path.startswith(".nola/")
            or rel_path == ".nola"
        ):
            continue
        file_path = repo_dir / rel_path
        if not file_path.is_file():
            continue
        new_file_proc = _run(
            [
                "git",
                "-C",
                str(repo_dir),
                "diff",
                "--no-color",
                "--no-index",
                "--",
                "/dev/null",
                rel_path,
            ]
        )
        if new_file_proc.returncode not in {0, 1}:
            raise RuntimeError(f"git diff --no-index failed: {new_file_proc.stderr.strip()}")
        patch_parts.append(_coerce_text(new_file_proc.stdout))

    return "\n".join(part.strip("\n") for part in patch_parts if part.strip())


def _run_project_context_init(
    *,
    cli_binary: Path,
    instance_repo: Path,
    endpoint: str,
    model_name: str,
    api_key: str,
    network_policy: str,
    env: dict[str, str],
    timeout_sec: int,
) -> tuple[bool, str]:
    cmd = [
        str(cli_binary),
        "init",
        "--yes",
        "--force",
        "--project",
        str(instance_repo),
        "--output",
        "NOLA.md",
        "--endpoint",
        endpoint,
        "--model",
        model_name,
        "--network-policy",
        network_policy,
    ]
    if api_key.strip():
        cmd.extend(["--api-key", api_key.strip()])
    try:
        proc = _run(cmd, cwd=instance_repo, env=env, timeout_sec=timeout_sec)
    except subprocess.TimeoutExpired:
        return False, f"nola init timed out after {timeout_sec}s"
    if proc.returncode != 0:
        reason = (_coerce_text(proc.stderr) or _coerce_text(proc.stdout)).strip()
        return False, reason[:1000] or f"nola init failed with exit {proc.returncode}"
    if not (instance_repo / "NOLA.md").exists():
        return False, "nola init completed but NOLA.md was not created"
    _exclude_generated_context_from_git(instance_repo)
    return True, "generated NOLA.md"


def _exclude_generated_context_from_git(instance_repo: Path) -> None:
    exclude_path = instance_repo / ".git" / "info" / "exclude"
    if not exclude_path.exists():
        return
    existing = exclude_path.read_text(encoding="utf-8", errors="replace")
    additions = ["/NOLA.md", "/NOLA.local.md", "/.nola/"]
    missing = [item for item in additions if item not in existing.splitlines()]
    if not missing:
        return
    with exclude_path.open("a", encoding="utf-8") as handle:
        if existing and not existing.endswith("\n"):
            handle.write("\n")
        for item in missing:
            handle.write(f"{item}\n")


def _run_env_fix(instance_repo: Path) -> bool:
    if os.environ.get("NOLA_SWEBENCH_DISABLE_ENV_FIX", "1").strip().lower() not in {"0", "false", "no", "off"}:
        return False
    build_cmds = []
    setup_cfg = instance_repo / "pyproject.toml"
    setup_py = instance_repo / "setup.py"
    if setup_cfg.exists():
        build_cmds.append(["python", "-m", "pip", "install", "-e", "."])
    if setup_py.exists():
        build_cmds.append(["python", "setup.py", "build_ext", "--inplace"])
    if not build_cmds:
        return False

    fixed = False
    for cmd in build_cmds:
        proc = _run(cmd, cwd=instance_repo, timeout_sec=900)
        if proc.returncode == 0:
            fixed = True
            break
    return fixed


def _build_salvage_prompt(
    base_prompt: str,
    failed_tests: list[str],
    attempt: int,
    patch_path: Path | None = None,
    selected_tests: list[str] | None = None,
    test_output_snippet: str = "",
) -> str:
    if failed_tests:
        failed_block = "\n".join(f"- {item}" for item in failed_tests)
    else:
        failed_block = "(none)"

    patch_snippet = ""
    if patch_path is not None and patch_path.exists():
        patch_snippet = _coerce_text(patch_path.read_text(encoding="utf-8"))
        patch_snippet = _truncate_text(patch_snippet, max_chars=3000)
    if not patch_snippet:
        patch_snippet = "(no patch captured yet)"

    return (
        base_prompt
        + "\n\nIteration Instructions:\n"
        + f"Attempt {attempt}: final iteration must produce a concrete code edit.\n"
        + "1) Do not continue read/search-only calls. Make one write/edit action now.\n"
        + "2) Use the target files and first failing test as your only focus.\n"
        + "3) Keep the patch minimal (one symbol or one small block) and rerun the first failing test.\n"
        + "4) If the failure remains, make exactly one more minimal follow-up edit and rerun.\n\n"
        + "Failing tests:\n"
        + f"{failed_block}\n\n"
        + "Run only these test targets this iteration:\n"
        + (("\n".join(f"- {item}" for item in (selected_tests or [])) if (selected_tests or []) else "(none)")
           + "\n\n")
        + "Recent failing output (trimmed):\n"
        + "```\n"
        + f"{_truncate_text(test_output_snippet, max_chars=6000)}\n"
        + "```\n\n"
        + "Current patch (truncated):\n"
        + f"{patch_snippet}\n"
    )


def _parse_cli_events(stdout_text: str) -> tuple[str, int, int, int, list[str], int, int, int]:
    done_reason = ""
    tool_use_count = 0
    tool_error_count = 0
    text_event_count = 0
    tool_uses: list[str] = []
    read_tool_count = 0
    write_tool_count = 0
    for line in stdout_text.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(event, dict):
            continue
        event_type = str(event.get("type") or "")
        if event_type in {"tool_use", "tool"}:
            tool_use_count += 1
            tool_name = str(
                event.get("name") or event.get("tool_name") or event.get("tool") or ""
            ).strip()
            if tool_name:
                tool_uses.append(tool_name)
                category = _classify_swe_tool(tool_name)
                if category == "read":
                    read_tool_count += 1
                elif category == "write":
                    write_tool_count += 1
        elif event_type in {"tool_call", "tool-call", "tool_event"}:
            tool_use_count += 1
            tool_name = str(
                event.get("name") or event.get("tool_name") or event.get("tool") or ""
            ).strip()
            if tool_name:
                tool_uses.append(tool_name)
                category = _classify_swe_tool(tool_name)
                if category == "read":
                    read_tool_count += 1
                elif category == "write":
                    write_tool_count += 1
        elif event_type == "tool_result":
            if bool(event.get("error")):
                tool_error_count += 1
        elif event_type in {"text", "reasoning"}:
            text_event_count += 1
        elif event_type == "done":
            done_reason = str(event.get("reason") or "").strip()
        elif event_type in {"done_reason", "finish"}:
            done_reason = str(
                event.get("reason") or event.get("status") or event.get("reasoning") or ""
            ).strip()
    return (
        done_reason,
        tool_use_count,
        tool_error_count,
        text_event_count,
        tool_uses,
        read_tool_count,
        write_tool_count,
        len(tool_uses),
    )


def _is_llm_request_error(output_text: str) -> bool:
    text = output_text.lower()
    return any(marker in text for marker in _LLM_REQUEST_ERROR_MARKERS)


def _is_network_blocked_error(output_text: str) -> bool:
    text = output_text.lower()
    return any(marker in text for marker in _LLM_NETWORK_ENV_ERROR_MARKERS)


def _load_tool_counts_from_stdout_path(stdout_path: str) -> tuple[int, int, int, list[str]]:
    if not stdout_path:
        return 0, 0, 0, []
    path = Path(stdout_path)
    if not path.is_file():
        return 0, 0, 0, []
    try:
        text = path.read_text(encoding="utf-8")
    except Exception:
        return 0, 0, 0, []
    (
        _,
        tool_use_count,
        _,
        _,
        tool_uses,
        read_tool_count,
        write_tool_count,
        _,
    ) = _parse_cli_events(text)
    return tool_use_count, read_tool_count, write_tool_count, tool_uses


def _row_tool_counts(row: dict[str, Any]) -> tuple[int, int, int, int, list[str]]:
    tool_use_count = row.get("tool_use_count")
    tool_error_count = row.get("tool_error_count")
    tool_read_tool_count = row.get("tool_read_tool_count")
    tool_write_tool_count = row.get("tool_write_tool_count")
    tool_uses = row.get("tool_uses")
    tool_name_count = row.get("tool_use_name_count")
    try:
        tool_use_count_i = int(tool_use_count) if tool_use_count is not None else None
    except Exception:
        tool_use_count_i = None
    try:
        tool_error_count_i = int(tool_error_count) if tool_error_count is not None else None
    except Exception:
        tool_error_count_i = None
    try:
        tool_read_tool_count_i = int(tool_read_tool_count) if tool_read_tool_count is not None else None
    except Exception:
        tool_read_tool_count_i = None
    try:
        tool_write_tool_count_i = int(tool_write_tool_count) if tool_write_tool_count is not None else None
    except Exception:
        tool_write_tool_count_i = None
    try:
        tool_name_count_i = int(tool_name_count) if tool_name_count is not None else None
    except Exception:
        tool_name_count_i = None

    if isinstance(tool_uses, list) and any(str(item).strip() for item in tool_uses):
        uses = [str(item).strip() for item in tool_uses if str(item).strip()]
        if tool_read_tool_count_i is None or tool_write_tool_count_i is None or tool_name_count_i is None:
            local_read = 0
            local_write = 0
            for tool_name in uses:
                category = _classify_swe_tool(tool_name)
                if category == "read":
                    local_read += 1
                elif category == "write":
                    local_write += 1
            if tool_read_tool_count_i is None:
                tool_read_tool_count_i = local_read
            if tool_write_tool_count_i is None:
                tool_write_tool_count_i = local_write
            if tool_name_count_i is None:
                tool_name_count_i = len(uses)

    if (
        any(
            value is None
            for value in (
                tool_use_count_i,
                tool_error_count_i,
                tool_read_tool_count_i,
                tool_write_tool_count_i,
                tool_name_count_i,
            )
        )
        or not isinstance(tool_uses, list)
    ):
        stdout_path = row.get("stdout_path")
        if isinstance(stdout_path, str):
            parsed_tool_use_count, parsed_read_count, parsed_write_count, parsed_tool_uses = _load_tool_counts_from_stdout_path(
                stdout_path
            )
            if parsed_tool_uses:
                use_names = parsed_tool_uses
            else:
                use_names = []
            if tool_use_count_i is None:
                tool_use_count_i = parsed_tool_use_count
            if tool_error_count_i is None:
                tool_error_count_i = 0
            if tool_read_tool_count_i is None:
                tool_read_tool_count_i = parsed_read_count
            if tool_write_tool_count_i is None:
                tool_write_tool_count_i = parsed_write_count
            if tool_name_count_i is None:
                tool_name_count_i = len(use_names)
            if parsed_tool_use_count and (
                not isinstance(tool_uses, list) or not any(str(item).strip() for item in tool_uses)
            ):
                tool_uses = use_names

    if not isinstance(tool_uses, list):
        tool_uses = []

    return (
        tool_use_count_i or 0,
        tool_error_count_i or 0,
        tool_read_tool_count_i or 0,
        tool_write_tool_count_i or 0,
        tool_uses,
    )


def _build_cli_binary(cli_dir: Path, release: bool) -> Path:
    mode_args = ["--release"] if release else []
    proc = _run(["cargo", "build", *mode_args], cwd=cli_dir)
    if proc.returncode != 0:
        raise RuntimeError(f"cargo build failed:\n{proc.stderr}")
    profile = "release" if release else "debug"
    binary = cli_dir / "target" / profile / "nola"
    if not binary.exists():
        raise RuntimeError(f"CLI binary not found at {binary}")
    return binary


def _write_jsonl(path: Path, rows: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as f:
        for row in rows:
            f.write(json.dumps(row, ensure_ascii=False))
            f.write("\n")


def _run_harness_eval(
    *,
    dataset_name: str,
    split: str,
    predictions_path: Path,
    run_id: str,
    max_workers: int,
    timeout_sec: int,
    instance_ids: list[str],
    workdir: Path,
    use_modal: bool,
) -> subprocess.CompletedProcess[str]:
    cmd = [
        sys.executable,
        "-m",
        "swebench.harness.run_evaluation",
        "--dataset_name",
        dataset_name,
        "--split",
        split,
        "--predictions_path",
        str(predictions_path),
        "--max_workers",
        str(max_workers),
        "--run_id",
        run_id,
        "--timeout",
        str(timeout_sec),
    ]
    if instance_ids:
        cmd.extend(["--instance_ids", *instance_ids])
    if use_modal:
        cmd.extend(["--modal", "true"])
    return _run(cmd, cwd=workdir)


def _is_swebench_pro_dataset(dataset_name: str) -> bool:
    normalized = dataset_name.strip().lower().replace("_", "-")
    return "swe-bench-pro" in normalized or "swebench-pro" in normalized


def _default_swepro_harness_dir() -> Path:
    configured = _first_env(["NOLA_SWEPRO_HARNESS_DIR", "SWEPRO_HARNESS_DIR"])
    if configured:
        return Path(configured).expanduser()
    return Path("/private/tmp/SWE-bench_Pro-os")


def _csv_scalar(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, (list, tuple, dict)):
        return repr(value)
    return str(value)


def _write_swepro_raw_csv(path: Path, instances: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fieldnames: list[str] = []
    seen: set[str] = set()
    for preferred in ("instance_id", "repo", "base_commit", "problem_statement", "fail_to_pass", "pass_to_pass"):
        for row in instances:
            if preferred in row and preferred not in seen:
                seen.add(preferred)
                fieldnames.append(preferred)
                break
    for row in instances:
        for key in row.keys():
            if key not in seen:
                seen.add(key)
                fieldnames.append(key)
    with path.open("w", encoding="utf-8", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=fieldnames)
        writer.writeheader()
        for row in instances:
            writer.writerow({key: _csv_scalar(row.get(key)) for key in fieldnames})


def _write_swepro_patch_json(path: Path, predictions: list[dict[str, Any]], prefix: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    rows = []
    for prediction in predictions:
        instance_id = str(prediction.get("instance_id") or "").strip()
        if not instance_id:
            continue
        patch = str(prediction.get("model_patch") or prediction.get("patch") or "")
        rows.append(
            {
                "instance_id": instance_id,
                "model_patch": patch,
                "patch": patch,
                "prefix": prefix,
            }
        )
    path.write_text(json.dumps(rows, ensure_ascii=False, indent=2), encoding="utf-8")


def _run_swepro_harness_eval(
    *,
    instances: list[dict[str, Any]],
    predictions: list[dict[str, Any]],
    work_dir: Path,
    harness_dir: Path,
    python_bin: str,
    max_workers: int,
    dockerhub_username: str,
    docker_platform: str,
    use_local_docker: bool,
    redo: bool,
) -> tuple[subprocess.CompletedProcess[str], Path]:
    eval_script = harness_dir / "swe_bench_pro_eval.py"
    scripts_dir = harness_dir / "run_scripts"
    if not eval_script.exists():
        raise RuntimeError(f"SWE-bench Pro harness script not found: {eval_script}")
    if not scripts_dir.exists():
        raise RuntimeError(f"SWE-bench Pro run_scripts directory not found: {scripts_dir}")

    raw_sample_path = work_dir / "swepro_raw.csv"
    patch_path = work_dir / "swepro_patches.json"
    output_dir = work_dir / "swepro_harness"
    _write_swepro_raw_csv(raw_sample_path, instances)
    _write_swepro_patch_json(patch_path, predictions, prefix="nola-cli")

    cmd = [
        python_bin,
        str(eval_script),
        "--raw_sample_path",
        str(raw_sample_path),
        "--patch_path",
        str(patch_path),
        "--output_dir",
        str(output_dir),
        "--scripts_dir",
        str(scripts_dir),
        "--num_workers",
        str(max(1, int(max_workers))),
        "--dockerhub_username",
        dockerhub_username,
    ]
    if use_local_docker:
        cmd.append("--use_local_docker")
    if docker_platform.strip():
        cmd.extend(["--docker_platform", docker_platform.strip()])
    if redo:
        cmd.append("--redo")
    return _run(cmd, cwd=harness_dir), output_dir


def _read_swepro_harness_result(output_dir: Path) -> dict[str, Any]:
    report_path = output_dir / "eval_results.json"
    if not report_path.exists():
        return {}
    try:
        results = json.loads(report_path.read_text(encoding="utf-8"))
    except Exception:
        return {}
    if not isinstance(results, dict):
        return {}
    total = len(results)
    resolved = sum(1 for value in results.values() if bool(value))
    return {
        "report_path": str(report_path),
        "resolved_instances": resolved,
        "unresolved_instances": total - resolved,
        "total_instances": total,
        "empty_patch_instances": 0,
        "error_instances": 0,
        "raw_results": results,
    }


def _read_harness_result(work_dir: Path, run_id: str) -> dict[str, Any]:
    report_path = work_dir / f"nola-cli.{run_id}.json"
    if not report_path.exists():
        return {}
    try:
        return json.loads(report_path.read_text(encoding="utf-8"))
    except Exception:
        return {}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Evaluate Nola CLI on SWE-bench Verified")
    parser.add_argument("--dataset-name", default="SWE-bench/SWE-bench_Verified")
    parser.add_argument("--split", default="test")
    parser.add_argument("--offset", type=int, default=0)
    parser.add_argument("--limit", type=int, default=50)
    parser.add_argument("--instance-id", action="append", default=[], help="Restrict execution to specified SWE-bench instance ids")
    parser.add_argument("--run-id", default=f"nola-cli-swebench-verified-{_now_ts()}")
    parser.add_argument("--work-dir", default="/tmp/nola_swebench_cli_eval")
    parser.add_argument("--release", action="store_true", default=True)
    parser.add_argument("--no-release", action="store_false", dest="release")
    parser.add_argument("--max-iterations", type=int, default=60)
    parser.add_argument(
        "--timeout-per-instance-sec",
        type=int,
        default=7200,
        help="Seconds to allow each CLI solve attempt; set 0 to disable the outer solve timeout.",
    )
    parser.add_argument(
        "--model-name",
        default=_first_env(
            [
                "NOLA_LLM_MODEL",
                "OPENROUTER_API_MODEL",
                "OPENROUTER_MODEL",
                "MODEL_API_MODEL",
                "MODEL_NAME",
                "DEEPSEEK_MODEL",
                "OPENAI_MODEL",
            ]
        ),
    )
    parser.add_argument(
        "--endpoint",
        default=_first_env(
            [
                "NOLA_LLM_ENDPOINT",
                "OPENROUTER_API_URL",
                "MODEL_API_URL",
                "DEEPSEEK_API_URL",
                "DEEPSEEK_BASE_URL",
                "OPENAI_BASE_URL",
            ]
        ),
    )
    parser.add_argument(
        "--api-key",
        default=_first_env(
            [
                "NOLA_LLM_API_KEY",
                "MODEL_API_KEY",
                "OPENROUTER_API_KEY",
                "DEEPSEEK_API_KEY",
                "OPENAI_API_KEY",
            ]
        ),
    )
    parser.add_argument("--bash-prefix", action="append", default=[])
    parser.add_argument("--open-test-folder-permissions", action="store_true", default=True)
    parser.add_argument("--no-open-test-folder-permissions", action="store_false", dest="open_test_folder_permissions")
    parser.add_argument("--skip-existing", action="store_true", default=True)
    parser.add_argument("--no-skip-existing", action="store_false", dest="skip_existing")
    parser.add_argument("--evaluate-with-harness", action="store_true", default=False)
    parser.add_argument(
        "--harness-kind",
        choices=["auto", "swebench", "swepro"],
        default="auto",
        help="Harness backend to use. auto selects SWE-bench Pro harness for SWE-bench Pro datasets.",
    )
    parser.add_argument("--harness-max-workers", type=int, default=4)
    parser.add_argument("--harness-timeout-sec", type=int, default=7200)
    parser.add_argument("--harness-modal", action="store_true", default=False)
    parser.add_argument(
        "--swepro-harness-dir",
        default=str(_default_swepro_harness_dir()),
        help="Path to the official SWE-bench Pro harness checkout.",
    )
    parser.add_argument(
        "--swepro-python",
        default=_first_env(["NOLA_SWEPRO_PYTHON", "SWEPRO_PYTHON"]) or sys.executable,
        help="Python executable used to run the SWE-bench Pro harness.",
    )
    parser.add_argument(
        "--swepro-dockerhub-username",
        default=_first_env(["NOLA_SWEPRO_DOCKERHUB_USERNAME", "SWEPRO_DOCKERHUB_USERNAME"]) or "jefzda",
        help="Docker Hub username for SWE-bench Pro images.",
    )
    parser.add_argument(
        "--swepro-docker-platform",
        default=_first_env(["NOLA_SWEPRO_DOCKER_PLATFORM", "SWEPRO_DOCKER_PLATFORM"]) or "",
        help="Optional Docker platform override for SWE-bench Pro, e.g. linux/amd64.",
    )
    parser.add_argument(
        "--swepro-use-modal",
        action="store_true",
        default=False,
        help="Use Modal for SWE-bench Pro harness instead of local Docker.",
    )
    parser.add_argument(
        "--swepro-no-redo",
        action="store_true",
        default=False,
        help="Do not pass --redo to the SWE-bench Pro harness.",
    )
    parser.add_argument("--local-test-timeout-sec", type=int, default=7200)
    parser.add_argument("--refine-attempts", type=int, default=2)
    parser.add_argument(
        "--init-project-context",
        action="store_true",
        default=True,
        help="Run `nola init` in each checkout before solving so NOLA.md/wiki guidance is available to the agent.",
    )
    parser.add_argument(
        "--no-init-project-context",
        action="store_false",
        dest="init_project_context",
        help="Skip per-checkout NOLA.md/wiki generation before solving.",
    )
    parser.add_argument("--init-timeout-sec", type=int, default=7200)
    parser.add_argument(
        "--llm-http-timeout-sec",
        type=int,
        default=7200,
        help="NOLA_LLM_HTTP_TIMEOUT_SEC passed to CLI attempts; set 0 to disable reqwest HTTP timeout.",
    )
    parser.add_argument(
        "--llm-stream-idle-timeout-sec",
        type=int,
        default=7200,
        help="NOLA_LLM_STREAM_IDLE_TIMEOUT_SEC passed to CLI attempts.",
    )
    parser.add_argument(
        "--llm-stream-tail-timeout-sec",
        type=int,
        default=0,
        help="NOLA_LLM_STREAM_TAIL_IDLE_TIMEOUT_SEC passed to CLI attempts; 0 uses min(stream idle, 120).",
    )
    parser.add_argument(
        "--llm-stream-first-event-timeout-sec",
        type=int,
        default=0,
        help="NOLA_LLM_STREAM_FIRST_EVENT_TIMEOUT_SEC passed to CLI attempts; 0 uses min(stream idle, 120).",
    )
    parser.add_argument(
        "--llm-max-tokens",
        type=int,
        default=int(_first_env(["NOLA_LLM_MAX_TOKENS", "NOLA_CLI_LLM_MAX_TOKENS"]) or "0"),
        help="Optional NOLA_LLM_MAX_TOKENS passed to CLI attempts; 0 keeps the CLI/provider default.",
    )
    parser.add_argument(
        "--agent-context-token-threshold",
        type=int,
        default=int(_first_env(["NOLA_AGENT_CONTEXT_TOKEN_THRESHOLD"]) or "0"),
        help="Optional NOLA_AGENT_CONTEXT_TOKEN_THRESHOLD passed to CLI attempts; 0 keeps provider-derived default.",
    )
    parser.add_argument(
        "--llm-retry-attempts",
        type=int,
        default=3,
        help="Retry a CLI attempt when the model request fails before marking the instance inconclusive.",
    )
    parser.add_argument("--llm-retry-initial-backoff-sec", type=float, default=10.0)
    parser.add_argument("--llm-retry-max-backoff-sec", type=float, default=120.0)
    parser.add_argument("--skip-local-tests", action="store_true", default=False)
    parser.add_argument(
        "--agent-mode",
        choices=["auto", "swe", "standard"],
        default="auto",
        help="Run swe issue mode for SWE tasks or standard mode when set.",
    )
    parser.add_argument(
        "--network-policy",
        default=_normalize_network_policy(_first_env(["NOLA_CLI_NETWORK_POLICY", "NOLA_NETWORK_POLICY"])),
        choices=["allow", "ask", "deny"],
        help="Pass-through to CLI: allow, ask, deny.",
    )
    parser.add_argument(
        "--network-check-timeout-sec",
        type=int,
        default=4,
        help="Seconds to wait when probing LLM endpoint connectivity.",
    )
    parser.add_argument(
        "--skip-git-network-check",
        action="store_true",
        default=False,
        help="Skip the GitHub git preflight check before running instances.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    repo_root = Path(__file__).resolve().parents[1]
    cli_dir = repo_root / "cli"
    requested_agent_mode = args.agent_mode.strip().lower()
    if requested_agent_mode == "auto":
        requested_agent_mode = "swe" if "swe" in args.dataset_name.lower() else "standard"
    network_policy = _normalize_network_policy(str(args.network_policy or "allow"))
    network_check_timeout = max(1, int(args.network_check_timeout_sec))

    endpoint = _normalize_chat_endpoint(str(args.endpoint or ""))
    if not endpoint:
        print("ERROR: missing endpoint. Set --endpoint or MODEL_API_URL/NOLA_LLM_ENDPOINT.", file=sys.stderr)
        return 2

    endpoint_host = ""
    try:
        endpoint_host, endpoint_port = _normalize_network_probe_endpoint(endpoint)
        _ = endpoint_port
    except Exception:
        endpoint_host = ""
    env_with_dns_fallback = dict(os.environ)
    if endpoint_host:
        _append_dns_fallbacks(env_with_dns_fallback, endpoint_host)

    work_dir = Path(args.work_dir).expanduser().resolve() / args.run_id
    repos_dir = work_dir / "repos_cache"
    instances_dir = work_dir / "instances"
    runs_dir = work_dir / "runs"
    summary_path = work_dir / "summary.json"
    predictions_path = work_dir / "predictions.jsonl"
    runs_dir.mkdir(parents=True, exist_ok=True)

    print(f"[1/6] Loading dataset {args.dataset_name} ({args.split}) offset={args.offset} limit={args.limit}")
    instances = _load_dataset_instances(args.dataset_name, args.split, args.offset, args.limit)
    instances = _filter_instance_subset(instances, args.instance_id)
    if not instances:
        print("No instances selected.")
        return 0
    print(f"Selected {len(instances)} instances.")

    if network_policy == "allow":
        endpoint_access_available = True
        endpoint_access_error = ""
    elif network_policy == "deny":
        endpoint_access_error = "network usage disabled by --network-policy"
        endpoint_access_available = False
    else:
        try:
            endpoint_access_available, endpoint_access_error = _probe_network_endpoint(
                endpoint, timeout_sec=network_check_timeout
            )
        except Exception as exc:
            endpoint_access_available = False
            endpoint_access_error = str(exc)
        if (not endpoint_access_available) and (not sys.stdin.isatty()):
            print("[network] non-interactive run with --network-policy=ask: forcing allow for batch mode.")
            network_policy = "allow"
            endpoint_access_available = True
            endpoint_access_error = ""
        if not endpoint_access_available and _ask_network_access(endpoint, endpoint_access_error):
            network_policy = "allow"
            endpoint_access_available = True
            endpoint_access_error = ""

    if not endpoint_access_available:
        print(f"[network] model calls disabled for this run (reason: {endpoint_access_error})")

    if network_policy != "deny" and not args.skip_git_network_check:
        first_repo = next((str(row.get("repo") or "").strip() for row in instances if row.get("repo")), "")
        if first_repo:
            git_ok, git_error = _probe_git_remote(first_repo, timeout_sec=max(4, int(args.network_check_timeout_sec)))
            if not git_ok:
                print(
                    "[network] GitHub git access is unavailable; aborting before running instances.\n"
                    f"[network] probe repo: {first_repo}\n"
                    f"[network] error: {git_error}\n"
                    "[network] This check guards against producing empty patches from stale or unreachable repo caches. "
                    "Run outside the execution sandbox or fix DNS/network access, then retry."
                )
                return 2

    print(f"[2/6] Building CLI binary (release={args.release})")
    cli_binary = _build_cli_binary(cli_dir, args.release)
    print(f"CLI binary: {cli_binary}")
    effective_bash_prefixes = list(args.bash_prefix or [])
    if args.open_test_folder_permissions:
        for prefix in DEFAULT_TEST_BASH_PREFIXES:
            if prefix not in effective_bash_prefixes:
                effective_bash_prefixes.append(prefix)

    predictions: list[dict[str, Any]] = []
    per_instance_results: list[dict[str, Any]] = []

    print("[3/6] Running instances")
    for idx, instance in enumerate(instances, start=1):
        instance_id = str(instance.get("instance_id") or "").strip()
        repo = str(instance.get("repo") or "").strip()
        base_commit = str(instance.get("base_commit") or "").strip()
        if not instance_id or not repo or not base_commit:
            print(f"[{idx}/{len(instances)}] skip invalid row: instance_id/repo/base_commit missing")
            continue

        run_dir = runs_dir / instance_id
        stdout_path = run_dir / "cli_stdout.jsonl"
        stderr_path = run_dir / "cli_stderr.txt"
        patch_path = run_dir / "model_patch.diff"
        meta_path = run_dir / "meta.json"
        if args.skip_existing and meta_path.exists() and patch_path.exists():
            try:
                meta = json.loads(meta_path.read_text(encoding="utf-8"))
            except Exception:
                meta = {}
            predictions.append(
                {
                    "instance_id": instance_id,
                    "model_name_or_path": "nola-cli",
                    "model_patch": patch_path.read_text(encoding="utf-8"),
                }
            )
            per_instance_results.append(meta)
            print(f"[{idx}/{len(instances)}] {instance_id} reused existing artifacts")
            continue

        if not endpoint_access_available:
            run_start = time.time()
            error_payload = {
                "instance_id": instance_id,
                "repo": repo,
                "base_commit": base_commit,
                "exit_code": -1,
                "timeout": False,
                "elapsed_sec": time.time() - run_start,
                "done_reason": "network_unavailable",
                "tool_use_count": 0,
                "tool_error_count": 0,
                "text_event_count": 0,
                "tool_read_tool_count": 0,
                "tool_write_tool_count": 0,
                "tool_use_name_count": 0,
                "tool_uses": [],
                "patch_chars": 0,
                "patch_nonempty": False,
                "patch_path": str(patch_path),
                "stdout_path": str(stdout_path),
                "stderr_path": str(stderr_path),
                "meta_path": str(meta_path),
                "local_test_attempts": 0,
                "local_tests_passed": False,
                "local_test_failed_count": 0,
                "local_tests_inconclusive": True,
                "local_test_failure_type": "network_unavailable",
            }
            run_dir.mkdir(parents=True, exist_ok=True)
            patch_path.write_text("", encoding="utf-8")
            meta_path.write_text(json.dumps(error_payload, ensure_ascii=False, indent=2), encoding="utf-8")
            per_instance_results.append(error_payload)
            predictions.append(
                {
                    "instance_id": instance_id,
                    "model_name_or_path": "nola-cli",
                    "model_patch": "",
                }
            )
            print(
                f"[{idx}/{len(instances)}] {instance_id} skipped due to network unavailable: {endpoint_access_error}"
            )
            continue

        print(f"[{idx}/{len(instances)}] {instance_id} ({repo}@{base_commit[:12]})")
        run_dir.mkdir(parents=True, exist_ok=True)
        try:
            run_start = time.time()
            timeout_hit = False

            repo_cache = _ensure_repo_cache(repo, repos_dir)
            instance_repo = instances_dir / instance_id
            _prepare_instance_checkout(repo_cache, instance_repo, base_commit)
            base_cli_env = dict(env_with_dns_fallback)
            if args.open_test_folder_permissions:
                base_cli_env["NOLA_TEST_FOLDER_PERMISSIVE"] = "1"
                base_cli_env["NOLA_CLI_SANDBOX_BASH_PERMISSIVE"] = "1"
            if int(args.llm_max_tokens) > 0:
                base_cli_env["NOLA_LLM_MAX_TOKENS"] = str(max(256, int(args.llm_max_tokens)))
            if int(args.agent_context_token_threshold) > 0:
                base_cli_env["NOLA_AGENT_CONTEXT_TOKEN_THRESHOLD"] = str(
                    max(1_000, int(args.agent_context_token_threshold))
                )
            base_cli_env["NOLA_LLM_HTTP_TIMEOUT_SEC"] = str(max(0, int(args.llm_http_timeout_sec)))
            base_cli_env["NOLA_LLM_STREAM_IDLE_TIMEOUT_SEC"] = str(
                max(1, int(args.llm_stream_idle_timeout_sec))
            )
            first_event_timeout_sec = int(args.llm_stream_first_event_timeout_sec)
            if first_event_timeout_sec <= 0:
                first_event_timeout_sec = min(max(1, int(args.llm_stream_idle_timeout_sec)), 120)
            base_cli_env["NOLA_LLM_STREAM_FIRST_EVENT_TIMEOUT_SEC"] = str(
                max(1, first_event_timeout_sec)
            )
            tail_timeout_sec = int(args.llm_stream_tail_timeout_sec)
            if tail_timeout_sec <= 0:
                tail_timeout_sec = min(max(1, int(args.llm_stream_idle_timeout_sec)), 120)
            base_cli_env["NOLA_LLM_STREAM_TAIL_IDLE_TIMEOUT_SEC"] = str(
                max(1, tail_timeout_sec)
            )
            if int(args.timeout_per_instance_sec) <= 0:
                base_cli_env["NOLA_LLM_HTTP_TIMEOUT_SEC"] = "0"
            _append_dns_fallbacks(base_cli_env, _normalize_network_endpoint_host(endpoint)[0])
            context_init_ok = False
            context_init_reason = "disabled"
            if bool(args.init_project_context) and endpoint_access_available:
                context_init_ok, context_init_reason = _run_project_context_init(
                    cli_binary=cli_binary,
                    instance_repo=instance_repo,
                    endpoint=endpoint,
                    model_name=str(args.model_name or ""),
                    api_key=str(args.api_key or ""),
                    network_policy=network_policy,
                    env=base_cli_env,
                    timeout_sec=max(1, int(args.init_timeout_sec)),
                )
                if context_init_ok:
                    print(f"    init context: {context_init_reason}")
                else:
                    print(f"    init context skipped/failed: {context_init_reason}")
            base_prompt = _build_prompt(instance)
            fail_to_pass_tests = _extract_fail_to_pass_tests(instance)
            pass_to_pass_tests = _extract_pass_to_pass_tests(instance)
            max_attempts = max(1, int(args.refine_attempts) + 1)
            local_test_attempts = 0
            local_tests_passed = False
            local_test_failed_count = 0
            local_tests_inconclusive = False
            local_test_failure_type = ""
            failed_tests: list[str] = []
            test_output = ""
            env_repaired = False
            no_write_streak = 0
            empty_patch_streak = 0
            attempt_done_reason = ""
            attempt_tool_use_count = 0
            attempt_tool_error_count = 0
            attempt_text_event_count = 0
            attempt_tool_uses: list[str] = []
            attempt_read_tool_count = 0
            attempt_write_tool_count = 0
            attempt_tool_name_count = 0
            total_tool_use_count = 0
            total_tool_error_count = 0
            total_read_tool_count = 0
            total_write_tool_count = 0
            total_tool_uses: list[str] = []
            latest_patch_chars = 0
            latest_patch_text = ""

            final_proc: subprocess.CompletedProcess[str] | None = None
            final_stdout_text = ""
            final_stderr_text = ""
            llm_retry_count = 0

            for attempt in range(1, max_attempts + 1):
                test_candidates = _local_test_candidates(
                    fail_to_pass_tests=fail_to_pass_tests,
                    pass_to_pass_tests=pass_to_pass_tests,
                    prior_failed_tests=failed_tests if attempt > 1 else [],
                    attempt=attempt,
                )
                failure_snippet = _extract_failure_snippet(test_output)
                force_write_retry = no_write_streak >= 1 or empty_patch_streak >= 1

                if attempt > 1:
                    require_write = force_write_retry
                    prompt = _build_refine_prompt(
                        base_prompt,
                        failed_tests,
                        test_output,
                        attempt,
                        patch_path=patch_path,
                        selected_tests=test_candidates,
                        test_output_snippet=failure_snippet,
                        require_write=require_write,
                    )
                else:
                    prompt = base_prompt
                attempt_model_name = str(args.model_name or "")
                if attempt > 1 and force_write_retry:
                    fallback_model = _fallback_tooling_model(attempt_model_name, endpoint)
                    if fallback_model:
                        print(
                            f"    attempt {attempt}/{max_attempts} switching model {attempt_model_name} -> {fallback_model} for write-focused retry"
                        )
                        attempt_model_name = fallback_model

                cli_cmd = [
                    str(cli_binary),
                    "run",
                    "--json",
                    "--yes",
                    "--no-save",
                    "--endpoint",
                    endpoint,
                    "--model",
                    attempt_model_name,
                    "--allow-dir",
                    str(instance_repo),
                    "--max-iterations",
                    str(args.max_iterations),
                ]
                if requested_agent_mode in {"swe", "standard"}:
                    cli_cmd.extend(["--agent-mode", requested_agent_mode])
                if network_policy in {"allow", "ask", "deny"}:
                    cli_cmd.extend(["--network-policy", network_policy])
                if str(args.api_key or "").strip():
                    cli_cmd.extend(["--api-key", str(args.api_key).strip()])
                for prefix in effective_bash_prefixes:
                    cli_cmd.extend(["--bash-prefix", prefix])

                env = dict(base_cli_env)

                solve_timeout_sec = (
                    None
                    if int(args.timeout_per_instance_sec) <= 0
                    else int(args.timeout_per_instance_sec)
                )
                llm_attempts = max(1, int(args.llm_retry_attempts))
                initial_backoff = max(0.0, float(args.llm_retry_initial_backoff_sec))
                max_backoff = max(initial_backoff, float(args.llm_retry_max_backoff_sec))
                for llm_try in range(1, llm_attempts + 1):
                    try:
                        final_proc = _run(
                            cli_cmd,
                            cwd=instance_repo,
                            env=env,
                            input_text=prompt,
                            timeout_sec=solve_timeout_sec,
                        )
                    except subprocess.TimeoutExpired as timeout_exc:
                        timeout_hit = True
                        final_proc = subprocess.CompletedProcess(
                            args=timeout_exc.cmd,
                            returncode=124,
                            stdout=_coerce_text(timeout_exc.stdout),
                            stderr=_coerce_text(timeout_exc.stderr) or "timeout",
                        )

                    final_stdout_text = _coerce_text(final_proc.stdout)
                    final_stderr_text = _coerce_text(final_proc.stderr)
                    if not _is_llm_request_error(final_stdout_text):
                        break
                    if llm_try >= llm_attempts:
                        break
                    llm_retry_count += 1
                    backoff_sec = min(max_backoff, initial_backoff * (2 ** (llm_try - 1)))
                    print(
                        f"    attempt {attempt}/{max_attempts} LLM request failed; "
                        f"retrying model call {llm_try + 1}/{llm_attempts} after {backoff_sec:.1f}s"
                    )
                    if backoff_sec > 0:
                        time.sleep(backoff_sec)

                (
                    attempt_done_reason,
                    attempt_tool_use_count,
                    attempt_tool_error_count,
                    attempt_text_event_count,
                    attempt_tool_uses,
                    attempt_read_tool_count,
                    attempt_write_tool_count,
                    attempt_tool_name_count,
                ) = _parse_cli_events(final_stdout_text)
                total_tool_use_count += attempt_tool_use_count
                total_tool_error_count += attempt_tool_error_count
                total_read_tool_count += attempt_read_tool_count
                total_write_tool_count += attempt_write_tool_count
                total_tool_uses.extend(attempt_tool_uses)
                if _is_llm_request_error(final_stdout_text):
                    attempt_done_reason = "llm_request_error"
                    local_tests_passed = False
                    local_tests_inconclusive = True
                    local_test_failure_type = "llm_request_error"
                    failed_tests = ["<llm_request_error>"]
                    print(
                        f"    attempt {attempt}/{max_attempts} LLM request failed after "
                        f"{max(1, int(args.llm_retry_attempts))} try/tries; aborting retries for this instance"
                    )
                    break
                if attempt_write_tool_count <= 0:
                    no_write_streak += 1
                    if no_write_streak == 1:
                        print(f"    attempt {attempt}/{max_attempts} had no write/edit tool calls")
                    if attempt >= max_attempts and latest_patch_chars <= 0:
                        attempt_done_reason = "no_write_tool_calls"
                        local_tests_passed = False
                        local_tests_inconclusive = True
                        local_test_failure_type = "no_write_tool_calls"
                        failed_tests = ["<no_write_tool_calls>"]
                        print(
                            f"    attempt {attempt}/{max_attempts} reached write-tool limit without edit calls; stopping retries"
                        )
                        break
                else:
                    no_write_streak = 0

                if args.skip_local_tests or not (fail_to_pass_tests or pass_to_pass_tests):
                    local_tests_passed = True
                    local_test_failed_count = 0
                    local_tests_inconclusive = False
                    local_test_failure_type = ""
                    failed_tests = []
                    if attempt_write_tool_count <= 0 and no_write_streak >= 1 and attempt < max_attempts:
                        print(
                            f"    attempt {attempt}/{max_attempts} no write action; retrying with salvage prompt"
                        )
                        continue
                    break

                local_test_attempts = attempt
                (
                    local_tests_passed,
                    local_test_failed_count,
                    failed_tests,
                    test_output,
                    local_tests_inconclusive,
                    local_test_failure_type,
                ) = _run_local_tests(
                    instance_repo,
                    test_candidates,
                    timeout_sec=args.local_test_timeout_sec,
                )
                (run_dir / f"local_tests_attempt_{attempt}.txt").write_text(
                    test_output,
                    encoding="utf-8",
                )
                patch_text = _collect_patch(instance_repo)
                patch_chars = len(patch_text)
                latest_patch_chars = patch_chars
                latest_patch_text = patch_text
                if patch_chars == 0:
                    empty_patch_streak += 1
                else:
                    empty_patch_streak = 0
                if local_tests_passed:
                    if patch_chars == 0 and attempt < max_attempts:
                        print(
                            f"    attempt {attempt}/{max_attempts} tests pass but no effective patch; retrying with write-required prompt"
                        )
                        continue
                    break
                if local_tests_inconclusive:
                    if patch_chars == 0 and attempt < max_attempts:
                        print(
                            f"    attempt {attempt}/{max_attempts} local tests inconclusive ({local_test_failure_type}); "
                            "retrying with write-required prompt because no effective patch was produced"
                        )
                        continue
                    if patch_chars > 0 and local_test_failure_type in _LOCAL_TEST_KEEP_PATCH_INCONCLUSIVE_TYPES:
                        print(
                            f"    attempt {attempt}/{max_attempts} local tests inconclusive ({local_test_failure_type}); "
                            "keeping non-empty patch for harness evaluation"
                        )
                        break
                    if local_test_failure_type == "environment" and not env_repaired:
                        if _run_env_fix(instance_repo):
                            env_repaired = True
                            print(
                                f"    attempt {attempt}/{max_attempts} environment issue detected; rerunning local tests"
                            )
                            (
                                local_tests_passed,
                                local_test_failed_count,
                                failed_tests,
                                test_output,
                                local_tests_inconclusive,
                                local_test_failure_type,
                            ) = _run_local_tests(
                                instance_repo,
                                test_candidates,
                                timeout_sec=args.local_test_timeout_sec,
                            )
                            if local_tests_passed:
                                latest_patch_chars = patch_chars
                                if patch_chars == 0 and attempt < max_attempts:
                                    print(
                                        f"    attempt {attempt}/{max_attempts} post-fix tests pass but still no effective patch; retrying"
                                    )
                                    continue
                                break
                            if local_tests_inconclusive:
                                if patch_chars == 0 and attempt < max_attempts:
                                    print(
                                        f"    attempt {attempt}/{max_attempts} local tests inconclusive ({local_test_failure_type}); "
                                        "retrying with write-required prompt because no effective patch was produced"
                                    )
                                    continue
                                if patch_chars > 0 and local_test_failure_type in _LOCAL_TEST_KEEP_PATCH_INCONCLUSIVE_TYPES:
                                    print(
                                        f"    attempt {attempt}/{max_attempts} local tests inconclusive ({local_test_failure_type}); "
                                        "keeping non-empty patch for harness evaluation"
                                    )
                                    break
                                if attempt >= max_attempts:
                                    print(
                                        f"    attempt {attempt}/{max_attempts} local tests inconclusive ({local_test_failure_type}); "
                                        "max attempts reached, keeping final patch state"
                                    )
                                    break
                                print(
                                    f"    attempt {attempt}/{max_attempts} local tests inconclusive ({local_test_failure_type}); "
                                    "retrying model with refined failure context"
                                )
                                continue
                if local_tests_passed:
                    latest_patch_chars = patch_chars
                    if patch_chars == 0 and attempt < max_attempts:
                        print(
                            f"    attempt {attempt}/{max_attempts} post-fix tests pass but still no effective patch; retrying"
                        )
                        continue
                    break
                if local_tests_inconclusive:
                    if patch_chars == 0 and attempt < max_attempts:
                        print(
                            f"    attempt {attempt}/{max_attempts} local tests inconclusive ({local_test_failure_type}); "
                            "retrying with write-required prompt because no effective patch was produced"
                        )
                        continue
                    if latest_patch_chars > 0 and local_test_failure_type in _LOCAL_TEST_KEEP_PATCH_INCONCLUSIVE_TYPES:
                        print(
                            f"    attempt {attempt}/{max_attempts} local tests inconclusive ({local_test_failure_type}); "
                            "keeping non-empty patch for harness evaluation"
                        )
                        break
                    if attempt >= max_attempts:
                        print(
                            f"    attempt {attempt}/{max_attempts} local tests inconclusive ({local_test_failure_type}); "
                            "max attempts reached, keeping final patch state"
                        )
                        break
                    print(
                        f"    attempt {attempt}/{max_attempts} local tests inconclusive ({local_test_failure_type}); "
                        "retrying model with refined failure context"
                    )
                    continue
                if attempt >= max_attempts:
                    break

                print(
                    f"    attempt {attempt}/{max_attempts} local tests still failing: {local_test_failed_count} test(s); "
                    "re-running model with targeted failure context"
                )

            proc = final_proc if final_proc is not None else subprocess.CompletedProcess(args=[], returncode=-1, stdout="", stderr="")
            stdout_text = final_stdout_text
            stderr_text = final_stderr_text
            if not latest_patch_text:
                patch_text = _collect_patch(instance_repo)
                latest_patch_chars = len(patch_text)
            else:
                patch_text = latest_patch_text
            elapsed = time.time() - run_start
            cli_done_reason = attempt_done_reason
            done_reason = attempt_done_reason
            if (
                done_reason == "validated"
                and not bool(local_tests_passed)
                and not bool(local_tests_inconclusive)
            ):
                done_reason = "validated_but_local_tests_failed"
            tool_use_count = total_tool_use_count or attempt_tool_use_count
            tool_error_count = total_tool_error_count or attempt_tool_error_count
            text_event_count = attempt_text_event_count
            tool_read_count = total_read_tool_count or attempt_read_tool_count
            tool_write_count = total_write_tool_count or attempt_write_tool_count
            tool_name_count = len(total_tool_uses) or attempt_tool_name_count
            tool_uses = total_tool_uses or attempt_tool_uses

            stdout_path.write_text(stdout_text, encoding="utf-8")
            stderr_path.write_text(stderr_text, encoding="utf-8")
            patch_path.write_text(patch_text, encoding="utf-8")

            result = InstanceRunResult(
                instance_id=instance_id,
                repo=repo,
                base_commit=base_commit,
                exit_code=int(proc.returncode),
                timeout=timeout_hit,
                elapsed_sec=elapsed,
                done_reason=done_reason,
                cli_done_reason=cli_done_reason,
                tool_use_count=tool_use_count,
                tool_error_count=tool_error_count,
                text_event_count=text_event_count,
                tool_read_tool_count=tool_read_count,
                tool_write_tool_count=tool_write_count,
                tool_use_name_count=tool_name_count,
                tool_uses=tool_uses,
                patch_chars=len(patch_text),
                patch_nonempty=bool(patch_text.strip()),
                patch_path=str(patch_path),
                stdout_path=str(stdout_path),
                stderr_path=str(stderr_path),
                meta_path=str(meta_path),
                local_test_attempts=local_test_attempts,
                local_tests_passed=bool(local_tests_passed),
                local_test_failed_count=int(local_test_failed_count),
                local_tests_inconclusive=bool(local_tests_inconclusive),
                local_test_failure_type=str(local_test_failure_type),
            )
            result_dict = dataclasses.asdict(result)
            result_dict["llm_retry_count"] = llm_retry_count
            result_dict["context_init_ok"] = context_init_ok
            result_dict["context_init_reason"] = context_init_reason
            meta_path.write_text(json.dumps(result_dict, ensure_ascii=False, indent=2), encoding="utf-8")
            per_instance_results.append(result_dict)
            predictions.append(
                {
                    "instance_id": instance_id,
                    "model_name_or_path": "nola-cli",
                    "model_patch": patch_text,
                }
            )
            print(
                f"    done={done_reason or '-'} exit={proc.returncode} timeout={timeout_hit} "
                f"patch_chars={len(patch_text)} tool_uses={tool_use_count} write_uses={tool_write_count} "
                f"tool_errors={tool_error_count} "
                f"local_tests_passed={local_tests_passed} local_tests_inconclusive={local_tests_inconclusive} "
                f"local_tests_attempts={local_test_attempts}"
            )
        except Exception as exc:  # pragma: no cover - runtime safeguard
            elapsed = time.time() - run_start
            error_payload = {
                "instance_id": instance_id,
                "repo": repo,
                "base_commit": base_commit,
                "exit_code": -1,
                "timeout": False,
                "elapsed_sec": elapsed,
                "error": str(exc),
                "patch_chars": 0,
                "patch_nonempty": False,
                "local_test_attempts": 0,
                "local_tests_passed": False,
                "local_test_failed_count": 0,
                "local_tests_inconclusive": True,
                "local_test_failure_type": "error",
                "patch_path": str(patch_path),
                "stdout_path": str(stdout_path),
                "stderr_path": str(stderr_path),
                "meta_path": str(meta_path),
            }
            meta_path.write_text(json.dumps(error_payload, ensure_ascii=False, indent=2), encoding="utf-8")
            per_instance_results.append(error_payload)
            predictions.append(
                {
                    "instance_id": instance_id,
                    "model_name_or_path": "nola-cli",
                    "model_patch": "",
                }
            )
            print(f"    ERROR: {exc}")

        _write_jsonl(predictions_path, predictions)

    print("[4/6] Writing predictions and summary")
    _write_jsonl(predictions_path, predictions)

    normalized_results: list[dict[str, Any]] = []
    for row in per_instance_results:
        (
            tool_use_count_i,
            tool_error_count_i,
            tool_read_count_i,
            tool_write_count_i,
            tool_uses_i,
        ) = _row_tool_counts(row)
        row["tool_use_count"] = tool_use_count_i
        row["tool_error_count"] = tool_error_count_i
        row["tool_read_tool_count"] = tool_read_count_i
        row["tool_write_tool_count"] = tool_write_count_i
        row["tool_use_name_count"] = len(tool_uses_i)
        row["tool_uses"] = tool_uses_i
        if not row.get("done_reason"):
            exit_code = row.get("exit_code")
            if bool(row.get("timeout")):
                row["done_reason"] = "timeout"
            elif exit_code is None:
                row["done_reason"] = "none"
            elif int(exit_code) == 0:
                row["done_reason"] = "stop"
            else:
                row["done_reason"] = "error"
        normalized_results.append(row)

    per_instance_results = normalized_results

    solved_like = sum(1 for row in per_instance_results if bool(row.get("patch_nonempty")))
    empty_patch_count = sum(1 for row in per_instance_results if not bool(row.get("patch_nonempty")))
    local_test_inconclusive_count = sum(1 for row in per_instance_results if bool(row.get("local_tests_inconclusive")))
    no_write_tool_count = sum(
        1 for row in per_instance_results if int(row.get("tool_write_tool_count") or 0) == 0
    )
    final_max_iter_no_edit_count = sum(
        1
        for row in per_instance_results
        if str(row.get("done_reason") or "") == "max_iter"
        and int(row.get("tool_write_tool_count") or 0) == 0
    )
    final_max_iter_empty_patch_count = sum(
        1
        for row in per_instance_results
        if str(row.get("done_reason") or "") == "max_iter"
        and not bool(row.get("patch_nonempty"))
    )
    local_test_failure_type_counts: dict[str, int] = {}
    for row in per_instance_results:
        failure_type = str(row.get("local_test_failure_type") or "")
        if not failure_type:
            continue
        local_test_failure_type_counts[failure_type] = local_test_failure_type_counts.get(failure_type, 0) + 1
    timeout_count = sum(1 for row in per_instance_results if bool(row.get("timeout")))
    done_timeout_count = sum(1 for row in per_instance_results if str(row.get("done_reason") or "") == "timeout")
    terminal_done_reasons = {"stop", "validated", "repeated_tool_call_blocked", "max_iter"}
    successful_done_reasons = {"stop", "validated"}
    done_stop_count = sum(1 for row in per_instance_results if str(row.get("done_reason") or "") == "stop")
    done_validated_count = sum(
        1 for row in per_instance_results if str(row.get("done_reason") or "") == "validated"
    )
    done_validated_but_local_tests_failed_count = sum(
        1
        for row in per_instance_results
        if str(row.get("done_reason") or "") == "validated_but_local_tests_failed"
    )
    done_repeated_tool_call_blocked_count = sum(
        1
        for row in per_instance_results
        if str(row.get("done_reason") or "") == "repeated_tool_call_blocked"
    )
    done_max_iter_count = sum(1 for row in per_instance_results if str(row.get("done_reason") or "") == "max_iter")
    non_timeout_done_rows = [
        row
        for row in per_instance_results
        if (not bool(row.get("timeout"))) and str(row.get("done_reason") or "") in terminal_done_reasons
    ]
    non_timeout_done_count = len(non_timeout_done_rows)
    stop_in_non_timeout_done_count = sum(
        1 for row in non_timeout_done_rows if str(row.get("done_reason") or "") in successful_done_reasons
    )
    successful_done_count = done_stop_count + done_validated_count
    stop_rate_overall = (successful_done_count / len(per_instance_results)) if per_instance_results else 0.0
    stop_rate_in_non_timeout_done = (
        (stop_in_non_timeout_done_count / non_timeout_done_count) if non_timeout_done_count else 0.0
    )
    done_reasons: dict[str, int] = {}
    for row in per_instance_results:
        key = str(row.get("done_reason") or "none")
        done_reasons[key] = done_reasons.get(key, 0) + 1
    summary = {
        "run_id": args.run_id,
        "dataset_name": args.dataset_name,
        "split": args.split,
        "offset": args.offset,
        "limit": args.limit,
        "selected_instances": len(instances),
        "evaluated_instances": len(per_instance_results),
        "nonempty_patch_count": solved_like,
        "local_test_inconclusive_count": local_test_inconclusive_count,
        "empty_patch_count": empty_patch_count,
        "no_write_tool_count": no_write_tool_count,
        "done_max_iter_no_write_count": final_max_iter_no_edit_count,
        "done_max_iter_empty_patch_count": final_max_iter_empty_patch_count,
        "local_test_failure_type_counts": local_test_failure_type_counts,
        "timeout_count": timeout_count,
        "done_timeout_count": done_timeout_count,
        "done_stop_count": done_stop_count,
        "done_validated_count": done_validated_count,
        "done_validated_but_local_tests_failed_count": done_validated_but_local_tests_failed_count,
        "done_repeated_tool_call_blocked_count": done_repeated_tool_call_blocked_count,
        "done_max_iter_count": done_max_iter_count,
        "successful_done_count": successful_done_count,
        "done_reasons": done_reasons,
        "non_timeout_done_count": non_timeout_done_count,
        "stop_in_non_timeout_done_count": stop_in_non_timeout_done_count,
        "stop_rate_overall": stop_rate_overall,
        "stop_rate_in_non_timeout_done": stop_rate_in_non_timeout_done,
        "non_timeout_done_instance_ids": [
            str(row.get("instance_id") or "") for row in non_timeout_done_rows if str(row.get("instance_id") or "")
        ],
        "predictions_path": str(predictions_path),
        "work_dir": str(work_dir),
        "endpoint": endpoint,
        "model_name": str(args.model_name or ""),
        "agent_mode": requested_agent_mode,
        "open_test_folder_permissions": bool(args.open_test_folder_permissions),
        "bash_prefix_count": len(effective_bash_prefixes),
        "network_policy": network_policy,
        "endpoint_access_available": bool(endpoint_access_available),
        "network_check_timeout_sec": network_check_timeout,
        "network_access_error": endpoint_access_error if not endpoint_access_available else "",
        "init_project_context": bool(args.init_project_context),
        "context_init_ok_count": sum(1 for row in per_instance_results if bool(row.get("context_init_ok"))),
        "context_init_failed_count": sum(
            1
            for row in per_instance_results
            if bool(args.init_project_context) and not bool(row.get("context_init_ok"))
        ),
        "llm_http_timeout_sec": max(0, int(args.llm_http_timeout_sec)),
        "llm_stream_idle_timeout_sec": max(1, int(args.llm_stream_idle_timeout_sec)),
        "llm_stream_first_event_timeout_sec": (
            max(1, int(args.llm_stream_first_event_timeout_sec))
            if int(args.llm_stream_first_event_timeout_sec) > 0
            else min(max(1, int(args.llm_stream_idle_timeout_sec)), 120)
        ),
        "llm_stream_tail_timeout_sec": (
            max(1, int(args.llm_stream_tail_timeout_sec))
            if int(args.llm_stream_tail_timeout_sec) > 0
            else min(max(1, int(args.llm_stream_idle_timeout_sec)), 120)
        ),
        "llm_retry_attempts": max(1, int(args.llm_retry_attempts)),
        "llm_retry_initial_backoff_sec": max(0.0, float(args.llm_retry_initial_backoff_sec)),
        "llm_retry_max_backoff_sec": max(
            max(0.0, float(args.llm_retry_initial_backoff_sec)),
            float(args.llm_retry_max_backoff_sec),
        ),
        "llm_retry_count": sum(int(row.get("llm_retry_count") or 0) for row in per_instance_results),
    }

    if args.evaluate_with_harness:
        harness_kind = str(args.harness_kind or "auto").strip().lower()
        if harness_kind == "auto":
            harness_kind = "swepro" if _is_swebench_pro_dataset(args.dataset_name) else "swebench"
        summary["harness_kind"] = harness_kind
        print(f"[5/6] Running official {harness_kind} harness")
        use_local_docker = harness_kind != "swepro" or not bool(args.swepro_use_modal)
        if use_local_docker and not args.harness_modal and shutil.which("docker") is None:
            print(
                "Harness requires Docker CLI for local execution. "
                "Install Docker or rerun with --harness-modal/--swepro-use-modal.",
                file=sys.stderr,
            )
            return 3
        instance_ids = [str(item.get("instance_id") or "") for item in instances if str(item.get("instance_id") or "").strip()]
        if harness_kind == "swepro":
            try:
                harness_proc, swepro_output_dir = _run_swepro_harness_eval(
                    instances=instances,
                    predictions=predictions,
                    work_dir=work_dir,
                    harness_dir=Path(str(args.swepro_harness_dir)).expanduser(),
                    python_bin=str(args.swepro_python),
                    max_workers=args.harness_max_workers,
                    dockerhub_username=str(args.swepro_dockerhub_username),
                    docker_platform=str(args.swepro_docker_platform or ""),
                    use_local_docker=not bool(args.swepro_use_modal),
                    redo=not bool(args.swepro_no_redo),
                )
            except Exception as exc:
                print(f"SWE-bench Pro harness failed to start: {exc}", file=sys.stderr)
                summary["harness_exit_code"] = 1
                summary["harness_report_path"] = ""
                summary["harness_resolved_instances"] = 0
                summary["harness_unresolved_instances"] = 0
                summary["harness_total_instances"] = 0
                summary["harness_empty_patch_instances"] = 0
                summary["harness_error_instances"] = 0
                summary_path.write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
                print(json.dumps(summary, ensure_ascii=False, indent=2))
                return 1
        else:
            harness_proc = _run_harness_eval(
                dataset_name=args.dataset_name,
                split=args.split,
                predictions_path=predictions_path,
                run_id=args.run_id,
                max_workers=args.harness_max_workers,
                timeout_sec=args.harness_timeout_sec,
                instance_ids=instance_ids,
                workdir=work_dir,
                use_modal=bool(args.harness_modal),
            )
            swepro_output_dir = Path()
        harness_stdout = work_dir / "harness_stdout.txt"
        harness_stderr = work_dir / "harness_stderr.txt"
        harness_stdout.write_text(harness_proc.stdout or "", encoding="utf-8")
        harness_stderr.write_text(harness_proc.stderr or "", encoding="utf-8")
        summary["harness_exit_code"] = harness_proc.returncode
        print(f"Harness exit code: {harness_proc.returncode}")
        harness_result = (
            _read_swepro_harness_result(swepro_output_dir)
            if harness_kind == "swepro"
            else _read_harness_result(work_dir, args.run_id)
        )
        if harness_result:
            summary["harness_report_path"] = str(
                harness_result.get("report_path") or work_dir / f"nola-cli.{args.run_id}.json"
            )
            summary["harness_resolved_instances"] = int(harness_result.get("resolved_instances", 0))
            summary["harness_unresolved_instances"] = int(harness_result.get("unresolved_instances", 0))
            summary["harness_total_instances"] = int(harness_result.get("total_instances", 0))
            empty_patch_instances = harness_result.get("empty_patch_instances", 0)
            if isinstance(empty_patch_instances, list):
                summary["harness_empty_patch_instances"] = len(empty_patch_instances)
            else:
                summary["harness_empty_patch_instances"] = int(empty_patch_instances or 0)
            error_instances = harness_result.get("error_instances", 0)
            if isinstance(error_instances, list):
                summary["harness_error_instances"] = len(error_instances)
            else:
                summary["harness_error_instances"] = int(error_instances or 0)
        else:
            summary["harness_report_path"] = str(work_dir / f"nola-cli.{args.run_id}.json")
            summary["harness_resolved_instances"] = 0
            summary["harness_unresolved_instances"] = 0
            summary["harness_total_instances"] = 0
            summary["harness_empty_patch_instances"] = 0
            summary["harness_error_instances"] = 0
        if harness_proc.returncode != 0:
            print("Harness failed; see harness_stderr.txt", file=sys.stderr)
            summary_path.write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
            print(json.dumps(summary, ensure_ascii=False, indent=2))
            return harness_proc.returncode
    else:
        print("[5/6] Harness evaluation skipped (--evaluate-with-harness not set)")

    summary_path.write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    print("[6/6] Done")
    print(f"Artifacts: {work_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
