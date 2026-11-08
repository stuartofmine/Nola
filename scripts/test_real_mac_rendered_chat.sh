#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Runs the real-model macOS chat path through the production bubble renderer.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NOLA_MAC_REAL_ONLY_TESTING="NolaMacTests/MacAgentCapabilityEndToEndTests/testRealConfiguredModelRendersProductionChatBubbleWithToolAndMarkdown" \
NOLA_MAC_REAL_EXPECTED_TEST_COUNT=1 \
NOLA_MAC_REAL_EVIDENCE_PROFILE='product_ui' \
exec "$ROOT/scripts/run_real_mac_xctests.sh"
