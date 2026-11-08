CREATE TABLE prepared_host_tool_plans(
    manifest_id TEXT PRIMARY KEY,
    host_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    snapshot_json TEXT NOT NULL
);
CREATE INDEX prepared_host_tool_plans_expiry ON prepared_host_tool_plans(expires_at_ms);
