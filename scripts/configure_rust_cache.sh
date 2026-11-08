#!/usr/bin/env sh
# SPDX-License-Identifier: Apache-2.0
# Explicit machine policy, never called implicitly by a build. mbx does not
# permit storage/GC settings in a repository-owned .mbx.toml.
set -eu

case "${1:---help}" in
    --apply)
        [ "$#" -eq 1 ] || exit 2
        command -v mbx >/dev/null
        # These are shared-machine settings. No credentials or remote settings
        # are changed, and no immediate collection is performed here.
        mbx settings set learned_incremental false
        mbx settings set eager_incremental false
        mbx settings set gc.max_total_size 40GiB
        mbx settings set gc.max_size 12GiB
        mbx settings set gc.incremental_max_size 4GiB
        mbx settings set learned_incremental_max_size 1GiB
        mbx settings set gc.incremental_max_age 7d
        mbx settings set target.max_size 24GiB
        mbx settings set target.max_age 7d
        mbx settings set gc.min_free_size 12GiB
        mbx settings set gc.interval 15m
        mbx settings set savings plain
        printf '%s\n' 'Applied shared-machine mbx policy. Inspect collection with: mbx gc --dry-run'
        ;;
    --help)
        printf '%s\n' \
            'Usage: sh scripts/configure_rust_cache.sh --apply' \
            'Explicitly changes shared-machine mbx settings (affects other projects).' \
            '40 GiB combined soft budget; 12 GiB actions, 24 GiB managed targets,' \
            '4 GiB incremental state; 12 GiB disk reserve; 7 day inactive age.' \
            'Disables automatic learned/eager incremental state; keeps shared cache.' \
            'Does not delete files. Review mbx gc --dry-run before running mbx gc.' \
            'Unmanaged target directories are NOT covered: adopt this checkout once.'
        ;;
    *) printf '%s\n' 'error: expected --apply or --help' >&2; exit 2 ;;
esac
