#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# File Overview:
# - Purpose: Automates operational steps for run client tests in the project module.
# - Interfaces:
#   - _run_xcodebuild
# - APIs:
#   - none
#
# scripts/run_client_tests.sh
# Run ALL iOS client unit tests (NolaTests) via xcodebuild.
# UI tests (NolaUITests) are skipped by default since they require a running app;
# set INCLUDE_UI_TESTS=1 to include them.
#
# Usage:
#   ./scripts/run_client_tests.sh                     # run full NolaTests suite
#   ./scripts/run_client_tests.sh -only-testing NolaTests/RunNowTests  # one class
#   INCLUDE_UI_TESTS=1 ./scripts/run_client_tests.sh  # also run UI tests
#   DESTINATION="platform=iOS Simulator,id=<simulator-udid>" ./scripts/run_client_tests.sh
#   SKIP_BUILD=1 ./scripts/run_client_tests.sh        # skip build, tests only
#
# Environment variables:
#   SCHEME           - Xcode scheme (default: Nola)
#   DERIVED_DATA     - DerivedData path (default: ./build)
#   DESTINATION      - xcodebuild destination string (auto-detected if unset)
#   RESULT_BUNDLE    - path for .xcresult (default: ./build/TestResults.xcresult)
#   INCLUDE_UI_TESTS - set 1 to also run NolaUITests (default: 0)
#   SKIP_BUILD       - set 1 to run tests without building (default: 0)
#   PARALLEL_TEST_WORKERS - maximum parallel XCTest workers (default: 4)
#   XCODE_OPTS       - extra flags forwarded verbatim to xcodebuild
#   SIMULATOR_SIGNING - unsigned (default), or adhoc for real Keychain UI tests

# Preserve xcodebuild failures when its output is piped through xcbeautify.
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$SCRIPT_DIR"

SCHEME="${SCHEME:-Nola}"
DERIVED_DATA="${DERIVED_DATA:-./build}"
RESULT_BUNDLE="${RESULT_BUNDLE:-$DERIVED_DATA/TestResults.xcresult}"
XCODE_OPTS="${XCODE_OPTS:-}"
INCLUDE_UI_TESTS="${INCLUDE_UI_TESTS:-0}"
PARALLEL_TEST_WORKERS="${PARALLEL_TEST_WORKERS:-4}"
XCODE_DESTINATION_TIMEOUT="${XCODE_DESTINATION_TIMEOUT:-10}"
case "${SIMULATOR_SIGNING:-unsigned}" in
    unsigned) IOS_TEST_SIGNING_FLAGS=(CODE_SIGNING_ALLOWED=NO) ;;
    # Do not globally force CODE_SIGNING_ALLOWED=YES: static frameworks such as
    # NolaCore intentionally disable signing in their own target settings.
    adhoc) IOS_TEST_SIGNING_FLAGS=(CODE_SIGN_IDENTITY=-) ;;
    *) echo "SIMULATOR_SIGNING must be unsigned or adhoc." >&2; exit 64 ;;
esac

case "$PARALLEL_TEST_WORKERS" in
    ''|*[!0-9]*|0)
        echo "PARALLEL_TEST_WORKERS must be a positive integer." >&2
        exit 64
        ;;
esac

. "$SCRIPT_DIR/scripts/lib/xcode_test_device_guard.sh"
nola_xctest_device_guard_begin "$SCRIPT_DIR" ios_simulator
nola_xctest_device_guard_install_traps "$SCRIPT_DIR" ios_simulator

# ----------------------------------------------------------------------
# Destination: auto-detect if not supplied
# ----------------------------------------------------------------------
if [ -z "${DESTINATION:-}" ]; then
    DETECTED=$(xcrun simctl list devices available --json 2>/dev/null | python3 -c '
import sys, json
d = json.load(sys.stdin)
candidates = []
for runtime, devs in d["devices"].items():
    if "iOS" not in runtime:
        continue
    for dev in devs:
        if not dev.get("isAvailable"):
            continue
        name = dev["name"]
        version = tuple(int(part) for part in runtime.rsplit("iOS-", 1)[-1].split("-") if part.isdigit())
        priority = (
            1 if dev.get("state") == "Booted" else 0,
            1 if "Nola" in name else 0,
            1 if "iPhone" in name else 0,
            version,
        )
        candidates.append((priority, dev["udid"]))
if candidates:
    print(max(candidates)[1])
'
    )
    if [ -z "$DETECTED" ]; then
        echo "No available iOS simulator found. Launch Xcode and create a simulator." >&2
        exit 1
    fi
    DESTINATION="platform=iOS Simulator,id=$DETECTED"
fi

# The unit-test suite must never attach to a physical device. Besides prompting
# for trust/unlock, Xcode can keep polling a locked phone after XCTest exits.
# Require a concrete Simulator UDID rather than a name or generic destination.
case "$DESTINATION" in
    "platform=iOS Simulator,id="*) ;;
    *)
        echo "DESTINATION must identify an iOS Simulator by UDID; physical and generic destinations are not allowed." >&2
        exit 64
        ;;
esac

# ----------------------------------------------------------------------
# Build the skip-testing flags so we always run the full suite by default
# but can exclude slow/UI tests when not needed.
# Without any -only-testing or -skip-testing, xcodebuild runs ALL test
# targets in the scheme. We skip NolaUITests unless opted-in.
# ----------------------------------------------------------------------
SKIP_FLAGS=""
if [ "$INCLUDE_UI_TESTS" != "1" ]; then
    SKIP_FLAGS="-skip-testing NolaUITests"
fi

echo "=== iOS client tests (full suite) ===" >&2
echo "scheme:      $SCHEME" >&2
echo "destination: $DESTINATION" >&2
echo "derivedData: $DERIVED_DATA" >&2
echo "workers:     $PARALLEL_TEST_WORKERS" >&2
if [ "$INCLUDE_UI_TESTS" = "1" ]; then
    echo "UI tests:    included" >&2
else
    echo "UI tests:    skipped (set INCLUDE_UI_TESTS=1 to enable)" >&2
fi
echo "" >&2

# Remove old result bundle so results are always fresh
rm -rf "$RESULT_BUNDLE"

# Build action
BUILD_ACTION="test"
if [ "${SKIP_BUILD:-0}" = "1" ]; then
    BUILD_ACTION="test-without-building"
fi

_run_xcodebuild() {
    # shellcheck disable=SC2086
    NOLA_TEST_MODE=1 xcodebuild \
        "$BUILD_ACTION" \
        -project "$SCRIPT_DIR/apple/Nola.xcodeproj" \
        -scheme "$SCHEME" \
        $SKIP_FLAGS \
        -destination "$DESTINATION" \
        -destination-timeout "$XCODE_DESTINATION_TIMEOUT" \
        -derivedDataPath "$DERIVED_DATA" \
        -resultBundlePath "$RESULT_BUNDLE" \
        -parallel-testing-enabled YES \
        -maximum-parallel-testing-workers "$PARALLEL_TEST_WORKERS" \
        "${IOS_TEST_SIGNING_FLAGS[@]}" \
        $XCODE_OPTS \
        "$@"
}

# Pretty output via xcbeautify if available, plain xcodebuild otherwise
if command -v xcbeautify >/dev/null 2>&1; then
    _run_xcodebuild "$@" | xcbeautify
else
    _run_xcodebuild "$@"
fi
