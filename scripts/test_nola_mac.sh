#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# One entry point, no parallel smoke-model implementation or CLI success guesses.
set -euo pipefail

NOLA_MAC_ACCEPTANCE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [[ $# -gt 1 ]]; then
    printf 'Usage: %s [real|automation|contracts|host-e2e]\n' "$0" >&2
    exit 64
fi
case "${1:-real}" in
    real)
        exec bash "$NOLA_MAC_ACCEPTANCE_ROOT/scripts/test_real_mac_agent_host.sh"
        ;;
    automation)
        : "${NOLA_MAC_RUNTIME_ACCEPTANCE_WEB_SOURCE_URL:?Set an explicit fixture web-source URL}"
        : "${NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_SOURCE_TOKEN:?Set the expected fixture source token}"
        export NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_CHAIN=1
        export NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_OUTPUT=automation-chain-result.json
        exec bash "$NOLA_MAC_ACCEPTANCE_ROOT/scripts/test_real_mac_agent_host.sh"
        ;;
    contracts|host-e2e)
        exec bash "$NOLA_MAC_ACCEPTANCE_ROOT/scripts/test_mac_layers.sh" "$1"
        ;;
    *)
        printf 'Usage: %s [real|automation|contracts|host-e2e]; unknown mode rejected.\n' "$0" >&2
        exit 64
        ;;
esac
