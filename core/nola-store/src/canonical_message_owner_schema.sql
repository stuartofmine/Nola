-- Canonical content ownership survives a trusted internal execution handoff.
CREATE TABLE canonical_message_run_owners(
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL,
    PRIMARY KEY(conversation_id,message_id,run_id)
);
-- Runtime admission fact. Legacy databases are not backfilled by guessing
-- the first Part; internal Turns without a placeholder intentionally have none.
CREATE TABLE canonical_run_initial_parts(
    run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    part_id TEXT NOT NULL REFERENCES message_parts(id) ON DELETE CASCADE
);
