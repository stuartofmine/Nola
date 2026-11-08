CREATE TABLE plugin_installations(
    id TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK(revision > 0),
    record_json TEXT NOT NULL CHECK(json_valid(record_json)),
    removed_at_ms INTEGER
);
CREATE INDEX plugin_installations_active ON plugin_installations(id)
    WHERE removed_at_ms IS NULL;
