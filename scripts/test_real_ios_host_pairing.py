#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Run iPhone keyboard and pairing UI acceptance against an isolated loopback Nola Host."""
import argparse
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
from urllib.error import URLError
from urllib.request import urlopen

from test_real_ios_github_skills import ROOT, run_tests


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/nola")
    parser.add_argument("--evidence", type=Path, default=ROOT / ".artifacts/ios-host-pairing")
    args = parser.parse_args()
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error("Build the nola CLI first, or supply --binary")
    evidence = args.evidence.resolve() / time.strftime("%Y%m%d-%H%M%S")
    evidence.mkdir(parents=True)
    tests = [
        "testBlankTapAndFocusChangesInHostSheet",
        "testBlankTapPreservesSearchAndConversationDrafts",
        "testCancelSlowVerificationPreservesHostDraftInSettings",
        "testCapabilityLibraryPairsAndReturnsFromLiveHostManagement",
        "testFailedHostCredentialsCanBeCorrectedAgainstIsolatedServer",
        "testManualAddressEditDropsPastedTokenUntilNewSetupIsProvided",
        "testScrollInExpertEditorWithTheKeyboardOpen",
    ]
    with tempfile.TemporaryDirectory(prefix="nola-ios-host-") as folder:
        workspace = Path(folder)
        home = workspace / "home"
        home.mkdir()
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        origin = f"http://127.0.0.1:{port}"
        host_env = {k: v for k, v in os.environ.items() if not k.startswith("NOLA_")}
        host_env.update(HOME=str(home), NOLA_HOME=str(home), NOLA_MAC_APP_SUPPORT=str(workspace / "support"),
                        NOLA_DB_PATH=str(workspace / "host.sqlite"))
        with (workspace / "host.log").open("w") as log:
            host = subprocess.Popen(
                [binary, "web", "--host", "127.0.0.1", "--port", str(port), "--web-root", ROOT / "apps/web"],
                cwd=workspace, env=host_env, stdout=log, stderr=subprocess.STDOUT,
            )
            try:
                deadline = time.monotonic() + 30
                while True:
                    if host.poll() is not None:
                        raise RuntimeError("Isolated Host exited before it was ready")
                    try:
                        with urlopen(origin + "/login", timeout=1):
                            break
                    except URLError:
                        if time.monotonic() >= deadline:
                            raise RuntimeError("Isolated Host did not accept connections")
                        time.sleep(.1)
                token = (home / ".nola/web/mobile-access-token").read_text().strip()
                identity = (home / ".nola/web/host-identity").read_text().strip()
                env = dict(os.environ, PARALLEL_TEST_WORKERS="1", IOS_PARALLEL_TESTING_ENABLED="NO", SIMULATOR_SIGNING="adhoc",
                           XCODE_OPTS="-collect-test-diagnostics never ONLY_ACTIVE_ARCH=YES",
                           TEST_RUNNER_NOLA_TEST_PAIRING_URL=origin, TEST_RUNNER_NOLA_TEST_PAIRING_TOKEN=token,
                           TEST_RUNNER_NOLA_TEST_PAIRING_ID=identity)
                run_tests(env, evidence, "pairing", ["NolaUITests/IOSKeyboardInteractionTests/" + name for name in tests],
                          ui=True)
            finally:
                host.terminate()
                try:
                    host.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    host.kill()
                    host.wait()
    print(evidence)


if __name__ == "__main__":
    main()
