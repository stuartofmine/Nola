#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Core contract gate with a structured evidence receipt.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
(cd "$ROOT/core" && cargo test --quiet)
python3 - "$ROOT" <<'PY'
from pathlib import Path
import sys

root = Path(sys.argv[1])
sys.path.insert(0, str(root / "scripts"))
from real_e2e_evidence import record_step

record_step(
    "core-contract-suite",
    platforms=["core"],
    modules=["core"],
    boundaries=["core"],
    real_provider_requests=0,
    assertions={"cargo_contract_tests_passed": True},
)
PY
