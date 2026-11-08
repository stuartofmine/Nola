#!/bin/bash
# Explicit developer build only; no tool install and no device/key operations.
set -euo pipefail
if [[ $# != 3 || "$1" != /* || "$2" != /* || "$3" != /* ]]; then
  echo 'Usage: build-sdk.sh /absolute/go /absolute/pinned/gomobile /absolute/pinned/gobind' >&2
  exit 2
fi
cd "$(dirname "$0")"
if [[ "$(dirname "$2")" != "$(dirname "$3")" ]]; then
  echo 'Use matching pinned gomobile/gobind in one approved tool directory' >&2
  exit 2
fi
export PATH="$(dirname "$2"):$(dirname "$1"):/usr/bin:/bin"
export GOENV=off GOTOOLCHAIN=local
mkdir -p build/tmp
export TMPDIR="$PWD/build/tmp/"
mkdir -p build/apple build/android
"$2" bind -target=macos/arm64,ios/arm64,iossimulator/arm64 -tags=ts_omit_ssh -prefix=NolaTailcat -o build/apple/NolaTailcat.xcframework ./mobile
"$2" bind -target=android/arm64 -androidapi=26 -tags=ts_omit_ssh -o build/android/nola-tailcat.aar ./mobile
cat > build/apple/SDK.xcconfig <<'CONFIG'
// Generated optional SDK. Never contains connection credentials.
FRAMEWORK_SEARCH_PATHS[sdk=macosx*] = $(inherited) "$(SRCROOT)/transport/tailcat/build/apple/NolaTailcat.xcframework/macos-arm64"
FRAMEWORK_SEARCH_PATHS[sdk=iphoneos*] = $(inherited) "$(SRCROOT)/transport/tailcat/build/apple/NolaTailcat.xcframework/ios-arm64"
FRAMEWORK_SEARCH_PATHS[sdk=iphonesimulator*] = $(inherited) "$(SRCROOT)/transport/tailcat/build/apple/NolaTailcat.xcframework/ios-arm64-simulator"
OTHER_LDFLAGS = $(inherited) -framework NolaTailcat
CONFIG
