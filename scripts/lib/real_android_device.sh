#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Shared Android device lifecycle for real product tests.

real_android_adb() {
    local timeout_sec="${NOLA_ANDROID_ADB_TIMEOUT_SEC:-8}"
    perl -e 'alarm shift; exec @ARGV or exit 127' \
        "$timeout_sec" "$REAL_ANDROID_ADB" "$@"
}

real_android_device() {
    real_android_adb devices 2>/dev/null |
        awk -v requested="${NOLA_ANDROID_REAL_SERIAL:-}" \
            'NR > 1 && $2 == "device" && (requested == "" || $1 == requested) { print $1; exit }'
}

real_android_boot_completed() {
    local device_id="$1"
    [[ "$(real_android_adb -s "$device_id" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == 1 ]]
}

real_android_recover_adb() {
    printf '[real-android-device] recovering unresponsive ADB server\n'
    real_android_adb kill-server >/dev/null 2>&1 || true
    real_android_adb start-server >/dev/null 2>&1 || true
}

real_android_cleanup_runtime_tmp() {
    if [[ -n "${REAL_ANDROID_RUNTIME_TMP:-}" ]]; then
        rm -rf "$REAL_ANDROID_RUNTIME_TMP"
        REAL_ANDROID_RUNTIME_TMP=""
    fi
}

real_android_require_device() {
    REAL_ANDROID_DEVICE_ID="$(real_android_device)"
    if [[ -n "${NOLA_ANDROID_REAL_SERIAL:-}" && -z "$REAL_ANDROID_DEVICE_ID" ]]; then
        printf 'The explicitly selected Android test device is unavailable.\n' >&2
        return 2
    fi
    if [[ -z "$REAL_ANDROID_DEVICE_ID" ]]; then
        real_android_recover_adb
        REAL_ANDROID_DEVICE_ID="$(real_android_device)"
    fi
    REAL_ANDROID_STARTED_EMULATOR=0
    if [[ "$REAL_ANDROID_DEVICE_ID" == emulator-* ]] &&
       ! real_android_boot_completed "$REAL_ANDROID_DEVICE_ID"; then
        printf '[real-android-device] discarding unresponsive %s\n' "$REAL_ANDROID_DEVICE_ID"
        real_android_adb -s "$REAL_ANDROID_DEVICE_ID" emu kill >/dev/null 2>&1 || true
        REAL_ANDROID_DEVICE_ID=""
    fi
    if [[ -z "$REAL_ANDROID_DEVICE_ID" ]]; then
        local avd="${NOLA_ANDROID_REAL_AVD:-nola_avd}"
        [[ -x "$REAL_ANDROID_EMULATOR" ]] || {
            printf 'Android emulator is missing: %s\n' "$REAL_ANDROID_EMULATOR" >&2
            return 2
        }
        printf '[real-android-device] starting AVD %s\n' "$avd"
        REAL_ANDROID_RUNTIME_TMP="$(mktemp -d "${TMPDIR:-/tmp}/nola-android-emulator.XXXXXX")"
        ANDROID_TMP="$REAL_ANDROID_RUNTIME_TMP" \
        ANDROID_EMULATOR_TMPDIR="$REAL_ANDROID_RUNTIME_TMP" \
        TMPDIR="$REAL_ANDROID_RUNTIME_TMP/" \
        "$REAL_ANDROID_EMULATOR" -avd "$avd" -no-window -no-audio -no-boot-anim \
            -gpu swiftshader_indirect -no-snapshot-save \
            >"${NOLA_ANDROID_EMULATOR_LOG:-$REAL_ANDROID_RUNTIME_TMP/emulator.log}" 2>&1 &
        REAL_ANDROID_STARTED_EMULATOR=1
    else
        printf '[real-android-device] reusing %s\n' "$REAL_ANDROID_DEVICE_ID"
    fi
    local booted=0
    printf '[real-android-device] waiting for Android boot completion\n'
    for _ in $(seq 1 "${NOLA_ANDROID_BOOT_TIMEOUT_SEC:-180}"); do
        REAL_ANDROID_DEVICE_ID="$(real_android_device)"
        if [[ -n "$REAL_ANDROID_DEVICE_ID" ]] &&
           real_android_boot_completed "$REAL_ANDROID_DEVICE_ID"; then
            booted=1
            break
        fi
        sleep 1
    done
    [[ "$booted" == 1 ]] || {
        printf 'Android device did not boot before the deadline.\n' >&2
        return 2
    }
    REAL_ANDROID_ABI="$(real_android_adb -s "$REAL_ANDROID_DEVICE_ID" shell getprop ro.product.cpu.abi | tr -d '\r[:space:]')"
    case "$REAL_ANDROID_ABI" in
        arm64-v8a|armeabi-v7a|x86|x86_64) ;;
        *) printf 'Unsupported Android device ABI: %s\n' "$REAL_ANDROID_ABI" >&2; return 2 ;;
    esac
    printf '[real-android-device] ready %s (%s)\n' "$REAL_ANDROID_DEVICE_ID" "$REAL_ANDROID_ABI"
}

real_android_release_device() {
    if [[ "${REAL_ANDROID_STARTED_EMULATOR:-0}" == 1 && -n "${REAL_ANDROID_DEVICE_ID:-}" ]]; then
        local released_device="$REAL_ANDROID_DEVICE_ID"
        printf '[real-android-device] stopping owned emulator %s\n' "$released_device"
        real_android_adb -s "$REAL_ANDROID_DEVICE_ID" emu kill >/dev/null 2>&1 || true
        for _ in $(seq 1 "${NOLA_ANDROID_SHUTDOWN_TIMEOUT_SEC:-60}"); do
            if ! real_android_adb devices 2>/dev/null |
                awk 'NR > 1 { print $1 }' | grep -Fxq "$released_device"; then
                printf '[real-android-device] stopped %s\n' "$released_device"
                real_android_cleanup_runtime_tmp
                return 0
            fi
            sleep 1
        done
        printf 'Android emulator %s did not stop before the deadline.\n' "$released_device" >&2
        real_android_cleanup_runtime_tmp
        return 2
    fi
    real_android_cleanup_runtime_tmp
}
