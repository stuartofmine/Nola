#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Run built Mac history tests with isolated state and explicit result accounting.

Build NolaMac with build-for-testing first. This runner uses an explicit Mac
destination and aborts if Xcode nevertheless attempts remote-device discovery.
It never touches the normal application database and retains diagnostic output.
"""

import argparse
import json
import os
from pathlib import Path
import plistlib
import signal
import subprocess
import sys
import tempfile
import time


class DeviceDiscoveryDetected(RuntimeError):
    pass


def wait_for_test_process(process, log_path, timeout):
    deadline = time.monotonic() + timeout
    markers = (b"DTDKRemoteDeviceConnection", b"device is passcode protected",
               b"com.apple.mobile.notification_proxy")
    overlap = b""
    with log_path.open("rb") as log:
        while True:
            if time.monotonic() >= deadline:
                raise subprocess.TimeoutExpired(process.args, timeout)
            chunk = log.read(64 * 1024)
            if chunk:
                text = overlap + chunk
                if any(marker in text for marker in markers):
                    stop_test_group(process)
                    raise DeviceDiscoveryDetected(
                        "Xcode attempted remote-device discovery; stopped this test group")
                overlap = text[-256:]
                continue
            status = process.poll()
            if status is not None:
                return status
            time.sleep(min(0.1, max(0, deadline - time.monotonic())))


def configure_targets(value, environment):
    count = 0
    if isinstance(value, dict):
        if "NolaMacTests" in str(value.get("TestBundlePath", "")):
            value.setdefault("EnvironmentVariables", {}).update(environment)
            count += 1
        for child in value.values():
            count += configure_targets(child, environment)
    elif isinstance(value, list):
        for child in value:
            count += configure_targets(child, environment)
    return count


def stop_test_group(process):
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
    except ProcessLookupError:
        process.wait()


def main():
    repository = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--derived-data", type=Path,
                        default=repository / ".build/real-mac-xctest-derived-data")
    parser.add_argument("--timeout", type=int, default=240)
    parser.add_argument("--only-testing", action="append", dest="tests")
    args = parser.parse_args()
    if args.timeout < 1:
        parser.error("--timeout must be positive")
    tests = args.tests or ["NolaMacTests/MacConversationLazyLoadingTests"]
    if any(not name.startswith("NolaMacTests/") for name in tests):
        parser.error("Only explicit NolaMacTests test classes or methods are supported")
    products = args.derived_data.resolve() / "Build/Products"
    candidates = list(products.glob("*NolaMac*.xctestrun"))
    if not candidates:
        parser.error("No NolaMac test run found; run build-for-testing first")
    source = max(candidates, key=lambda path: path.stat().st_mtime)
    root = Path(tempfile.mkdtemp(prefix="nola-mac-history-acceptance-"))
    for directory in ("home", "support/State", "cache", "tmp", "config", "data"):
        (root / directory).mkdir(parents=True, exist_ok=True)
    environment = {
        "NOLA_TEST_MODE": "1",
        "NOLA_REQUIRE_MAC_SYSTEM_PERMISSIONS": "0",
        "NOLA_USE_REAL_KEYCHAIN_IN_TESTS": "0",
        "NOLA_REAL_LLM_E2E": "0",
        "HOME": str(root / "home"),
        "NOLA_MAC_APP_SUPPORT": str(root / "support"),
        "NOLA_DB_PATH": str(root / "support/State/nola.db"),
        "NOLA_MAC_CACHE_ROOT": str(root / "cache"),
        "XDG_CONFIG_HOME": str(root / "config"),
        "XDG_DATA_HOME": str(root / "data"),
        "XDG_CACHE_HOME": str(root / "cache"),
        "TMPDIR": str(root / "tmp") + "/",
        "NOLA_NATIVE_BROWSER_PORT": "0",
    }
    with source.open("rb") as stream:
        specification = plistlib.load(stream)
    if configure_targets(specification, environment) == 0:
        parser.error("The test run contains no NolaMacTests target")
    print(f"Artifacts: {root}", flush=True)
    print(f"Built test specification: {source}", flush=True)
    # Keep the copy beside the original so __TESTROOT__ references stay valid.
    descriptor, temporary = tempfile.mkstemp(prefix="history-acceptance-",
                                            suffix=".xctestrun", dir=products)
    result = root / "result.xcresult"
    process = None
    timed_out = False
    try:
        with os.fdopen(descriptor, "wb") as stream:
            plistlib.dump(specification, stream)
        command = ["/usr/bin/caffeinate", "-is", "xcodebuild", "test-without-building",
                   "-xctestrun", temporary, "-destination", "platform=macOS,arch=arm64",
                   "-destination-timeout", "10", "-parallel-testing-enabled", "NO",
                   "-resultBundlePath", str(result), "-quiet"]
        command.extend(f"-only-testing:{name}" for name in tests)
        with (root / "test.log").open("w") as log:
            process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT,
                                       start_new_session=True)
            try:
                status = wait_for_test_process(process, root / "test.log", args.timeout)
            except DeviceDiscoveryDetected as error:
                print(f"FAIL: {error}", file=sys.stderr)
                status = 125
            except subprocess.TimeoutExpired:
                timed_out = True
                stop_test_group(process)
                status = 124
    finally:
        if process is not None:
            stop_test_group(process)
        Path(temporary).unlink(missing_ok=True)
    if timed_out:
        print("FAIL: test execution exceeded its deadline", file=sys.stderr)
    summary_process = subprocess.run(
        ["xcrun", "xcresulttool", "get", "test-results", "summary",
         "--path", str(result), "--format", "json"],
        capture_output=True, text=True, timeout=60)
    if summary_process.returncode != 0:
        print(summary_process.stderr[-4000:], file=sys.stderr)
        return 1
    summary = json.loads(summary_process.stdout)
    (root / "summary.json").write_text(summary_process.stdout)
    print(json.dumps(summary, indent=2))
    passed = summary.get("passedTests", 0)
    failed = summary.get("failedTests", 0)
    skipped = summary.get("skippedTests", 0)
    total = summary.get("totalTestCount", 0)
    if status != 0 or passed < 1 or failed != 0 or skipped != 0 or total != passed:
        print("FAIL: process failure, missing tests, failures, or skipped coverage", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
