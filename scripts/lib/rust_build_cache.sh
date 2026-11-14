#!/usr/bin/env sh
# SPDX-License-Identifier: Apache-2.0
# Shared, process-scoped Rust cache selection. No setup, global configuration,
# target adoption, installation, or build retry is performed here.
# NOLA_RUST_BUILD_CACHE: auto (default), off, mbx/required, sccache (strict).
# NOLA_MBX_BIN optionally selects one executable, never a shell expression.

nola_rust_command_path() (
    nola_command=$(command -v "$1" 2>/dev/null) || exit 127
    [ -f "$nola_command" ] && [ -x "$nola_command" ] || exit 127
    case "$nola_command" in
        /*) printf '%s\n' "$nola_command" ;;
        *)
            nola_command_directory=${nola_command%/*}
            [ "$nola_command_directory" != "$nola_command" ] || nola_command_directory=.
            printf '%s/%s\n' "$(cd "$nola_command_directory" && pwd -P)" "${nola_command##*/}"
            ;;
    esac
)

# Bound compiler parallelism without changing profiles, target identity or caches.
# Budget four GiB per Cargo job, capped at four; CLI -j still wins in Cargo.
nola_rust_jobs_for_resources() (
    nola_cpus=${1:-2}
    nola_memory=${2:-0}
    case "$nola_cpus" in ''|*[!0-9]*) nola_cpus=2 ;; esac
    case "$nola_memory" in ''|*[!0-9]*) nola_memory=0 ;; esac
    nola_jobs=4
    [ "$nola_cpus" -ge "$nola_jobs" ] || nola_jobs=$nola_cpus
    if [ "$nola_memory" -gt 0 ]; then
        nola_memory_jobs=$((nola_memory / 4294967296))
        [ "$nola_memory_jobs" -ge "$nola_jobs" ] || nola_jobs=$nola_memory_jobs
    else
        [ "$nola_jobs" -le 2 ] || nola_jobs=2
    fi
    [ "$nola_jobs" -ge 1 ] || nola_jobs=1
    printf '%s\n' "$nola_jobs"
)

nola_rust_build_jobs_init() {
    # Even an explicit empty value belongs to the caller; Cargo validates it.
    [ "${CARGO_BUILD_JOBS+x}" != x ] || return 0
    case "${NOLA_RUST_BUILD_JOBS:-auto}" in
        cargo) return 0 ;;
        auto)
            nola_build_cpus=2
            nola_build_memory=0
            case "$(uname -s 2>/dev/null || :)" in
                Darwin)
                    nola_build_cpus=$(sysctl -n hw.logicalcpu 2>/dev/null || printf 2)
                    nola_build_memory=$(sysctl -n hw.memsize 2>/dev/null || printf 0)
                    ;;
                Linux)
                    nola_build_cpus=$(getconf _NPROCESSORS_ONLN 2>/dev/null || printf 2)
                    nola_build_memory=$(awk '/^MemTotal:/ {printf "%.0f", $2 * 1024}' /proc/meminfo 2>/dev/null || printf 0)
                    # Container memory limits take precedence over host RAM.
                    for nola_limit_file in /sys/fs/cgroup/memory.max /sys/fs/cgroup/memory/memory.limit_in_bytes; do
                        [ -r "$nola_limit_file" ] || continue
                        IFS= read -r nola_memory_limit < "$nola_limit_file" || continue
                        case "$nola_memory_limit" in ''|*[!0-9]*) continue ;; esac
                        if [ "$nola_memory_limit" -gt 0 ] && { [ "${nola_build_memory:-0}" -eq 0 ] || [ "$nola_memory_limit" -lt "$nola_build_memory" ]; }; then
                            nola_build_memory=$nola_memory_limit
                        fi
                    done
                    ;;
            esac
            CARGO_BUILD_JOBS=$(nola_rust_jobs_for_resources "$nola_build_cpus" "$nola_build_memory")
            ;;
        ''|*[!0-9]*|0*)
            printf '%s\n' 'error: NOLA_RUST_BUILD_JOBS must be auto, cargo or a positive integer.' >&2
            return 2 ;;
        *) CARGO_BUILD_JOBS=$NOLA_RUST_BUILD_JOBS ;;
    esac
    export CARGO_BUILD_JOBS
}

nola_rust_build_cache_init() {
    NOLA_RUST_CARGO_COMMAND=$(nola_rust_command_path "${1:-${CARGO:-cargo}}") || {
        printf '%s\n' 'error: Cargo executable unavailable; set CARGO to an executable path.' >&2
        return 127
    }
    nola_rust_build_jobs_init || return $?
    NOLA_RUST_CACHE_BACKEND=cargo
    NOLA_RUST_CACHE_MBX=
    NOLA_RUST_CACHE_SCCACHE=
    NOLA_RUST_CACHE_MODE=${NOLA_RUST_BUILD_CACHE:-auto}
    case "$NOLA_RUST_CACHE_MODE" in
        auto|off|mbx|required|sccache) ;;
        *) printf 'error: invalid NOLA_RUST_BUILD_CACHE=%s (auto, off, mbx, required, sccache).\n' "$NOLA_RUST_CACHE_MODE" >&2; return 2 ;;
    esac
    if [ "$NOLA_RUST_CACHE_MODE" = off ]; then
        printf '%s\n' 'Nola Rust cache: direct Cargo (repository cache integration disabled).' >&2
        return 0
    fi
    # An explicit empty wrapper is meaningful to Cargo too. Do not replace it
    # or layer another cache over a caller-selected compiler wrapper.
    if [ "${RUSTC_WRAPPER+x}" = x ] || [ "${RUSTC_WORKSPACE_WRAPPER+x}" = x ]; then
        if [ "$NOLA_RUST_CACHE_MODE" != auto ]; then
            printf '%s\n' 'error: required Rust cache conflicts with an explicitly configured Rust compiler wrapper.' >&2
            return 2
        fi
        printf '%s\n' 'Nola Rust cache: direct Cargo (preserving explicit Rust compiler wrapper).' >&2
        return 0
    fi
    # staticlib is cacheable by sccache, but not by mbx. This hint only
    # changes auto selection for the native archive build entry points.
    if [ "$NOLA_RUST_CACHE_MODE" = sccache ] || { [ "$NOLA_RUST_CACHE_MODE" = auto ] && [ "${2:-}" = staticlib ]; }; then
        if [ "${NOLA_SCCACHE_BIN+x}" = x ]; then
            NOLA_RUST_CACHE_SCCACHE=$(nola_rust_command_path "$NOLA_SCCACHE_BIN") || NOLA_RUST_CACHE_SCCACHE=
        else
            NOLA_RUST_CACHE_SCCACHE=$(nola_rust_command_path sccache) || NOLA_RUST_CACHE_SCCACHE=
            if [ -z "$NOLA_RUST_CACHE_SCCACHE" ] && [ -n "${HOME:-}" ]; then
                for nola_candidate in "$HOME/.local/bin/sccache" "$HOME/.cargo/bin/sccache"; do
                    if [ -f "$nola_candidate" ] && [ -x "$nola_candidate" ]; then
                        NOLA_RUST_CACHE_SCCACHE=$nola_candidate
                        break
                    fi
                done
            fi
        fi
        if [ -n "$NOLA_RUST_CACHE_SCCACHE" ]; then
            NOLA_RUST_CACHE_BACKEND=sccache
            printf 'Nola Rust cache: sccache selected (%s); static archives use a compiler wrapper, without mbx stacking.\n' "$NOLA_RUST_CACHE_SCCACHE" >&2
            return 0
        elif [ "$NOLA_RUST_CACHE_MODE" = sccache ]; then
            printf '%s\n' 'error: required sccache is unavailable; install it or set NOLA_SCCACHE_BIN.' >&2
            return 127
        fi
    fi
    if [ "${NOLA_MBX_BIN+x}" = x ]; then
        NOLA_RUST_CACHE_MBX=$(nola_rust_command_path "$NOLA_MBX_BIN") || NOLA_RUST_CACHE_MBX=
    else
        NOLA_RUST_CACHE_MBX=$(nola_rust_command_path mbx) || NOLA_RUST_CACHE_MBX=
        # Xcode does not normally inherit an interactive shell's local PATH.
        # These are the official release/Cargo installation locations.
        if [ -z "$NOLA_RUST_CACHE_MBX" ] && [ -n "${HOME:-}" ]; then
            for nola_candidate in "$HOME/.local/bin/mbx" "$HOME/.cargo/bin/mbx"; do
                if [ -f "$nola_candidate" ] && [ -x "$nola_candidate" ]; then
                    NOLA_RUST_CACHE_MBX=$nola_candidate
                    break
                fi
            done
        fi
    fi
    if [ -z "$NOLA_RUST_CACHE_MBX" ]; then
        if [ "$NOLA_RUST_CACHE_MODE" != auto ]; then
            printf '%s\n' 'error: required mbx is unavailable; install it or set NOLA_MBX_BIN.' >&2
            return 127
        fi
        printf '%s\n' 'Nola Rust cache: direct Cargo (mbx unavailable; set NOLA_MBX_BIN to select it).' >&2
        return 0
    fi
    NOLA_RUST_CACHE_BACKEND=mbx
    # Cargo incremental=false alone does not disable mbx's learned private
    # incremental state. Keep the default path shared and bounded; developers
    # can explicitly opt into local incremental work via MBX_* overrides.
    export MBX_LEARNED_INCREMENTAL=${MBX_LEARNED_INCREMENTAL:-false}
    export MBX_EAGER_INCREMENTAL=${MBX_EAGER_INCREMENTAL:-false}
    printf 'Nola Rust cache: mbx wrapper selected (%s); eligible compiler work shares its configured cache.\n' "$NOLA_RUST_CACHE_MBX" >&2
}

nola_rust_cache_wraps_command() (
    case "${1:-}" in +*) shift ;; esac
    nola_subcommand=${1:-}
    for nola_argument in "$@"; do
        case "$nola_argument" in
            --) break ;;
            -h|--help|-V|--version) exit 1 ;;
        esac
    done
    case "$nola_subcommand" in
        build|check|test|run|bench|clippy|rustc|rustdoc|doc|fix) exit 0 ;;
        ndk)
            # cargo-ndk performs the cross-build; inspection still uses Cargo.
            for nola_argument in "$@"; do
                case "$nola_argument" in
                    --) break ;;
                    build|check|test|clippy|rustc) exit 0 ;;
                    metadata) exit 1 ;;
                esac
            done
            ;;
    esac
    exit 1
)

# mbx owns and removes its random session directory under TMPDIR. Its Unix
# socket adds a fixed filename/prefix; long XCTest temporary roots can exceed
# macOS sockaddr_un's limit before Cargo starts. Only that mbx child uses the
# short system temp root. Keep caller/direct Cargo environments unchanged.
nola_rust_cache_needs_short_tmpdir() (
    LC_ALL=C
    export LC_ALL
    [ "${#TMPDIR}" -gt 60 ]
)

nola_rust_target_storage_warning() {
    [ "${NOLA_RUST_TARGET_SIZE_CHECKED:-0}" = 0 ] || return 0
    nola_rust_cache_wraps_command "$@" || return 0
    NOLA_RUST_TARGET_SIZE_CHECKED=1
    # The mbx target budget cannot evict its most-recent target. Surface that
    # blind spot without silently deleting the developer's current binaries.
    case "${CARGO_TARGET_DIR:-}" in /*) ;; *) return 0 ;; esac
    [ -d "$CARGO_TARGET_DIR" ] || return 0
    command -v du >/dev/null 2>&1 || return 0
    nola_target_size=$(du -sk "$CARGO_TARGET_DIR/" 2>/dev/null) || return 0
    nola_target_size=${nola_target_size%%[!0-9]*}
    [ -n "$nola_target_size" ] || return 0
    if [ "$nola_target_size" -gt 25165824 ]; then
        printf '%s\n' 'warning: Rust target exceeds 24 GiB; mbx may retain the entire newest target. Preview unused integration-test runner cleanup with: python3 scripts/maintain_build_storage.py' >&2
    fi
}

nola_cargo() {
    nola_rust_target_storage_warning "$@"
    if [ "$NOLA_RUST_CACHE_BACKEND" = mbx ] && nola_rust_cache_wraps_command "$@"; then
        if [ -n "${TMPDIR:-}" ] && nola_rust_cache_needs_short_tmpdir; then
            TMPDIR=/tmp CARGO="$NOLA_RUST_CARGO_COMMAND" "$NOLA_RUST_CACHE_MBX" "$@"
        else
            CARGO="$NOLA_RUST_CARGO_COMMAND" "$NOLA_RUST_CACHE_MBX" "$@"
        fi
    elif [ "$NOLA_RUST_CACHE_BACKEND" = sccache ] && nola_rust_cache_wraps_command "$@"; then
        RUSTC_WRAPPER="$NOLA_RUST_CACHE_SCCACHE" SCCACHE_CACHE_SIZE="${SCCACHE_CACHE_SIZE:-4G}" "$NOLA_RUST_CARGO_COMMAND" "$@"
    else
        "$NOLA_RUST_CARGO_COMMAND" "$@"
    fi
}

nola_cargo_exec() {
    nola_rust_target_storage_warning "$@"
    if [ "$NOLA_RUST_CACHE_BACKEND" = mbx ] && nola_rust_cache_wraps_command "$@"; then
        if [ -n "${TMPDIR:-}" ] && nola_rust_cache_needs_short_tmpdir; then
            TMPDIR=/tmp CARGO="$NOLA_RUST_CARGO_COMMAND" exec "$NOLA_RUST_CACHE_MBX" "$@"
        else
            CARGO="$NOLA_RUST_CARGO_COMMAND" exec "$NOLA_RUST_CACHE_MBX" "$@"
        fi
    elif [ "$NOLA_RUST_CACHE_BACKEND" = sccache ] && nola_rust_cache_wraps_command "$@"; then
        RUSTC_WRAPPER="$NOLA_RUST_CACHE_SCCACHE" SCCACHE_CACHE_SIZE="${SCCACHE_CACHE_SIZE:-4G}" exec "$NOLA_RUST_CARGO_COMMAND" "$@"
    else
        exec "$NOLA_RUST_CARGO_COMMAND" "$@"
    fi
}

# Only invoke in a newly started shell whose PID is the private job's process
# group leader. Verify that fact before starting Cargo or a wrapper. This is
# used by the Xcode phase; it never selects or scans other process groups.
nola_rust_process_group() {
    # macOS ps is privileged and cannot execute in Xcode's script sandbox.
    # The system Perl interpreter invokes getpgrp/getpgid without privileges;
    # query only the current group (0) or a concrete owned child's PID.
    /usr/bin/perl -e '
        @ARGV == 1 && $ARGV[0] =~ /\A(?:0|[1-9][0-9]*)\z/ or exit 2;
        my $group = getpgrp(0 + $ARGV[0]);
        defined($group) && $group > 0 or exit 2;
        print "$group\n";
    ' "${1:-}"
}

nola_cargo_exec_owned_group() {
    nola_owned_group=$(nola_rust_process_group 0) || {
        printf '%s\n' 'error: cannot verify the private Rust build process group.' >&2
        return 2
    }
    if [ "$nola_owned_group" != "$$" ]; then
        printf '%s\n' 'error: Rust build process is not its private group leader; refusing to start Cargo.' >&2
        return 2
    fi
    nola_cargo_exec "$@"
}
