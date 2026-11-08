#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real-provider macOS built-in Skill enable/edit/disable lifecycle acceptance.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

NOLA_MAC_REAL_ONLY_TESTING='NolaMacTests/MacAgentCapabilityEndToEndTests/testRealConfiguredModelObservesEditedAndDisabledMacSkillLifecycle' \
NOLA_MAC_REAL_EXPECTED_TEST_COUNT=1 \
NOLA_MAC_REAL_EVIDENCE_STEP='skill-enable-edit-disable' \
NOLA_MAC_REAL_EVIDENCE_MODULES='chat,agent,core,provider,skills,tools,persistence' \
NOLA_MAC_REAL_EVIDENCE_BOUNDARIES='mac-client-runtime,agent-host,provider,persistence' \
exec "$ROOT/scripts/run_real_mac_xctests.sh"
