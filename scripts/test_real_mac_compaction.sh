#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real-provider macOS Agent Host compaction and continuation acceptance.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Keep the trigger below the hard window: compression itself needs headroom.
export NOLA_REAL_COMPACTION_WINDOW="${NOLA_REAL_COMPACTION_WINDOW:-64000}"
export NOLA_AGENT_CONTEXT_TOKEN_THRESHOLD="${NOLA_AGENT_CONTEXT_TOKEN_THRESHOLD:-30000}"
export NOLA_MAC_REAL_REQUIRE_CONTEXT_BUDGET=1

NOLA_MAC_REAL_ONLY_TESTING='NolaMacTests/MacAgentCapabilityEndToEndTests/testRealConfiguredModelCompactsAndRecallsMacAgentHostConversation' \
NOLA_MAC_REAL_EXPECTED_TEST_COUNT=1 \
NOLA_MAC_REAL_EVIDENCE_STEP='compaction-and-recall' \
NOLA_MAC_REAL_EVIDENCE_MODULES='chat,agent,core,provider,compaction,memory,persistence' \
NOLA_MAC_REAL_EVIDENCE_BOUNDARIES='mac-client-runtime,agent-host,provider,persistence' \
exec "$ROOT/scripts/run_real_mac_xctests.sh"
