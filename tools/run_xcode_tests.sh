#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# File Overview:
# - Purpose: Automates operational steps for run xcode tests in the project module.
# - Interfaces:
#   - Internal implementation details only.
# - APIs:
#   - none
#
set -euo pipefail

# This script relies on bash features (arrays). If invoked via `sh`, re-exec under bash.
if [ -z "${BASH_VERSION:-}" ]; then
  exec /usr/bin/env bash "$0" "$@"
fi

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT_DIR"

PARALLEL_TEST_WORKERS="${PARALLEL_TEST_WORKERS:-4}"
if ! [[ "$PARALLEL_TEST_WORKERS" =~ ^[1-9][0-9]*$ ]]; then
  echo "PARALLEL_TEST_WORKERS must be a positive integer." >&2
  exit 64
fi

source "$ROOT_DIR/scripts/lib/xcode_test_device_guard.sh"
nola_xctest_device_guard_begin "$ROOT_DIR"
nola_xctest_device_guard_install_traps "$ROOT_DIR"

echo "Finding an available iOS Simulator..."

# We only run on Simulator. If a physical iOS device is connected, xcodebuild may
# still try to start device services (e.g., notification proxy) which can hang or
# spam errors when the device is locked. Default: fail fast.
# Override: set ALLOW_PHYSICAL_IOS_DEVICES=1.
if [ "${ALLOW_PHYSICAL_IOS_DEVICES:-0}" != "1" ]; then
  if xcrun xcdevice list --json >/dev/null 2>&1; then
    PHYS_IOS_INFO=$(python3 - <<'PY'
import json
import subprocess

# Some Xcode versions print a spurious "unrecognized option --json" message to stderr
# while still returning valid JSON on stdout. Suppress stderr.
raw = subprocess.check_output(
  ["xcrun", "xcdevice", "list", "--json"],
  text=True,
  stderr=subprocess.DEVNULL,
)
devices = json.loads(raw)

physical_ios = []
for d in devices:
  if d.get("simulator") is True:
    continue
  if d.get("platform") == "com.apple.platform.iphoneos":
    name = d.get("name") or d.get("modelName") or "(unknown)"
    ident = d.get("identifier") or "(no-id)"
    avail = d.get("available")
    physical_ios.append(f"{name}\t{ident}\tavailable={avail}")

print("\n".join(physical_ios))
PY
)

    if [ -n "${PHYS_IOS_INFO}" ]; then
      echo "ERROR: Detected connected physical iOS device(s). This script runs simulator-only."
      echo "Detected:"
      echo "${PHYS_IOS_INFO}" | sed 's/^/  - /'
      echo "Action: unplug the iPhone/iPad, then re-run."
      echo "Override: set ALLOW_PHYSICAL_IOS_DEVICES=1 if you accept potential hangs/errors."
      exit 75
    fi
  fi
fi

# NOTE: We intentionally do not try to handle the "locked device" case separately.
# If a physical iOS device is present, we fail fast above (unless overridden).

# Use JSON output for robust parsing (avoids terminal wrapping/grep fragility).
SIM_INFO=$(python3 - <<'PY'
import json
import subprocess

raw = subprocess.check_output(["xcrun", "simctl", "list", "-j", "devices", "available"], text=True)
data = json.loads(raw)
devices_by_runtime = data.get("devices", {})

def is_ios_runtime(runtime_id: str) -> bool:
  return runtime_id.startswith("com.apple.CoreSimulator.SimRuntime.iOS")

def iter_ios_devices():
  # Prefer newer iOS runtimes first (lexicographic works for iOS-26-0 vs iOS-17-0, etc.)
  for runtime_id in sorted((r for r in devices_by_runtime if is_ios_runtime(r)), reverse=True):
    for d in devices_by_runtime.get(runtime_id, []):
      # "available" list should already filter, but be defensive.
      if d.get("isAvailable") is False:
        continue
      if d.get("availabilityError"):
        continue
      yield d

def pick_device(devices):
  devices = list(devices)
  if not devices:
    return None

  def score(d):
    name = d.get("name", "")
    state = d.get("state", "")
    s = 0
    if state == "Booted":
      s += 1000
    if "Nola" in name:
      s += 100
    # Prefer iPhone over other device types
    if "iPhone" in name:
      s += 10
    # Small nudge towards "iPhone 15" style devices if present
    if "iPhone 15" in name:
      s += 1
    return s

  devices.sort(key=score, reverse=True)
  return devices[0]

picked = pick_device(iter_ios_devices())
if not picked:
  print("")
else:
  print(f"{picked.get('udid','')}\t{picked.get('name','')}")
PY
) || true

SIM_ID="${SIM_INFO%%$'\t'*}"
SIM_NAME="${SIM_INFO#*$'\t'}"

if [ -z "$SIM_ID" ]; then
  echo "No iPhone simulator found via simctl; falling back to 'iPhone 15' name."
  DEST="platform=iOS Simulator,name=iPhone 15"
else
  echo "Using simulator: $SIM_NAME (id: $SIM_ID)"
  # Ensure the simulator is booted and ready before xcodebuild tries to prepare it.
  xcrun simctl boot "$SIM_ID" >/dev/null 2>&1 || true
  xcrun simctl bootstatus "$SIM_ID" -b
  DEST="platform=iOS Simulator,id=$SIM_ID"
fi

echo "Running xcodebuild tests using destination: $DEST"

# Force simulator build/test (avoid accidental device builds when a device is connected).
SDK="${SDK:-iphonesimulator}"
echo "Using SDK: $SDK"
echo "Parallel XCTest workers: $PARALLEL_TEST_WORKERS"

# UI tests can be flaky in CI/local due to permission prompts and onboarding state.
# Default: skip UI tests. Set RUN_UI_TESTS=1 to include them.
XCODEBUILD_TEST_ARGS=()
if [ "${RUN_UI_TESTS:-0}" = "1" ]; then
  echo "RUN_UI_TESTS=1 -> Including UI tests"
else
  echo "Skipping UI tests (set RUN_UI_TESTS=1 to include)"
  XCODEBUILD_TEST_ARGS+=( -skip-testing:NolaUITests )
fi

# Allow callers to further narrow or exclude tests.
#
# Examples:
#   ONLY_TESTING='NolaTests/RustInteropTests' ./tools/run_xcode_tests.sh
#   SKIP_TESTING='NolaTests/Integration/EndToEndFlowTests,NolaTests/QuickMLXTest' ./tools/run_xcode_tests.sh
if [ -n "${ONLY_TESTING:-}" ]; then
  echo "ONLY_TESTING set -> running only specified tests"
  IFS=', ' read -r -a ONLY_LIST <<< "${ONLY_TESTING}"
  for item in "${ONLY_LIST[@]}"; do
    if [ -n "${item}" ]; then
      XCODEBUILD_TEST_ARGS+=( -only-testing:"${item}" )
    fi
  done
fi

if [ -n "${SKIP_TESTING:-}" ]; then
  echo "SKIP_TESTING set -> skipping specified tests"
  IFS=', ' read -r -a SKIP_LIST <<< "${SKIP_TESTING}"
  for item in "${SKIP_LIST[@]}"; do
    if [ -n "${item}" ]; then
      XCODEBUILD_TEST_ARGS+=( -skip-testing:"${item}" )
    fi
  done
fi

# Adjust workspace/scheme if needed
SCHEME="Nola"

if [ -d "apple/Nola.xcodeproj/project.xcworkspace" ]; then
  WORKSPACE="apple/Nola.xcodeproj/project.xcworkspace"
  echo "Using workspace: $WORKSPACE"
  xcodebuild test -workspace "$WORKSPACE" -scheme "$SCHEME" -sdk "$SDK" -destination "$DEST" -parallel-testing-enabled YES -maximum-parallel-testing-workers "$PARALLEL_TEST_WORKERS" "${XCODEBUILD_TEST_ARGS[@]}"
else
  echo "Workspace not found; using project: apple/Nola.xcodeproj"
  xcodebuild test -project "apple/Nola.xcodeproj" -scheme "$SCHEME" -sdk "$SDK" -destination "$DEST" -parallel-testing-enabled YES -maximum-parallel-testing-workers "$PARALLEL_TEST_WORKERS" "${XCODEBUILD_TEST_ARGS[@]}"
fi
