CREATE TABLE IF NOT EXISTS memory_index_heads(
    memory_id TEXT PRIMARY KEY,
    stamp TEXT NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS memory_bm25 USING fts5(
    memory_id UNINDEXED,
    owner_json UNINDEXED,
    namespace_json UNINDEXED,
    scope UNINDEXED,
    terms,
    tokenize='unicode61'
);
CREATE TRIGGER IF NOT EXISTS memory_index_delete AFTER DELETE ON memory_records BEGIN
    DELETE FROM memory_bm25 WHERE memory_id=OLD.id;
    DELETE FROM memory_index_heads WHERE memory_id=OLD.id;
END;
CREATE TRIGGER IF NOT EXISTS memory_index_invalidate
AFTER UPDATE OF content,summary,tags_json,owner_json,namespace_json,scope,deleted_at_ms,revision ON memory_records BEGIN
    DELETE FROM memory_bm25 WHERE memory_id=OLD.id;
    DELETE FROM memory_index_heads WHERE memory_id=OLD.id;
END;
