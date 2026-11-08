#!/usr/bin/env python3
"""Mac acceptance against the exact dirty-worktree fixture audited by the CLI."""
from __future__ import annotations

import argparse
import json
import os
import pathlib
import subprocess
import tempfile

from test_real_cli_dirty_audit import fingerprint


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("workspace", type=pathlib.Path)
    args = parser.parse_args()
    workspace = args.workspace.resolve()
    assert (workspace / ".git").is_dir(), "Use the isolated CLI audit fixture"
    root = pathlib.Path(__file__).resolve().parents[1]
    assert workspace != root and root not in workspace.parents, "Never run this acceptance in the source repository"
    before = fingerprint(workspace)
    evidence = pathlib.Path(tempfile.mkdtemp(prefix="nola-mac-dirty-audit-"))
    (evidence / "workspace-before.json").write_text(json.dumps(before, sort_keys=True))
    print(f"Preserved Mac audit evidence: {evidence}", flush=True)
    env = dict(os.environ,
        NOLA_MAC_REAL_WORKSPACE=str(workspace),
        NOLA_MAC_REAL_ARTIFACT_ROOT=str(evidence),
        NOLA_MAC_REAL_SEED_FIXTURE="0",
        NOLA_MAC_REAL_PRESERVE_WORKSPACE="1",
        NOLA_MAC_REAL_SCREENSHOT=str(evidence / "audit.png"),
        NOLA_MAC_REAL_MIN_RENDERED_TABLES="1",
        NOLA_MAC_REAL_MAX_COMPLETED_TOOLS="80",
        NOLA_MAC_REAL_MAX_TOOL_ERRORS="3",
        NOLA_MAC_REAL_EXPECTED_EXACT="",
        NOLA_MAC_REAL_EXPECTED_CONTAINS="",
        NOLA_MAC_REAL_PROMPT="只读审计当前 Nola 仓库的未提交变更，重点检查 harness 的工具协议、队列、参数纠正、结果保存和读取，以及 Mac 对话展示。不要修改文件或运行测试。用中文 Markdown 表格给出有代码证据的问题，最后说明审计覆盖范围，不要声称未检查的文件已通过。",
        NOLA_MAC_REAL_FOLLOW_UP_PROMPT="第一个问题会影响什么？直接根据刚才的审计回答，不必重新读文件。",
        NOLA_MAC_REAL_FOLLOW_UP_MAX_TOOLS="0",
        NOLA_MAC_RUNTIME_ACCEPTANCE_SCENARIO_TIMEOUT_SECONDS="480",
        NOLA_MAC_RUNTIME_ACCEPTANCE_FOLLOW_UP_TIMEOUT_SECONDS="120",
        NOLA_MAC_REAL_WAIT_TICKS="6500",
        NOLA_MAC_REAL_JUDGE="1")
    result = subprocess.run(["bash", str(root / "scripts/test_real_mac_agent_host.sh")], env=env)
    after = fingerprint(workspace)
    (evidence / "workspace-after.json").write_text(json.dumps(after, sort_keys=True))
    assert before == after, f"Read-only Mac audit changed its fixture: {evidence}"
    assert result.returncode == 0, f"Mac acceptance failed; inspect {evidence}"
    print(f"Inspect {evidence / 'audit.png'} before declaring visual acceptance", flush=True)


if __name__ == "__main__":
    main()
