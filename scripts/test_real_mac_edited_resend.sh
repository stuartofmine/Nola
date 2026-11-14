#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real product UI: cancel/edit/resend, same-conversation follow-up, cold restore and long streaming.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT/scripts/lib/xcode_test_device_guard.sh"
nola_xctest_device_guard_begin "$ROOT" macos
nola_xctest_device_guard_install_traps "$ROOT" macos

export NOLA_MAC_REAL_ONLY_TESTING='NolaMacTests/MacRealProductUIEndToEndTests/testRealOrdinaryMessageEditResendFollowupAndColdRestore|||NolaMacTests/MacRealProductUIEndToEndTests/testRealStreamingSurvivesConversationSwitchAndKeepsLatestVisible'
export NOLA_MAC_REAL_EXPECTED_TEST_COUNT=2
export NOLA_MAC_REAL_MIN_SUCCESSFUL_RUNS=5
export NOLA_MAC_REAL_EVIDENCE_PROFILE=product_ui
export NOLA_MAC_REAL_MEMORY_PRESERVE_ARTIFACTS=1
export NOLA_MAC_REAL_CAPTURE_TEST_CONTEXT=1
export NOLA_MAC_REAL_XCTEST_TIMEOUT_SECONDS=900
export NOLA_MAC_REAL_EVIDENCE_STEP=mac-real-edited-resend-and-streaming
export NOLA_MAC_REAL_EVIDENCE_MODULES=chat,editing,streaming,core,provider,persistence,ui
export NOLA_MAC_REAL_EVIDENCE_BOUNDARIES=mac-ui,mac-client-runtime,agent-host,provider,persistence
# Keep this shell alive so its EXIT trap owns the XCTest guard through cleanup.
"$ROOT/scripts/run_real_mac_xctests.sh"
