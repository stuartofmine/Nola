#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Real, opt-in product E2E for installed external execution units.

set -eu

if [ "${NOLA_RUN_REAL_EXTERNAL_EXECUTOR_E2E:-0}" != "1" ]; then
    echo "skip: set NOLA_RUN_REAL_EXTERNAL_EXECUTOR_E2E=1"
    exit 0
fi

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
. "$repo_root/scripts/e2e/cargo_paths.sh"
cd "$repo_root"
cargo build -p nola_service --bin nola-service -p nola_cli --bin nola-agent-host
export NOLA_APPLICATION_SERVICE_EXECUTABLE="${NOLA_APPLICATION_SERVICE_EXECUTABLE:-$(nola_cargo_binary "$repo_root" debug nola-service)}"
host=${NOLA_AGENT_HOST_BIN:-"$(nola_cargo_binary "$repo_root" debug nola-agent-host)"}
if [ ! -x "$host" ]; then
    echo "error: build nola-agent-host first: $host" >&2
    exit 1
fi

catalog=$("$host" --list-execution-backends)

available=$(printf '%s' "$catalog" | jq -r '.[] | select(.id != "nola" and .availability == "available") | .id')
requested=$(printf '%s' "${NOLA_EXTERNAL_EXECUTOR_BACKENDS:-$available}" | tr ',' ' ')
if [ -z "$requested" ]; then
    echo "error: no installed external execution unit" >&2
    exit 1
fi

for backend in $requested; do
    printf '%s\n' "$available" | grep -Fxq "$backend" || { echo "unavailable executor: $backend" >&2; exit 1; }
    case "$backend" in
        codex) model=${NOLA_CODEX_MODEL:-} ;;
        claude_code) model=${NOLA_CLAUDE_CODE_MODEL:-} ;;
        opencode) model=${NOLA_OPENCODE_MODEL:-} ;;
        antigravity) model=${NOLA_ANTIGRAVITY_MODEL:-} ;;
        *) echo "unknown executor: $backend" >&2; exit 1 ;;
    esac
    NOLA_TEST_EXTERNAL_BACKEND="$backend" NOLA_TEST_EXTERNAL_MODEL="$model" \
        cargo test -p nola_cli --test cli_contracts \
        desktop_host_canonical_identity_e2e::real_external_executor_resumes_committed_session -- --exact --ignored
done
