#!/bin/zsh
# SPDX-License-Identifier: Apache-2.0
# File Overview:
# - Purpose: Automates operational steps for android emulator fix in the project module.
# - Interfaces:
#   - Internal implementation details only.
# - APIs:
#   - none
#
# Backup, clean and reinstall Android emulator + system image (arm64)
# Usage: ./android_emulator_fix.sh

set -u

ANDROID_SDK_ROOT="$HOME/Library/Android/sdk"
TS=$(date +%s)
BACKUP="$ANDROID_SDK_ROOT/backup_$TS"
mkdir -p "$BACKUP"

echo "[1/7] Backing up possible leftover dirs to: $BACKUP"
mv "$ANDROID_SDK_ROOT/.temp" "$BACKUP/" 2>/dev/null || true
mv "$ANDROID_SDK_ROOT/.installer" "$BACKUP/" 2>/dev/null || true
mv "$ANDROID_SDK_ROOT/.install" "$BACKUP/" 2>/dev/null || true
if [ -d "$ANDROID_SDK_ROOT/emulator" ]; then
  mv "$ANDROID_SDK_ROOT/emulator" "$BACKUP/" 2>/dev/null || true
fi

echo "Backup done. Listing backup:"
ls -la "$BACKUP" || true

SDKMAN="$ANDROID_SDK_ROOT/cmdline-tools/latest/bin/sdkmanager"
if [ ! -x "$SDKMAN" ]; then
  echo "ERROR: sdkmanager not found or not executable: $SDKMAN"
  exit 1
fi

echo "[2/7] Running sdkmanager --update"
"$SDKMAN" --sdk_root="$ANDROID_SDK_ROOT" --update || true

LOG="$HOME/sdkmanager_emulator_retry_run.log"
LOG_NOHTTPS="$HOME/sdkmanager_emulator_retry_no_https_run.log"
LOG_IMG="$HOME/sdkmanager_systemimage_run.log"

echo "[3/7] Installing emulator (verbose). Log -> $LOG"
"$SDKMAN" --sdk_root="$ANDROID_SDK_ROOT" --install "emulator" --verbose > "$LOG" 2>&1 || true

if grep -q -E "Archive is not a ZIP archive|ZipException" "$LOG" 2>/dev/null || ! grep -q "Done" "$LOG" 2>/dev/null; then
  echo "[4/7] Detected corrupt archive or incomplete install in $LOG. Retrying with --no_https. Log -> $LOG_NOHTTPS"
  "$SDKMAN" --no_https --sdk_root="$ANDROID_SDK_ROOT" --install "emulator" --verbose > "$LOG_NOHTTPS" 2>&1 || true
else
  echo "[4/7] Emulator installation log does not show ZipException. Continuing."
fi

# Try to install arm64 system image for API 35
echo "[5/7] Installing system image (android-35;google_apis;arm64-v8a). Log -> $LOG_IMG"
"$SDKMAN" --sdk_root="$ANDROID_SDK_ROOT" --install "system-images;android-35;google_apis;arm64-v8a" --verbose > "$LOG_IMG" 2>&1 || true

# Show summaries
echo "[6/7] Summary:"
echo "emulator dir listing:"
ls -la "$ANDROID_SDK_ROOT/emulator" || echo "emulator dir missing"

echo "platform-tools listing:"
ls -la "$ANDROID_SDK_ROOT/platform-tools" || echo "platform-tools missing"

echo "system images (platforms) listing:"
ls -la "$ANDROID_SDK_ROOT/system-images" || ls -la "$ANDROID_SDK_ROOT/platforms" || true

echo "[7/7] Logs saved to:"
echo " - $LOG"
echo " - $LOG_NOHTTPS (if used)"
echo " - $LOG_IMG"

echo "Script finished. If installation still failed, please attach the first ~200 lines of the above logs for analysis."

# --- AVD create + start (non-interactive) ---
echo "\n[8/9] Creating AVD 'nola_avd' (non-interactive, will overwrite if exists)"
AVDMAN="$ANDROID_SDK_ROOT/cmdline-tools/latest/bin/avdmanager"
EMULATOR_BIN="$ANDROID_SDK_ROOT/emulator/emulator"
AVD_NAME="nola_avd"
AVD_DIR="$HOME/.android/avd/${AVD_NAME}.avd"
AVD_INI="$HOME/.android/avd/${AVD_NAME}.ini"

if [ -d "$AVD_DIR" ] || [ -f "$AVD_INI" ]; then
  echo "Existing AVD found; removing to recreate"
  rm -rf "$AVD_DIR" "$AVD_INI" || true
fi

echo "Creating AVD using: $AVDMAN"
"$AVDMAN" create avd -n "$AVD_NAME" -k "system-images;android-35;google_apis;arm64-v8a" --device "pixel" --force 2>/dev/null || \
  (echo "avdmanager create failed or prompted; trying without --force" && "$AVDMAN" create avd -n "$AVD_NAME" -k "system-images;android-35;google_apis;arm64-v8a" --device "pixel" )

echo "[9/9] Starting emulator 'nola_avd' in background (logs -> $HOME/emulator_start.log)"
mkdir -p "$HOME"
"$EMULATOR_BIN" -avd "$AVD_NAME" -no-snapshot-load -no-audio -no-boot-anim -gpu host > "$HOME/emulator_start.log" 2>&1 &
EMUPID=$!
echo "Emulator PID: $EMUPID"
echo "Sleeping 20s for emulator to initialize..."
sleep 20
echo "Checking adb devices:"
if command -v adb >/dev/null 2>&1; then
  adb devices || true
else
  echo "adb not in PATH; check PATH or run: export PATH=\"$ANDROID_SDK_ROOT/platform-tools:$PATH\""
fi

echo "AVD creation and emulator startup attempted. Check $HOME/emulator_start.log for details."
