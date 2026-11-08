#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
BACKEND_DIR="$ROOT/backend"
NPM_BIN="${NPM_BIN:-npm}"
DEPLOY_RUN_TYPECHECK="${DEPLOY_RUN_TYPECHECK:-1}"
DEPLOY_RUN_TESTS="${DEPLOY_RUN_TESTS:-0}"

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
log()  { echo -e "${CYAN}[deploy]${NC} $*"; }
ok()   { echo -e "${GREEN}[ok]${NC}  $*"; }
warn() { echo -e "${YELLOW}[warn]${NC} $*"; }
err()  { echo -e "${RED}[err]${NC}  $*"; }

usage() {
  cat <<'USAGE'
用法: ./deploy.sh [wrangler deploy 参数...]

默认行为（release）：
1) 校验 nola_backend_env / NOLA_BACKEND_ENV JSON
2) 执行 Worker TypeScript typecheck
3) 执行 wrangler deploy（生产发布）

可选环境变量：
  DEPLOY_RUN_TYPECHECK=0   跳过 typecheck
  DEPLOY_RUN_TESTS=1       额外执行 backend pytest
  NPM_BIN=pnpm             使用自定义 npm 可执行文件

示例：
  ./deploy.sh
  DEPLOY_RUN_TESTS=1 ./deploy.sh
  ./deploy.sh --env production
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if ! command -v "$NPM_BIN" >/dev/null 2>&1; then
  err "未找到 npm，请先安装 Node.js/npm"
  exit 1
fi

if [ -z "${nola_backend_env:-${NOLA_BACKEND_ENV:-}}" ]; then
  err "缺少 nola_backend_env（或 NOLA_BACKEND_ENV）JSON 配置"
  err "可参考 backend/.env.example"
  exit 1
fi

if [ -z "${CLOUDFLARE_API_TOKEN:-${CF_API_TOKEN:-}}" ]; then
  warn "未检测到 CLOUDFLARE_API_TOKEN/CF_API_TOKEN，wrangler 可能需要交互登录"
fi

cd "$BACKEND_DIR"

log "校验后端环境 JSON..."
"$NPM_BIN" run --silent check-env
ok "环境配置校验通过"

if [ "$DEPLOY_RUN_TYPECHECK" = "1" ]; then
  log "执行 Worker typecheck..."
  "$NPM_BIN" run --silent typecheck
  ok "typecheck 通过"
fi

if [ "$DEPLOY_RUN_TESTS" = "1" ]; then
  log "执行 backend pytest..."
  python -m pytest -q
  ok "pytest 通过"
fi

log "开始发布 release 版本..."
"$NPM_BIN" run deploy -- "$@"
ok "release 版本发布完成"
