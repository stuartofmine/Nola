CREATE TABLE IF NOT EXISTS remote_submission_intents (
    submission_id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    identity_json TEXT NOT NULL,
    input_json TEXT NOT NULL,
    request_json TEXT,
    request_digest TEXT,
    phase TEXT NOT NULL CHECK(phase IN ('prepared','bound','admitted','rejected','cancelled')),
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS remote_submission_intent_conversation
    ON remote_submission_intents(conversation_id,created_at_ms,submission_id);
CREATE TABLE IF NOT EXISTS source_submission_receipts (
    submission_id TEXT PRIMARY KEY,
    identity_json TEXT NOT NULL,
    request_json TEXT,
    receipt_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS remote_observed_run_receipts (
    run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    receipt_json TEXT NOT NULL
);
