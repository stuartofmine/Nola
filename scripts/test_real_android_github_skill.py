#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Test on-device GitHub Skill installation with a real model and public repository."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
from urllib.parse import urlsplit

from test_real_cli_long_tasks import observed_provider, selected_route


ROOT = Path(__file__).resolve().parents[1]
RUNNER = "com.nola.test/androidx.test.runner.AndroidJUnitRunner"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True, help="Running Android emulator")
    parser.add_argument("--user", type=int, required=True, help="Fresh disposable secondary Android user")
    parser.add_argument("--config", type=Path, default=Path.home() / ".nola/config.json")
    parser.add_argument("--out", type=Path, help="New evidence directory")
    parser.add_argument("--skip-build", action="store_true", help="Use already built debug APKs")
    args = parser.parse_args()
    if not args.serial.startswith("emulator-") or args.user < 1:
        parser.error("Use an emulator and a disposable secondary user; primary user is unsupported")
    if args.out:
        out = args.out.resolve()
        out.mkdir(parents=True, exist_ok=False)
    else:
        out = Path(tempfile.mkdtemp(prefix="nola-android-github-skill-"))
    print(f"Android Skill evidence: {out}", flush=True)
    sdk = Path(os.environ.get("ANDROID_SDK_ROOT") or os.environ.get("ANDROID_HOME") or Path.home() / "Library/Android/sdk")
    adb = shutil.which("adb") or sdk / "platform-tools/adb"
    user = str(args.user)

    def device(*arguments, **options):
        options.setdefault("check", True)
        return subprocess.run([adb, "-s", args.serial, *map(str, arguments)], **options)

    def shell(*arguments):
        return device("shell", *arguments, capture_output=True, text=True).stdout.strip()

    def instrument(test, log, extras):
        with log.open("w") as output:
            device("shell", "am", "instrument", "--user", user, "-w", "-e", "class", test,
                   *extras, RUNNER, stdout=output, stderr=subprocess.STDOUT, timeout=540)
        return "OK (1 test)" in log.read_text()

    upstream, model, key = selected_route(args.config)
    previous_user = shell("am", "get-current-user")
    if not args.skip_build:
        android = ROOT / "apps/android"
        env = dict(os.environ, NOLA_ANDROID_ABIS=shell("getprop", "ro.product.cpu.abi"))
        if env.get("NOLA_ANDROID_JAVA_HOME"):
            env["JAVA_HOME"] = env["NOLA_ANDROID_JAVA_HOME"]
        with (out / "build.log").open("w") as output:
            subprocess.run(["./gradlew", ":app:assembleDebug", ":app:assembleDebugAndroidTest"],
                           cwd=android, env=env, stdout=output, stderr=subprocess.STDOUT, check=True)
    apk = ROOT / "apps/android/app/build/outputs/apk"
    for artifact in [apk / "debug/app-debug.apk", apk / "androidTest/debug/app-debug-androidTest.apk"]:
        device("install", "--user", user, "-r", artifact)
    timeout = shell("settings", "--user", user, "get", "system", "screen_off_timeout")
    try:
        shell("am", "start-user", "-w", user)
        shell("am", "switch-user", user)
        if shell("am", "get-current-user") != user:
            raise RuntimeError("Isolated Android user is not active")
        shell("settings", "--user", user, "put", "system", "screen_off_timeout", "1800000")
        shell("input", "keyevent", "KEYCODE_WAKEUP")
        package_ok = instrument("com.nola.core.AndroidGithubSkillPackageTest", out / "package.log",
                                ["-e", "liveGithub", "true"])
        trace = out / "provider.ndjson"
        with observed_provider(upstream, model, key, trace) as relay:
            port = urlsplit(relay).port
            device("reverse", f"tcp:{port}", f"tcp:{port}")
            try:
                fixture = {"endpoint": relay, "apiKey": "isolated-relay", "model": model, "protocol": "responses"}
                device("shell", f"run-as com.nola --user {user} sh -c 'mkdir -p files && cat > files/nola-real-llm-e2e.json'",
                       input=json.dumps(fixture), text=True)
                ui_ok = instrument("com.nola.ui.screen.chat.RealAndroidChatProductUiEndToEndTest#"
                                   "installsGithubSkillOnPhoneAndReadsResourceInNewChat", out / "real.log",
                                   ["-e", "isolatedUserId", user])
                for name in ["approval", "installed", "used", "settings"]:
                    result = device("exec-out", "run-as", "com.nola", "--user", user, "cat",
                                    f"files/github-skill-{name}.png", capture_output=True, check=False)
                    if result.stdout.startswith(b"\x89PNG\r\n\x1a\n"):
                        (out / f"android-{name}.png").write_bytes(result.stdout)
            finally:
                device("reverse", "--remove", f"tcp:{port}")
                shell("run-as", "com.nola", "--user", user, "rm", "-f", "files/nola-real-llm-e2e.json")
        events = [json.loads(line) for line in trace.read_text().splitlines()]
        used = {name for event in events if event.get("status") == 200 for name in event.get("responseToolNames", [])}
        expected = {"device_github_skill_find", "device_github_skill_install", "skill_invoke", "device_skill_read_resource"}
        screenshots = all((out / f"android-{name}.png").is_file() for name in ["approval", "installed", "used", "settings"])
        if not (package_ok and ui_ok and screenshots and expected <= used):
            raise SystemExit(f"Failed: package={package_ok}, ui={ui_ok}, screenshots={screenshots}, missing calls={expected - used}")
        print("Passed Android GitHub Skill search, approval, install, use in a new chat, and settings.")
    finally:
        if timeout == "null":
            shell("settings", "--user", user, "delete", "system", "screen_off_timeout")
        else:
            shell("settings", "--user", user, "put", "system", "screen_off_timeout", timeout)
        if previous_user != user:
            shell("am", "switch-user", previous_user)


if __name__ == "__main__":
    main()
