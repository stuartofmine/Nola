#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real Compose/provider A2UI acceptance on an explicitly disposable emulator.

Reads current llm.connections/default_model only. Credentials travel in memory
through adb stdin to app-private fixture storage, then into Android Keystore.
No logcat, model settings export, provider body capture or production data reset.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
from urllib.parse import urlsplit


def selected_model(config: dict, environment: dict) -> dict:
    llm = config["llm"]
    selected = llm["default_model"]
    connections = [entry for entry in llm["connections"] if entry["id"] == selected["connection"]]
    if len(connections) != 1:
        raise ValueError("Current model connection is missing or ambiguous")
    connection = connections[0]
    models = [entry for entry in connection["models"] if entry["id"] == selected["model"]]
    if len(models) != 1:
        raise ValueError("Current model is missing or ambiguous")
    protocol = connection["protocol"]
    if protocol not in ("chat_completions", "responses"):
        raise ValueError("Current Android model protocol is unsupported; no fallback was selected")
    key = environment.get(connection["api_key_env"], "")
    if not key:
        raise ValueError("Current model credential environment is unavailable")
    endpoint = connection["endpoint"]
    parsed = urlsplit(endpoint)
    if parsed.scheme not in ("http", "https") or not parsed.hostname or parsed.username or parsed.password:
        raise ValueError("Model endpoint must be an HTTP(S) URL without embedded credentials")
    return dict(disposableFixture=True, endpoint=endpoint, apiKey=key,
                model=models[0]["id"], wireProtocol=protocol)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--config", type=Path, default=Path.home() / ".nola/config.json")
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()
    if not args.serial.startswith("emulator-"):
        raise SystemExit("A disposable emulator is required; physical devices are not accepted")
    root = Path(__file__).resolve().parents[2]
    sdk = Path(os.environ.get("ANDROID_SDK_ROOT", os.environ.get("ANDROID_HOME", str(Path.home() / "Library/Android/sdk"))))
    adb = str(sdk / "platform-tools/adb")
    fixture = selected_model(json.loads(args.config.read_text()), os.environ)
    secret = fixture["apiKey"]
    evidence = Path(tempfile.mkdtemp(prefix="nola-a2ui-android-real-"))
    os.chmod(evidence, 0o700)

    def run(command, *, data=None, timeout=120):
        result = subprocess.run(command, input=data, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout)
        if result.returncode:
            raise RuntimeError("Command failed: " + result.stdout.decode(errors="replace").replace(secret, "[redacted]")[-2000:])
        return result.stdout

    def device(*command, **kwargs):
        return run([adb, "-s", args.serial, *command], **kwargs)

    avd = device("emu", "avd", "name").decode().splitlines()[0].strip()
    if not avd.startswith("nola_a2ui_isolated_"):
        raise SystemExit("Only an AVD named nola_a2ui_isolated_* is accepted")
    if device("shell", "getprop", "sys.boot_completed").strip() != b"1":
        raise SystemExit("The disposable emulator has not booted")
    abi = device("shell", "getprop", "ro.product.cpu.abi").decode().strip()
    if abi not in ("arm64-v8a", "x86_64"):
        raise SystemExit("Unsupported emulator ABI")
    print(json.dumps({"stage": "validated_disposable_emulator", "evidence": str(evidence)}), flush=True)
    if not args.skip_build:
        print(json.dumps({"stage": "building_android"}), flush=True)
        env = dict(os.environ, NOLA_ANDROID_ABIS=abi)
        if env.get("NOLA_ANDROID_JAVA_HOME"):
            env["JAVA_HOME"] = env["NOLA_ANDROID_JAVA_HOME"]
        result = subprocess.run([str(root / "android/gradlew"), ":app:assembleDebug", ":app:assembleDebugAndroidTest", "--no-daemon", "--max-workers=2", "--quiet"],
                                cwd=root / "android", env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        (evidence / "build.log").write_bytes(result.stdout.replace(secret.encode(), b"[redacted]"))
        if result.returncode:
            raise SystemExit("Android build failed; see isolated evidence directory: " + str(evidence))
    print(json.dumps({"stage": "installing_android"}), flush=True)
    device("install", "-r", str(root / "android/app/build/outputs/apk/debug/app-debug.apk"))
    device("install", "-r", str(root / "android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk"))
    target = "com.nola.ui.generative.GenerativeUiRealModelEndToEndTest"
    reverse = None
    try:
        parsed = urlsplit(fixture["endpoint"])
        if parsed.hostname in ("localhost", "127.0.0.1"):
            reverse = f"tcp:{parsed.port or (443 if parsed.scheme == 'https' else 80)}"
            device("reverse", reverse, reverse)
        device("shell", "run-as", "com.nola", "mkdir", "-p", "files")
        device("shell", "run-as", "com.nola", "sh", "-c", "'cat > files/a2ui-real-model-fixture.json'",
               data=json.dumps(fixture).encode())
        print(json.dumps({"stage": "running_real_model_compose_acceptance"}), flush=True)
        output = device("shell", "am", "instrument", "-w", "-r", "-e", "disposableA2uiFixture", "true",
                        "-e", "class", target + "#realProviderFormEditsSubmitsUpdatesAndReopens",
                        "com.nola.test/androidx.test.runner.AndroidJUnitRunner", timeout=650)
        (evidence / "instrumentation.log").write_bytes(output.replace(secret.encode(), b"[redacted]"))
        if b"OK (1 test)" not in output:
            raise RuntimeError("Real Android A2UI acceptance failed; see isolated instrumentation evidence")
        result = json.loads(device("exec-out", "run-as", "com.nola", "cat", "files/a2ui-real-model-result.json"))
        if result.get("passed") is not True or result.get("realProvider") is not True:
            raise RuntimeError("No complete real-model acceptance receipt")
        (evidence / "result.json").write_text(json.dumps(result, indent=2))
        print(json.dumps({"passed": True, "evidence": str(evidence)}))
        return 0
    finally:
        # Also runs when the test process fails before Kotlin's finally block.
        try:
            device("shell", "am", "instrument", "-w", "-r", "-e", "disposableA2uiFixture", "true",
                   "-e", "class", target + "#clearFixtureCredential",
                   "com.nola.test/androidx.test.runner.AndroidJUnitRunner", timeout=60)
        finally:
            device("shell", "run-as", "com.nola", "rm", "-f", "files/a2ui-real-model-fixture.json")
            if reverse:
                device("reverse", "--remove", reverse)


if __name__ == "__main__":
    raise SystemExit(main())
