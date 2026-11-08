#!/bin/zsh
# SPDX-License-Identifier: Apache-2.0
# File Overview:
# - Purpose: Automates operational steps for android emulator diagnose and start in the project module.
# - Interfaces:
#   - Internal implementation details only.
# - APIs:
#   - none
#
# Diagnose emulator installation, reinstall if needed, create AVD and start emulator
set -u

ANDROID_SDK_ROOT="$HOME/Library/Android/sdk"
SDKMAN="$ANDROID_SDK_ROOT/cmdline-tools/latest/bin/sdkmanager"
AVDMAN="$ANDROID_SDK_ROOT/cmdline-tools/latest/bin/avdmanager"
EMULATOR_BIN="$ANDROID_SDK_ROOT/emulator/emulator"

echo "== SDK root: $ANDROID_SDK_ROOT =="
echo "\n== emulator dir =="
ls -la "$ANDROID_SDK_ROOT/emulator" || echo "emulator dir missing"

echo "\n== find emulator binaries =="
find "$ANDROID_SDK_ROOT" -maxdepth 4 -type f -name emulator -print || true

echo "\n== file check on expected binary =="
if [ -f "$EMULATOR_BIN" ]; then
  file "$EMULATOR_BIN" || true
  ls -la "$EMULATOR_BIN" || true
else
  echo "emulator binary not found at $EMULATOR_BIN"
fi

echo "\n== avdmanager =="
ls -la "$AVDMAN" || echo "avdmanager missing"

echo "\n== adb =="
which adb || echo "adb not in PATH (temporarily adding platform-tools to PATH)"
export PATH="$ANDROID_SDK_ROOT/platform-tools:$PATH"
which adb || echo "adb still not found"

echo "\n== adb devices (before emulator start) =="
adb devices || true

# If emulator binary missing, attempt reinstall
if [ ! -f "$EMULATOR_BIN" ]; then
  echo "\n== Emulator binary missing; reinstalling emulator package =="
  LOG="$HOME/sdkmanager_emulator_reinstall.log"
  "$SDKMAN" --sdk_root="$ANDROID_SDK_ROOT" --install "emulator" --verbose > "$LOG" 2>&1 || true
  echo "Reinstall log saved to: $LOG"
  sed -n '1,100p' "$LOG" || true
  
  if [ ! -f "$EMULATOR_BIN" ]; then
    echo "ERROR: emulator binary still missing after reinstall. Check $LOG"
    exit 1
  fi
fi

echo "\n== Confirmed emulator binary exists =="
ls -la "$EMULATOR_BIN"

# Create AVD if not exists (or recreate)
AVD_NAME="nola_avd"
AVD_DIR="$HOME/.android/avd/${AVD_NAME}.avd"
AVD_INI="$HOME/.android/avd/${AVD_NAME}.ini"

echo "\n== Creating AVD: $AVD_NAME =="
if [ -d "$AVD_DIR" ] || [ -f "$AVD_INI" ]; then
  echo "Existing AVD found; removing to recreate"
  rm -rf "$AVD_DIR" "$AVD_INI" || true
fi

# Create AVD (non-interactive with --force)
echo "no" | "$AVDMAN" create avd -n "$AVD_NAME" -k "system-images;android-35;google_apis;arm64-v8a" --device "pixel" || true

# Verify AVD created
if [ -d "$AVD_DIR" ]; then
  echo "AVD created successfully at: $AVD_DIR"
else
  echo "WARNING: AVD directory not found after creation. Check avdmanager output above."
fi

# Start emulator in background
EMU_LOG="$HOME/emulator_start.log"
echo "\n== Starting emulator (logs -> $EMU_LOG) =="
"$EMULATOR_BIN" -avd "$AVD_NAME" -no-snapshot-load -no-audio -no-boot-anim -gpu host > "$EMU_LOG" 2>&1 &
EMUPID=$!
echo "Emulator PID: $EMUPID"

# Wait for emulator to initialize
echo "Waiting 25s for emulator to boot..."
sleep 25

# Check adb devices
echo "\n== adb devices (after emulator start) =="
adb devices

# Show emulator log head
echo "\n== Emulator log (first 100 lines) =="
sed -n '1,100p' "$EMU_LOG" || true

# Check emulator process
echo "\n== Emulator process =="
ps aux | grep -E "[e]mulator|qemu" || echo "No emulator process found"

echo "\n== Script complete =="
echo "If no device shown in 'adb devices', check $EMU_LOG for errors."
echo "To manually check emulator: ps aux | grep emulator"
echo "To stop emulator: pkill -f emulator"
