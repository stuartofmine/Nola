#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Real model must deliver a runnable script including its filesystem mode.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export NOLA_MAC_REAL_WORKSPACE="$(mktemp -d "${TMPDIR:-/tmp}/nola-mac-executable.XXXXXX")"
printf '[real-mac-executable] isolated workspace: %s\n' "$NOLA_MAC_REAL_WORKSPACE"
runtime_status=0
printf '# Owned acceptance placeholder; replace this existing file.\n' >"$NOLA_MAC_REAL_WORKSPACE/hello.sh"
NOLA_MAC_REAL_PROMPT='读取并替换当前目录已经存在的 hello.sh，使用 /bin/sh，运行时只输出 NOLA_EXECUTABLE_OK 和换行。然后用单独的一次 bash 工具调用 chmod +x hello.sh，为该调用声明 operation_kind=mutate、output_artifacts=[{"path":"hello.sh","operation":"edit"}]。再直接运行 ./hello.sh 验证结果。交付文件，完成后只用一句话说明结果，不要贴代码。' \
NOLA_MAC_REAL_PERMISSION_MODE=cautious \
NOLA_MAC_REAL_PERMISSION_DECISION=allow \
NOLA_MAC_REAL_PERMISSION_SCOPE=once \
NOLA_MAC_REAL_EXPECTED_PERMISSION_REQUESTS=-1 \
NOLA_MAC_REAL_MIN_PERMISSION_REQUESTS=1 \
NOLA_MAC_REAL_EXPECTED_EXACT='' \
NOLA_MAC_REAL_EXPECTED_RESULT=action_completed \
NOLA_MAC_REAL_MAX_TOOL_ERRORS=0 \
"$ROOT/scripts/test_real_mac_agent_host.sh" || runtime_status=$?
python3 - "$NOLA_MAC_REAL_WORKSPACE" <<'PY'
import json
from pathlib import Path
import subprocess
import sys

workspace = Path(sys.argv[1])
events = json.loads((workspace / "result.json").read_text())["events"]
requests = [json.loads(e["detail"]) for e in events
            if e.get("kind") == "toolStarted" and e.get("tool_name") == "bash"]
if not any(r.get("command", "").strip() in (
               "chmod +x hello.sh", "chmod +x hello.sh && ls -l hello.sh")
           and r.get("operation_kind") == "mutate"
           and r.get("output_artifacts") == [{"path": "hello.sh", "operation": "edit"}]
           for r in requests):
    raise SystemExit("Executable acceptance failed: declared permission-only edit not exercised")
result = subprocess.run([str(workspace / "hello.sh")], cwd=workspace,
                        capture_output=True, text=True, timeout=10, check=True)
if result.stdout != "NOLA_EXECUTABLE_OK\n":
    raise SystemExit("Executable acceptance failed: incorrect program output")
print("[real-mac-executable] direct execution passed")
PY
exit "$runtime_status"
