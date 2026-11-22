#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Exercise a real provider and public GitHub package in an isolated Nola Host."""
import argparse
from contextlib import contextmanager
import os
from http.cookiejar import CookieJar
import json
from pathlib import Path
import socket
import subprocess
import time
from urllib.request import Request, build_opener, HTTPCookieProcessor
import tempfile

from real_e2e_events import summarize
from test_real_remote_host_long_tasks import committed_tools
from test_real_cli_long_tasks import fixture, observed_provider, selected_route

ROOT = Path(__file__).resolve().parents[1]
SKILL = "nature-paper-card"
REPO = "https://github.com/Yuan1z0825/nature-skills"
LINK = f"{REPO}/tree/main/skills/{SKILL}"


@contextmanager
def github_skill_fixture_directory(prefix):
    with tempfile.TemporaryDirectory(prefix=prefix) as directory:
        try:
            yield Path(directory)
        finally:
            # The database writer finishes publishing generations after its CLI
            # parent exits. Wait for ownership to be released before cleanup.
            lock = Path(directory) / "fixture/nola.db.service.lock"
            if lock.exists():
                with lock.open("r+b") as stream:
                    deadline = time.monotonic() + 30
                    while True:
                        try:
                            if os.name == "nt":
                                import msvcrt
                                msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
                            else:
                                import fcntl
                                fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                            break
                        except BlockingIOError:
                            if time.monotonic() >= deadline:
                                raise RuntimeError("The fixture database writer did not stop in time")
                            time.sleep(.1)


def save_web_settings(binary, project, env, browser_evidence=None):
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    origin = f"http://127.0.0.1:{port}"
    opener = build_opener(HTTPCookieProcessor(CookieJar()))

    def request(path, body=None):
        data = json.dumps(body).encode() if body is not None else None
        req = Request(origin + path, data=data, headers={"Content-Type": "application/json", "Origin": origin})
        with opener.open(req, timeout=30) as response:
            return json.load(response)

    command = [binary, "web", "--host", "127.0.0.1", "--port", str(port), "--project", str(project),
               "--web-root", str(ROOT / "apps/web")]
    with (project.parent / "web.log").open("w") as log:
        process = subprocess.Popen(command, cwd=project, env=env, stdout=log, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + 20
            while True:
                if process.poll() is not None:
                    raise RuntimeError("Isolated Web Host stopped during startup")
                try:
                    with opener.open(origin + "/login", timeout=1):
                        break
                except OSError:
                    if time.monotonic() > deadline:
                        raise RuntimeError("Isolated Web Host startup timed out")
                    time.sleep(0.1)
            token = Path(env["HOME"]) / ".nola/web/mobile-access-token"
            request("/api/web/login", {"token": token.read_text().strip()})
            if browser_evidence is not None:
                browser_evidence.mkdir(parents=True, exist_ok=True)
                subprocess.run(["node", ROOT / "apps/web/playwright_runtime/real_github_skill.cjs",
                                origin, project.parent, browser_evidence], check=True, timeout=600)
                conversations = json.loads((browser_evidence / "web-conversations.json").read_text())
                database = Path(env["NOLA_DB_PATH"])
                installs = committed_tools(database, conversations["install"])
                uses = committed_tools(database, conversations["fresh"])
                assert any(t["tool"] == "github_skill_install" and t["state"] == "completed" for t in installs)
                assert any(t["tool"].lower() == "skill" and t["state"] == "completed" for t in uses)
                assert any("terminology-ledger.md" in json.dumps(t["arguments"]) and t["state"] == "completed" for t in uses)
            request("/api/agent/settings", {})
            settings = request("/api/agent/settings")
            skills = settings.get("uploadedSkills", settings.get("skills", []))
            installed = next(s for s in skills if s["id"] == SKILL)
            assert installed["enabled"] and installed["source_url"].startswith(REPO + "/tree/")
            baseline = [{"id": s["id"], "revision": s["revision"]} for s in skills]
            installed["description"] += " (acceptance)"
            request("/api/agent/settings", {"skills": skills, "skill_baseline": baseline})
            after = request("/api/agent/settings")
            stored = next(s for s in after.get("uploadedSkills", after.get("skills", [])) if s["id"] == SKILL)
            assert stored["description"] == installed["description"] and stored["source_url"] == installed["source_url"]
        finally:
            process.terminate()
            process.wait(timeout=15)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--entry", choices=("url", "name", "web"), default="url")
    parser.add_argument("--config", type=Path, default=Path.home() / ".nola/config.json")
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/debug")
    args = parser.parse_args()
    binary = args.bin_dir.resolve() / "nola"
    upstream, model, credential = selected_route(args.config)
    with github_skill_fixture_directory("nola-real-github-skill-") as base:
        with observed_provider(upstream, model, credential, base / "provider.ndjson") as relay:
            project, env, config = fixture(base / "fixture", model, relay)
            env.update(HOME=env["NOLA_HOME"], NOLA_APPLICATION_SERVICE_EXECUTABLE=str(binary.with_name("nola-service")))

            def cli(*arguments):
                result = subprocess.run([binary, *arguments], cwd=project, env=env, check=True,
                                        capture_output=True, text=True, timeout=600)
                return result.stdout

            def turn(session, prompt):
                events = base / f"{session}.jsonl"
                events.write_text(cli(prompt, "--project", str(project), "--lang", "zh", "--json",
                                      "--network-policy", "allow", "--session", session, "--max-iterations", "20"))
                return summarize(events, Path(env["NOLA_DB_PATH"]))

            def registration():
                skills = json.loads(cli("skills", "list", "--json"))["skills"]
                return [s for s in skills if s["id"] == SKILL]

            cli("model", "apply-config", str(config))
            assert not registration(), "Target already exists before the installation"
            if args.entry == "web":
                save_web_settings(binary, project, env, ROOT / ".artifacts/platform-github-skill")
                return
            target = LINK if args.entry == "url" else "GitHub 上 Nature Skills 里做论文精读卡的 nature-paper-card"
            installed = turn("install", f"请找到并安装 {target}。启用它，让我在新会话中使用。")
            assert "github_skill_install" in installed["toolsCompleted"], installed["toolsCompleted"]
            records = registration()
            assert len(records) == 1, f"Expected the installed Skill in settings. Model reply: {installed['assistant']}"
            registered, = records
            assert registered["enabled"] and registered["source"] == "custom", registered["id"]
            assert registered["source_url"].startswith(REPO + "/tree/"), registered["source_url"]
            root = Path(env["HOME"]) / ".nola/cli/skills/custom" / SKILL
            package = root / ".nola-package/skills" / SKILL
            script = package / "scripts/prepare_paper.py"
            template = package.parent / "nature-shared/core/terminology-ledger.md"
            original = [file.read_bytes() for file in (script, template)]
            cli("skills", "disable", SKILL)
            assert not root.exists() and not registration()[0]["enabled"]
            cli("skills", "enable", SKILL)
            assert [file.read_bytes() for file in (script, template)] == original
            save_web_settings(binary, project, env)
            assert [file.read_bytes() for file in (script, template)] == original
            result = turn("fresh", "请实际调用 nature-paper-card，确认准备 PDF 输入用哪个随包脚本，并读取清单要求的术语表文件，引用它的英文标题。")
            assert "Skill" in result["toolsCompleted"], result["toolsCompleted"]
            assert "prepare_paper.py" in result["assistant"], result["assistant"]
            assert "read_file" in result["toolsCompleted"] or "read_many_files" in result["toolsCompleted"], result["toolsCompleted"]
            assert "terminology ledger" in result["assistant"].lower(), result["assistant"]
            print(f"{args.entry}: {model} installed and invoked {SKILL} from {registered['source_url']}")


if __name__ == "__main__":
    main()
