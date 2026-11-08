#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# File Overview:
# - Purpose: Automates operational steps for build xcframework in the Rust core module.
# - Interfaces:
#   - Internal implementation details only.
# - APIs:
#   - none
#
set -euo pipefail

# Build xcframework for macOS/iOS (device+sim) and produce an XCFramework that Swift can link against.

# Add Rust toolchain to PATH (common installation locations)
if [ -n "${CARGO_HOME:-}" ] && [ -d "$CARGO_HOME/bin" ]; then
  export PATH="$CARGO_HOME/bin:$PATH"
elif [ -f "$HOME/.cargo/env" ]; then
  source "$HOME/.cargo/env"
elif [ -d "$HOME/.cargo/bin" ]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi

# Verify Rust is available
if ! command -v rustup &> /dev/null; then
  echo "❌ Error: Rust toolchain not found in PATH"
  echo "Please install Rust from https://rustup.rs/ or ensure ~/.cargo/bin is in your PATH"
  exit 1
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
. "$ROOT/../scripts/lib/rust_build_cache.sh"
nola_rust_build_cache_init "${CARGO:-cargo}"
BINDINGS_MANIFEST="$ROOT/nola-mobile-bindings/Cargo.toml"
OUT_DIR="$ROOT/target_xcframework"
XCFRAMEWORK_PATH="$OUT_DIR/nola_core.xcframework"
DEST_PROJECT_FRAMEWORKS="$(cd "$ROOT/.." && pwd -P)/Frameworks"

build_xcode_target_archive() {
  local platform="${PLATFORM_NAME:-iphonesimulator}"
  local derived_dir="${DERIVED_FILE_DIR:-$ROOT/target_xcode}/nola_core"
  local output="$derived_dir/libnola_core.a"
  local requested_archs="${ARCHS:-arm64}"
  local cargo_profile="${NOLA_CARGO_PROFILE:-}"
  if [[ -z "$cargo_profile" ]]; then
    if [[ "$platform" == "macosx" ]]; then
      case "${CONFIGURATION:-Debug}" in
        Release) cargo_profile="release" ;;
        *) cargo_profile="dev" ;;
      esac
    else
      # The iOS FFI runtime currently requires optimized Rust code. Dev builds
      # reproducibly fault in AgentRuntime::try_new on device/simulator.
      cargo_profile="release"
    fi
  fi
  local cargo_output_profile="$cargo_profile"
  [[ "$cargo_output_profile" == "dev" ]] && cargo_output_profile="debug"
  local profile_stamp="$derived_dir/cargo-profile"
  local targets=()
  local libraries=()

  echo "Nola Core build configuration: platform=$platform configuration=${CONFIGURATION:-unknown} archs=$requested_archs cargo_profile=$cargo_profile"

  escape_dependency_path() {
    printf '%s' "$1" | sed 's/[\\ #]/\\&/g'
  }

  write_dependency_file() {
    local dependency_file="${NOLA_DEPENDENCY_FILE:-}"
    [[ -z "$dependency_file" ]] && return
    mkdir -p "$(dirname "$dependency_file")"
    {
      printf '%s:' "$(escape_dependency_path "$output")"
      find "$ROOT" "$ROOT/../third_party/a2ui/spec" \
        -type d \( -name target -o -name 'target_*' \) -prune -o \
        -type f \( -name '*.rs' -o -name 'Cargo.toml' -o -name 'build.rs' -o -name '*.sql' -o -name '*.json' -o -name '*.h' -o -name 'module.modulemap' \) \
        -print | sort | while IFS= read -r path; do
          printf ' %s' "$(escape_dependency_path "$path")"
        done
      local workspace_root
      workspace_root="$(cd "$ROOT/.." && pwd -P)"
      for path in \
        "$workspace_root/Cargo.toml" \
        "$workspace_root/Cargo.lock" \
        "$ROOT/scripts/build_xcframework.sh" \
        "$workspace_root/scripts/lib/rust_build_cache.sh"; do
        printf ' %s' "$(escape_dependency_path "$path")"
      done
      printf '\n'
    } > "$dependency_file"
  }

  core_sources_are_newer() {
    local artifact="$1"
    [[ ! -f "$artifact" ]] && return 0
    find "$ROOT" \
      -type d \( -name target -o -name 'target_*' \) -prune -o \
      -type f \( -name '*.rs' -o -name 'Cargo.toml' -o -name 'build.rs' -o -name '*.sql' -o -name '*.h' -o -name 'module.modulemap' \) \
      -newer "$artifact" -print -quit | grep -q . && return 0
    local workspace_root
    workspace_root="$(cd "$ROOT/.." && pwd -P)"
    [[ "$workspace_root/Cargo.toml" -nt "$artifact" || \
       "$workspace_root/Cargo.lock" -nt "$artifact" || \
       "$ROOT/scripts/build_xcframework.sh" -nt "$artifact" || \
       "$workspace_root/scripts/lib/rust_build_cache.sh" -nt "$artifact" ]]
  }

  for arch in $requested_archs; do
    case "$platform:$arch" in
      iphoneos:arm64)
        targets+=("aarch64-apple-ios")
        ;;
      iphonesimulator:arm64)
        targets+=("aarch64-apple-ios-sim")
        ;;
      iphonesimulator:x86_64)
        targets+=("x86_64-apple-ios")
        ;;
      macosx:arm64)
        targets+=("aarch64-apple-darwin")
        ;;
      macosx:x86_64)
        targets+=("x86_64-apple-darwin")
        ;;
      *)
        echo "Unsupported Xcode Rust target: platform=$platform arch=$arch" >&2
        exit 1
        ;;
    esac
  done

  mkdir -p "$derived_dir"
  if [[ -f "$output" ]] && \
     [[ -f "$profile_stamp" ]] && \
     [[ "$(cat "$profile_stamp")" == "$cargo_profile" ]] && \
     ! core_sources_are_newer "$output"; then
    echo "Xcode Rust core is up to date; skipping Cargo build"
    write_dependency_file
    return
  fi
  for target in "${targets[@]}"; do
    if ! rustup target list --installed | grep -q "^$target$"; then
      rustup target add "$target"
    fi
    local library="$ROOT/target/$target/$cargo_output_profile/libnola_mobile_bindings.a"
    if core_sources_are_newer "$library"; then
      CARGO_TARGET_DIR="$ROOT/target" nola_cargo build \
        --locked \
        --manifest-path "$BINDINGS_MANIFEST" \
        --target "$target" \
        --profile "$cargo_profile"
    else
      echo "Reusing current Rust core artifact: $library"
    fi
    libraries+=("$library")
  done

  if [ "${#libraries[@]}" -eq 1 ]; then
    cp "${libraries[0]}" "$output"
  else
    lipo -create "${libraries[@]}" -output "$output"
  fi
  printf '%s\n' "$cargo_profile" > "$profile_stamp"
  write_dependency_file
  echo "Built Xcode Rust core: $output"
}

if [[ "${1:-}" == "--xcode-target" ]]; then
  build_xcode_target_archive
  exit 0
fi

mkdir -p "$OUT_DIR"

echo "🦀 Building xcframework for nola_core in $OUT_DIR"
echo "Using Rust: $(rustup --version)"
echo "Using Cargo: $(nola_cargo --version)"

# Check if we need to rebuild by comparing source timestamps
NEEDS_REBUILD=false

if [ ! -d "$XCFRAMEWORK_PATH" ]; then
  echo "📦 XCFramework doesn't exist, full build required"
  NEEDS_REBUILD=true
else
  # Find the newest source file
  NEWEST_SOURCE=$(
    {
      find "$ROOT/src" -type f -name "*.rs" -print
      find "$ROOT/nola-mobile-bindings/src" -type f -name "*.rs" -print
      find "$ROOT/nola-store/src" -type f -name "*.rs" -print
      find "$ROOT/nola-domain/src" -type f -name "*.rs" -print
      find "$ROOT/nola-host-protocol/src" -type f -name "*.rs" -print
      find "$ROOT/swift_shims" -type f \( -name "*.h" -o -name "module.modulemap" \) -print
      printf '%s\n' \
        "$ROOT/../Cargo.toml" \
        "$ROOT/../Cargo.lock" \
        "$ROOT/../.cargo/config.toml" \
        "$ROOT/Cargo.toml" \
        "$ROOT/Cargo.lock" \
        "$ROOT/nola-mobile-bindings/Cargo.toml" \
        "$ROOT/nola-store/Cargo.toml" \
        "$ROOT/nola-domain/Cargo.toml" \
        "$ROOT/nola-host-protocol/Cargo.toml" \
        "$ROOT/../scripts/lib/rust_build_cache.sh"
    } | xargs stat -f "%m %N" 2>/dev/null | sort -rn | head -1 | cut -d' ' -f2-
  )
  if [ -n "$NEWEST_SOURCE" ]; then
    SOURCE_TIME=$(stat -f "%m" "$NEWEST_SOURCE" 2>/dev/null || echo "0")
    XCFRAMEWORK_TIME=$(stat -f "%m" "$XCFRAMEWORK_PATH" 2>/dev/null || echo "0")
    
    if [ "$SOURCE_TIME" -gt "$XCFRAMEWORK_TIME" ]; then
      echo "🔄 Source files newer than XCFramework, rebuilding..."
      NEEDS_REBUILD=true
    else
      echo "✅ XCFramework is up-to-date, skipping build"
      mkdir -p "$DEST_PROJECT_FRAMEWORKS"
      rm -rf "$DEST_PROJECT_FRAMEWORKS/nola_core.xcframework"
      cp -R "$XCFRAMEWORK_PATH" "$DEST_PROJECT_FRAMEWORKS/"
      echo "💡 To force rebuild: rm -rf $XCFRAMEWORK_PATH"
      exit 0
    fi
  fi
fi

# Targets to build
TARGETS=(
  "aarch64-apple-ios" # iOS device arm64
  "x86_64-apple-ios"  # iOS simulator x86
  "aarch64-apple-ios-sim" # iOS simulator arm64 (M1/M2)
  "x86_64-apple-darwin" # macOS x86
  "aarch64-apple-darwin" # macOS arm
)

# Build each target; Cargo fingerprints reuse unchanged artifacts.
for T in "${TARGETS[@]}"; do
  echo "Building for $T..."
  # Check if target installed
  if ! rustup target list --installed | grep -q "^$T$"; then
    echo "Adding missing target: $T"
    rustup target add $T
  fi
  # Workspace profiles disable incremental caches and debug information.
  CARGO_TARGET_DIR="$ROOT/target" nola_cargo build --manifest-path "$BINDINGS_MANIFEST" --target "$T" --release
done

LIBS=()
IOS_DEVICE_LIB=""
IOS_SIM_LIBS=()
MAC_LIBS=()

# Find built libs and add to list
for T in "${TARGETS[@]}"; do
  LIB_PATH="$ROOT/target/$T/release/libnola_mobile_bindings.a"
  if [ -f "$LIB_PATH" ]; then
    case "$T" in
      *-apple-ios*)
        if [[ "$T" == "aarch64-apple-ios" ]]; then
          IOS_DEVICE_LIB="$LIB_PATH"
        else
          IOS_SIM_LIBS+=("$LIB_PATH")
        fi
        ;;
      *-apple-darwin)
        MAC_LIBS+=("$LIB_PATH")
        ;;
      *)
        LIBS+=("$LIB_PATH:$T")
        ;;
    esac
  else
    echo "Warning: $LIB_PATH not found; ensure target is installed (rustup target add $T)"
  fi
done

# Ensure header available
HDR="$ROOT/swift_shims/nola_core.h"
if [ ! -f "$HDR" ]; then
  echo "Error: header $HDR not found"
  exit 1
fi

XCFRAMEWORK_PATH="$OUT_DIR/nola_core.xcframework"
rm -rf "$XCFRAMEWORK_PATH"

echo "Creating xcframework..."

# For simulator (if multiple sim libs exist), create a fat library using lipo
if [ ${#IOS_SIM_LIBS[@]} -gt 0 ]; then
  SIM_UNIV="$OUT_DIR/libnola_core_ios_sim_universal.a"
  rm -f "$SIM_UNIV"
  echo "Creating iOS simulator universal with lipo: ${IOS_SIM_LIBS[*]} -> $SIM_UNIV"
  lipo -create ${IOS_SIM_LIBS[*]} -output "$SIM_UNIV"
  LIBS+=("$SIM_UNIV:ios-sim")
fi

# For macOS, create universal library
if [ ${#MAC_LIBS[@]} -gt 0 ]; then
  MAC_UNIV="$OUT_DIR/libnola_core_macos_universal.a"
  rm -f "$MAC_UNIV"
  echo "Creating macOS universal with lipo: ${MAC_LIBS[*]} -> $MAC_UNIV"
  lipo -create ${MAC_LIBS[*]} -output "$MAC_UNIV"
  LIBS+=("$MAC_UNIV:macos")
fi

# Add iOS device lib if present
if [[ -n "$IOS_DEVICE_LIB" ]]; then
  LIBS+=("$IOS_DEVICE_LIB:ios")
fi

# Build xcodebuild create args
CREATE_ARGS=()
for LIB_PAIR in "${LIBS[@]}"; do
  IFS=":" read -r LIBPATH TGT <<< "$LIB_PAIR"
  CREATE_ARGS+=("-library" "$LIBPATH" "-headers" "$ROOT/swift_shims")
done

# Debug: print args
echo "xcodebuild -create-xcframework ${CREATE_ARGS[*]} -output $XCFRAMEWORK_PATH"
set -x
# Run xcodebuild to create xcframework
xcodebuild -create-xcframework "${CREATE_ARGS[@]}" -output "$XCFRAMEWORK_PATH"
set +x

echo "XCFramework created at $XCFRAMEWORK_PATH"

# Copy xcframework to Nola project (optional)
mkdir -p "$DEST_PROJECT_FRAMEWORKS"
rm -rf "$DEST_PROJECT_FRAMEWORKS/nola_core.xcframework"
cp -R "$XCFRAMEWORK_PATH" "$DEST_PROJECT_FRAMEWORKS/"
echo "Copied xcframework to $DEST_PROJECT_FRAMEWORKS"

echo "Done"

exit 0
