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
      # Keep mobile optimization and one shared release cache: opt-level=0
      # has caused runtime initialization faults, and a new profile duplicates
      # every SDK dependency tree.
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
      {
        find "$ROOT" "$ROOT/../third_party/a2ui/spec" \
          -type d \( -name target -o -name 'target_*' -o -name tests \) -prune -o \
          -type f \( -name '*.rs' -o -name 'Cargo.toml' -o -name 'build.rs' -o -name '*.sql' -o -name '*.json' -o -name '*.h' -o -name 'module.modulemap' \) \
          -print
        local workspace_root
        workspace_root="$(cd "$ROOT/.." && pwd -P)"
        printf '%s\n' \
          "$workspace_root/Cargo.toml" \
          "$workspace_root/Cargo.lock" \
          "$workspace_root/.cargo/config.toml" \
          "$workspace_root/rust-toolchain.toml" \
          "$ROOT/scripts/build_xcframework.sh" \
          "$workspace_root/scripts/lib/rust_build_cache.sh"
      } | LC_ALL=C sort -u | sed 's/[\\ #]/\\&/g' | awk '{ printf " %s", $0 }'
      printf '\n'
    } > "$dependency_file"
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
  for target in "${targets[@]}"; do
    if ! rustup target list --installed | grep -q "^$target$"; then
      rustup target add "$target"
    fi
    local target_dir="${NOLA_CORE_CARGO_TARGET_DIR:-${CARGO_TARGET_DIR:-$ROOT/target}}"
    local target_args=(--target "$target")
    # Native Mac builds reuse the CLI's host dependency graph. Cross builds
    # retain their existing target cache instead of cold-migrating SDK outputs.
    if [[ "$platform" == macosx && "$target" == "$(rustc -vV | sed -n 's/^host: //p')" ]]; then
      target_dir="${NOLA_CORE_CARGO_TARGET_DIR:-${CARGO_TARGET_DIR:-$ROOT/../target}}"
      target_args=()
    fi
    local artifact_dir="$target_dir/$target/$cargo_output_profile"
    [[ ${#target_args[@]} -ne 0 ]] || artifact_dir="$target_dir/$cargo_output_profile"
    local library="$artifact_dir/libnola_mobile_bindings.a"
    # Cargo validates the real transitive inputs, including JSON resources,
    # features, flags and toolchain. An archive mtime is not a freshness proof.
    CARGO_TARGET_DIR="$target_dir" nola_cargo rustc \
      --locked --manifest-path "$ROOT/../Cargo.toml" -p nola_mobile_bindings \
      --lib --crate-type staticlib ${target_args[@]+"${target_args[@]}"} --profile "$cargo_profile"
    libraries+=("$library")
  done

  if [ "${#libraries[@]}" -eq 1 ]; then
    if ! cmp -s "${libraries[0]}" "$output"; then cp "${libraries[0]}" "$output"; fi
  else
    local merged="$derived_dir/libnola_core.merged.a"
    lipo -create "${libraries[@]}" -output "$merged"
    if ! cmp -s "$merged" "$output"; then mv "$merged" "$output"; else rm "$merged"; fi
  fi
  if [[ ! -f "$profile_stamp" || "$(cat "$profile_stamp")" != "$cargo_profile" ]]; then
    printf '%s\n' "$cargo_profile" > "$profile_stamp"
  fi
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

# Cargo checks all transitive inputs for each requested SDK.

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
  CARGO_TARGET_DIR="$ROOT/target" nola_cargo rustc --manifest-path "$BINDINGS_MANIFEST" --lib --crate-type staticlib --target "$T" --release
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
