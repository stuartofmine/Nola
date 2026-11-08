#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# File Overview:
# - Purpose: Automates operational steps for dev in the repository development workflow.
# - Interfaces:
#   - log
#   - ok
#   - warn
#   - err
#   - ensure_npm
# - APIs:
#   - none
#
# dev.sh — 一键启动 Nola Worker 后端 + CLI Web
set -e

ROOT="$(cd "$(dirname "$0")" && pwd)"
BACKEND_DIR="$ROOT/backend"
WEB_DIR="$ROOT/web"
BACKEND_NPM_BIN="${BACKEND_NPM_BIN:-npm}"

BACKEND_PORT="${BACKEND_PORT:-8000}"
WEB_PORT="${WEB_PORT:-5001}"
NOLA_LAN="${NOLA_LAN:-1}"
DEV_RELOAD="${DEV_RELOAD:-1}"
BACKEND_RELOAD="${BACKEND_RELOAD:-$DEV_RELOAD}"
BACKEND_HOST="${BACKEND_HOST:-127.0.0.1}"
WEB_HOST="${WEB_HOST:-127.0.0.1}"
WEB_SCHEME="http"
BACKEND_LOG="/tmp/nola-backend.log"
WEB_LOG="/tmp/nola-web.log"
BACKEND_PID_FILE="/tmp/nola-backend.pid"
WEB_PID_FILE="/tmp/nola-web.pid"

# ---- 颜色 ----
GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'

log()  { echo -e "${CYAN}[dev]${NC} $*"; }
ok()   { echo -e "${GREEN}[ok]${NC}  $*"; }
warn() { echo -e "${YELLOW}[warn]${NC} $*"; }
err()  { echo -e "${RED}[err]${NC}  $*"; }

ensure_npm() {
    if command -v "$BACKEND_NPM_BIN" >/dev/null 2>&1; then
        return
    fi
    err "未找到 npm，请先安装 Node.js/npm"
    exit 1
}

detect_lan_ip() {
    if command -v ipconfig >/dev/null 2>&1; then
        ipconfig getifaddr en0 2>/dev/null || ipconfig getifaddr en1 2>/dev/null || true
        return
    fi
    hostname -I 2>/dev/null | awk '{print $1}' || true
}

stop_all() {
    log "正在关闭服务..."
    if [ -f "$BACKEND_PID_FILE" ]; then
        PID=$(cat "$BACKEND_PID_FILE")
        kill "$PID" 2>/dev/null && ok "后端已停止 (PID $PID)" || true
        rm -f "$BACKEND_PID_FILE"
    fi
    if [ -f "$WEB_PID_FILE" ]; then
        PID=$(cat "$WEB_PID_FILE")
        kill "$PID" 2>/dev/null && ok "Web 已停止 (PID $PID)" || true
        rm -f "$WEB_PID_FILE"
    fi
}

# Ctrl+C 时优雅退出
trap 'echo ""; stop_all; exit 0' INT TERM

# ---- 清理旧进程 ----
log "检查旧进程..."
lsof -ti ":$BACKEND_PORT" | xargs kill -9 2>/dev/null || true
lsof -ti ":$WEB_PORT"     | xargs kill -9 2>/dev/null || true
sleep 1

# ---- 依赖环境 ----
ensure_npm
log "安装/校验 Worker 依赖..."
"$BACKEND_DIR/start.sh" install
log "Web 后端使用 Rust CLI；Flask legacy 已移除"
log "构建 Svelte Web 界面..."
(cd "$WEB_DIR/frontend" && "$BACKEND_NPM_BIN" ci && "$BACKEND_NPM_BIN" run build)

if [ "$NOLA_LAN" = "1" ] || [ "$NOLA_LAN" = "true" ]; then
    LAN_IP="${LAN_IP:-$(detect_lan_ip)}"
    BACKEND_HOST="${BACKEND_HOST:-0.0.0.0}"
    WEB_HOST="${WEB_HOST:-0.0.0.0}"
    # If the caller did not explicitly override hosts above, force LAN binding.
    [ "$BACKEND_HOST" = "127.0.0.1" ] && BACKEND_HOST="0.0.0.0"
    [ "$WEB_HOST" = "127.0.0.1" ] && WEB_HOST="0.0.0.0"
    WEB_SCHEME="http"
    warn "局域网模式已开启：Web/后端会监听 0.0.0.0，并使用 HTTP；请只在可信网络使用。"
fi

reload_enabled() {
    case "$(printf '%s' "${1:-}" | tr '[:upper:]' '[:lower:]')" in
        1|true|yes|on) return 0 ;;
        *) return 1 ;;
    esac
}

# ---- 启动后端（Worker） ----
log "启动后端 ($BACKEND_HOST:$BACKEND_PORT)..."
if reload_enabled "$BACKEND_RELOAD"; then
    log "后端热重载已开启：Worker 代码变更会自动生效"
else
    warn "BACKEND_RELOAD=0 已设置；Worker dev 模式仍会自动重载"
fi
cd "$BACKEND_DIR"
"$BACKEND_DIR/start.sh" dev --ip "$BACKEND_HOST" --port "$BACKEND_PORT" > "$BACKEND_LOG" 2>&1 &
echo $! > "$BACKEND_PID_FILE"

# 等待后端就绪
for i in $(seq 1 15); do
    if curl -s --max-time 1 "http://127.0.0.1:$BACKEND_PORT/healthz" > /dev/null 2>&1; then
        ok "后端启动成功 → http://127.0.0.1:$BACKEND_PORT"
        break
    fi
    if [ "$i" -eq 15 ]; then
        err "后端启动超时，查看日志：$BACKEND_LOG"
        tail -20 "$BACKEND_LOG"
        stop_all; exit 1
    fi
    sleep 1
done

# ---- 启动 Web ----
log "启动 Web ($WEB_HOST:$WEB_PORT, backend=rust-cli)..."
NOLA_BUILD_PROFILE=dev "$ROOT/scripts/build_cli_runtime.sh"
"$ROOT/target/debug/nola" web --host "$WEB_HOST" --port "$WEB_PORT" --web-root "$WEB_DIR" > "$WEB_LOG" 2>&1 &
echo $! > "$WEB_PID_FILE"

# 等待 Web 就绪
for i in $(seq 1 12); do
    if curl -k -s --max-time 1 "$WEB_SCHEME://127.0.0.1:$WEB_PORT/" > /dev/null 2>&1; then
        ok "Web 启动成功 → $WEB_SCHEME://127.0.0.1:$WEB_PORT"
        break
    fi
    if [ "$i" -eq 12 ]; then
        err "Web 启动超时，查看日志：$WEB_LOG"
        tail -20 "$WEB_LOG"
        stop_all; exit 1
    fi
    sleep 1
done

echo ""
echo -e "${GREEN}===== Nola 开发环境已就绪 =====${NC}"
echo -e "  后端  → http://127.0.0.1:$BACKEND_PORT"
echo -e "  Web   → $WEB_SCHEME://127.0.0.1:$WEB_PORT"
if [ "$NOLA_LAN" = "1" ] || [ "$NOLA_LAN" = "true" ]; then
    echo -e "  LAN   → $WEB_SCHEME://${LAN_IP:-<你的局域网IP>}:$WEB_PORT"
    echo -e "  安全  → HTTP 局域网模式；Agent body/SSE 仍使用 AES-GCM 加密，但账号密码/token header 依赖可信局域网"
fi
echo -e "  日志  → $BACKEND_LOG  /  $WEB_LOG"
echo -e "  按 Ctrl+C 停止所有服务"
echo ""

# 持续尾随日志（两路合并）
tail -f "$BACKEND_LOG" "$WEB_LOG"
