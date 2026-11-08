#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Runs a real Agent Host request through the production iOS chat UI.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# The simulator shares this Mac's loopback interface. LAN discovery is needed
# by the shared Android runner, not by this simulator-only entry point.
NOLA_REAL_WEB_AGENT_HOST_IP="${NOLA_REAL_WEB_AGENT_HOST_IP:-127.0.0.1}" \
NOLA_REAL_IOS_UI_E2E=1 \
NOLA_REAL_IOS_UI_ONLY=1 \
NOLA_SKIP_ANDROID=1 \
exec "$ROOT/scripts/test_real_mobile_agent_host.sh"
