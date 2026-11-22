#!/usr/bin/env python3
"""Exercise file selection, preview and Host persistence through the installed CLI."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import time


class ModelEndpoint(BaseHTTPRequestHandler):
    models = []
    authorizations = []

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        assert self.path == "/v1/chat/completions"
        self.models.append(request["model"])
        self.authorizations.append(self.headers.get("Authorization"))
        message = {"role": "assistant", "content": "STARTUP_TEST_OK"}
        base = {"id": "fixture", "model": request["model"], "created": 1}
        if request.get("stream"):
            frames = [
                {"index": 0, "delta": message, "finish_reason": None},
                {"index": 0, "delta": {}, "finish_reason": "stop"},
            ]
            body = "".join("data: " + json.dumps(dict(base, object="chat.completion.chunk", choices=[frame])) + "\n\n"
                           for frame in frames) + "data: [DONE]\n\n"
            content_type = "text/event-stream"
        else:
            body = json.dumps(dict(base, object="chat.completion", choices=[{"index": 0, "message": message, "finish_reason": "stop"}]))
            content_type = "application/json"
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body.encode())))
        self.end_headers()
        self.wfile.write(body.encode())

    def log_message(self, *args):
        pass


def exercise(binary, root, endpoint):
    home = root / "home"
    user = home / ".nola"
    project = root / "repo"
    subdir = project / "src"
    db = root / "state" / "config.sqlite"
    user.mkdir(parents=True)
    project.mkdir()
    subprocess.run(["git", "init", "--quiet", str(project)], check=True)
    subprocess.run(["git", "-C", str(project), "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                    "-c", "commit.gpgsign=false", "commit", "--quiet", "--allow-empty", "-m", "fixture"], check=True)
    subdir.mkdir()
    env = {k: v for k, v in os.environ.items() if not k.startswith("NOLA_")}
    env.update(HOME=str(home), NOLA_HOME=str(home), NOLA_DB_PATH=str(db), NOLA_CLI_LANG="en")

    def run(*args, ok=True):
        result = subprocess.run([binary, *args], cwd=subdir, env=env, stdin=subprocess.DEVNULL,
                                capture_output=True, text=True, timeout=40)
        assert (result.returncode == 0) == ok, (args, result.returncode, result.stderr)
        return result.stdout if ok else result.stderr

    def result(*args):
        return json.loads(run("--json", *args))

    template = result("config", "example")
    template["llm"]["connections"][0]["endpoint"] = endpoint
    assert not result("config")["files"]
    assert result("config")["saved_default"] == {"kind": "automatic"}
    assert not db.parent.exists()

    def catalog(path, identifier):
        value = json.loads(json.dumps(template))
        value["llm"]["connections"][0]["id"] = identifier
        value["llm"]["default_model"]["connection"] = identifier
        path.write_text("// catalog\n" + json.dumps(value, indent=2)[:-1] + ",\n}\n")

    global_file = user / "config.jsonc"
    project_file = project / "nola.jsonc"
    child_file = subdir / "nola.json"
    catalog(global_file, "personal")
    catalog(project_file, "work")
    child_file.write_text('{"language":"en"}')
    sources = [str(global_file), str(project_file), str(child_file)]
    state = result("config")
    assert state["files"] == sources and state["catalog_source"] == str(project_file), state
    assert state["default_model"]["connection"] == "work"
    assert result("config", "check")["status"] == "checked"
    assert json.loads(run("config", "--json", "show"))["runtime_settings"] == {"language":"en"}
    preview = result("config", "apply", "--dry-run")
    assert preview["status"] == "preview" and preview["default_model"] == state["default_model"]
    legacy = result("model", "apply-config", "--dry-run")
    assert legacy["status"] == "preview" and legacy["catalog_source"] == str(global_file)
    loop = root / "gateway.json"
    copy = json.loads(json.dumps(template))
    copy["llm"]["connections"][0]["endpoint"] = "http://localhost:5001/v1"
    loop.write_text(json.dumps(copy))
    assert "nola_gateway_loop" in run("config", "apply", str(loop), "--dry-run", ok=False)
    assert not db.parent.exists(), "Preview created Host persistence"
    print("PASS file precedence, shared JSONC syntax, effective catalog and read-only preview")

    link = subdir / "linked.jsonc"
    link.symlink_to(project_file)
    assert result("config", "check", str(link))["default_model"]["connection"] == "work"
    assert "source_not_a_bounded_regular_file" in run("config", "apply", str(link), "--dry-run", ok=False)
    assert not db.parent.exists()
    direct = subdir / "route.json"
    direct.write_text('{"llm":{"endpoint":"http://127.0.0.1:9/v1","model":"direct","api_key":"marker-private"}}')
    raw = run("config", "apply", "--dry-run", "route.json", "--json")
    assert "marker-private" not in raw and json.loads(raw)["status"] == "nothing_to_apply"
    assert result("config", "apply", "route.json")["status"] == "nothing_to_apply"
    assert not db.parent.exists()

    child_file.write_text('{\n "llm": [x]\n}')
    error = run("config", "check", ok=False)
    assert str(child_file) in error and "line 2" in error and "column" in error, error
    assert result("config", "check", "../nola.jsonc")["default_model"]["connection"] == "work"
    editor = root / "fixture-editor.py"
    editor.write_text("from pathlib import Path\nimport sys\n"
                      "assert sys.argv[1] == '--wait'\n"
                      "Path(sys.argv[2]).write_text('{\"language\":\"en\"}')\n")
    env["VISUAL"] = f'"{sys.executable}" "{editor}" --wait'
    assert result("config", "edit")["files"] == [str(child_file)]
    assert result("config", "check")["status"] == "checked"
    extra = subdir / "with spaces.jsonc"
    assert result("config", "edit", str(extra))["files"] == [str(extra)]
    assert not db.parent.exists()
    child_file.write_text('{"language":"en"}')
    print("PASS explicit relative file, direct route without import, redaction and isolated recovery")

    def chat():
        response = subprocess.run([binary, "run", "--no-tools", "--no-save", "Reply with STARTUP_TEST_OK"],
                                  cwd=subdir, env=env, stdin=subprocess.DEVNULL, capture_output=True,
                                  text=True, timeout=40)
        assert response.returncode == 0 and "STARTUP_TEST_OK" in response.stdout, response.stderr
        return response.stderr

    original = project_file.read_text()
    project_file.write_text('{"llm":{"connections":[],"default_model":{}}}')
    first_error = run("run", "--no-tools", "--no-save", "First launch", ok=False)
    assert str(project_file) in first_error and "nola config edit" in first_error
    assert not ModelEndpoint.models and not db.exists()
    project_file.write_text(original)
    chat()
    assert db.is_file() and ModelEndpoint.models == ["my-model"]
    original_catalog = project_file.read_text()
    chat()
    bad = json.loads(json.dumps(template))
    bad["llm"]["default_model"]["model"] = "undeclared"
    project_file.write_text(json.dumps(bad))
    diagnostic = chat()
    assert str(project_file) in diagnostic and "undeclared" in diagnostic, diagnostic
    assert "nola config edit" in diagnostic
    assert ModelEndpoint.models == ["my-model"] * 3
    bad["llm"]["default_model"]["model"] = "my-model"
    bad["llm"]["connections"][0]["id"] = "foreign"
    bad["llm"]["connections"][0]["name"] = "Foreign"
    bad["llm"]["default_model"]["connection"] = "foreign"
    bad["llm"]["connections"][0]["api_key_env"] = "CONFIG_TEST_PRIVATE_KEY"
    env["CONFIG_TEST_PRIVATE_KEY"] = "test-marker"
    project_file.write_text(json.dumps(bad))
    diagnostic = chat()
    assert "api_key_env" in diagnostic and "test-marker" not in diagnostic, diagnostic
    assert ModelEndpoint.models == ["my-model"] * 4
    project_file.write_text(original_catalog.replace("my-model", "next-model"))
    chat()
    assert ModelEndpoint.models[-1] == "next-model"
    project_file.write_text(original_catalog)
    chat()
    assert ModelEndpoint.models[-1] == "my-model"
    print("PASS first-start import, repeat start, rejected catalog fallback, credential scope and changed default")

    applied = result("config", "apply")
    assert applied["status"] == "applied" and applied["receipt"]["connections"] == ["config-work"]
    assert db.is_file()
    saved_default = result("config")["saved_default"]
    assert saved_default == {"kind":"profile","connection":"config-work","model":"my-model","enabled":True}, saved_default
    saved = run("model", "connections")
    assert "config-work" in saved and "config-personal" not in saved
    conflict = run("model", "apply-config", ok=False)
    assert "config-work" in conflict and "name" in conflict, conflict
    # The original catalog used the same connection display name. Correct the
    # declaration in place, then retry the published command.
    global_file.write_text(global_file.read_text().replace('"name": "Local"', '"name": "Personal"'))
    original = result("model", "apply-config")
    assert original["connections"] == ["config-personal"] and original["default_profile_id"]
    assert result("config")["saved_default"]["connection"] == "config-personal"
    assert result("config")["default_model"]["connection"] == "work"
    print("PASS actual Host apply, persisted connection readback and original command receipt")

    # The user's declared route can bind and rotate a key at startup. A later
    # process without that variable can resolve the same private Host binding.
    project_file.unlink()
    personal = json.loads(json.dumps(template))
    connection = personal["llm"]["connections"][0]
    connection.update(id="personal", name="Personal", api_key_env="CONFIG_TEST_PRIVATE_KEY")
    personal["llm"]["default_model"]["connection"] = "personal"
    global_file.write_text(json.dumps(personal))
    chat()
    assert ModelEndpoint.authorizations[-1] == "Bearer test-marker"
    saved = run("model", "connections")
    chat()
    assert run("model", "connections") == saved
    del env["CONFIG_TEST_PRIVATE_KEY"]
    chat()
    assert ModelEndpoint.authorizations[-1] == "Bearer test-marker"
    env["CONFIG_TEST_PRIVATE_KEY"] = "rotated-marker"
    chat()
    assert ModelEndpoint.authorizations[-1] == "Bearer rotated-marker"
    print("PASS automatic credential binding, unchanged restart, reuse and key rotation")

    for command in ("web", "server"):
        connection["models"][0]["id"] = command + "-model"
        personal["llm"]["default_model"]["model"] = command + "-model"
        global_file.write_text(json.dumps(personal))
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        process = subprocess.Popen([binary, command, "--host", "127.0.0.1", "--port", str(port)],
                                   cwd=subdir, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                   stderr=subprocess.PIPE, text=True)
        try:
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                assert process.poll() is None, process.stderr.read()
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=0.3):
                        break
                except OSError:
                    time.sleep(0.05)
            else:
                raise AssertionError(command + " did not listen")
            assert result("config")["saved_default"]["model"] == command + "-model"
        finally:
            process.terminate()
            process.wait(timeout=10)
            process.stderr.close()
    print("PASS Web and server startup publish the changed default before listening")



if __name__ == "__main__":
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/nola").resolve()
    with tempfile.TemporaryDirectory(prefix="nola-config-") as directory:
        with ThreadingHTTPServer(("127.0.0.1", 0), ModelEndpoint) as server:
            threading.Thread(target=server.serve_forever, daemon=True).start()
            try:
                exercise(binary, Path(directory).resolve(), f"http://127.0.0.1:{server.server_port}/v1")
            finally:
                server.shutdown()
