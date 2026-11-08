#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real upstream truncation, continuation, canonical commit, and original recall.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export NOLA_MAC_REAL_WIRE_PROTOCOL=responses
export NOLA_MAC_REAL_COMPACTION_TRUNCATE_TOKENS="${NOLA_MAC_REAL_COMPACTION_TRUNCATE_TOKENS:-192}"
exec bash "$ROOT/scripts/test_real_mac_compaction.sh"
