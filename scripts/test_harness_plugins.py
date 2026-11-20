# SPDX-License-Identifier: Apache-2.0
"""Exercise installed Pi and dsh CLIs with local plugin packages, without a provider."""
import argparse
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import tempfile
import termios
import fcntl
import time


def native_boot(argv, cwd, env, marker):
    master, slave = pty.openpty()
    process = None
    try:
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 110, 0, 0))
        process = subprocess.Popen(argv, cwd=cwd, env=env, stdin=slave, stdout=slave,
                                   stderr=slave, start_new_session=True)
        os.close(slave)
        slave = None
        deadline = time.monotonic() + 45
        output = bytearray()
        while time.monotonic() < deadline:
            if marker.exists():
                assert marker.read_text() == "loaded"
                return
            if select.select([master], [], [], 0.1)[0]:
                try:
                    output.extend(os.read(master, 8192))
                except OSError:
                    break
                if b"\x1b[6n" in output[-64:]:
                    os.write(master, b"\x1b[1;1R")
            if process.poll() is not None:
                break
        raise AssertionError(f"{argv[-2]} plugin did not load: {output[-3000:]!r}")
    finally:
        if process is not None:
            try:
                os.killpg(process.pid, signal.SIGTERM)
                process.wait(timeout=5)
            except ProcessLookupError:
                pass
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
        os.close(master)
        if slave is not None:
            os.close(slave)


def exercise(binary, root, runtime, host_binary=None):
    project = root / runtime
    package = project / "demo package"
    package.mkdir(parents=True)
    home = project / "home"
    home.mkdir()
    env = {**os.environ, "HOME": str(home), "NOLA_HOME": str(home),
           "NOLA_DB_PATH": str(home / "test.db"), "TERM": "xterm-256color",
           "NOLA_PRIVATE_TEST_CANARY": "private-test-value"}
    marker = project / "loaded.txt"
    name = f"nola-{runtime}-fixture"
    manifest = {"name": name, "version": "1.0.0", "type": "module", "main": "./index.js"}
    board = '<!doctype html><title>Task board</title><button>Start task</button>'
    (package / "board.html").write_text(board)
    manifest["nola"] = {"schema_version": 1, "pages": [
        {"id": "board", "title": "Task board", "entry": "./board.html"},
        {"id": "missing", "title": "Missing", "entry": "absent.html"},
        {"id": "invalid", "title": "Invalid", "entry": "../board.html"}
    ]}
    if runtime == "pi":
        manifest["pi"] = {"extensions": ["./index.js"]}
        entry = "export default function activate(pi)"
    else:
        manifest["dsh"] = {"bundle": {"patch": "./cordis.patch.yml"}}
        (package / "cordis.patch.yml").write_text(f"- insert:\n    - id: nola-fixture\n      name: {name}\n")
        entry = "export function apply(ctx)"
    (package / "package.json").write_text(json.dumps(manifest))
    (package / "index.js").write_text(
        "import {writeFileSync} from 'node:fs';\n" + entry + " {\n"
        "  if (process.env.NOLA_PRIVATE_TEST_CANARY) throw Error('environment leaked');\n"
        f"  writeFileSync({json.dumps(str(marker))}, 'loaded');\n}}\n"
    )
    cli = [str(binary), "plugin", "pi" if runtime == "pi" else "dsh"]
    if runtime == "dsh-web":
        cli.extend(["--profile", "web"])

    def invoke(*args):
        result = subprocess.run([*cli, *args], cwd=project, env=env,
                                capture_output=True, text=True, timeout=180)
        if result.returncode:
            raise RuntimeError(f"{runtime} {args[0]}: {result.stdout[-1500:]} {result.stderr[-2000:]}")
        return result.stdout

    missing = subprocess.run([*cli, "list"], cwd=project, env={**env, "PATH": str(home)},
                             capture_output=True, text=True, timeout=15)
    assert missing.returncode and "PATH" in missing.stderr, missing.stderr
    source = "./demo package"
    invoke("install", source)
    def desktop(*arguments, executable=binary, check=True):
        return subprocess.run([str(executable), "plugin", *arguments], cwd=project, env=env,
                              capture_output=True, text=True, timeout=60, check=check)

    listed = invoke("list")
    profile_name = {"pi": "pi", "dsh": "tui", "dsh-web": "web"}[runtime]
    expected_id = f"{profile_name}:{name}:board"
    metadata = {"package": name, "id": expected_id, "title": "Task board"}
    found = json.loads(desktop("desktop-pages").stdout)
    assert found == [metadata], found
    assert json.loads(desktop("desktop-pages", expected_id).stdout) == {**metadata, "html": board}
    inventory_label = "demo package" if runtime == "pi" else name
    assert inventory_label in listed, listed
    profile = home / "test.plugins" / "harness" / cli[2]
    if runtime == "pi":
        saved = json.loads((profile / "profile" / "settings.json").read_text())["packages"][0]
        package_key = saved if isinstance(saved, str) else saved["source"]
    else:
        package_key = name
    page = {"package": package_key, "title": "Board", "url": "https://board.example.org/tasks"}
    catalog = profile / ("web-pages.json" if runtime == "dsh-web" else "pages.json")
    catalog.write_text(json.dumps({"version": 1, "pages": [page, {**page, "package": "absent-package"}]}))
    assert json.loads(invoke("pages")) == [page]
    def library(executable=binary):
        profiles = json.loads(desktop("desktop-library", executable=executable).stdout)["profiles"]
        return {p["profile"]: p for p in profiles}

    link = {"title": page["title"], "url": page["url"]}
    listing = {"source": package_key, "pages": [{"id": expected_id, "title": "Task board"}], "links": [link]}
    assert library()[profile_name]["packages"] == [listing]
    if host_binary:
        assert library(host_binary.resolve())[profile_name]["packages"] == [listing]

    # Damage a different, otherwise empty fixture profile. Existing healthy pages and links must survive.
    peer = "web" if profile_name != "web" else "tui"
    damaged = home / "test.plugins" / "harness" / "dsh" / "profile" / "profiles" / peer / "package.json"
    original = damaged.read_bytes() if damaged.is_file() else None
    damaged.parent.mkdir(parents=True, exist_ok=True)
    damaged.write_text("{")
    assert library()[peer]["error"]
    assert library()[profile_name]["packages"] == [listing]
    assert json.loads(desktop("desktop-pages").stdout) == [metadata]
    assert json.loads(desktop("desktop-pages", expected_id).stdout)["html"] == board
    if original is None:
        damaged.unlink()
    else:
        damaged.write_bytes(original)
    assert "error" not in library()[peer]

    if runtime == "pi":
        # Two installed sources claiming the same page may retain their own service links, but
        # neither list may advertise an ambiguous desktop destination.
        from shutil import copytree
        duplicate = project / "same-name"
        copytree(package, duplicate)
        settings = profile / "profile" / "settings.json"
        original = settings.read_bytes()
        values = json.loads(original)
        values["packages"].append(str(duplicate))
        settings.write_text(json.dumps(values))
        records = library()["pi"]["packages"]
        assert len(records) == 2 and all(not row["pages"] for row in records)
        assert {row["source"]: row["links"] for row in records} == {package_key: [link], str(duplicate): []}
        assert desktop("desktop-pages", expected_id, check=False).returncode != 0
        settings.write_bytes(original)

    failed = subprocess.run([*cli, "install", "./missing package"], cwd=project, env=env,
                            capture_output=True, text=True, timeout=60)
    assert failed.returncode and cli[2] in failed.stderr, failed.stderr
    assert inventory_label in invoke("list")
    marker.unlink(missing_ok=True)
    native_boot([*cli, "open"], project, env, marker)
    invoke("remove", source if runtime == "pi" else name)
    assert inventory_label not in invoke("list")
    assert json.loads(invoke("pages")) == []
    assert json.loads(desktop("desktop-pages").stdout) == []
    assert desktop("desktop-pages", expected_id, check=False).returncode != 0
    if host_binary:
        runtime_name = cli[2]

        def host(*arguments):
            return json.loads(desktop(*arguments, executable=host_binary.resolve()).stdout)
        assert host("desktop-manage", runtime_name, profile_name, "list")["available"]
        saved = host("desktop-manage", runtime_name, profile_name, "install", str(package))
        assert host("desktop-pages") == [metadata]
        assert host("desktop-pages", expected_id)["html"] == board
        assert saved["packages"]
        host("desktop-manage", runtime_name, profile_name, "remove", saved["packages"][0])
        assert host("desktop-pages") == []
    assert not (home / ".pi" / "agent" / "settings.json").exists()
    assert not (home / ".dsh" / "profiles").exists()
    print(f"{runtime}: install, list, desktop page discovery/read, native activation, remove and profile isolation passed", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--nola", required=True, type=Path)
    parser.add_argument("--host", type=Path, help="Also exercise the native desktop helper")
    parser.add_argument("--runtime", choices=["pi", "dsh", "dsh-web"], action="append")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="nola-harness-e2e-") as tmp:
        for runtime in args.runtime or ["pi", "dsh", "dsh-web"]:
            exercise(args.nola.resolve(), Path(tmp), runtime, args.host)
