#!/usr/bin/env sh
# SPDX-License-Identifier: Apache-2.0
# File Overview:
# - Purpose: Build script for the Nola CLI binary.
# - Usage:
#   - Build in release: ./build.sh
#   - Build in debug: ./build.sh --debug
#   - Build for target: ./build.sh --target <triple>
#   - Build and run: ./build.sh --run [-- run_args]
#   - Build with features: ./build.sh --features <feature..>
#   - Clean before build: ./build.sh --clean

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# Always build through the workspace manifest. Building the same binary once
# through cli/Cargo.toml and once through the virtual workspace can resolve a
# different dependency feature graph while copying both cached artifacts to
# target/<profile>/nola. That makes a later build appear to restore an older
# executable even though Cargo reports every unit as fresh.
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd -P)"
. "$REPO_ROOT/scripts/lib/rust_build_cache.sh"
MANIFEST_PATH="$REPO_ROOT/Cargo.toml"
BINARY_NAME="nola"

print_help() {
cat <<'EOF'
Usage: ./build.sh [options]

Options:
  --debug                  Build in debug mode (default: --release).
  --release                Build in release mode (default).
  --target <triple>        Build for a specific target triple (e.g. x86_64-apple-darwin).
  --targets <triple,...>   Build for multiple explicit target triples, comma-separated.
  --preset <name>          Add a target preset: macos, linux, windows, all.
                           macos/linux/windows 都会按 arm/x86 双架构补齐。
  --install-targets       Allow rustup to install missing Rust targets without prompting.
  --features <feature..>   Pass cargo --features argument.
  --clean                  Run cargo clean before building.
  --run                    Build then start Nola immediately.
  --                       Separate build flags from runtime args and pass the rest to nola.
  -h, --help               Show this help message.
EOF
}

validate_target() {
  value="$1"
  case "$value" in
    ""|*[!A-Za-z0-9_.-]*)
      return 1
      ;;
    *)
      return 0
      ;;
  esac
}

append_target() {
  target="$1"
  case " $TARGETS " in
    *" $target "*) return 0 ;;
    *) ;;
  esac
  if [ -n "$TARGETS" ]; then
    TARGETS="$TARGETS $target"
  else
    TARGETS="$target"
  fi
}

append_preset_targets() {
  preset="$1"
  case "$preset" in
    macos|mac|darwin)
      append_target "x86_64-apple-darwin"
      append_target "aarch64-apple-darwin"
      ;;
    linux)
      append_target "x86_64-unknown-linux-gnu"
      append_target "aarch64-unknown-linux-gnu"
      ;;
    windows|win)
      append_target "x86_64-pc-windows-gnu"
      append_target "aarch64-pc-windows-msvc"
      ;;
    all)
      append_preset_targets "macos"
      append_preset_targets "linux"
      append_preset_targets "windows"
      ;;
    *)
      echo "Unknown preset: $preset" >&2
      return 1
      ;;
  esac
}

binary_ext() {
  target="$1"
  case "$target" in
    *-pc-windows-*) echo ".exe" ;;
    *) echo "" ;;
  esac
}

is_windows_target() {
  target="$1"
  case "$target" in
    *-pc-windows-*) return 0 ;;
    *) return 1 ;;
  esac
}

count_tokens() {
  set -- $1
  echo "$#"
}

MODE="release"
TARGETS=""
FEATURES=""
CLEAN=0
RUN_CLI=0
INSTALL_TARGETS=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    debug|--debug)
      MODE="debug"
      shift
      ;;
    release|--release)
      MODE="release"
      shift
      ;;
    --target)
      if [ "$#" -lt 2 ]; then
        echo "Missing value for --target" >&2
        exit 1
      fi
      TARGET_VAL="$2"
      if ! validate_target "$TARGET_VAL"; then
        echo "Error: Invalid target triple format. Expected format like x86_64-unknown-linux-gnu." >&2
        exit 1
      fi
      append_target "$TARGET_VAL"
      shift 2
      ;;
    --targets)
      if [ "$#" -lt 2 ]; then
        echo "Missing value for --targets" >&2
        exit 1
      fi
      target_list="$2"
      old_ifs="$IFS"
      IFS=','
      for TARGET_VAL in $target_list; do
        if [ -z "$TARGET_VAL" ]; then
          continue
        fi
        if ! validate_target "$TARGET_VAL"; then
          echo "Error: Invalid target triple format: $TARGET_VAL" >&2
          IFS="$old_ifs"
          exit 1
        fi
        append_target "$TARGET_VAL"
      done
      IFS="$old_ifs"
      shift 2
      ;;
    --preset)
      if [ "$#" -lt 2 ]; then
        echo "Missing value for --preset" >&2
        exit 1
      fi
      if ! append_preset_targets "$2"; then
        exit 1
      fi
      shift 2
      ;;
    --install-targets)
      INSTALL_TARGETS=1
      shift
      ;;
    --features)
      if [ "$#" -lt 2 ]; then
        echo "Missing value for --features" >&2
        exit 1
      fi
      FEATURES="$2"
      if echo "$FEATURES" | grep -Eq '[[:space:]]'; then
        echo "Error: --features value contains invalid characters (spaces)." >&2
        exit 1
      fi
      shift 2
      ;;
    --clean)
      CLEAN=1
      shift
      ;;
    --run)
      RUN_CLI=1
      shift
      ;;
    --)
      shift
      break
      ;;
    -h|--help)
      print_help
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      print_help
      exit 1
      ;;
  esac
done

if [ "$#" -gt 0 ]; then
  RUN_CLI=1
fi

if [ ! -f "$MANIFEST_PATH" ]; then
  echo "Cannot find Cargo manifest at $MANIFEST_PATH" >&2
  exit 1
fi

nola_rust_build_cache_init "${CARGO:-cargo}"

if [ ! -x "$(command -v rustc || true)" ]; then
  echo "rustc not found in PATH. Please install Rust toolchain." >&2
  exit 1
fi

TARGET_DIR="$(
  nola_cargo metadata --manifest-path "$MANIFEST_PATH" --no-deps --format-version 1 |
    sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p'
)"
if [ -z "$TARGET_DIR" ]; then
  echo "Cannot determine Cargo target directory for $MANIFEST_PATH" >&2
  exit 1
fi
# Preserve Cargo's existing artifact layout rather than adopting/moving target/.
if [ "${CARGO_TARGET_DIR+x}" != x ]; then
  export CARGO_TARGET_DIR="$TARGET_DIR"
fi

HOST_TARGET="$(rustc -vV | awk '/^host:/ { print $2 }')"

NATIVE_BUILD=0
if [ -z "$TARGETS" ]; then
  NATIVE_BUILD=1
  append_target "$HOST_TARGET"
fi

if [ "$CLEAN" -eq 1 ]; then
  echo "Running cargo clean..."
  nola_cargo clean --manifest-path "$MANIFEST_PATH"
fi

if [ "$MODE" = "release" ]; then
  MODE_FLAG="--release"
else
  MODE_FLAG=""
fi

if [ "$MODE" = "release" ]; then
  PROFILE_DIR="release"
else
  PROFILE_DIR="debug"
fi

TARGETS_COUNT=$(count_tokens "$TARGETS")
if [ "$RUN_CLI" -eq 1 ] && [ "$TARGETS_COUNT" -gt 1 ]; then
  echo "--run only supports a single target build." >&2
  exit 1
fi

if [ "$RUN_CLI" -eq 1 ]; then
  SINGLE_TARGET=""
  for T in $TARGETS; do
    if [ -z "$SINGLE_TARGET" ]; then
      SINGLE_TARGET="$T"
      break
    fi
  done
  if [ "$SINGLE_TARGET" != "$HOST_TARGET" ]; then
    echo "--run requires the host target $HOST_TARGET; $SINGLE_TARGET can only be built on this host." >&2
    exit 1
  fi
fi

SUCCESS_BINARY=""
for TARGET in $TARGETS; do
  EXT="$(binary_ext "$TARGET")"
  if [ "$NATIVE_BUILD" -eq 1 ]; then
    OUT_BIN="$TARGET_DIR/$PROFILE_DIR/$BINARY_NAME$EXT"
    CARGO_TARGET_ARG=""
    BUILD_DESTINATION="native host $HOST_TARGET"
  else
    OUT_BIN="$TARGET_DIR/$TARGET/$PROFILE_DIR/$BINARY_NAME$EXT"
    CARGO_TARGET_ARG="--target $TARGET"
    BUILD_DESTINATION="$TARGET"
  fi

  if [ "$NATIVE_BUILD" -eq 0 ] && command -v rustup >/dev/null; then
    if ! rustup target list --installed | grep -qx "$TARGET"; then
      if [ "$INSTALL_TARGETS" -eq 1 ]; then
        echo "Installing missing Rust target $TARGET ..."
        rustup target add "$TARGET"
      elif [ -t 0 ]; then
        printf "Rust target %s is not installed. Install it now? [y/N] " "$TARGET"
        reply=""
        IFS= read -r reply || true
        case "$reply" in
          y|Y|yes|YES|Yes)
            rustup target add "$TARGET"
            ;;
          *)
            echo "Target installation declined. Re-run with --install-targets to grant permission explicitly." >&2
            exit 1
            ;;
        esac
      else
        echo "Rust target $TARGET is not installed. Re-run interactively or pass --install-targets to grant installation permission." >&2
        exit 1
      fi
    fi
  fi

  if [ -n "$MODE_FLAG" ]; then
    if [ -n "$FEATURES" ]; then
      echo "Building CLI ($MODE) for $BUILD_DESTINATION..."
      nola_cargo build --manifest-path "$MANIFEST_PATH" $MODE_FLAG $CARGO_TARGET_ARG --bin "$BINARY_NAME" --bin nola-agent-host --features "$FEATURES"
    else
      echo "Building CLI ($MODE) for $BUILD_DESTINATION..."
      nola_cargo build --manifest-path "$MANIFEST_PATH" $MODE_FLAG $CARGO_TARGET_ARG --bin "$BINARY_NAME" --bin nola-agent-host
    fi
  else
    if [ -n "$FEATURES" ]; then
      echo "Building CLI ($MODE) for $BUILD_DESTINATION..."
      nola_cargo build --manifest-path "$MANIFEST_PATH" $CARGO_TARGET_ARG --bin "$BINARY_NAME" --bin nola-agent-host --features "$FEATURES"
    else
      echo "Building CLI ($MODE) for $BUILD_DESTINATION..."
      nola_cargo build --manifest-path "$MANIFEST_PATH" $CARGO_TARGET_ARG --bin "$BINARY_NAME" --bin nola-agent-host
    fi
  fi

  if [ -f "$OUT_BIN" ]; then
    echo "Build success: $OUT_BIN"
    if [ -z "$SUCCESS_BINARY" ]; then
      SUCCESS_BINARY="$OUT_BIN"
    fi
  else
    echo "Build completed for $TARGET but binary not found at $OUT_BIN" >&2
    echo "Possible reasons:" >&2
    echo "  - Build failed (check previous error messages)"
    echo "  - Target triple is unsupported by your system"
    echo "  - Cargo did not output the expected binary path"
    exit 1
  fi
done

if [ "$RUN_CLI" -eq 1 ]; then
  if [ -z "$SUCCESS_BINARY" ]; then
    echo "Build completed but no binary was produced." >&2
    exit 1
  fi
  echo "Starting Nola interactive CLI..."
  exec "$SUCCESS_BINARY" "$@"
fi
