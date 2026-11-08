#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Separate deterministic Core/Host and opt-in LAN-model permission evidence.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
case "${1:-fixture}" in
  fixture)
    # The fixture provider chooses precise tool calls; this is not an LLM score.
    . "$ROOT/scripts/lib/xcode_test_device_guard.sh"
    nola_xctest_device_guard_begin "$ROOT" macos
    nola_xctest_device_guard_install_traps "$ROOT" macos
    args=(test -quiet -project "$ROOT/Nola.xcodeproj" -scheme NolaMac -configuration Test
      -destination "platform=macOS,arch=$(uname -m)" -destination-timeout 10
      -only-testing:NolaMacTests/MacAgentCapabilityEndToEndTests/testCautiousConversationGrantReusesOnlyOriginalResourceAndOwner
      CODE_SIGNING_ALLOWED=NO ONLY_ACTIVE_ARCH=YES)
    if [[ -n "${NOLA_MAC_DERIVED_DATA:-}" ]]; then args+=(-derivedDataPath "$NOLA_MAC_DERIVED_DATA"); fi
    /usr/bin/caffeinate -is xcodebuild "${args[@]}"
    ;;
  real)
    : "${NOLA_CONFIG_PATH:?Select an isolated test config pointing to the 192.168 LAN service}"
    python3 - "$NOLA_CONFIG_PATH" <<'PY'
import json, sys
from urllib.parse import urlparse
with open(sys.argv[1], encoding="utf-8") as handle:
    endpoint = json.load(handle).get("llm", {}).get("endpoint", "")
host = urlparse(endpoint).hostname or ""
if not host.startswith("192.168."):
    raise SystemExit("This acceptance requires the requested 192.168 LAN provider; no cloud fallback")
PY
    export NOLA_MAC_REAL_ONLY_TESTING='NolaMacTests/MacAgentCapabilityEndToEndTests/testRealConfiguredModelCautiousGrantReusePreservesResourceAndConversationBoundary'
    export NOLA_MAC_REAL_EXPECTED_TEST_COUNT=1
    export NOLA_MAC_REAL_REQUIRE_MODEL_FIXTURE=1
    export NOLA_MAC_REAL_EVIDENCE_STEP=mac-cautious-permission-reuse
    export NOLA_MAC_REAL_EVIDENCE_MODULES='permissions,agent,provider,persistence'
    export NOLA_MAC_REAL_EVIDENCE_BOUNDARIES='mac-client-runtime,agent-host,provider,persistence'
    # Acceptance evidence must remain inspectable after a passing run.
    export NOLA_MAC_REAL_MEMORY_PRESERVE_ARTIFACTS=1
    exec "$ROOT/scripts/run_real_mac_xctests.sh"
    ;;
  *) printf 'Usage: bash %s [fixture|real]\n' "$0" >&2; exit 64 ;;
esac
