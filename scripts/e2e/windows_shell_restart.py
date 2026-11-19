#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Run the installed Tauri runtime and real local Host on a Windows worker."""
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

    installed = os.environ.get("NOLA_INSTALLED_DESKTOP")
    if not installed or not Path(installed).is_file():
        raise SystemExit("Set NOLA_INSTALLED_DESKTOP to the installed Tauri executable")
    with tempfile.TemporaryDirectory(prefix="nola-shell-evidence-") as temp:
        received = Path(temp) / "restart.json"
        env = dict(os.environ, NOLA_DESKTOP_RESTART_EVIDENCE=str(received))
        directory = ROOT / "apps/windows"
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
                    boundaries=["tauri-webview2", "local-host", "indexeddb"], assertions=checks,
                    assertion_evidence={key: ["restart-result", "draft-audit"] for key in checks},
                    observations=[{"id": "restart-result", "kind": "test_report", "path": received},
                                  {"id": "draft-audit", "kind": "persistence_audit", "path": audit}])


if __name__ == "__main__":
    main()
