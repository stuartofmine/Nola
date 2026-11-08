#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real Mac permission cards -> write -> read-back verification -> canonical completion.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export NOLA_MAC_REAL_WORKSPACE="$(mktemp -d /tmp/nola-mac-write-acceptance.XXXXXX)"
export NOLA_MAC_REAL_ARTIFACT_ROOT="$NOLA_MAC_REAL_WORKSPACE"
printf '[real-mac-write] isolated workspace: %s\n' "$NOLA_MAC_REAL_WORKSPACE"
runtime_status=0
printf 'OWNED_APPROVAL_ORIGINAL\n' >"$NOLA_MAC_REAL_WORKSPACE/permission-approved.txt"

NOLA_MAC_REAL_PROMPT='读取工作目录中已经存在的 permission-approved.txt，然后将它的内容整体替换为 APPROVED_PROBE 和一个换行；这是覆盖已有文件，不是创建新文件。需要时先发现文件工具。覆盖后必须读回原文件核对内容，验证成功后只回复 APPROVED_PROBE。' \
NOLA_MAC_REAL_PERMISSION_MODE=cautious \
NOLA_MAC_REAL_PERMISSION_FIXTURE=permission-approved.txt \
NOLA_MAC_REAL_PERMISSION_ORIGINAL=$'OWNED_APPROVAL_ORIGINAL\n' \
NOLA_MAC_REAL_PERMISSION_EXPECTED=$'APPROVED_PROBE\n' \
NOLA_MAC_REAL_PERMISSION_DECISION=allow \
NOLA_MAC_REAL_PERMISSION_SCOPE=once \
NOLA_MAC_REAL_EXPECTED_PERMISSION_REQUESTS=1 \
NOLA_MAC_REAL_MIN_PERMISSION_REQUESTS=1 \
NOLA_MAC_REAL_EXPECTED_EXACT=APPROVED_PROBE \
NOLA_MAC_REAL_EXPECTED_RESULT=action_completed \
NOLA_MAC_REAL_MAX_TOOL_ERRORS=0 \
"$ROOT/scripts/test_real_mac_agent_host.sh" || runtime_status=$?

# A successful answer or tool event is not proof that the requested artifact exists.
python3 - "$NOLA_MAC_REAL_WORKSPACE/permission-approved.txt" <<'PY'
from pathlib import Path
import sys

artifact = Path(sys.argv[1])
if not artifact.is_file() or artifact.read_text(encoding="utf-8").strip() != "APPROVED_PROBE":
    raise SystemExit("Permission approval acceptance failed: requested artifact missing or incorrect")
print("[real-mac-write] requested artifact verified")
PY
exit "$runtime_status"
