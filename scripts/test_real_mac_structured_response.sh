#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real-provider Mac acceptance for a multi-tool, structured Markdown response.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCREENSHOT="${NOLA_MAC_STRUCTURED_SCREENSHOT:-/tmp/nola-mac-structured-response.png}"

NOLA_MAC_REAL_PROMPT='Read architecture.md, sample.swift, and note.txt using file tools. Then return a concise Chinese review directly in the assistant response with these exact visible sections: ## 架构结论, ## 风险清单, ## 示例代码. Include one Markdown table under 架构结论, three numbered risks, and a fenced swift code block under 示例代码. Mention StableStream and NOLA_MAC_TOOL_REAL_OK. Do not create or edit any file, and do not repeat any section.' \
NOLA_MAC_REAL_EXPECTED_EXACT='' \
NOLA_MAC_REAL_EXPECTED_CONTAINS='## 架构结论|||## 风险清单|||## 示例代码|||StableStream|||NOLA_MAC_TOOL_REAL_OK' \
NOLA_MAC_REAL_MIN_ASSISTANT_DELTAS=3 \
NOLA_MAC_REAL_MIN_MUTABLE_TAIL_UPDATES=2 \
NOLA_MAC_REAL_MAX_THINKING_DELTAS=24 \
NOLA_MAC_REAL_MIN_MARKDOWN_NODES=3 \
NOLA_MAC_REAL_MIN_CODE_NODES=1 \
NOLA_MAC_REAL_MIN_VISIBLE_TOOLS=1 \
NOLA_MAC_REAL_FOLLOW_UP_PROMPT='This is a new conversational turn. Reply with exactly NOLA_MAC_FOLLOW_UP_OK and do not call any tool.' \
NOLA_MAC_REAL_FOLLOW_UP_EXPECTED='NOLA_MAC_FOLLOW_UP_OK' \
NOLA_MAC_REAL_WAIT_TICKS="${NOLA_MAC_REAL_WAIT_TICKS:-1800}" \
NOLA_MAC_REAL_SCREENSHOT="$SCREENSHOT" \
"$ROOT/scripts/test_real_mac_agent_host.sh"
