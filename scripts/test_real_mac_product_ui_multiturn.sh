#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real-provider Mac product UI chain: web evidence -> Word Skill -> persistence.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

NOLA_MAC_REAL_ONLY_TESTING='NolaMacTests/MacRealProductUIEndToEndTests/testRealTransformerResearchCppAndWordWorkflowInProductUI' \
NOLA_MAC_REAL_EXPECTED_TEST_COUNT=1 \
NOLA_MAC_REAL_EVIDENCE_PROFILE='product_ui' \
NOLA_MAC_REAL_MEMORY_PRESERVE_ARTIFACTS="${NOLA_MAC_REAL_MEMORY_PRESERVE_ARTIFACTS:-1}" \
NOLA_MAC_REAL_EVIDENCE_STEP='mac-product-ui-transformer-cpp-word' \
NOLA_MAC_REAL_EVIDENCE_MODULES='chat,agent,core,provider,web-search,skills,tools,permissions,persistence,rendering,ui' \
NOLA_MAC_REAL_EVIDENCE_BOUNDARIES='mac-ui,mac-client-runtime,agent-host,provider,tool-runtime,persistence' \
exec "$ROOT/scripts/run_real_mac_xctests.sh"
