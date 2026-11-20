#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Opt-in, paid Main → Antigravity implementation → Codex review on disposable sources."""
import argparse
import json
import os
from pathlib import Path
import re
import sqlite3
import subprocess
import sys
import tempfile

from e2e.cargo_paths import cargo_binary
from real_e2e_events import summarize
from test_real_cli_long_tasks import fixture, observed_provider, selected_route


ROOT = Path(__file__).resolve().parents[1]
MAIN_SESSION = "coding-main-live"
BRIEF = """Fix duration.py so test_duration.py passes. Delegate the implementation to
Antigravity and the review to Codex, using their named agent profiles. Have the
implementer commit only duration.py in its assigned checkout. Review and test that
exact commit, then report the commit ID, review findings, and test result here.
Keep the original checkout and tests intact. Do not push."""


def git(project, *args):
    return subprocess.check_output(["git", "-C", str(project), *args], text=True).strip()


def identifies_commit(project, text, commit):
    # Reviewers often display Git's abbreviated object name. Resolve it in this
    # repository so that ambiguous prefixes and unrelated revisions fail.
    for ref in re.findall(r"\b[0-9a-f]{7,64}\b", text):
        result = subprocess.run(["git", "-C", str(project), "rev-parse", "--verify", "--quiet",
                                 ref + "^{commit}"], text=True, capture_output=True)
        if result.returncode == 0 and result.stdout.strip() == commit:
            return True
    return False


def check_evidence(folder):
    project = folder / "qa/project"
    with sqlite3.connect(folder / "qa/nola.db") as db:
        db.row_factory = sqlite3.Row
        result = db.execute(
            "SELECT rr.result_json FROM run_results rr JOIN runs r ON r.id=rr.run_id WHERE r.conversation_id=?", (MAIN_SESSION,)
        ).fetchone()
        if not result or json.loads(result[0])["value"]["kind"] != "answered":
            raise RuntimeError("Main did not seal an answer")
        rows = db.execute("""
            SELECT a.agent_type, a.result_summary, x.project_root, i.backend_session_id
            FROM durable_tasks a
            JOIN durable_tasks x ON x.owner_session_id=a.child_session_id AND x.agent_type=a.agent_type
            JOIN runs r ON r.conversation_id=a.child_session_id
            JOIN delegated_invocations i ON i.run_id=r.id AND i.backend_id=x.agent_type
            WHERE a.owner_session_id=? AND a.kind='agent' AND a.status='sealed'
                AND x.kind='external_executor' AND x.status='completed' AND i.state='succeeded'
            ORDER BY a.created_at_ms
        """, (MAIN_SESSION,)).fetchall()
    if [row["agent_type"] for row in rows] != ["antigravity", "codex"]:
        raise RuntimeError("Expected successful implementation and review under this Main, in that order")
    if any(not row["backend_session_id"] for row in rows):
        raise RuntimeError("Missing committed native executor session")
    implementation, review = rows
    commit = git(implementation["project_root"], "rev-parse", "HEAD")
    if commit == git(project, "rev-parse", "HEAD"):
        raise RuntimeError("Expected an independent implementation commit")
    if not all(identifies_commit(project, row["result_summary"], commit) for row in rows):
        raise RuntimeError("Implementation and review did not identify the same commit")
    answer = summarize(folder / "events.ndjson", folder / "qa/nola.db")["assistant"]
    if not identifies_commit(project, answer, commit):
        raise RuntimeError("Main did not identify the reviewed revision in its final answer")
    if git(project, "diff", "--name-only", "HEAD", commit) != "duration.py":
        raise RuntimeError("Delivery modifies files outside the task")
    if git(project, "status", "--porcelain", "--untracked-files=no"):
        raise RuntimeError("Original checkout was modified")
    with tempfile.TemporaryDirectory(prefix="nola-verify-commit-") as scratch:
        for revision, should_pass in [("HEAD", False), (commit, True)]:
            for name in ("duration.py", "test_duration.py"):
                content = subprocess.check_output(["git", "-C", str(project), "show", f"{revision}:{name}"])
                (Path(scratch) / name).write_bytes(content)
            run = subprocess.run([sys.executable, "-E", "-s", "-B", "test_duration.py"],
                                 cwd=scratch, capture_output=True)
            if (run.returncode == 0) != should_pass or (not should_pass and b"AssertionError" not in run.stderr):
                raise RuntimeError(f"Unexpected independent test result at {revision}")
    return {"implementation": implementation["agent_type"], "review": review["agent_type"],
            "commit": commit, "baseline_test": "failed", "delivery_test": "passed"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=Path.home() / ".nola/config.json")
    parser.add_argument("--binary", type=Path, default=cargo_binary(ROOT, profile="debug"))
    parser.add_argument("--implementation-model", help="Exact ID reported by agy models")
    parser.add_argument("--review-model", help="Exact model ID from the installed Codex")
    parser.add_argument("--check-existing", type=Path, help="Recheck saved evidence without calling a model")
    args = parser.parse_args()
    if args.check_existing:
        print(json.dumps(check_evidence(args.check_existing), indent=2))
        return
    upstream, main_model, credential = selected_route(args.config)
    folder = Path(tempfile.mkdtemp(prefix="nola-coding-handoff-"))
    print(f"Evidence directory: {folder}", flush=True)
    binary = str(args.binary.resolve())
    with observed_provider(upstream, main_model, credential, folder / "provider.ndjson") as url:
        project, env, config = fixture(folder / "qa", main_model, url)
        env["NOLA_EXTERNAL_EXECUTION_HOME"] = str(Path.home())
        env["PATH"] = os.environ["PATH"]
        settings = json.loads(config.read_text())
        settings["agents"] = {}
        for backend, model in [("antigravity", args.implementation_model), ("codex", args.review_model)]:
            executor = {"backend": backend}
            if model:
                executor["model"] = model
            settings["agents"][backend] = {"executor": executor}
        config.write_text(json.dumps(settings))
        (project / "duration.py").write_text("def minutes(seconds):\n    return seconds * 60\n")
        (project / "test_duration.py").write_text("from duration import minutes\nassert minutes(120) == 2\n")
        git(project, "init", "-q")
        git(project, "add", "duration.py", "test_duration.py")
        git(project, "config", "user.name", "Nola fixture")
        git(project, "config", "user.email", "fixture@example.invalid")
        git(project, "commit", "-qm", "Initial test fixture")
        subprocess.run([binary, "model", "apply-config", str(config)], cwd=project, env=env, check=True,
                       stdout=subprocess.DEVNULL)
        with (folder / "events.ndjson").open("w") as out, (folder / "stderr").open("w") as err:
            subprocess.run([binary, BRIEF, "--project", str(project), "--allow-dir", str(folder),
                            "--bash-prefix", "git", "--bash-prefix", "python3", "--session", MAIN_SESSION,
                            "--json", "--yes", "--max-iterations", "32"], cwd=project, env=env,
                           stdout=out, stderr=err, timeout=600, check=True)
    print(json.dumps(check_evidence(folder), indent=2))


if __name__ == "__main__":
    main()
