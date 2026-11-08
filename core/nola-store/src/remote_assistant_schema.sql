CREATE TABLE remote_run_sources (
    run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    source_host_id TEXT NOT NULL CHECK(length(source_host_id)>0),
    source_revision INTEGER,
    source_cursor INTEGER,
    content_digest TEXT
);
