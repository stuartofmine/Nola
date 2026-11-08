-- Remote epoch is bound only after an authenticated, already admitted Run and
-- a complete record have validated. Never infer it from a global chunk digest.
CREATE TABLE IF NOT EXISTS remote_assistant_journal_sources (
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    source_host_id TEXT NOT NULL,
    source_store_epoch TEXT NOT NULL,
    assistant_message_id TEXT NOT NULL,
    PRIMARY KEY(run_id,assistant_message_id)
);
-- Preparation is record-scoped and deliberately separate from readable public
-- manifests. It cannot reserve or poison a public Part generation.
CREATE TABLE IF NOT EXISTS remote_assistant_content_stages (
    id TEXT PRIMARY KEY,
    owner TEXT NOT NULL,
    record_digest TEXT NOT NULL,
    part_id TEXT NOT NULL,
    base_json TEXT NOT NULL,
    content_json TEXT NOT NULL,
    prefix_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS remote_assistant_content_stage_chunks (
    stage_id TEXT NOT NULL REFERENCES remote_assistant_content_stages(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    digest TEXT NOT NULL,
    result_root TEXT NOT NULL,
    bytes BLOB NOT NULL CHECK(length(bytes) BETWEEN 1 AND 65536),
    PRIMARY KEY(stage_id,chunk_index)
);
