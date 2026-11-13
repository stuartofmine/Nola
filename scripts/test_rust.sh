#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Unit-test default: a name filter alone must not link every integration test.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
if [[ $# -lt 1 || "$1" == -* ]]; then
  echo 'Usage: bash scripts/test_rust.sh <package> [filter] [--test <integration-target> | --tests | --all-targets] [Cargo test options]' >&2
  exit 2
fi
PACKAGE=$1
shift
. "$ROOT/scripts/lib/rust_build_cache.sh"
nola_rust_build_cache_init "${CARGO:-cargo}"
SELECTOR=(--lib)
for arg in "$@"; do
  [[ "$arg" != -- ]] || break
  case "$arg" in
    --lib|--test|--test=*|--tests|--all-targets|--bin|--bin=*|--bins|--example|--example=*|--examples|--doc)
      SELECTOR=(); break ;;
  esac
done
nola_cargo test --locked --manifest-path "$ROOT/Cargo.toml" -p "$PACKAGE" ${SELECTOR[@]+"${SELECTOR[@]}"} "$@"
