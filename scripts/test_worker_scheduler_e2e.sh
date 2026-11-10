#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORKER="$ROOT/legacy/backend/worker"
STATE="$(mktemp -d "${TMPDIR:-/tmp}/nola-worker-scheduler.XXXXXX")"
LOG="$STATE/wrangler.log"
PORT="${NOLA_WORKER_E2E_PORT:-18791}"
PID=""

cleanup() {
    if [[ -n "$PID" ]]; then kill "$PID" 2>/dev/null || true; fi
    rm -rf "$STATE"
}
trap cleanup EXIT

cd "$WORKER"
npx wrangler d1 migrations apply nola_backend_fresh --local --persist-to "$STATE" >/dev/null

NOW_ISO="$(date -u +%Y-%m-%dT%H:%M:00.000Z)"
NOW_HHMM="$(date -u +%H:%M)"
SOURCE_ID="scheduler-e2e-$(date +%s)"
npx wrangler d1 execute nola_backend_fresh --local --persist-to "$STATE" --command \
  "INSERT INTO scheduledsourcerecord (client_uuid, source_id, name, url, format, enabled, schedule_frequency, scheduled_time_iso, timezone, updated_at) VALUES ('scheduler-e2e-client', '$SOURCE_ID', 'Scheduler E2E', 'https://example.com/', 'html', 1, 'daily', '$NOW_HHMM', 'UTC', '$NOW_ISO');" >/dev/null

NOLA_BACKEND_ENV='{"PUSH_FEATURE_ENABLED":true,"PUSH_SCHEDULER_ENABLED":true}' \
  npx wrangler dev --local --test-scheduled --persist-to "$STATE" --port "$PORT" >"$LOG" 2>&1 &
PID=$!

for _ in $(seq 1 60); do
    if curl -fsS "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1; then break; fi
    sleep 0.25
done
curl -fsS "http://127.0.0.1:$PORT/__scheduled?cron=*%20*%20*%20*%20*" >/dev/null

for _ in $(seq 1 80); do
    RESULT="$(npx wrangler d1 execute nola_backend_fresh --local --persist-to "$STATE" --json --command "SELECT j.status, r.llm_summary_text FROM scheduledjobrecord j LEFT JOIN scheduledresultrecord r ON r.job_id = j.job_id WHERE j.source_id = '$SOURCE_ID';")"
    if printf '%s' "$RESULT" | grep -q '"status": "success"'; then
        printf '%s' "$RESULT" | grep -q 'Example Domain'
        SECOND="$(curl -fsS "http://127.0.0.1:$PORT/__scheduled?cron=*%20*%20*%20*%20*")"
        COUNT="$(npx wrangler d1 execute nola_backend_fresh --local --persist-to "$STATE" --json --command "SELECT COUNT(*) AS count FROM scheduledjobrecord WHERE source_id = '$SOURCE_ID';")"
        printf '%s' "$COUNT" | grep -q '"count": 1'
        printf '{"case":"worker-cron-scheduler","source":"%s","result":"success","idempotent":true}\n' "$SOURCE_ID"
        exit 0
    fi
    sleep 0.25
done

cat "$LOG" >&2
printf '%s\n' "$RESULT" >&2
exit 1
