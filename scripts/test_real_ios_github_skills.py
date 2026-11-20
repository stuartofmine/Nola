#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Exercise independent iPhone and paired Host Skill installs, then inspect native settings."""
import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time
import uuid
from urllib.error import URLError
from urllib.request import urlopen

from test_real_cli_long_tasks import fixture, observed_provider, selected_route
from test_real_github_skill_install import github_skill_fixture_directory

ROOT = Path(__file__).resolve().parents[1]


def run_tests(env, evidence, name, tests, *, ui=False, skip_build=False):
    bundle = evidence / f"{name}.xcresult"
    environment = dict(env, RESULT_BUNDLE=str(bundle), INCLUDE_UI_TESTS="1" if ui else "0",
                       SKIP_BUILD="1" if skip_build else "0")
    with (evidence / f"{name}.log").open("w") as log:
        result = subprocess.run([ROOT / "scripts/run_client_tests.sh",
                                 *(f"-only-testing:{test}" for test in tests)],
                                cwd=ROOT, env=environment, stdout=log, stderr=subprocess.STDOUT)
    report = subprocess.run(["xcrun", "xcresulttool", "get", "test-results", "summary", "--path", bundle],
                            check=True, capture_output=True, text=True)
    summary = json.loads(report.stdout)
    (evidence / f"{name}.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2))
    if result.returncode or summary["passedTests"] != len(tests) or summary["skippedTests"]:
        raise RuntimeError(f"{name} failed or skipped tests; see {bundle}")
    if ui:
        subprocess.run(["xcrun", "xcresulttool", "export", "attachments", "--path", bundle,
                        "--output-path", evidence / "screenshots"], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=Path.home() / ".nola/config.json")
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/debug")
    parser.add_argument("--evidence", type=Path, default=ROOT / ".artifacts/github-skill-ios")
    parser.add_argument("--skip-build", action="store_true", help="Reuse previously built Test configuration")
    args = parser.parse_args()
    evidence = args.evidence.resolve() / time.strftime("%Y%m%d-%H%M%S")
    evidence.mkdir(parents=True)
    binary = args.bin_dir.resolve() / "nola"
    service = binary.with_name("nola-service")
    if not all(p.is_file() for p in (binary, service)):
        parser.error("Build the current nola and nola-service binaries before starting the paired Host test")
    upstream, model, credential = selected_route(args.config)
    phone_fixture = Path("/tmp") / f"nola-ios-github-skill-{uuid.uuid4()}"
    phone_fixture.mkdir(mode=0o700)
    print(f"iOS Skill acceptance using {model}; results in {evidence}", flush=True)
    try:
        with github_skill_fixture_directory("nola-ios-skill-host-") as tmp, observed_provider(
                upstream, model, credential, evidence / "provider.ndjson") as relay:
            project, host_env, config = fixture(Path(tmp) / "fixture", model, relay)
            host_env.update(HOME=host_env["NOLA_HOME"], NOLA_APPLICATION_SERVICE_EXECUTABLE=str(service))
            subprocess.run([binary, "model", "apply-config", config], cwd=project, env=host_env,
                           check=True, stdout=subprocess.DEVNULL)
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", 0))
                port = listener.getsockname()[1]
            origin = f"http://127.0.0.1:{port}"
            # The Host prints its ephemeral access token on startup; keep its log inside the temporary fixture.
            with (Path(tmp) / "host.log").open("w") as output:
                host = subprocess.Popen([binary, "web", "--host", "127.0.0.1", "--port", str(port),
                                         "--project", project, "--web-root", ROOT / "apps/web"],
                                        cwd=project, env=host_env, stdout=output, stderr=subprocess.STDOUT)
                try:
                    deadline = time.monotonic() + 30
                    while True:
                        if host.poll() is not None:
                            raise RuntimeError("Isolated Host exited before accepting connections")
                        try:
                            with urlopen(origin + "/login", timeout=1):
                                break
                        except URLError:
                            if time.monotonic() >= deadline:
                                raise RuntimeError("Isolated Host failed to start")
                            time.sleep(.1)
                    token = (Path(host_env["HOME"]) / ".nola/web/mobile-access-token").read_text().strip()
                    env = dict(os.environ, PARALLEL_TEST_WORKERS="1", SIMULATOR_SIGNING="adhoc",
                               XCODE_OPTS="-jobs 2 -collect-test-diagnostics never ONLY_ACTIVE_ARCH=YES ARCHS=arm64",
                               TEST_RUNNER_NOLA_REAL_LLM_E2E="1", TEST_RUNNER_NOLA_REAL_LLM_MODEL=model,
                               TEST_RUNNER_NOLA_REAL_LLM_ENDPOINT=relay, TEST_RUNNER_NOLA_REAL_LLM_API_KEY="local-relay",
                               TEST_RUNNER_NOLA_REAL_GITHUB_SKILL_HOST_E2E="1",
                               TEST_RUNNER_NOLA_REAL_WEB_AGENT_HOST_URL=origin, TEST_RUNNER_NOLA_REAL_WEB_AGENT_HOST_TOKEN=token,
                               TEST_RUNNER_NOLA_GITHUB_SKILL_UI_FIXTURE_ROOT=str(phone_fixture))
                    env.setdefault("CARGO_BUILD_JOBS", "2")
                    tests = ["IOSGitHubSkillTests/testInstalledPackagesRequireActivationAndHonorCurrentEnablement",
                             "IOSGitHubSkillTests/testRealChatInstallsPackageThenReadsSharedFileInFreshConversation",
                             "IOSGitHubSkillTests/testPairedHostInstallsSkillInItsOwnSettingsFromNativeChatStream",
                             "AgentCoreToolCatalogTests/testNativeCatalogDeclaresSemanticRiskWithoutGuessingOperationNames",
                             "AgentCoreToolCatalogTests/testCatalogAndExecutorShareOnlyCanonicalDeviceToolIDs"]
                    run_tests(env, evidence, "device-and-host", ["NolaTests/" + test for test in tests],
                              skip_build=args.skip_build)
                    run_tests(env, evidence, "settings",
                              ["NolaUITests/IOSWorkspaceNavigationSmokeTests/testInstalledGitHubSkillAppearsInSettingsWithPinnedSource"],
                              ui=True, skip_build=True)
                finally:
                    host.terminate()
                    host.wait(timeout=20)
    finally:
        shutil.rmtree(phone_fixture)


if __name__ == "__main__":
    main()
