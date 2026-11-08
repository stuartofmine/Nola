#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real native Host policy ACKs in both directions, isolated conversation owners.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

NOLA_MAC_REAL_ONLY_TESTING='NolaMacTests/MacRealProductUIEndToEndTests/testRealActivePermissionPolicySwitchesBothWaysWithoutCrossingConversationOwners' \
NOLA_MAC_REAL_EXPECTED_TEST_COUNT=1 \
NOLA_MAC_REAL_REQUIRE_MODEL_FIXTURE=1 \
NOLA_MAC_REAL_EVIDENCE_PROFILE=product_ui \
NOLA_MAC_REAL_MEMORY_PRESERVE_ARTIFACTS=1 \
NOLA_MAC_REAL_EVIDENCE_STEP=mac-active-permission-policy-bidirectional \
NOLA_MAC_REAL_EVIDENCE_MODULES='chat,permissions,configuration,agent,provider,persistence,ui' \
NOLA_MAC_REAL_EVIDENCE_BOUNDARIES='mac-ui,mac-client-runtime,agent-host,provider,persistence' \
exec "$ROOT/scripts/run_real_mac_xctests.sh"
