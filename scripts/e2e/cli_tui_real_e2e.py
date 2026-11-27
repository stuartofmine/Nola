#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real-model end-to-end verification for Nola CLI and TUI."""

import json
import os
import pty
import select
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
NOLA_BIN = REPO_ROOT / "target" / "debug" / "nola"


def resolve_real_llm():
    endpoint = os.environ.get("NOLA_REAL_LLM_ENDPOINT", "http://192.168.31.18:8081/v1").strip()
    model = os.environ.get("NOLA_REAL_LLM_MODEL", "qwen3.8-flash-next-iq3_s").strip()
    api_key = (
        os.environ.get("NOLA_REAL_LLM_API_KEY")
        or os.environ.get("NOLA_PRIMARY_API_KEY")
        or ""
    ).strip()
    if not api_key:
        raise RuntimeError("Set NOLA_REAL_LLM_API_KEY or NOLA_PRIMARY_API_KEY for real-model E2E.")
    return endpoint, model, api_key


def run_cli(args, env, timeout=90):
    return subprocess.run(
        [str(NOLA_BIN)] + args,
        env=env,
        capture_output=True,
        text=True,
        timeout=timeout,
    )


def main():
    evidence_dir = Path(
        os.environ.get("NOLA_E2E_EVIDENCE_DIR", "/tmp/nola-real-e2e-evidence")
    ) / "cli-tui"
    evidence_dir.mkdir(parents=True, exist_ok=True)
    endpoint, model, api_key = resolve_real_llm()

    tmp = Path(tempfile.mkdtemp(prefix="nola-cli-real-e2e-"))
    home = tmp / "home"
    data = tmp / "data"
    ws = tmp / "workspace"
    home.mkdir(parents=True, exist_ok=True)
    data.mkdir(parents=True, exist_ok=True)
    ws.mkdir(parents=True, exist_ok=True)

    # Seed isolated config pointing to the real LAN model via env var (never written in plaintext)
    nola_dir = home / ".nola"
    nola_dir.mkdir(parents=True, exist_ok=True)
    cfg = {
        "llm": {
            "connections": [
                {
                    "id": "lan-e2e",
                    "name": "LAN Real Model",
                    "endpoint": endpoint,
                    "protocol": "chat_completions",
                    "api_key_env": "NOLA_E2E_KEY",
                    "models": [{"id": model, "name": model, "context_window": 32768}],
                }
            ],
            "default_model": {"connection": "lan-e2e", "model": model},
        }
    }
    (nola_dir / "config.json").write_text(json.dumps(cfg, indent=2), encoding="utf-8")

    env = os.environ.copy()
    env["HOME"] = str(home)
    env["NOLA_DATA_DIR"] = str(data)
    env["NOLA_DB_PATH"] = str(data / "nola.db")
    env["NOLA_E2E_KEY"] = api_key
    env["OPENAI_API_KEY"] = api_key

    report = {"endpoint": endpoint, "model": model, "checks": []}
    try:
        # 1. Real model + real write_file tool execution on disk
        marker = "CLI_REAL_TOOL_PROOF_2026"
        prompt = (
            f"Use the write_file tool to create a file named proof.txt in the workspace "
            f"containing exactly '{marker}'. After writing it, reply with 'DONE_{marker}'."
        )
        p = run_cli(
            [
                "run",
                "--json",
                "--project",
                str(ws),
                "--endpoint",
                endpoint,
                "-m",
                model,
                "--profile",
                "allow_all",
                prompt,
            ],
            env,
            timeout=120,
        )
        proof_file = ws / "proof.txt"
        file_ok = proof_file.is_file() and marker in proof_file.read_text(encoding="utf-8", errors="ignore")
        stdout_ok = p.returncode == 0 and (marker in p.stdout or file_ok)
        report["checks"].append(
            {
                "name": "cli_real_model_write_file_tool",
                "ok": p.returncode == 0 and file_ok,
                "returncode": p.returncode,
                "file_exists": proof_file.is_file(),
                "file_content": proof_file.read_text(encoding="utf-8", errors="ignore").strip()[:120]
                if proof_file.is_file()
                else None,
                "stdout_snippet": p.stdout.strip()[:400],
                "stderr_snippet": p.stderr.strip()[:200],
            }
        )

        # 2. Core CLI commands (memory, goal, sessions, wiki)
        for name, args in [
            ("cli_memory_create", ["memory", "create", "--title", "E2E Fact", "--content", "Verified real CLI"]),
            ("cli_memory_list", ["memory", "list"]),
            ("cli_goal_create", ["goal", "create", "E2E Goal"]),
            ("cli_sessions_list", ["sessions", "list"]),
            ("cli_wiki_crate", ["wiki", str(REPO_ROOT / "crates" / "nola-types"), "--json"]),
        ]:
            res = run_cli(args, env, timeout=45)
            report["checks"].append(
                {
                    "name": name,
                    "ok": res.returncode == 0,
                    "returncode": res.returncode,
                    "output_len": len(res.stdout),
                }
            )

        # 3. Interactive TUI PTY check with real render capture
        master, slave = pty.openpty()
        proc = subprocess.Popen(
            [str(NOLA_BIN), "tui", "--db", str(data / "nola.db"), "-m", model, "-y"],
            stdin=slave,
            stdout=slave,
            stderr=slave,
            env=env,
            close_fds=True,
        )
        os.close(slave)
        buf = bytearray()

        def drain(sec=0.8):
            end = time.time() + sec
            while time.time() < end:
                r, _, _ = select.select([master], [], [], 0.1)
                if r:
                    try:
                        chunk = os.read(master, 8192)
                        if not chunk:
                            break
                        buf.extend(chunk)
                    except OSError:
                        break

        def send_pty(data):
            try:
                if proc.poll() is None:
                    os.write(master, data)
            except OSError:
                pass

        drain(1.2)
        send_pty(b"/help\r")
        drain(0.8)
        send_pty(b"\x1b")
        drain(0.3)
        send_pty(b"/quit\r")
        drain(0.8)
        if proc.poll() is None:
            send_pty(b"\x03")
            drain(0.5)
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=3)
            except subprocess.TimeoutExpired:
                proc.kill()
        os.close(master)
        transcript = buf.decode("utf-8", "replace")
        (evidence_dir / "tui-transcript.ansi.txt").write_text(transcript, encoding="utf-8")
        tui_ok = proc.returncode == 0 and len(transcript) > 500 and "panic" not in transcript.lower()
        report["checks"].append(
            {
                "name": "tui_pty_render_and_help",
                "ok": tui_ok,
                "returncode": proc.returncode,
                "bytes": len(transcript),
            }
        )

        report["passed"] = all(c["ok"] for c in report["checks"])
        (evidence_dir / "summary.json").write_text(
            json.dumps(report, indent=2, ensure_ascii=False), encoding="utf-8"
        )
        print(json.dumps(report, indent=2, ensure_ascii=False))
        sys.exit(0 if report["passed"] else 1)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
