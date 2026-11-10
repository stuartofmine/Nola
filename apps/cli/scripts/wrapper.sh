#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# File Overview:
# - Purpose: Wrapper script that parses arguments and delegates to build.sh
# - Usage: ./wrapper.sh [build_options] -- [nola_args]
#   Example: ./wrapper.sh --debug --run -- nola_arg1 nola_arg2

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_SCRIPT="$SCRIPT_DIR/build.sh"

print_help() {
  cat <<'EOF'
Usage: ./wrapper.sh [options] -- [args]

Wrapper script for build.sh with argument forwarding.

Build Options:
  --debug                  Build in debug mode
  --release                Build in release mode (default)
  --target <triple>        Build for a specific target triple
  --targets <list>         Build comma-separated target triples
  --preset <name>          Build a target preset: macos, linux, windows, all
  --install-targets       Allow installing missing Rust targets
  --features <feature..>   Pass cargo --features argument
  --clean                  Run cargo clean before building
  --run                    Build then start Nola immediately

Argument Forwarding:
  --                       Separate build flags from runtime args
  [args...]                Arguments passed to nola after build

Examples:
  ./wrapper.sh --debug --run -- nola_arg1 nola_arg2
  ./wrapper.sh --target x86_64-unknown-linux-gnu --features json -- 
  ./wrapper.sh --clean -- nola_help

EOF
}

# Arrays to hold build options and runtime args
BUILD_ARGS=()
RUN_ARGS=()

# Parse arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --debug)
      BUILD_ARGS+=("--debug")
      shift
      ;;
    --release)
      BUILD_ARGS+=("--release")
      shift
      ;;
    --target)
      if [[ $# -lt 2 ]]; then
        echo "Error: Missing value for --target" >&2
        exit 1
      fi
      BUILD_ARGS+=("--target" "$2")
      shift 2
      ;;
    --targets|--preset)
      if [[ $# -lt 2 ]]; then
        echo "Error: Missing value for $1" >&2
        exit 1
      fi
      BUILD_ARGS+=("$1" "$2")
      shift 2
      ;;
    --install-targets)
      BUILD_ARGS+=("--install-targets")
      shift
      ;;
    --features)
      if [[ $# -lt 2 ]]; then
        echo "Error: Missing value for --features" >&2
        exit 1
      fi
      BUILD_ARGS+=("--features" "$2")
      shift 2
      ;;
    --clean)
      BUILD_ARGS+=("--clean")
      shift
      ;;
    --run)
      BUILD_ARGS+=("--run")
      shift
      ;;
    --)
      # Everything after -- is passed to nola
      shift
      RUN_ARGS=("$@")
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

# Validate that build.sh exists
if [[ ! -f "$BUILD_SCRIPT" ]]; then
  echo "Error: build.sh not found at $BUILD_SCRIPT" >&2
  exit 1
fi

# Build a command string to pass to build.sh
if [[ ${#BUILD_ARGS[@]} -gt 0 ]]; then
  # Join build args with spaces
  BUILD_CMD="${BUILD_ARGS[*]}"
  
  # Check if we need to add -- separator for nola args
  if [[ ${#RUN_ARGS[@]} -gt 0 ]]; then
    # Run build.sh with build args, then nola with runtime args
    echo "Running build with args: $BUILD_CMD"
    exec bash "$BUILD_SCRIPT" "${BUILD_ARGS[@]}" -- "${RUN_ARGS[@]}"
  else
    # Just run build.sh with build args
    echo "Running build with args: $BUILD_CMD"
    exec bash "$BUILD_SCRIPT" "${BUILD_ARGS[@]}"
  fi
elif [[ ${#RUN_ARGS[@]} -gt 0 ]]; then
  # No build args, just pass runtime args directly to build.sh
  # This allows build.sh to receive them as --run args
  echo "Running build with runtime args: ${RUN_ARGS[*]}"
  exec bash "$BUILD_SCRIPT" --run -- "${RUN_ARGS[@]}"
else
  exec bash "$BUILD_SCRIPT"
fi
