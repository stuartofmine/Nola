#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Unified real-model end-to-end verification runner across all Nola platforms.

Runs real-model E2E suites and captures screenshots + videos into a single evidence directory:
  python3 scripts/e2e/run_real_e2e.py --platforms cli,web,macos,ios,android,windows
"""

import argparse
import json
import os
import platform
import shutil
import signal
import socket
import subprocess
import sys
import threading
import tempfile
import time
import urllib.parse
import uuid
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_ENDPOINT = "http://192.168.31.18:8081/v1"
DEFAULT_MODEL = "qwen3.8-flash-next-iq3_s"
IOS_SIM_UDID = "8F179B32-2478-4862-83A5-22DAB8A4D1BB"
ADB = Path.home() / "Library/Android/sdk/platform-tools/adb"
EMULATOR = Path.home() / "Library/Android/sdk/emulator/emulator"


def get_llm_env():
    endpoint = os.environ.get("NOLA_REAL_LLM_ENDPOINT", DEFAULT_ENDPOINT).strip()
    model = os.environ.get("NOLA_REAL_LLM_MODEL", DEFAULT_MODEL).strip()
    api_key = (
        os.environ.get("NOLA_REAL_LLM_API_KEY")
        or os.environ.get("NOLA_PRIMARY_API_KEY")
        or ""
    ).strip()
    if not api_key:
        raise RuntimeError("Set NOLA_REAL_LLM_API_KEY or NOLA_PRIMARY_API_KEY.")
    return endpoint, model, api_key


class LoopbackTcpRelay:
    """Transparent loopback TCP forwarder so unsigned macOS XCTest hosts (blocked from
    RFC1918 LAN by macOS Local Network TCC) reach the real LAN LLM verbatim."""

    def __init__(self, target_host: str, target_port: int):
        self._target = (target_host, target_port)
        self._stop = threading.Event()
        self._srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self._srv.bind(("127.0.0.1", 0))
        self._srv.listen(32)
        self._srv.settimeout(0.5)
        self.port = self._srv.getsockname()[1]
        self._thread = threading.Thread(target=self._accept_loop, daemon=True)
        self._thread.start()

    def _pipe(self, src: socket.socket, dst: socket.socket):
        try:
            while not self._stop.is_set():
                data = src.recv(65536)
                if not data:
                    break
                dst.sendall(data)
        except OSError:
            pass
        finally:
            try:
                dst.shutdown(socket.SHUT_WR)
            except OSError:
                pass

    def _handle(self, client: socket.socket):
        try:
            upstream = socket.create_connection(self._target, timeout=15)
        except OSError:
            client.close()
            return
        t1 = threading.Thread(target=self._pipe, args=(client, upstream), daemon=True)
        t2 = threading.Thread(target=self._pipe, args=(upstream, client), daemon=True)
        t1.start()
        t2.start()
        t1.join()
        t2.join()
        client.close()
        upstream.close()

    def _accept_loop(self):
        while not self._stop.is_set():
            try:
                conn, _ = self._srv.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            threading.Thread(target=self._handle, args=(conn,), daemon=True).start()

    def close(self):
        self._stop.set()
        try:
            self._srv.close()
        except OSError:
            pass


def run_step(cmd, env, cwd=REPO_ROOT, timeout=300):
    t0 = time.time()
    p = subprocess.run(cmd, env=env, cwd=str(cwd), capture_output=True, text=True, timeout=timeout)
    return {
        "ok": p.returncode == 0,
        "returncode": p.returncode,
        "elapsed_s": round(time.time() - t0, 2),
        "stdout_tail": p.stdout[-1500:],
        "stderr_tail": p.stderr[-1500:],
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--platforms",
        default="cli,web,macos,ios,android,windows",
        help="Comma-separated list: cli,web,macos,ios,android,windows",
    )
    ap.add_argument(
        "--evidence-dir",
        default=str(Path(tempfile.gettempdir()) / "nola-real-e2e-evidence"),
        help="Directory to store screenshots, videos, and summary.json",
    )
    args = ap.parse_args()
    platforms = [p.strip().lower() for p in args.platforms.split(",") if p.strip()]
    supported = {"cli", "web", "macos", "ios", "android", "windows"}
    if not platforms or not set(platforms) <= supported:
        ap.error("select at least one supported platform: " + ", ".join(sorted(supported)))
    evidence = Path(args.evidence_dir)
    evidence.mkdir(parents=True, exist_ok=True)

    endpoint, model, api_key = get_llm_env()
    env = os.environ.copy()
    env["NOLA_REAL_LLM_E2E"] = "1"
    env["NOLA_REAL_LLM_ENDPOINT"] = endpoint
    env["NOLA_REAL_LLM_MODEL"] = model
    env["NOLA_REAL_LLM_API_KEY"] = api_key
    env["NOLA_E2E_EVIDENCE_DIR"] = str(evidence)

    # Write disposable fixture file with 0600 perms for sandboxed Apple/Android test runners, deleted on exit
    fixture_path = Path("/tmp/nola-real-llm-e2e.json")
    fixture_payload = json.dumps(
        {
            "endpoint": endpoint,
            "model": model,
            "apiKey": api_key,
            "protocol": "chat_completions",
            "evidenceDir": str(evidence),
        }
    )
    results = {}
    needs_fixture = bool({"ios", "macos"} & set(platforms))
    owns_fixture = False
    try:
        if needs_fixture:
            fd = os.open(str(fixture_path), os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            owns_fixture = True
            with os.fdopen(fd, "w") as f:
                f.write(fixture_payload)

        if "cli" in platforms:
            print("[e2e] Running CLI & TUI real-model E2E...")
            results["cli"] = run_step(["python3", "scripts/e2e/cli_tui_real_e2e.py"], env, timeout=240)

        if "web" in platforms:
            print("[e2e] Running Web Chrome real-model E2E (with video + screenshots)...")
            results["web"] = run_step(["node", "apps/web/tests/real_e2e.mjs"], env, timeout=300)

        if "windows" in platforms:
            if os.name != "nt":
                results["windows"] = {"ok": False, "reason": "installed Tauri E2E requires a Windows host"}
            else:
                win_env = dict(env, NOLA_LLM_ENDPOINT=endpoint, NOLA_LLM_MODEL=model,
                               NOLA_API_KEY=api_key, NOLA_UI_SCREENSHOTS=str(evidence / "windows"))
                results["windows"] = run_step(
                    ["powershell.exe", "-STA", "-NoProfile", "-ExecutionPolicy", "Bypass",
                     "-File", "apps/windows/scripts/test-installed-ui.ps1"], win_env, timeout=300)

        if "macos" in platforms:
            print("[e2e] Running macOS NolaMac real-model E2E (with NSWindow screenshots)...")
            arch = platform.machine()
            parsed_ep = urllib.parse.urlparse(endpoint)
            relay = None
            mac_env = env.copy()
            if parsed_ep.hostname and parsed_ep.hostname not in ("127.0.0.1", "localhost", "::1"):
                target_port = parsed_ep.port or (443 if parsed_ep.scheme == "https" else 80)
                relay = LoopbackTcpRelay(parsed_ep.hostname, target_port)
                relayed_endpoint = parsed_ep._replace(netloc=f"127.0.0.1:{relay.port}").geturl()
                mac_env["NOLA_REAL_LLM_ENDPOINT"] = relayed_endpoint
                fd_m = os.open(str(fixture_path), os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
                with os.fdopen(fd_m, "w") as fm:
                    fm.write(json.dumps({
                        "endpoint": relayed_endpoint,
                        "model": model,
                        "apiKey": api_key,
                        "protocol": "chat_completions",
                        "evidenceDir": str(evidence),
                    }))
            try:
                results["macos"] = run_step(
                    [
                        "xcodebuild",
                        "test",
                        "-project",
                        str(REPO_ROOT / "apple/Nola.xcodeproj"),
                        "-scheme",
                        "NolaMac",
                        "-configuration",
                        "Test",
                        "-destination",
                        f"platform=macOS,arch={arch}",
                        "-destination-timeout",
                        "15",
                        f"ARCHS={arch}",
                        "ONLY_ACTIVE_ARCH=YES",
                        "CODE_SIGNING_ALLOWED=NO",
                        "-only-testing:NolaMacTests/MacCoreContractsTests",
                        "-only-testing:NolaMacTests/MacRealModelE2ETests",
                    ],
                    mac_env,
                    timeout=360,
                )
            finally:
                if relay is not None:
                    relay.close()
                    fd_r = os.open(str(fixture_path), os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
                    with os.fdopen(fd_r, "w") as fr:
                        fr.write(fixture_payload)

        if "ios" in platforms:
            print("[e2e] Running iOS Simulator real-model E2E (with mp4 video + screenshots)...")
            ios_dir = evidence / "ios"
            ios_dir.mkdir(parents=True, exist_ok=True)
            shutil.rmtree(REPO_ROOT / "build/TestResults.xcresult", ignore_errors=True)
            ios_env = env.copy()
            ios_env["INCLUDE_UI_TESTS"] = "1"
            ios_env["IOS_PARALLEL_TESTING_ENABLED"] = "NO"
            ios_env["SIMULATOR_SIGNING"] = "adhoc"
            ios_env["DESTINATION"] = f"platform=iOS Simulator,id={IOS_SIM_UDID}"
            parsed_ios_ep = urllib.parse.urlparse(endpoint)
            ios_relay = None
            if parsed_ios_ep.hostname and parsed_ios_ep.hostname not in ("127.0.0.1", "localhost", "::1"):
                target_port = parsed_ios_ep.port or (443 if parsed_ios_ep.scheme == "https" else 80)
                ios_relay = LoopbackTcpRelay(parsed_ios_ep.hostname, target_port)
                relayed_ios_ep = parsed_ios_ep._replace(netloc=f"127.0.0.1:{ios_relay.port}").geturl()
                ios_env["NOLA_REAL_LLM_ENDPOINT"] = relayed_ios_ep
                fd_i = os.open(str(fixture_path), os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
                with os.fdopen(fd_i, "w") as fi:
                    fi.write(json.dumps({
                        "endpoint": relayed_ios_ep,
                        "model": model,
                        "apiKey": api_key,
                        "protocol": "chat_completions",
                        "evidenceDir": str(evidence),
                    }))
            try:
                # Build for testing BEFORE starting video recording so recording captures only the UI run.
                build_res = run_step(
                    [
                        "xcodebuild",
                        "build-for-testing",
                        "-project",
                        str(REPO_ROOT / "apple/Nola.xcodeproj"),
                        "-scheme",
                        "Nola",
                        "-configuration",
                        "Test",
                        "-destination",
                        f"platform=iOS Simulator,id={IOS_SIM_UDID}",
                        "-derivedDataPath",
                        "./build",
                        "CODE_SIGN_IDENTITY=-",
                    ],
                    ios_env,
                    timeout=360,
                )
                if not build_res["ok"]:
                    results["ios"] = build_res
                else:
                    subprocess.run(["xcrun", "simctl", "boot", IOS_SIM_UDID], capture_output=True)
                    subprocess.run(["xcrun", "simctl", "bootstatus", IOS_SIM_UDID, "-b"], capture_output=True, timeout=60)
                    video_path = ios_dir / "ios-e2e.mp4"
                    for stale in ios_dir.glob("ios-e2e.mp4*"):
                        stale.unlink(missing_ok=True)
                    rec = subprocess.Popen(
                        ["xcrun", "simctl", "io", IOS_SIM_UDID, "recordVideo", "--force", str(video_path)],
                        stdout=subprocess.DEVNULL,
                        stderr=subprocess.DEVNULL,
                    )
                    try:
                        ios_env["SKIP_BUILD"] = "1"
                        results["ios"] = run_step(
                            [
                                "./scripts/run_client_tests.sh",
                                "-only-testing:NolaTests/IOSCoreContractsTests",
                                "-only-testing:NolaUITests/IOSRealModelE2ETests",
                            ],
                            ios_env,
                            timeout=300,
                        )
                    finally:
                        rec.send_signal(signal.SIGINT)
                        try:
                            rec.wait(timeout=15)
                        except subprocess.TimeoutExpired:
                            rec.terminate()
                            try:
                                rec.wait(timeout=5)
                            except subprocess.TimeoutExpired:
                                rec.kill()
                        subprocess.run(["xcrun", "simctl", "shutdown", IOS_SIM_UDID], capture_output=True)
            finally:
                if ios_relay is not None:
                    ios_relay.close()
                    fd_r2 = os.open(str(fixture_path), os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
                    with os.fdopen(fd_r2, "w") as fr2:
                        fr2.write(fixture_payload)

        if "android" in platforms:
            print("[e2e] Running Android Emulator real-model E2E (with mp4 video + screenshots)...")
            and_dir = evidence / "android"
            and_dir.mkdir(parents=True, exist_ok=True)
            emu = subprocess.Popen(
                [str(EMULATOR), "-avd", "nola_avd", "-no-window", "-no-audio", "-no-boot-anim", "-gpu", "swiftshader_indirect"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            try:
                subprocess.run([str(ADB), "wait-for-device"], timeout=90, check=True)
                for _ in range(60):
                    b = subprocess.run([str(ADB), "shell", "getprop", "sys.boot_completed"], capture_output=True, text=True).stdout.strip()
                    if b == "1":
                        break
                    time.sleep(2)
                subprocess.run([str(ADB), "install", "-r", "-t", str(REPO_ROOT / "apps/android/app/build/outputs/apk/debug/app-debug.apk")], check=True)
                subprocess.run([str(ADB), "install", "-r", "-t", str(REPO_ROOT / "apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk")], check=True)
                run_uuid = str(uuid.uuid4())
                marker = json.dumps({"disposable": True, "run_uuid": run_uuid, "avd_name": "nola_avd", "user_id": 0})
                subprocess.run(
                    [str(ADB), "shell", "run-as com.nola sh -c 'mkdir -p files && cat > files/nola-owned-test-avd.json'"],
                    input=marker,
                    text=True,
                    check=True,
                )
                rec = subprocess.Popen(
                    [str(ADB), "shell", "screenrecord", "--time-limit", "180", "/sdcard/nola-android-e2e.mp4"],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
                try:
                    and_res = run_step(
                        [
                            str(ADB),
                            "shell",
                            "am",
                            "instrument",
                            "-w",
                            "-r",
                            "-e",
                            "class",
                            "com.nola.RealModelE2EAndroidTest",
                            "-e",
                            "isolatedUserId",
                            "0",
                            "-e",
                            "isolatedAvdRunId",
                            run_uuid,
                            "-e",
                            "isolatedAvdName",
                            "nola_avd",
                            "-e",
                            "realLlmEndpoint",
                            endpoint,
                            "-e",
                            "realLlmModel",
                            model,
                            "-e",
                            "realLlmApiKey",
                            api_key,
                            "com.nola.test/androidx.test.runner.AndroidJUnitRunner",
                        ],
                        env,
                        timeout=300,
                    )
                    and_res["ok"] = (
                        and_res["ok"]
                        and ("OK (" in and_res["stdout_tail"])
                        and ("FAILURES!!!" not in and_res["stdout_tail"])
                    )
                    results["android"] = and_res
                    # Pull screenshots saved inside app sandbox
                    ls_out = subprocess.run([str(ADB), "shell", "run-as com.nola ls files/e2e-screenshots 2>/dev/null"], capture_output=True, text=True).stdout.split()
                    for fname in ls_out:
                        if fname.endswith(".png"):
                            raw = subprocess.run([str(ADB), "exec-out", f"run-as com.nola cat files/e2e-screenshots/{fname}"], capture_output=True).stdout
                            if raw:
                                (and_dir / fname).write_bytes(raw)
                finally:
                    subprocess.run([str(ADB), "shell", "pkill -INT screenrecord"], capture_output=True)
                    try:
                        rec.wait(timeout=8)
                    except subprocess.TimeoutExpired:
                        rec.kill()
                    time.sleep(1)
                    subprocess.run([str(ADB), "pull", "/sdcard/nola-android-e2e.mp4", str(and_dir / "android-e2e.mp4")], capture_output=True)
                    subprocess.run([str(ADB), "shell", "rm -f /sdcard/nola-android-e2e.mp4"], capture_output=True)
                    subprocess.run([str(ADB), "shell", "run-as com.nola rm -rf files/nola-owned-test-avd.json files/e2e-screenshots"], capture_output=True)
            finally:
                emu.terminate()
                try:
                    emu.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    emu.kill()
    finally:
        if owns_fixture:
            fixture_path.unlink()

    manifest = {
        "endpoint": endpoint,
        "model": model,
        "evidence_dir": str(evidence),
        "platforms": results,
        "files": sorted(str(p.relative_to(evidence)) for p in evidence.rglob("*") if p.is_file()),
        "all_passed": all(results.get(p, {}).get("ok") for p in platforms),
    }
    (evidence / "manifest.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False), encoding="utf-8")
    print("\n=== REAL-MODEL E2E SUMMARY ===")
    for k, v in results.items():
        print(f"  {k:10s}: {'PASS' if v.get('ok') else 'FAIL'} ({v.get('elapsed_s')}s)")
    print(f"Evidence files ({len(manifest['files'])}): {manifest['files']}")
    sys.exit(0 if manifest["all_passed"] else 1)


if __name__ == "__main__":
    main()
