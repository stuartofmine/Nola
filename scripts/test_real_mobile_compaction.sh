#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real-provider iOS and Android automatic Agent Host compaction acceptance.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

NOLA_REAL_MOBILE_COMPACTION_E2E=1 \
exec "$ROOT/scripts/test_real_mobile_agent_host.sh"
