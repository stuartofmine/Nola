#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Auditable real-provider acceptance for the interactive CLI PTY.

set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/e2e/cargo_paths.sh"
if [[ "${NOLA_SKIP_CLI_BUILD:-0}" != "1" ]]; then
    cargo build --release --manifest-path "$ROOT/apps/cli/Cargo.toml" \
        -p nola_cli --bin nola --example terminal_acceptance_oracle \
        -p nola_service --bin nola-service
fi
NOLA_BIN="${NOLA_CLI_BIN:-$(nola_cargo_binary "$ROOT" release nola)}"
ORACLE_BIN="${NOLA_TERMINAL_ORACLE_BIN:-$(nola_cargo_example "$ROOT" release terminal_acceptance_oracle "$ROOT/apps/cli/Cargo.toml")}"
[[ -x "$NOLA_BIN" && -x "$ORACLE_BIN" ]] || { printf 'Missing current CLI or terminal oracle executable.\n' >&2; exit 2; }
export NOLA_TERMINAL_ORACLE_BIN="$ORACLE_BIN"
exec python3 "$ROOT/scripts/test_real_cli_terminal.py" "$NOLA_BIN" "$ORACLE_BIN"
