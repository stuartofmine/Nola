#!/bin/sh
# Prepare Xcode Cloud's clean worker before any sandboxed Xcode build phases.
set -eu

NOLA_CI_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export PATH="$CARGO_HOME/bin:$PATH"

if ! command -v rustup >/dev/null 2>&1; then
    NOLA_RUSTUP_INSTALLER=$(mktemp /tmp/nola-rustup-init.XXXXXX)
    trap 'rm -f "$NOLA_RUSTUP_INSTALLER"' EXIT HUP INT TERM
    curl --proto '=https' --tlsv1.2 --fail --location --retry 3 \
        https://sh.rustup.rs -o "$NOLA_RUSTUP_INSTALLER"
    sh "$NOLA_RUSTUP_INSTALLER" -y --profile minimal --default-toolchain none --no-modify-path
fi

cd "$NOLA_CI_ROOT"
# rustup reads the repository's rust-toolchain.toml, including LLVM tools.
rustup show active-toolchain
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
rustc --version
cargo --version
