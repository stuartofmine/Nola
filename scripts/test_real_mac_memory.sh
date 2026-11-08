#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real-provider macOS cross-conversation memory product-path acceptance.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

NOLA_MAC_REAL_ONLY_TESTING='NolaMacTests/MacAgentCapabilityEndToEndTests/testRealConfiguredModelPersistsAndRecallsMemoryAcrossMacAgentHostConversations' \
NOLA_MAC_REAL_EXPECTED_TEST_COUNT=1 \
NOLA_MAC_REAL_EVIDENCE_STEP='cross-conversation-memory' \
NOLA_MAC_REAL_EVIDENCE_MODULES='chat,agent,core,provider,memory,persistence' \
NOLA_MAC_REAL_EVIDENCE_BOUNDARIES='mac-client-runtime,agent-host,provider,persistence' \
exec "$ROOT/scripts/run_real_mac_xctests.sh"
