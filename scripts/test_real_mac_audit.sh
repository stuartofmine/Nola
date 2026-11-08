#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real model -> Mac runtime -> five findings -> native table -> natural follow-up.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
AUDIT_WORKSPACE="$(mktemp -d /tmp/nola-mac-audit-acceptance.XXXXXX)"
cp "$ROOT/scripts/fixtures/mac-audit/design.md" "$AUDIT_WORKSPACE/design.md"
cp "$ROOT/scripts/fixtures/mac-audit/service.py" "$AUDIT_WORKSPACE/service.py"
printf 'Preserved audit acceptance workspace: %s\n' "$AUDIT_WORKSPACE"

NOLA_MAC_REAL_WORKSPACE="$AUDIT_WORKSPACE" \
NOLA_MAC_REAL_PRESERVE_WORKSPACE=1 \
NOLA_MAC_REAL_SCREENSHOT="$AUDIT_WORKSPACE/audit.png" \
NOLA_MAC_REAL_MIN_RENDERED_TABLES=1 \
NOLA_MAC_REAL_MAX_COMPLETED_TOOLS=8 \
NOLA_MAC_REAL_PROMPT='只读审计工作目录的 service.py，对照 design.md，找出全部五项违反设计的行为。不要修改文件，不要运行测试。用中文 Markdown 表格列出五项问题，列为函数、问题、修复建议。最后写 审计完成。' \
NOLA_MAC_REAL_EXPECTED_EXACT='' \
NOLA_MAC_REAL_EXPECTED_CONTAINS='审计完成|||average|||can_edit|||page|||safe_label|||record_error' \
NOLA_MAC_REAL_FOLLOW_UP_PROMPT='第3项会带来什么影响？直接根据刚才的审计用中文回答，不必重新读文件。' \
NOLA_MAC_REAL_FOLLOW_UP_MAX_TOOLS=0 \
bash "$ROOT/scripts/test_real_mac_agent_host.sh"

# Also check read-only behavior independently of the model's final claim.
cmp "$ROOT/scripts/fixtures/mac-audit/design.md" "$AUDIT_WORKSPACE/design.md"
cmp "$ROOT/scripts/fixtures/mac-audit/service.py" "$AUDIT_WORKSPACE/service.py"
printf 'Inspect %s/audit.png before claiming visual acceptance.\n' "$AUDIT_WORKSPACE"
