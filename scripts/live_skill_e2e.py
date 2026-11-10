#!/usr/bin/env python3
"""Run every built-in Nola Skill through the real CLI and configured LLM.

This is intentionally separate from the hermetic test suite. It performs paid,
networked model calls and may create files inside an isolated temporary
workspace. Set NOLA_RUN_LIVE_SKILL_E2E=1 to acknowledge those side effects.
"""

from __future__ import annotations

import ast
import contextlib
import hashlib
import json
import os
from pathlib import Path
import signal
import shlex
import sqlite3
import struct
import subprocess
import sys
import tempfile
import time
import uuid
from typing import Any
from urllib.parse import urlsplit
import zlib
import zipfile

from e2e.cargo_paths import cargo_target_directory
from e2e.current_model import selected_model
from e2e.model_execution_fixture import fixture, runtime_capabilities
from real_e2e_events import committed_journal_parts
from test_real_cli_resilience import configured_environment, clear_owned_credentials
from test_real_cli_terminal import Oracle, readonly


SOURCE_CONFIGURATION: tuple[Path, bytes] | None = None


class InfrastructureFailure(RuntimeError):
    """The configured model service prevented the Skill from being exercised."""


CASES = [
    ("word-document-authoring", "Create and validate report.docx containing a title, two sections, and a native table with one header row plus exactly 3 data rows about quarterly planning.", (".docx",)),
    ("excel-workbook-authoring", "Create and validate budget.xlsx with source data, formulas, totals, and a summary chart.", (".xlsx",)),
    ("powerpoint-presentation-authoring", "Create and validate briefing.pptx with five concise slides and speaker notes about local-first AI.", (".pptx",)),
    ("work-with-pdf", "Create and validate handbook.pdf with a cover, two content sections, page numbers, and a small table.", (".pdf",)),
    ("collaborate-in-figma", "Audit figma_snapshot.json as an exported Figma design, then write a concrete accessibility and consistency review to figma-review.md. Do not require a live Figma account.", (".md",)),
    ("analyze-data", "Analyze sales.csv, reconcile every total to the source, and write a validated analysis.md with findings and limitations.", (".md",)),
    ("conduct-deep-research", "Research the current state of small local language models using multiple independent web sources and write research.md with directly supporting citations and access dates.", (".md",)),
    ("review-documents", "Review draft.md for unsupported claims, structure, ambiguity, and actionable revisions; write review.md with source anchors.", (".md",)),
    ("extract-image-text", "Extract the text from receipt.png, preserve line order, flag uncertain characters, and write receipt.txt.", (".txt",)),
    ("assist-meetings", "Turn meeting-notes.md into decisions, owners, deadlines, and follow-ups in meeting-output.md without using a live calendar.", (".md",)),
    ("plan-schedule-tasks", "Convert tasks.md into a conflict-aware weekly schedule and write schedule.md. Do not use a live calendar.", (".md",)),
    ("curate-knowledge-memory", "Curate preference.txt into durable personal knowledge: preserve provenance, define a review or expiry policy, check for conflicting memories, save one canonical entry, run a representative recall query, and report the verified audit trail.", ()),
    ("organize-files", "Organize the files under inbox into sensible subfolders, preserve every file, and write a validated move-manifest.json.", (".json",)),
    ("design-automations", "Use the built-in design-automations Skill to create an enabled daily 09:15 automation that analyzes sales.csv and writes a short report, then verify the persisted automation record.", ()),
    ("assist-email", "Create an unsent standards-compliant email draft at the exact path follow-up.eml from email-context.md. Preserve the recipient, subject, and review time. Do not send anything.", (".eml",)),
    ("nature-reader", "Build a bilingual, source-anchored paper reader from paper.md and save it as paper-reader.md. Do not reduce it to a summary.", (".md",)),
    ("nature-writing", "Draft a source-grounded scientific abstract from experiment.md and save it as manuscript-abstract.md, marking missing evidence explicitly.", (".md",)),
    ("nature-polishing", "Polish academic-draft.md into publication-quality English without changing its meaning; save the result and a change log to polished.md.", (".md",)),
    ("nature-reviewer", "Perform a claim-anchored peer review of manuscript.md and write peer-review.md with major and minor findings.", (".md",)),
    ("nature-response", "Create a point-by-point response to reviewer-comments.md using manuscript.md, and save it as response-letter.md without inventing changes.", (".md",)),
    ("nature-statistics", "Audit stats-report.md for experimental-unit, test, effect-size, uncertainty, and reporting issues; write statistics-audit.md.", (".md",)),
    ("nature-ref-verifier", "Verify every citation in references.md against multiple current sources and write reference-audit.md with unresolved conflicts.", (".md",)),
    ("nature-paper2ppt", "Create and validate paper-deck.pptx from paper.md with a coherent evidence-led story, readable layouts, and speaker notes.", (".pptx",)),
]


EXPECTED_ARTIFACTS = {
    "word-document-authoring": "report.docx",
    "excel-workbook-authoring": "budget.xlsx",
    "powerpoint-presentation-authoring": "briefing.pptx",
    "work-with-pdf": "handbook.pdf",
    "collaborate-in-figma": "figma-review.md",
    "analyze-data": "analysis.md",
    "conduct-deep-research": "research.md",
    "review-documents": "review.md",
    "extract-image-text": "receipt.txt",
    "assist-meetings": "meeting-output.md",
    "plan-schedule-tasks": "schedule.md",
    "organize-files": "move-manifest.json",
    "assist-email": "follow-up.eml",
    "nature-reader": "paper-reader.md",
    "nature-writing": "manuscript-abstract.md",
    "nature-polishing": "polished.md",
    "nature-reviewer": "peer-review.md",
    "nature-response": "response-letter.md",
    "nature-statistics": "statistics-audit.md",
    "nature-ref-verifier": "reference-audit.md",
    "nature-paper2ppt": "paper-deck.pptx",
}

SPECIAL_REQUIRED_TOOLS = {
    "conduct-deep-research": {"web_search"},
    "curate-knowledge-memory": {"memory_save", "memory_search"},
    "design-automations": {"nola_orchestrate"},
}

ARTIFACT_MARKERS = {
    # A concrete WCAG finding is stronger evidence of an accessibility audit
    # than requiring the report to repeat the generic word "accessibility".
    "collaborate-in-figma": ("contrast", "WCAG"),
    "analyze-data": ("revenue", "limitation"),
    "conduct-deep-research": ("http", "access"),
    "review-documents": ("unsupported", "claim"),
    "extract-image-text": ("TOTAL", "42.00"),
    "assist-meetings": ("Mei", "Arun"),
    "plan-schedule-tasks": ("Prepare demo", "analyze data"),
    "assist-email": ("Dana", "Friday", "10:00"),
    "nature-reader": ("81%", "86%"),
    "nature-writing": ("81%", "86%"),
    "nature-polishing": ("120", "uncertainty"),
    "nature-reviewer": ("86%", "latency"),
    "nature-response": ("uncertainty", "latency"),
    "nature-statistics": ("effect size", "confidence interval"),
    "nature-ref-verifier": ("1706.03762", "1810.04805"),
}

TEXT_MINIMUMS = {
    "extract-image-text": 5,
}

STRICT_FINAL_VALIDATION_SKILLS = {
    "analyze-data",
    "conduct-deep-research",
    "plan-schedule-tasks",
    "organize-files",
    "nature-reviewer",
    "assist-email",
    "extract-image-text",
    "work-with-pdf",
}

BUILTIN_SKILL_ROOT = (
    Path(__file__).resolve().parents[1]
    / "NolaMac/Resources/NolaBuiltInSkills.bundle"
)


def manifest_validator_contracts(skill_id: str) -> list[tuple[str, tuple[str, ...]]]:
    """Return executable validator contracts declared by a Skill manifest.

    A contract is the validator script basename plus required literal arguments.
    Human audit descriptions and placeholder values such as ``<report.md>`` are
    deliberately excluded; successful execution must still contain every option
    and fixed value that the manifest declares.
    """
    path = BUILTIN_SKILL_ROOT / skill_id / "NOLA_MANIFEST.json"
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError, json.JSONDecodeError):
        return []
    validation = manifest.get("validation") if isinstance(manifest, dict) else None
    if not isinstance(validation, list):
        return []
    contracts = []
    for declaration in validation:
        if not isinstance(declaration, str):
            continue
        try:
            tokens = shlex.split(declaration)
        except ValueError:
            continue
        if not tokens or Path(tokens[0]).suffix.casefold() not in {".py", ".swift"}:
            continue
        required = tuple(
            token
            for token in tokens[1:]
            if not (token.startswith("<") and token.endswith(">"))
        )
        contracts.append((Path(tokens[0]).name, required))
    return contracts


def command_matches_validator_contract(
    command: str, contract: tuple[str, tuple[str, ...]]
) -> bool:
    script_name, required = contract
    try:
        tokens = shlex.split(command)
    except ValueError:
        return False
    script_index = next(
        (index for index, token in enumerate(tokens) if Path(token).name == script_name),
        None,
    )
    if script_index is None:
        return False
    suffix = Path(script_name).suffix.casefold()
    if script_index > 0:
        interpreter = Path(tokens[script_index - 1]).name.casefold()
        allowed = {"python", "python3"} if suffix == ".py" else {"swift"}
        if interpreter not in allowed:
            return False
    tail = tokens[script_index + 1 :]
    return all(marker in tail for marker in required)

FORBIDDEN_DISCOVERY_COMMAND_MARKERS = (
    "pip install",
    "brew install",
    "which tesseract",
    "pip list",
)

READ_ONLY_POST_VALIDATION_TOOLS = {
    "read_file",
    "read_many_files",
    "list_dir",
    "find_files",
    "find_up",
    "glob",
    "grep",
    "file_tree",
    "project_overview",
    "file_outline",
    "read_symbol",
    "code_search",
    "workspace_symbols",
    "symbol_search",
    "workspace_status",
    "git_diff",
    "tool_output_read",
    "tool_output_search",
    "tool_output_find",
    "tool_output_list",
}

# These calls only commit lifecycle evidence to the Skill runtime ledger. They
# do not mutate the deliverable that was just validated, and are expected to
# follow the deterministic validator so the recorded evidence is truthful.
POST_VALIDATION_LEDGER_TOOLS = {"skill_update", "requirements_update"}


FIXTURES = {
    "sales.csv": "month,region,revenue,cost\nJan,North,120,80\nJan,South,150,95\nFeb,North,140,88\nFeb,South,165,102\n",
    "figma_snapshot.json": json.dumps({"frames": [{"name": "Checkout", "text_sizes": [12, 14, 24], "contrast": [3.1, 7.2], "spacing": [8, 12, 17]}]}, indent=2),
    "draft.md": "# Remote work\n\nRemote work always doubles productivity. The rollout should happen immediately.\n",
    "meeting-notes.md": "Launch review. Mei owns the migration plan by Friday. Arun will verify backups by Thursday. Decision: stage rollout at 10%. Risk: no rollback drill yet.\n",
    "tasks.md": "Prepare demo (2h, Tue deadline); analyze data (3h, Thu deadline); review launch (1h, after analysis). Available Mon-Fri 09:00-17:00.\n",
    "preference.txt": "When presenting financial data, show the source date and distinguish actuals from forecasts.",
    "email-context.md": "Recipient: Dana. We agreed to send the revised proposal on Tuesday. Ask whether Friday 10:00 works for a 20-minute review. Tone: professional and warm.\n",
    "paper.md": "# Adaptive inference\n\nWe evaluated an adaptive inference method on 120 held-out samples. Accuracy increased from 81% to 86%, while median latency rose from 42 ms to 47 ms. The comparison used the same samples, but confidence intervals were not reported.\n",
    "experiment.md": "Question: can adaptive inference improve accuracy? Data: 120 held-out samples. Baseline accuracy 81%; method 86%. Median latency 42 ms vs 47 ms. No confidence intervals yet.\n",
    "academic-draft.md": "Our method is very novel and clearly gets much better accuracy. It was tested on 120 samples and accuracy changed from 81% to 86%, but uncertainty was not calculated.\n",
    "manuscript.md": "# Adaptive inference\n\nWe introduce an adaptive method. On 120 held-out samples, accuracy was 86% versus 81% for baseline. Median latency was 47 ms versus 42 ms. We conclude it is universally superior.\n",
    "reviewer-comments.md": "1. Please moderate the universal-superiority claim. 2. Report uncertainty for accuracy. 3. Discuss the latency trade-off.\n",
    "stats-report.md": "We tested 120 images from 12 patients as independent samples using an unpaired t-test. Accuracy was 86% vs 81% (p<0.05). No effect size or confidence interval was reported.\n",
    "references.md": "Vaswani A, et al. Attention Is All You Need. NeurIPS 2017. arXiv:1706.03762.\nDevlin J, et al. BERT. NAACL 2019. arXiv:1810.04805.\n",
}


def find_binary(repo: Path) -> Path:
    newest_source = newest_runtime_source_mtime_ns(repo)

    def require_current(candidate: Path) -> Path:
        if newest_source is not None and candidate.stat().st_mtime_ns < newest_source:
            raise RuntimeError(
                f"Nola binary is stale relative to CLI/Core/Skill sources: {candidate}"
            )
        return candidate

    override = os.environ.get("NOLA_BIN")
    if override:
        selected = Path(override).expanduser()
        if selected.is_file() and os.access(selected, os.X_OK):
            return require_current(selected)
        raise RuntimeError(f"NOLA_BIN is not an executable file: {selected}")
    target_directory = cargo_target_directory(repo)
    candidates = [
        target_directory / "release" / "nola",
        target_directory / "debug" / "nola",
    ]
    available = [
        candidate
        for candidate in candidates
        if candidate.is_file() and os.access(candidate, os.X_OK)
    ]
    if available:
        return require_current(
            max(available, key=lambda candidate: candidate.stat().st_mtime_ns)
        )
    raise RuntimeError("Cannot find nola. Build cli or set NOLA_BIN to the executable path.")


def newest_runtime_source_mtime_ns(repo: Path) -> int | None:
    roots = [
        repo / "cli/src",
        repo / "core/src",
        repo / "NolaMac/Resources/NolaBuiltInSkills.bundle",
    ]
    files = [
        repo / "cli/Cargo.toml",
        repo / "cli/build.rs",
        repo / "core/Cargo.toml",
    ]
    for root in roots:
        if root.is_dir():
            files.extend(path for path in root.rglob("*") if path.is_file())
    mtimes = [path.stat().st_mtime_ns for path in files if path.is_file()]
    return max(mtimes) if mtimes else None


def build_or_find_binary(repo: Path) -> Path:
    try:
        return find_binary(repo)
    except RuntimeError as initial_error:
        if os.environ.get("NOLA_BIN"):
            raise
        command = [
            "cargo",
            "build",
            "--release",
            "-p", "nola_cli",
            "--bin",
            "nola",
            "--example", "terminal_acceptance_oracle",
            "-p", "nola_service", "--bin", "nola-service",
            "--manifest-path",
            str(repo / "cli/Cargo.toml"),
        ]
        print(
            f"[BUILD] current release binary required: {initial_error}", flush=True
        )
        completed = subprocess.run(command, cwd=repo, check=False)
        if completed.returncode != 0:
            raise RuntimeError(
                f"failed to build current nola release binary (exit {completed.returncode})"
            ) from initial_error
        return find_binary(repo)


def seed_workspace(root: Path) -> None:
    for name, content in FIXTURES.items():
        (root / name).write_text(content, encoding="utf-8")
    inbox = root / "inbox"
    inbox.mkdir()
    (inbox / "invoice.txt").write_text("invoice 2026-07\n", encoding="utf-8")
    (inbox / "notes.md").write_text("project notes\n", encoding="utf-8")
    write_text_png(root / "receipt.png", "TOTAL 42.00")


def write_text_png(path: Path, text: str) -> None:
    glyphs = {
        " ": ["00000", "00000", "00000", "00000", "00000", "00000", "00000"],
        ".": ["00000", "00000", "00000", "00000", "00000", "01100", "01100"],
        "0": ["01110", "10001", "10011", "10101", "11001", "10001", "01110"],
        "2": ["01110", "10001", "00001", "00010", "00100", "01000", "11111"],
        "4": ["00010", "00110", "01010", "10010", "11111", "00010", "00010"],
        "A": ["01110", "10001", "10001", "11111", "10001", "10001", "10001"],
        "L": ["10000", "10000", "10000", "10000", "10000", "10000", "11111"],
        "O": ["01110", "10001", "10001", "10001", "10001", "10001", "01110"],
        "T": ["11111", "00100", "00100", "00100", "00100", "00100", "00100"],
    }
    scale = 11
    margin = 28
    glyph_width = 5
    width = margin * 2 + len(text) * (glyph_width + 1) * scale
    height = margin * 2 + 7 * scale
    pixels = bytearray([255] * (width * height * 3))
    for index, character in enumerate(text.upper()):
        pattern = glyphs[character]
        origin_x = margin + index * (glyph_width + 1) * scale
        for row, bits in enumerate(pattern):
            for column, bit in enumerate(bits):
                if bit != "1":
                    continue
                for dy in range(scale):
                    for dx in range(scale):
                        x = origin_x + column * scale + dx
                        y = margin + row * scale + dy
                        offset = (y * width + x) * 3
                        pixels[offset:offset + 3] = b"\x00\x00\x00"

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    rows = b"".join(
        b"\x00" + bytes(pixels[row * width * 3:(row + 1) * width * 3])
        for row in range(height)
    )
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(rows, 9))
        + chunk(b"IEND", b"")
    )


def parse_events(stdout: str) -> list[dict[str, Any]]:
    events = []
    for line in stdout.splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError("CLI JSON transport emitted invalid NDJSON") from error
        if not isinstance(value, dict) or not isinstance(value.get("type"), str):
            raise ValueError("CLI transport requires a current typed event")
        events.append(value)
    return events


def journal_tool_events(transport: list[dict], database: Path, session: str) -> list[dict]:
    """Normalize committed Tool facts for the workflow auditor, not raw traces.

    The internal tool.started/tool.completed labels below are generated only
    here from accepted Journal heads/bodies; provider prose is never traversed.
    """
    events = []
    for part in committed_journal_parts(transport, database):
        head, bodies = part["head"], part["bodies"]
        if head["identity"]["conversation_id"] != session:
            raise ValueError("Skill journal crossed its isolated session owner")
        if head["metadata"]["kind"] != "tool":
            continue
        tool = head["metadata"]["data"]
        if tool["state"] != "completed":
            continue  # Pending/failed/cancelled is not workflow execution proof.
        encoded = json.loads(bodies["tool_input"])
        if set(encoded) != {"protocol_version", "value"} or encoded["protocol_version"] != 1 or not isinstance(encoded["value"], dict):
            raise ValueError("current structured Tool input body required")
        name = tool["capability_id"]
        if name == "skill":
            name = "Skill"  # Current normalized capability ID vs schema name.
        payload = {"capability_id": name, "call_id": tool["tool_call_id"], "arguments": encoded["value"]}
        events.append({"kind": "tool.started", "payload": payload})
        events.append({"kind": "tool.completed", "status": "completed", "payload": payload})
    return events


def event_tool_name(event: dict[str, Any]) -> str | None:
    payload = event.get("payload") if isinstance(event.get("payload"), dict) else {}
    value = payload.get("capability_id")
    return value if isinstance(value, str) and value else None


def observed_tool_names(events: list[dict[str, Any]]) -> list[str]:
    names = []
    for event in events:
        if event.get("kind") not in {"tool.started", "tool.completed"}:
            continue
        name = event_tool_name(event)
        if name and (not names or names[-1] != name):
            names.append(name)
    return names


def started_tool_calls(events: list[dict[str, Any]]) -> list[tuple[str, dict[str, Any]]]:
    # This is the sole internal normalization produced by journal_tool_events;
    # failed/pending Tool facts never enter this list in the first place.
    calls = []
    for event in events:
        if event.get("kind") != "tool.started":
            continue
        payload = event.get("payload") if isinstance(event.get("payload"), dict) else {}
        name = event_tool_name(event)
        arguments = payload.get("arguments")
        if name:
            calls.append((name, arguments if isinstance(arguments, dict) else {}))
    return calls


def read_only_inline_python(command: str) -> bool:
    """Accept a narrow `python -c` inspection after an artifact validator.

    The check intentionally rejects filesystem mutation, process spawning, and
    dynamic execution instead of treating every inline Python command as safe.
    """
    try:
        tokens = shlex.split(command)
    except ValueError:
        return False
    if len(tokens) != 3 or Path(tokens[0]).name not in {"python", "python3"} or tokens[1] != "-c":
        return False
    try:
        tree = ast.parse(tokens[2], mode="exec")
    except SyntaxError:
        return False

    denied_names = {"eval", "exec", "compile", "__import__"}
    denied_attributes = {
        "chmod", "chown", "link", "mkdir", "makedirs", "open", "popen",
        "remove", "removedirs", "rename", "renames", "replace", "rmdir",
        "run", "symlink", "system", "touch", "truncate", "unlink",
        "write", "write_bytes", "write_text", "writelines",
    }
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        if isinstance(node.func, ast.Name):
            if node.func.id in denied_names:
                return False
            if node.func.id == "open":
                mode: str | None = None
                if len(node.args) > 1 and isinstance(node.args[1], ast.Constant):
                    mode = node.args[1].value if isinstance(node.args[1].value, str) else None
                for keyword in node.keywords:
                    if keyword.arg == "mode" and isinstance(keyword.value, ast.Constant):
                        mode = keyword.value.value if isinstance(keyword.value.value, str) else None
                if mode is not None and any(marker in mode for marker in "wax+"):
                    return False
        elif isinstance(node.func, ast.Attribute) and node.func.attr in denied_attributes:
            return False
    return True


def read_only_shell_command(command: str) -> bool:
    if not command or any(marker in command for marker in ("`", "$(", ">", "<")):
        return False
    try:
        lexer = shlex.shlex(command, posix=True, punctuation_chars="|&;")
        lexer.whitespace_split = True
        tokens = list(lexer)
    except ValueError:
        return False

    separators = {"|", "||", "&&", ";", "&"}
    segments: list[list[str]] = [[]]
    for token in tokens:
        if token in separators:
            if not segments[-1]:
                return False
            segments.append([])
        else:
            segments[-1].append(token)
    if not segments[-1]:
        return False

    allowed = {
        "cat", "cut", "echo", "find", "grep", "head", "ls", "md5",
        "printf", "rg", "sed", "sha256sum", "shasum", "sort", "tail",
        "uniq", "wc",
    }
    for segment in segments:
        executable = Path(segment[0]).name
        if executable not in allowed:
            return False
        if executable == "sed" and any(token == "-i" or token.startswith("-i") for token in segment[1:]):
            return False
        if executable == "find" and any(
            token in {"-delete", "-exec", "-execdir", "-ok", "-okdir"}
            or token.startswith(("-fprint", "-fprintf"))
            for token in segment[1:]
        ):
            return False
        if executable == "sort" and any(token == "-o" or token.startswith("--output") for token in segment[1:]):
            return False
    return True


def validate_skill_tool_sequence(
    events: list[dict[str, Any]], skill_id: str
) -> list[str]:
    names = observed_tool_names(events)
    calls = started_tool_calls(events)
    errors = []
    if not names:
        return ["no task tool was observed"]
    skill_index = next(
        (index for index, call in enumerate(calls) if call[0] == "Skill"),
        None,
    )
    if skill_index is None:
        errors.append("matching Skill was never invoked")
    else:
        safe_preflight_tools = {
            "read_file",
            "read_many_files",
            "file_tree",
            "file_outline",
            "glob",
            "grep",
            "tool_search",
            "workspace_status",
        }
        unsafe_preflight = [
            name for name, _ in calls[:skill_index] if name not in safe_preflight_tools
        ]
        if unsafe_preflight:
            errors.append(
                "Skill was invoked after an executing, mutating, or external tool: "
                f"{unsafe_preflight[:3]}"
            )
        invoked_id = calls[skill_index][1].get("skill")
        if invoked_id != skill_id:
            errors.append(f"wrong Skill invoked: expected {skill_id}, got {invoked_id}")
        if not any(
            name != "Skill"
            for name, _ in calls[skill_index + 1 :]
        ):
            errors.append("Skill loaded but no execution tool followed")
    required = SPECIAL_REQUIRED_TOOLS.get(skill_id, set())
    missing = sorted(required - set(names))
    if missing:
        errors.append(f"missing required tool(s): {', '.join(missing)}")
    if skill_id == "nature-ref-verifier" and "web_search" not in names:
        fetched_hosts = {
            urlsplit(str(arguments.get("url", ""))).hostname
            for name, arguments in calls
            if name == "web_fetch" and arguments.get("url")
        }
        fetched_hosts.discard(None)
        if len(fetched_hosts) < 2:
            errors.append(
                "reference verification requires web_search or direct fetches "
                "from at least two independent source hosts"
            )
    started_serialized = json.dumps(calls, ensure_ascii=False)
    forbidden = [
        marker
        for marker in FORBIDDEN_DISCOVERY_COMMAND_MARKERS
        if marker.casefold() in started_serialized.casefold()
    ]
    if forbidden:
        errors.append(f"Skill bypassed its bundled workflow with dependency discovery: {forbidden}")
    if skill_id in STRICT_FINAL_VALIDATION_SKILLS:
        validator_contracts = manifest_validator_contracts(skill_id)
        validation_index = next(
            (
                index
                for index in range(len(calls) - 1, -1, -1)
                if calls[index][0] == "bash"
                and any(
                    command_matches_validator_contract(
                        str(calls[index][1].get("command", "")), contract
                    )
                    for contract in validator_contracts
                )
            ),
            None,
        )
        if validation_index is None:
            errors.append("strict Skill did not run its required final validation")
        else:
            for name, arguments in calls[validation_index + 1 :]:
                if name in READ_ONLY_POST_VALIDATION_TOOLS or name in POST_VALIDATION_LEDGER_TOOLS:
                    continue
                if name == "delete_file":
                    path = Path(str(arguments.get("path", "")))
                    lowered = path.name.casefold()
                    if lowered == "spec.json":
                        continue
                    if lowered.startswith(".") and lowered.endswith((".json", ".tmp", ".spec")):
                        continue
                    if lowered.endswith(("_spec.json", ".spec.json")):
                        continue
                if name == "bash":
                    command = str(arguments.get("command", "")).strip()
                    if read_only_inline_python(command) or read_only_shell_command(command):
                        continue
                    safe_cleanup = bool(command)
                    for segment in command.split("&&"):
                        try:
                            tokens = shlex.split(segment)
                        except ValueError:
                            tokens = []
                        if not tokens:
                            safe_cleanup = False
                            break
                        executable = Path(tokens[0]).name
                        if executable == "rmdir":
                            if len(tokens) < 2 or any(token.startswith("-") for token in tokens[1:]):
                                safe_cleanup = False
                                break
                            continue
                        if executable != "rm":
                            safe_cleanup = False
                            break
                        targets = [token for token in tokens[1:] if token not in {"-f", "--"}]
                        if not targets:
                            safe_cleanup = False
                            break
                        for target in targets:
                            lowered = Path(target).name.casefold()
                            if not (
                                lowered == "spec.json"
                                or (lowered.startswith(".") and lowered.endswith((".json", ".tmp", ".spec")))
                                or lowered.endswith(("_spec.json", ".spec.json"))
                            ):
                                safe_cleanup = False
                                break
                        if not safe_cleanup:
                            break
                    if safe_cleanup:
                        continue
                errors.append(
                    "task mutated state after its final bundled validation: "
                    f"{name} {json.dumps(arguments, ensure_ascii=False)[:300]}"
                )
    return errors


def validate_artifact(path: Path, skill_id: str) -> list[str]:
    errors = []
    if not path.is_file() or path.stat().st_size == 0:
        return [f"missing non-empty artifact: {path.name}"]
    suffix = path.suffix.lower()
    if suffix in {".docx", ".xlsx", ".pptx"}:
        try:
            with zipfile.ZipFile(path) as archive:
                names = set(archive.namelist())
                if suffix == ".docx":
                    required = {"[Content_Types].xml", "word/document.xml", "word/styles.xml"}
                    document = archive.read("word/document.xml") if "word/document.xml" in names else b""
                    if document.count(b"<w:tbl") < 1 or document.count(b"<w:tr") < 4:
                        errors.append("DOCX is missing the requested native 3-row data table")
                elif suffix == ".xlsx":
                    required = {"[Content_Types].xml", "xl/workbook.xml", "xl/worksheets/sheet1.xml"}
                    worksheets = b"".join(
                        archive.read(name)
                        for name in names
                        if name.startswith("xl/worksheets/") and name.endswith(".xml")
                    )
                    if b"<f" not in worksheets:
                        errors.append("XLSX contains no native formulas")
                    if not any(name.startswith("xl/charts/chart") for name in names):
                        errors.append("XLSX contains no native chart")
                else:
                    required = {"[Content_Types].xml", "ppt/presentation.xml", "ppt/slides/slide1.xml"}
                    slides = [name for name in names if name.startswith("ppt/slides/slide") and name.endswith(".xml")]
                    notes = [name for name in names if name.startswith("ppt/notesSlides/notesSlide") and name.endswith(".xml")]
                    if len(slides) < 5:
                        errors.append(f"PPTX has {len(slides)} slides; expected at least 5")
                    if len(notes) < 5:
                        errors.append(f"PPTX has {len(notes)} note pages; expected at least 5")
                missing = sorted(required - names)
                if missing:
                    errors.append(f"invalid {suffix[1:].upper()} package; missing {missing}")
        except (OSError, KeyError, zipfile.BadZipFile) as error:
            errors.append(f"invalid Office artifact: {error}")
    elif suffix == ".pdf":
        data = path.read_bytes()
        if not data.startswith(b"%PDF-"):
            errors.append("artifact is not a PDF file")
        if data.count(b"/Type /Page") < 2:
            errors.append("PDF does not contain the requested multi-page structure")
    elif suffix == ".json":
        try:
            value = json.loads(path.read_text(encoding="utf-8"))
            if not isinstance(value, (dict, list)) or not value:
                errors.append("JSON artifact is empty or not structured")
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
            errors.append(f"invalid JSON artifact: {error}")
    else:
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as error:
            return [f"artifact is not readable UTF-8 text: {error}"]
        minimum = TEXT_MINIMUMS.get(skill_id, 120)
        if len(text.strip()) < minimum:
            errors.append(f"text artifact is too small to satisfy the task ({len(text.strip())} chars)")
        missing = [marker for marker in ARTIFACT_MARKERS.get(skill_id, ()) if marker.casefold() not in text.casefold()]
        if missing:
            errors.append(f"artifact is missing source-grounded marker(s): {missing}")
        if suffix == ".eml":
            for header in ("To:", "Subject:", "Date:"):
                if header.casefold() not in text.casefold():
                    errors.append(f"email artifact is missing {header} header")
    return errors


def infrastructure_failure_detail(events: list[dict[str, Any]], stderr: str) -> str | None:
    markers = (
        "401 Unauthorized",
        "402 Payment Required",
        "403 Forbidden",
        "429 Too Many Requests",
        "Insufficient Balance",
        "Authentication Fails",
        "connection refused",
        "could not resolve host",
        "temporary failure in name resolution",
        "service unavailable",
        "gateway timeout",
    )
    diagnostics = [event["detail"] for event in events
                   if event.get("type") == "error" and isinstance(event.get("detail"), str) and event["detail"].strip()]
    diagnostics.extend(line for line in stderr.splitlines() if line.strip())
    return next((detail for detail in diagnostics
                 if any(marker.casefold() in detail.casefold() for marker in markers)), None)


def validate_persisted_side_effects(runtime_home: Path, skill_id: str, events: list[dict]) -> list[str]:
    if skill_id != "design-automations":
        return []
    database = runtime_home / "nola-e2e.db"
    if not database.is_file():
        return ["automation database was not created"]
    try:
        with readonly(database) as connection:
            rows = connection.execute(
                "SELECT enabled,metadata_json,execution_spec_json FROM automation_definitions WHERE deleted_at_ms IS NULL ORDER BY updated_at_ms"
            ).fetchall()
    except (sqlite3.Error, OSError) as error:
        return [f"automation database could not be inspected: {error}"]
    submitted = [arguments for name, arguments in started_tool_calls(events) if name == "nola_orchestrate"]
    if len(submitted) != 1:
        return ["expected one successful original automation submission"]
    original = submitted[0]
    if (original.get("frequency"), original.get("hour"), original.get("minute"), original.get("skill_id"), original.get("enabled", True)) != ("daily", 9, 15, "analyze-data", True):
        return ["automation submission does not match the requested daily 09:15 Skill"]
    records = []
    for enabled, raw, spec_raw in rows:
        try:
            value = json.loads(raw)
            spec = json.loads(spec_raw)
        except (TypeError, ValueError, json.JSONDecodeError):
            continue
        if isinstance(value, dict) and isinstance(spec, dict) and enabled == 1 and value.get("executionSpec") == spec == original.get("execution_spec") \
                and spec.get("kind") == "agent" and spec.get("spec", {}).get("prompt") == original.get("prompt") \
                and value.get("userPrompt") == original.get("prompt"):
            records.append(value)
    expected = {"enabled": True, "time": "09:15", "skillId": "analyze-data"}
    matching = [
        record
        for record in records
        if all(record.get(key) == value for key, value in expected.items())
    ]
    if len(matching) != 1:
        return [
            "expected exactly one persisted enabled daily 09:15 analyze-data "
            f"automation, found {len(matching)} among {len(records)} records"
        ]
    record = matching[0]
    mismatches = [
        f"{key}={record.get(key)!r}"
        for key, value in expected.items()
        if record.get(key) != value
    ]
    return [f"persisted automation mismatch: {', '.join(mismatches)}"] if mismatches else []


def text_output(value: str | bytes | None) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    return value


def terminate_process_group(
    process: subprocess.Popen[str],
    *,
    term_grace_seconds: float = 5,
    kill_grace_seconds: float = 2,
) -> tuple[str, str]:
    """Terminate a timed-out process tree without ever waiting indefinitely.

    A grandchild can daemonize or move to another process group while retaining
    the inherited stdout/stderr descriptors. In that case an unbounded final
    ``communicate()`` waits forever even after the original group received
    SIGKILL. Every drain and wait below is therefore bounded, and the parent
    closes its pipe endpoints as the final convergence step.
    """
    stdout = ""
    stderr = ""

    def capture(error: subprocess.TimeoutExpired) -> None:
        nonlocal stdout, stderr
        stdout = text_output(error.stdout) or stdout
        stderr = text_output(error.stderr) or stderr

    if process.poll() is None:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    try:
        drained_stdout, drained_stderr = process.communicate(timeout=term_grace_seconds)
        return text_output(drained_stdout), text_output(drained_stderr)
    except subprocess.TimeoutExpired as error:
        capture(error)

    if process.poll() is None:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    try:
        drained_stdout, drained_stderr = process.communicate(timeout=kill_grace_seconds)
        return text_output(drained_stdout) or stdout, text_output(drained_stderr) or stderr
    except subprocess.TimeoutExpired as error:
        capture(error)

    # A detached descendant may still own the write side of either pipe. Close
    # our read sides rather than turning timeout handling into another hang.
    for pipe in (process.stdout, process.stderr):
        if pipe is not None:
            try:
                pipe.close()
            except OSError:
                pass
    if process.poll() is None:
        try:
            process.kill()
        except ProcessLookupError:
            pass
    try:
        process.wait(timeout=kill_grace_seconds)
    except subprocess.TimeoutExpired:
        # The caller already has a deterministic timeout failure and transcript.
        # Never hide it behind an unbounded cleanup wait.
        pass
    return stdout, stderr


def live_llm_options() -> dict:
    global SOURCE_CONFIGURATION
    endpoint = os.environ.get("NOLA_LIVE_SKILL_ENDPOINT", "").strip()
    model = os.environ.get("NOLA_LIVE_SKILL_MODEL", "").strip()
    api_key = os.environ.get("NOLA_LIVE_SKILL_API_KEY", "").strip()
    config = {"llm": {"endpoint": endpoint, "model": model}}
    if not (endpoint and model and api_key):
        config_path = Path(os.environ.get("NOLA_CONFIG_PATH", str(Path.home() / ".nola/config.json"))).resolve()
        try:
            raw = config_path.read_bytes()
            config = json.loads(raw)
        except (OSError, ValueError) as error:
            raise InfrastructureFailure("explicit live model fixture/configuration is missing") from error
        SOURCE_CONFIGURATION = (config_path, hashlib.sha256(raw).digest())
        llm = config.get("llm", {})
        if "connections" in llm or "default_model" in llm:
            try:
                llm = selected_model(config, dict(os.environ))
            except ValueError as error:
                raise InfrastructureFailure(str(error)) from error
            config = {"llm": llm}
        endpoint = endpoint or str(llm.get("endpoint") or "").strip()
        model = model or str(llm.get("model") or "").strip()
        api_key = api_key or str(llm.get("api_key") or "").strip()
    if not endpoint or not model or not api_key:
        raise InfrastructureFailure(
            "live Skill E2E requires endpoint, model, and API key via NOLA_LIVE_SKILL_* "
            "or ~/.nola/config.json"
        )
    declaration = fixture(config, dict(os.environ), model=model, endpoint=endpoint, require_tool=True)
    return {"llm": {"endpoint": endpoint, "model": model, "api_key": api_key,
        "provider": {"chat_completions": "openai-compatible", "responses": "openai-responses",
                     "anthropic": "anthropic"}[declaration["wireProtocol"]],
        "capabilities": runtime_capabilities(declaration)}}


def skill_timeout_seconds(skill_id: str, configured: str | None = None) -> int:
    is_long_running_research_skill = (
        skill_id.startswith("nature-") or skill_id == "conduct-deep-research"
    )
    if is_long_running_research_skill:
        default_timeout = 600
        maximum_timeout = 900
    elif skill_id == "analyze-data":
        default_timeout = 180
        maximum_timeout = 600
    else:
        default_timeout = 300
        maximum_timeout = 600
    raw_timeout = configured
    if raw_timeout is None:
        raw_timeout = os.environ.get(
            "NOLA_LIVE_SKILL_TIMEOUT_SECONDS", str(default_timeout)
        )
    return max(60, min(maximum_timeout, int(raw_timeout)))


def validate_terminal_outcome(audit: dict, session: str) -> list[str]:
    runs = audit.get("runs", [])
    if len(runs) != 1 or runs[0].get("conversationId") != session:
        return ["expected one exact-owner canonical Run"]
    terminal = runs[0]
    if terminal.get("status") != "sealed" or terminal.get("kind") not in {"answered", "action_completed"}:
        return [f"task did not succeed: {terminal.get('status')} / {terminal.get('kind')}"]
    if not all(isinstance(terminal.get(key), str) and terminal[key] for key in ("runId", "sealDigest", "resultDigest")):
        return ["canonical Run is missing its validated receipt identities"]
    return []


def run_case(
    binary: Path,
    workspace: Path,
    runtime_home: Path,
    skill_id: str,
    prompt: str,
    extensions: tuple[str, ...],
    llm_options: dict,
    oracle_binary: Path,
) -> None:
    before = {path.resolve() for path in workspace.rglob("*") if path.is_file()}
    session = f"live-skill-{skill_id}-{uuid.uuid4().hex}"
    command = [
        str(binary), prompt, "--project", str(workspace), "--allow-dir", str(workspace),
        "--json", "--session", session,
    ]
    runtime_home.mkdir(parents=True, exist_ok=True)
    database = runtime_home / "nola-e2e.db"
    timeout_seconds = skill_timeout_seconds(skill_id)
    stdout, stderr = "", ""
    try:
        with configured_environment(runtime_home, "allow_all", model_configuration=llm_options) as environment:
            process = subprocess.Popen(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                env=environment, start_new_session=True, cwd=workspace)
            try:
                stdout, stderr = process.communicate(timeout=timeout_seconds)
            except subprocess.TimeoutExpired as error:
                stdout, stderr = terminate_process_group(process)
                stdout = stdout or text_output(error.stdout)
                stderr = stderr or text_output(error.stderr)
                transcript = workspace / f"timeout-{skill_id}.jsonl"
                transcript.write_text(stdout + "\nSTDERR\n" + stderr, encoding="utf-8")
                raise AssertionError(f"live Skill exceeded {timeout_seconds}s; transcript={transcript}") from error
            except BaseException:
                stdout, stderr = terminate_process_group(process)
                raise
    finally:
        (workspace / f"transcript-{skill_id}.jsonl").write_text(
            stdout + "\nSTDERR\n" + stderr, encoding="utf-8")
        clear_owned_credentials(database)
    result = subprocess.CompletedProcess(command, process.returncode, stdout, stderr)
    # Successful runs need the same audit trail as failures. Artifact existence
    # alone cannot establish source coverage or truthful validation claims.
    (workspace / f"transcript-{skill_id}.jsonl").write_text(
        stdout + "\nSTDERR\n" + stderr, encoding="utf-8"
    )
    transport = parse_events(result.stdout)
    infrastructure_detail = infrastructure_failure_detail(transport, result.stderr) if result.returncode != 0 else None
    if infrastructure_detail is not None:
        transcript = workspace / f"infrastructure-{skill_id}.jsonl"
        transcript.write_text(result.stdout + "\nSTDERR\n" + result.stderr, encoding="utf-8")
        raise InfrastructureFailure(
            f"model service unavailable before Skill execution: {infrastructure_detail}; "
            f"transcript={transcript}"
        )
    errors = []
    if result.returncode != 0:
        errors.append(f"process exited {result.returncode}: {result.stderr[-1000:]}")
    with contextlib.closing(Oracle(oracle_binary)) as oracle:
        audit = oracle.query(operation="audit", database=str(database), live=False)
    errors.extend(validate_terminal_outcome(audit, session))
    (workspace / f"canonical-{skill_id}.json").write_text(json.dumps(audit, ensure_ascii=False, indent=2), encoding="utf-8")
    events = journal_tool_events(transport, database, session)
    errors.extend(validate_skill_tool_sequence(events, skill_id))
    errors.extend(validate_persisted_side_effects(runtime_home, skill_id, events))
    expected_artifact = EXPECTED_ARTIFACTS.get(skill_id)
    if expected_artifact:
        artifact = workspace / expected_artifact
        if artifact.resolve() in before:
            errors.append(f"expected artifact was not created by this run: {expected_artifact}")
        errors.extend(validate_artifact(artifact, skill_id))
    elif extensions:
        errors.append(f"test case declares extensions {extensions} but no exact artifact contract")
    if errors:
        transcript = workspace / f"failure-{skill_id}.jsonl"
        transcript.write_text(result.stdout + "\nSTDERR\n" + result.stderr, encoding="utf-8")
        raise AssertionError("; ".join(errors) + f"; transcript={transcript}")


def main() -> int:
    import concurrent.futures
    import threading
    import time

    if os.environ.get("NOLA_RUN_LIVE_SKILL_E2E") != "1":
        print("Refusing paid live calls. Set NOLA_RUN_LIVE_SKILL_E2E=1 to continue.", file=sys.stderr)
        return 2
    repo = Path(__file__).resolve().parents[1]
    binary = build_or_find_binary(repo)
    oracle_binary = Path(os.environ.get("NOLA_LIVE_SKILL_ORACLE", str(binary.parent / "examples/terminal_acceptance_oracle"))).resolve()
    if not oracle_binary.is_file() or not os.access(oracle_binary, os.X_OK):
        raise InfrastructureFailure("build the current terminal acceptance oracle alongside the CLI")
    if not (binary.parent / "nola-service").is_file():
        raise InfrastructureFailure("build the current nola-service alongside the CLI")
    requested = set(sys.argv[1:])
    unknown = requested - {case[0] for case in CASES}
    if unknown:
        print(f"Unknown Skill ids: {', '.join(sorted(unknown))}", file=sys.stderr)
        return 2
    root = Path(tempfile.mkdtemp(prefix="nola-live-skills-"))
    llm_options = live_llm_options()
    print(f"Workspaces retained: {root}", flush=True)
    failures = []
    infrastructure_failure: tuple[str, str] | None = None
    try:
        selected_cases = [
            case for case in CASES if not requested or case[0] in requested
        ]
        try:
            concurrency = int(os.environ.get("NOLA_LIVE_SKILL_CONCURRENCY", "1"))
        except ValueError:
            concurrency = 1
        concurrency = max(1, min(concurrency, 4, len(selected_cases)))

        progress_lock = threading.Lock()
        started_cases = {}

        def execute_case(case):
            skill_id, prompt, extensions = case
            with progress_lock:
                started_cases[skill_id] = time.monotonic()
            print(f"[START] {skill_id}", flush=True)
            workspace = root / skill_id
            runtime_home = root / ".runtime" / skill_id
            workspace.mkdir(parents=True)
            seed_workspace(workspace)
            run_case(
                binary,
                workspace,
                runtime_home,
                skill_id,
                prompt,
                extensions,
                llm_options,
                oracle_binary,
            )
            return skill_id

        print(
            f"[PLAN] {len(selected_cases)} case(s), concurrency={concurrency}",
            flush=True,
        )
        executor = concurrent.futures.ThreadPoolExecutor(max_workers=concurrency)
        pending = {}
        try:
            for case in selected_cases:
                skill_id = case[0]
                print(f"[QUEUE] {skill_id}", flush=True)
                future = executor.submit(execute_case, case)
                pending[future] = skill_id

            while pending:
                done, _ = concurrent.futures.wait(
                    pending,
                    timeout=15,
                    return_when=concurrent.futures.FIRST_COMPLETED,
                )
                if not done:
                    with progress_lock:
                        active_cases = {
                            skill_id: started
                            for skill_id, started in started_cases.items()
                            if skill_id in pending.values()
                        }
                    active = ", ".join(
                        f"{skill_id}={int(time.monotonic() - started)}s"
                        for skill_id, started in sorted(active_cases.items())
                    )
                    queued = len(pending) - len(active_cases)
                    print(f"[WAIT] active: {active or 'none'}; queued={queued}", flush=True)
                    continue
                for future in done:
                    skill_id = pending.pop(future)
                    with progress_lock:
                        started_cases.pop(skill_id, None)
                    try:
                        future.result()
                        print(f"[PASS] {skill_id}", flush=True)
                    except InfrastructureFailure as error:
                        detail = str(error).replace(llm_options["llm"]["api_key"], "[redacted]")
                        if infrastructure_failure is None:
                            infrastructure_failure = (skill_id, detail)
                        print(f"[BLOCKED] {skill_id}: {detail}", flush=True)
                    except Exception as error:
                        detail = str(error).replace(llm_options["llm"]["api_key"], "[redacted]")
                        failures.append((skill_id, detail))
                        print(f"[FAIL] {skill_id}: {detail}", flush=True)
        finally:
            executor.shutdown(wait=True, cancel_futures=True)
        if infrastructure_failure is not None:
            skill_id, error = infrastructure_failure
            print("\nLive Skill E2E could not reach the configured model service.", file=sys.stderr)
            print(f"- {skill_id}: {error}", file=sys.stderr)
            print(f"Workspaces: {root}", file=sys.stderr)
            return 3
        if failures:
            print("\nFailures:", file=sys.stderr)
            for skill_id, error in failures:
                print(f"- {skill_id}: {error}", file=sys.stderr)
            print(f"Workspaces: {root}", file=sys.stderr)
            return 1
        return 0
    finally:
        if SOURCE_CONFIGURATION is not None:
            source, expected = SOURCE_CONFIGURATION
            if hashlib.sha256(source.read_bytes()).digest() != expected:
                raise RuntimeError("source user configuration changed during live Skill validation")


if __name__ == "__main__":
    raise SystemExit(main())
