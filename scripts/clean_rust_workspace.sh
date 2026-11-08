#!/usr/bin/env sh
# SPDX-License-Identifier: Apache-2.0
# Trim this checkout's native Debug workspace outputs, not dependency caches.
set -eu
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
REPOSITORY_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd -P)
case "${1:---dry-run}" in
    --dry-run|--apply) [ "$#" -le 1 ] || exit 2 ;;
    --help)
        printf '%s\n' 'Usage: sh scripts/clean_rust_workspace.sh [--dry-run|--apply]' \
            'Default: preview only. --apply removes Nola native Debug outputs (including current binaries).' \
            'Preserves third-party dependencies, Release, toolchains, mbx actions and app data.' \
            'Rebuild before launching target/debug binaries. Cargo owns build locking.'
        exit 0 ;;
    *) printf '%s\n' 'error: expected --dry-run, --apply or --help' >&2; exit 2 ;;
esac
# Intentionally ignore CARGO_TARGET_DIR: this maintenance command is scoped to
# this checkout even when invoked from another directory or a CI environment.
cd "$REPOSITORY_ROOT"
MODE=${1:---dry-run}
set -- clean --workspace --profile dev --frozen \
    --manifest-path "$REPOSITORY_ROOT/Cargo.toml" \
    --target-dir "$REPOSITORY_ROOT/target"
if [ "$MODE" = --dry-run ]; then set -- "$@" --dry-run; fi
exec "${CARGO:-cargo}" "$@"
