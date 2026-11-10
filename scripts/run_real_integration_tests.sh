#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# File Overview:
# - Purpose: Automates operational steps for run real integration tests in the project module.
# - Interfaces:
#   - create_venv_if_missing
#   - requirements_hash
#   - retry
#   - install_requirements
#   - ensure_test_tooling
# - APIs:
#   - none
#
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BACKEND_DIR="$ROOT/legacy/backend"
BACKEND_PY="$BACKEND_DIR/.venv/bin/python"
PIP_INDEX_URL="${PIP_INDEX_URL:-https://mirrors.aliyun.com/pypi/simple/}"
PIP_EXTRA_INDEX_URL="${PIP_EXTRA_INDEX_URL:-}"
PIP_COMMON_ARGS=(--quiet --retries 8 --timeout 120 --disable-pip-version-check)

create_venv_if_missing() {
  local py_path="$1"
  local dir_path="$2"
  if [ ! -x "$py_path" ]; then
    python3 -m venv "$dir_path/.venv"
  fi
}

requirements_hash() {
  local req_file="$1"
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$req_file" | awk '{print $1}'
  else
    python3 - <<'PY' "$req_file"
import hashlib, pathlib, sys
p = pathlib.Path(sys.argv[1])
print(hashlib.sha256(p.read_bytes()).hexdigest())
PY
  fi
}

retry() {
  local max_attempts="$1"
  shift
  local attempt=1
  while true; do
    if "$@"; then
      return 0
    fi
    if [ "$attempt" -ge "$max_attempts" ]; then
      return 1
    fi
    echo "[real-tests] command failed (attempt $attempt/$max_attempts), retrying..."
    attempt=$((attempt + 1))
    sleep 2
  done
}

install_requirements() {
  local py_path="$1"
  local req_file="$2"
  local stamp_file="${req_file}.installed.sha256"
  local current_hash
  current_hash="$(requirements_hash "$req_file")"
  local installed_hash=""
  if [ -f "$stamp_file" ]; then
    installed_hash="$(cat "$stamp_file" 2>/dev/null || true)"
  fi
  if [ "$installed_hash" = "$current_hash" ]; then
    echo "[real-tests] deps already up-to-date for $req_file"
    return 0
  fi

  export PIP_INDEX_URL
  if [ -n "$PIP_EXTRA_INDEX_URL" ]; then
    export PIP_EXTRA_INDEX_URL
  fi

  if ! retry 3 "$py_path" -m pip install "${PIP_COMMON_ARGS[@]}" --upgrade pip; then
    PIP_INDEX_URL="https://pypi.org/simple"
    export PIP_INDEX_URL
    retry 2 "$py_path" -m pip install "${PIP_COMMON_ARGS[@]}" --upgrade pip
  fi
  if ! retry 3 "$py_path" -m pip install "${PIP_COMMON_ARGS[@]}" -r "$req_file"; then
    PIP_INDEX_URL="https://pypi.org/simple"
    export PIP_INDEX_URL
    retry 2 "$py_path" -m pip install "${PIP_COMMON_ARGS[@]}" -r "$req_file"
  fi
  printf '%s' "$current_hash" > "$stamp_file"
}

ensure_test_tooling() {
  local py_path="$1"
  retry 3 "$py_path" -m pip install "${PIP_COMMON_ARGS[@]}" pytest
}

create_venv_if_missing "$BACKEND_PY" "$BACKEND_DIR"
install_requirements "$BACKEND_PY" "$BACKEND_DIR/requirements.txt"
ensure_test_tooling "$BACKEND_PY"

echo "[real-tests] backend integration"
cd "$ROOT"
if find backend/tests_integration -name 'test_*.py' -print -quit | grep -q .; then
  "$BACKEND_PY" -m pytest backend/tests_integration -q
else
  echo "[real-tests] no backend integration tests found; skipping"
fi

echo "[real-tests] cross-platform real-model smoke matrix"
python3 "$ROOT/scripts/run_real_system_tests.py" --suite smoke

echo "[real-tests] all real integration chains passed"
