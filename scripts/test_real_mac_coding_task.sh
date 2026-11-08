#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Actual model-driven coding task; independently verify the delivered artifact.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export NOLA_MAC_REAL_WORKSPACE="$(mktemp -d /tmp/nola-mac-coding.XXXXXX)"
printf '[real-mac-coding] isolated workspace: %s\n' "$NOLA_MAC_REAL_WORKSPACE"
runtime_status=0
printf '# Owned acceptance placeholder; replace this existing file.\n' >"$NOLA_MAC_REAL_WORKSPACE/summarize_csv.py"
NOLA_MAC_REAL_PROMPT='读取并替换当前工作目录已经存在的 summarize_csv.py，实现一个无需第三方依赖的 Python 命令行工具：接受一个 CSV 文件路径，按 category 分组，把 amount 按十进制定点数求和，向 stdout 输出 JSON 对象（键为分类，值为保留两位小数的字符串）。支持 Unicode 分类和负数，不要使用二进制浮点求和。自己创建小样例并实际运行验证。交付可执行的代码，而不是仅给代码建议；最后简要说明实现和验证结果。只操作当前工作目录。' \
NOLA_MAC_REAL_PERMISSION_MODE=cautious \
NOLA_MAC_REAL_PERMISSION_DECISION=allow \
NOLA_MAC_REAL_PERMISSION_SCOPE=once \
NOLA_MAC_REAL_EXPECTED_PERMISSION_REQUESTS=-1 \
NOLA_MAC_REAL_MIN_PERMISSION_REQUESTS=1 \
NOLA_MAC_REAL_EXPECTED_EXACT='' \
NOLA_MAC_REAL_EXPECTED_RESULT=action_completed \
"$ROOT/scripts/test_real_mac_agent_host.sh" || runtime_status=$?
python3 - "$NOLA_MAC_REAL_WORKSPACE" <<'PY'
import json
from pathlib import Path
import subprocess
import sys
import tempfile

workspace = Path(sys.argv[1])
script = workspace / "summarize_csv.py"
if not script.is_file():
    raise SystemExit("Coding acceptance failed: missing delivered program")
# Independent input is created only after the model has finished.
with tempfile.TemporaryDirectory(prefix="nola-csv-acceptance-") as directory:
    source = Path(directory) / "transactions.csv"
    source.write_text("category,amount\n餐饮,0.10\n餐饮,0.20\n退款,-7.25\n退款,2.00\n餐饮,1.11\n", encoding="utf-8")
    result = subprocess.run([sys.executable, str(script), str(source)], cwd=workspace,
                            capture_output=True, text=True, timeout=20, check=True)
    if json.loads(result.stdout) != {"餐饮": "1.41", "退款": "-5.25"}:
        raise SystemExit("Coding acceptance failed: incorrect independent totals")
print("[real-mac-coding] independent artifact acceptance passed")
PY
exit "$runtime_status"
