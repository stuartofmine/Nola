#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Serializes top-level xcodebuild test runs while preserving Xcode's internal
# parallel workers. Only an explicit iOS Simulator invocation manages clones;
# macOS owns the lock/tmp lifecycle without contacting Simulator tooling.

nola_xctest_device_guard_validate_platform() {
    case "$1" in
        macos|ios_simulator) return 0 ;;
        *) echo "XCTest guard requires explicit macos or ios_simulator platform." >&2; return 64 ;;
    esac
}

nola_xctest_device_guard_begin() {
    [ "$#" -eq 2 ] || { echo "Usage: nola_xctest_device_guard_begin ROOT PLATFORM" >&2; return 64; }
    nola_guard_root=$1
    nola_guard_platform=$2
    nola_xctest_device_guard_validate_platform "$nola_guard_platform" || return $?
    [ "$(uname -s)" = "Darwin" ] || return 0

    nola_guard_uid=$(id -u)
    NOLA_XCTEST_GUARD_LOCK=${NOLA_XCTEST_GUARD_LOCK:-${TMPDIR:-/tmp}/nola-xctest-device-guard-${nola_guard_uid}.lock}

    if ! mkdir "$NOLA_XCTEST_GUARD_LOCK" 2>/dev/null; then
        nola_guard_owner=$(sed -n '1p' "$NOLA_XCTEST_GUARD_LOCK/pid" 2>/dev/null || true)
        if [ -n "$nola_guard_owner" ] && kill -0 "$nola_guard_owner" 2>/dev/null; then
            echo "Another Nola xcodebuild test run is active (pid $nola_guard_owner)." >&2
            return 75
        fi
        rm -rf "$NOLA_XCTEST_GUARD_LOCK"
        mkdir "$NOLA_XCTEST_GUARD_LOCK"
    fi
    echo "$$" >"$NOLA_XCTEST_GUARD_LOCK/pid"
    export NOLA_XCTEST_GUARD_LOCK

    if [ "$nola_guard_platform" = "ios_simulator" ]; then
        NOLA_XCTEST_DEVICE_SET=${NOLA_XCTEST_DEVICE_SET:-${HOME}/Library/Developer/XCTestDevices}
        NOLA_XCTEST_STALE_DEVICE_AGE_SECONDS=${NOLA_XCTEST_STALE_DEVICE_AGE_SECONDS:-21600}
        # A prior xcodebuild can be killed before its EXIT trap runs. Do this
        # only for Simulator runs, under the cross-run lock and dedicated set.
        if ! python3 "$nola_guard_root/scripts/xctest_device_guard.py" prune-stale \
            --device-set "$NOLA_XCTEST_DEVICE_SET" \
            --minimum-age-seconds "$NOLA_XCTEST_STALE_DEVICE_AGE_SECONDS"; then
            rm -rf "$NOLA_XCTEST_GUARD_LOCK"
            return 1
        fi

        NOLA_XCTEST_GUARD_BASELINE=$(mktemp "${TMPDIR:-/tmp}/nola-xctest-baseline.XXXXXX")
        export NOLA_XCTEST_DEVICE_SET NOLA_XCTEST_GUARD_BASELINE
        if ! python3 "$nola_guard_root/scripts/xctest_device_guard.py" snapshot \
            --device-set "$NOLA_XCTEST_DEVICE_SET" \
            --output "$NOLA_XCTEST_GUARD_BASELINE"; then
            rm -f "$NOLA_XCTEST_GUARD_BASELINE"
            rm -rf "$NOLA_XCTEST_GUARD_LOCK"
            return 1
        fi
    fi

    # Keep every test process inside a run-owned temporary directory. XCTest
    # and Foundation otherwise accumulate bundle-scoped temporary files under
    # /var/folders, which can survive failed or interrupted test runs.
    NOLA_XCTEST_ORIGINAL_TMPDIR=${TMPDIR:-/tmp/}
    NOLA_XCTEST_RUN_TMP=$(mktemp -d "${NOLA_XCTEST_ORIGINAL_TMPDIR%/}/nola-xctest-run.XXXXXX")
    TMPDIR="$NOLA_XCTEST_RUN_TMP/"
    export NOLA_XCTEST_ORIGINAL_TMPDIR NOLA_XCTEST_RUN_TMP TMPDIR
}

nola_xctest_device_guard_end() {
    [ "$#" -eq 2 ] || { echo "Usage: nola_xctest_device_guard_end ROOT PLATFORM" >&2; return 64; }
    nola_guard_root=$1
    nola_guard_platform=$2
    nola_xctest_device_guard_validate_platform "$nola_guard_platform" || return $?
    if [ "$nola_guard_platform" = "ios_simulator" ] && [ -n "${NOLA_XCTEST_GUARD_BASELINE:-}" ]; then
        python3 "$nola_guard_root/scripts/xctest_device_guard.py" cleanup \
            --device-set "$NOLA_XCTEST_DEVICE_SET" \
            --baseline "$NOLA_XCTEST_GUARD_BASELINE" || true
        rm -f "$NOLA_XCTEST_GUARD_BASELINE"
    fi
    if [ -n "${NOLA_XCTEST_GUARD_LOCK:-}" ]; then
        rm -rf "$NOLA_XCTEST_GUARD_LOCK"
    fi
    if [ -n "${NOLA_XCTEST_RUN_TMP:-}" ]; then
        rm -rf "$NOLA_XCTEST_RUN_TMP"
    fi
    if [ -n "${NOLA_XCTEST_ORIGINAL_TMPDIR:-}" ]; then
        TMPDIR="$NOLA_XCTEST_ORIGINAL_TMPDIR"
        export TMPDIR
    fi
}

nola_xctest_device_guard_install_traps() {
    [ "$#" -eq 2 ] || { echo "Usage: nola_xctest_device_guard_install_traps ROOT PLATFORM" >&2; return 64; }
    nola_guard_root=$1
    nola_guard_platform=$2
    nola_xctest_device_guard_validate_platform "$nola_guard_platform" || return $?
    trap 'exit 130' INT
    trap 'exit 143' TERM
    trap 'nola_guard_status=$?; trap - EXIT INT TERM; nola_xctest_device_guard_end "'"$nola_guard_root"'" "'"$nola_guard_platform"'"; exit "$nola_guard_status"' EXIT
}
