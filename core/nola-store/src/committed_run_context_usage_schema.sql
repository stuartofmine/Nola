CREATE INDEX IF NOT EXISTS run_context_usage_by_sequence
ON run_events(run_id,event_kind,sequence);
CREATE INDEX IF NOT EXISTS admission_root_record_by_sequence
ON run_message_admission_records(owner,json_extract(record_json,'$.message_id'),sequence);
CREATE TABLE IF NOT EXISTS remote_run_context_usage (
    run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    source_host_id TEXT NOT NULL,
    event_sequence INTEGER NOT NULL CHECK(event_sequence>0),
    record_json TEXT NOT NULL CHECK(length(CAST(record_json AS BLOB))<=16384)
);
