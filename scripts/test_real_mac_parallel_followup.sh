#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real-model regression for heterogeneous parallel tools and turn isolation.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export NOLA_MAC_REAL_PROMPT='先获取当前时间，再用两次互补的网页搜索查找近期仍可报名的 AI 比赛。只列出工具返回的网页正文中同时明确证明目前仍开放、且出现完整报名截止年月日并晚于当前时刻的项目，并按比赛名称、报名截止时间、主办方和来源链接回答。不得从搜索摘要、截断文本或常识补全日期；截止时间缺失、正文被截断、当前状态未证实或已经截止的项目必须排除且不要在答案中列出。若没有合格结果，只说明未找到可验证的结果。工具可以并行时请并行调用。'
export NOLA_MAC_REAL_EXPECTED_EXACT=''
export NOLA_MAC_REAL_EXPECTED_CONTAINS=''
export NOLA_MAC_REAL_FORBIDDEN_CONTAINS=''
export NOLA_MAC_REAL_REQUIRE_TOOL=1
export NOLA_MAC_REAL_EXPECTED_TOOLS='system_time,web_search'
export NOLA_MAC_REAL_MIN_VISIBLE_TOOLS=3
export NOLA_MAC_REAL_MAX_TOOL_ERRORS=0
export NOLA_MAC_REAL_MAX_COMPLETED_TOOLS=8
export NOLA_MAC_REAL_WAIT_TICKS=1800
export NOLA_MAC_REAL_FOLLOW_UP_PROMPT='你好'
export NOLA_MAC_REAL_FOLLOW_UP_EXPECTED=''
export NOLA_MAC_REAL_FOLLOW_UP_EXPECTED_REGEX='^(你好|您好)'
export NOLA_MAC_REAL_FOLLOW_UP_FORBIDDEN_CONTAINS='比赛|||报名|||截止|||搜索'
export NOLA_MAC_REAL_FOLLOW_UP_MAX_TOOLS=0
export NOLA_MAC_REAL_REQUIRE_MULTITOOL_CONTRACT=1
export NOLA_MAC_REAL_JUDGE=1

exec "$ROOT/scripts/test_real_mac_agent_host.sh"
