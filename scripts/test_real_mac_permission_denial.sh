#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real-provider Mac acceptance for permission suspension, denial, and recovery.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export NOLA_MAC_REAL_WORKSPACE="$(mktemp -d "${TMPDIR:-/tmp}/nola-mac-denial-acceptance.XXXXXX")"
export NOLA_MAC_REAL_ARTIFACT_ROOT="$NOLA_MAC_REAL_WORKSPACE"
printf '[real-mac-denial] isolated workspace: %s\n' "$NOLA_MAC_REAL_WORKSPACE"
runtime_status=0
printf 'OWNED_DENIAL_ORIGINAL\n' >"$NOLA_MAC_REAL_WORKSPACE/permission-probe.txt"

NOLA_MAC_REAL_PROMPT='Read the existing permission-probe.txt in the workspace, then replace its entire content with DENIED_PROBE and one newline. This overwrites an existing file, not a new-file creation. Discover available file capabilities if needed, then attempt the replacement once. If permission is denied, do not retry or use an alternative; briefly report the result.' \
NOLA_MAC_REAL_PERMISSION_MODE=cautious \
NOLA_MAC_REAL_PERMISSION_FIXTURE=permission-probe.txt \
NOLA_MAC_REAL_PERMISSION_ORIGINAL=$'OWNED_DENIAL_ORIGINAL\n' \
NOLA_MAC_REAL_PERMISSION_EXPECTED=$'OWNED_DENIAL_ORIGINAL\n' \
NOLA_MAC_REAL_PERMISSION_DECISION=deny \
NOLA_MAC_REAL_PERMISSION_SCOPE=once \
NOLA_MAC_REAL_EXPECTED_PERMISSION_REQUESTS=1 \
NOLA_MAC_REAL_MAX_TOOL_ERRORS=1 \
NOLA_MAC_REAL_REQUIRE_TOOL=0 \
NOLA_MAC_REAL_EXPECTED_EXACT='' \
NOLA_MAC_REAL_EXPECTED_RESULT=blocked \
NOLA_MAC_REAL_WAIT_TICKS="${NOLA_MAC_REAL_WAIT_TICKS:-1200}" \
"$ROOT/scripts/test_real_mac_agent_host.sh" || runtime_status=$?

if ! cmp -s "$NOLA_MAC_REAL_WORKSPACE/permission-probe.txt" <(printf 'OWNED_DENIAL_ORIGINAL\n'); then
    printf 'Denied overwrite changed the original fixture bytes.\n' >&2
    exit 1
fi
exit "$runtime_status"
