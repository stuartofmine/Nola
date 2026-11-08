-- Canonical-private stream fields. Additive schema version 31.
CREATE TABLE canonical_stream_heads(
    part_id TEXT PRIMARY KEY REFERENCES message_parts(id) ON DELETE CASCADE,
    head_json TEXT NOT NULL CHECK(length(CAST(head_json AS BLOB)) <= 4096)
);
CREATE TABLE canonical_stream_manifests(
    part_id TEXT NOT NULL REFERENCES message_parts(id) ON DELETE CASCADE,
    root TEXT NOT NULL,
    previous_root TEXT,
    operation TEXT NOT NULL CHECK(operation IN ('declare','append','replace','close')),
    head_json TEXT NOT NULL CHECK(length(CAST(head_json AS BLOB)) <= 4096),
    PRIMARY KEY(part_id, root)
);
CREATE TABLE canonical_stream_chunks(
    part_id TEXT NOT NULL REFERENCES message_parts(id) ON DELETE CASCADE,
    generation INTEGER NOT NULL CHECK(generation >= 0),
    chunk_index INTEGER NOT NULL CHECK(chunk_index >= 0),
    offset_bytes INTEGER NOT NULL CHECK(offset_bytes >= 0),
    text TEXT NOT NULL CHECK(length(CAST(text AS BLOB)) BETWEEN 1 AND 32768),
    manifest_root TEXT NOT NULL,
    PRIMARY KEY(part_id, generation, chunk_index),
    UNIQUE(part_id, generation, offset_bytes),
    FOREIGN KEY(part_id, manifest_root)
        REFERENCES canonical_stream_manifests(part_id, root)
);
