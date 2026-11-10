#!/usr/bin/env bash
set -euo pipefail

ROOT=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
TMP=$(mktemp -d /tmp/nola-mac-session-show-e2e.XXXXXX)
trap 'rm -rf "$TMP"' EXIT INT TERM
DB="$TMP/nola.db"

/usr/bin/sqlite3 "$DB" < "$ROOT/core/nola-store/src/schema.sql"
/usr/bin/sqlite3 "$DB" "PRAGMA application_id=1313819713; PRAGMA user_version=2;"

/usr/bin/sqlite3 "$DB" <<'SQL'
INSERT INTO conversations(id, title, revision, created_at_ms, updated_at_ms)
VALUES('projection-e2e', 'Projection E2E', 9, 1, 9);

INSERT INTO messages(id, conversation_id, role, status, ordinal, created_at_ms, completed_at_ms) VALUES
 ('user-1', 'projection-e2e', 'user', 'completed', 0, 1, 1),
 ('assistant-1', 'projection-e2e', 'assistant', 'completed', 1, 2, 5),
 ('user-2', 'projection-e2e', 'user', 'completed', 2, 6, 6),
 ('assistant-2', 'projection-e2e', 'assistant', 'completed', 3, 7, 9);

INSERT INTO message_parts(id, message_id, kind, ordinal, state, content_text, content_json, created_at_ms, updated_at_ms) VALUES
 ('u1', 'user-1', 'text', 0, 'completed', 'First question', NULL, 1, 1),
 ('r1', 'assistant-1', 'reasoning', 0, 'completed', '', '{"kind":"reasoning","id":"r1","message_id":"assistant-1","turn_id":"turn-1","title":"Reasoning","detail":"Inspect input","is_complete":true}', 2, 3),
 ('t1', 'assistant-1', 'tool', 1, 'completed', '', '{"kind":"tool","id":"t1","message_id":"assistant-1","turn_id":"turn-1","call_id":"call-1","capability_id":"read_file","title":"Read file","state":"completed","input":{"path":"/tmp/a.txt"},"raw_input":"{\"path\":\"/tmp/a.txt\"}","permission":null,"output_preview":"fixture output","full_output_ref":null,"artifacts":[],"started_at_ms":3,"finished_at_ms":4}', 3, 4),
 ('x1', 'assistant-1', 'text', 2, 'completed', 'First final', '{"kind":"text","id":"x1","message_id":"assistant-1","turn_id":"turn-1","text":"First final","is_complete":true}', 4, 5),
 ('u2', 'user-2', 'text', 0, 'completed', 'Second question', NULL, 6, 6),
 ('x2', 'assistant-2', 'text', 0, 'completed', 'Second final', '{"kind":"text","id":"x2","message_id":"assistant-2","turn_id":"turn-2","text":"Second final","is_complete":true}', 7, 9);

INSERT INTO runs(id, conversation_id, kind, user_message_id, assistant_message_id, status, next_event_sequence, created_at_ms, updated_at_ms, completed_at_ms) VALUES
 ('run-1', 'projection-e2e', 'conversation', 'user-1', 'assistant-1', 'succeeded', 2, 2, 5, 5),
 ('run-2', 'projection-e2e', 'conversation', 'user-2', 'assistant-2', 'succeeded', 1, 7, 9, 9);

INSERT INTO trace_journal_runs(trace_id, run_id, conversation_id, message_id, turn_id, status, last_sequence, started_at, updated_at, finished_at)
VALUES('trace-1', 'run-1', 'projection-e2e', 'assistant-1', 'turn-1', 'succeeded', 1, '2026-08-02T00:00:00Z', '2026-08-02T00:00:02Z', '2026-08-02T00:00:02Z');
INSERT INTO trace_journal_events(event_id, trace_id, conversation_id, turn_id, sequence, kind, status, visibility, emitted_at, event_json)
VALUES('trace-event-1', 'trace-1', 'projection-e2e', 'turn-1', 1, 'assistant.delta', 'succeeded', 'summary', '2026-08-02T00:00:01Z', '{"schema_version":1,"trace_id":"trace-1","run_id":"run-1","conversation_id":"projection-e2e","message_id":"assistant-1","turn_id":"turn-1","event_id":"trace-event-1","sequence":1,"emitted_at":"2026-08-02T00:00:01Z","kind":"assistant.delta","status":"succeeded","visibility":"summary","payload":{"delta":"WRONG TRACE CONTENT"}}');
SQL

(cd "$ROOT/apps/cli" && cargo build --quiet --bin nola)
. "$ROOT/scripts/e2e/cargo_paths.sh"
NOLA=$(nola_cargo_binary "$ROOT" debug nola)
[[ -x "$NOLA" ]] || { echo "nola binary not found at $NOLA" >&2; exit 1; }

export NOLA_DB_PATH="$DB" NOLA_CLI_LANG=en
perl -e 'alarm shift; exec @ARGV' 10 "$NOLA" session show projection-e2e --json > "$TMP/session-show.json"

cp "$ROOT/tests/apple/macos/ProcessHarnesses/MacSessionShowProjectionHarnessMain.swift.fixture" "$TMP/main.swift"
xcrun swiftc "$ROOT/apps/macos/Models/MacSessionShowPayloadDecoder.swift" "$TMP/main.swift" -o "$TMP/mac-session-show-harness"
"$TMP/mac-session-show-harness" "$TMP/session-show.json"
