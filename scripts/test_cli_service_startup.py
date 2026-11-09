#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Exercise cold cross-process service ownership with the production CLI.

Uses a disposable database and an unreachable model endpoint. No inference,
user data, credentials, injected receipts, or application-command retries.
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

from test_real_cli_long_tasks import apply_config, fixture


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/nola"))
    args = parser.parse_args()
    root = Path(tempfile.mkdtemp(prefix="nola-startup-stress-"))
    root.chmod(0o700)
    (root / "bin").mkdir()
    binary = root / "bin/nola"
    shutil.copy2(args.binary.resolve(), binary)
    project, env, config = fixture(root / "case", "fixture-no-inference", "http://127.0.0.1:9/v1")
    apply_config(binary, project, env, config, root)

    def observe(index):
        result = subprocess.run([str(binary), "--json", "sessions"], cwd=project, env=env,
            capture_output=True, text=True, timeout=30)
        record = {"index": index, "exit_code": result.returncode,
            "stdout": result.stdout, "stderr": result.stderr}
        (root / f"client-{index}.json").write_text(json.dumps(record))
        assert result.returncode == 0, f"client {index} failed: {result.stderr}"
        output = json.loads(result.stdout)
        assert output["type"] == "sessions" and output["sessions"] == []
        return {"index": index, "exit_code": result.returncode}

    receipt = {"passed": False, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "evidence": str(root), "clients": [], "inference_requests": 0}
    try:
        for batch in range(5):
            with ThreadPoolExecutor(max_workers=8) as pool:
                # Collect every result, even after one client failed.
                futures = [pool.submit(observe, index) for index in range(batch * 8, (batch + 1) * 8)]
                for future in futures:
                    try:
                        receipt["clients"].append(future.result())
                    except Exception as error:
                        receipt.setdefault("errors", []).append(str(error))
            # Each launching parent has exited. The next batch races cold
            # election or graceful retirement rather than a permanent daemon.
            time.sleep(0.4)
        receipt["passed"] = len(receipt["clients"]) == 40 and not receipt.get("errors")
    finally:
        (root / "receipt.json").write_text(json.dumps(receipt, indent=2))
    print(json.dumps(receipt), flush=True)
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
