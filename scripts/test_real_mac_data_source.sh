#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real-provider macOS validated-data-source grounding acceptance.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

NOLA_MAC_REAL_ONLY_TESTING='NolaMacTests/MacWebSourceE2ETests/testRealConfiguredModelAnswersFromValidatedWebSourceThroughMacAgentHost' \
NOLA_MAC_REAL_EXPECTED_TEST_COUNT=1 \
NOLA_MAC_REAL_EVIDENCE_STEP='validated-source-grounding' \
NOLA_MAC_REAL_EVIDENCE_MODULES='chat,agent,core,provider,data-source,persistence,audit' \
NOLA_MAC_REAL_EVIDENCE_BOUNDARIES='mac-client-runtime,agent-host,provider,persistence' \
exec "$ROOT/scripts/run_real_mac_xctests.sh"
