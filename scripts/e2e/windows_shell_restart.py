#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Run the installed Electron runtime and real local Host on a Windows worker."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def main():
    if sys.platform != "win32":
        raise SystemExit("Windows desktop acceptance requires a Windows worker")
    sys.path.insert(0, str(ROOT / "scripts"))
    from real_e2e_evidence import record_step

    binary = Path(os.environ.get("NOLA_WINDOWS_CLI_PATH", ROOT / "target/debug/nola.exe")).resolve()
    if not binary.is_file():
        raise SystemExit("Build the current Windows CLI or set NOLA_WINDOWS_CLI_PATH")
    with tempfile.TemporaryDirectory(prefix="nola-shell-evidence-") as temp:
        received = Path(temp) / "restart.json"
        env = dict(os.environ, NOLA_WINDOWS_CLI_PATH=str(binary), NOLA_DESKTOP_RESTART_EVIDENCE=str(received))
        directory = ROOT / "apps/windows"
        subprocess.run(["npm.cmd", "run", "build:web"], cwd=directory, env=env, check=True)
        subprocess.run(["npm.cmd", "run", "build"], cwd=directory, env=env, check=True)
        subprocess.run(["npm.cmd", "run", "test:e2e"], cwd=directory, env=env, check=True)
        result = json.loads(received.read_text())
        checks = {
            "owned_origin_restored": result["platform"] == "win32" and result["beforeOrigin"] == result["afterOrigin"],
            "composer_draft_restored": result["draft"] == "Unsent desktop restart regression",
            "composer_attachment_restored": result["attachment"] == "restart-note.txt",
        }
        if not all(checks.values()):
            raise RuntimeError(f"Desktop restart failed validation: {checks}")
        audit = Path(temp) / "audit.json"
        audit.write_text(json.dumps({"passed": True, "assertions": checks}))
        record_step("desktop-host-restart", platforms=["windows"], modules=["persistence", "ui"],
                    boundaries=["electron-preload", "local-host", "indexeddb"], assertions=checks,
                    assertion_evidence={key: ["restart-result", "draft-audit"] for key in checks},
                    observations=[{"id": "restart-result", "kind": "test_report", "path": received},
                                  {"id": "draft-audit", "kind": "persistence_audit", "path": audit}])


if __name__ == "__main__":
    main()
