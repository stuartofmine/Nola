-- Local-only epoch is intentionally absent from sync outboxes and replicas.
INSERT INTO store_metadata(key,value) VALUES('assistant_source_epoch',lower(hex(randomblob(16))));
CREATE TABLE assistant_journal_heads(
    owner TEXT PRIMARY KEY, sequence INTEGER NOT NULL CHECK(sequence>=0),
    digest TEXT NOT NULL, retained_floor INTEGER NOT NULL DEFAULT 0);
CREATE TABLE assistant_journal_records(
    owner TEXT NOT NULL, sequence INTEGER NOT NULL CHECK(sequence>0),
    operation_id TEXT NOT NULL, source_revision INTEGER NOT NULL CHECK(source_revision>0),
    header_json TEXT NOT NULL, batch BLOB NOT NULL CHECK(length(batch)<=131072),
    PRIMARY KEY(owner,sequence), UNIQUE(owner,operation_id));
CREATE TABLE assistant_journal_parts(
    owner TEXT NOT NULL, part_id TEXT NOT NULL, head_json TEXT NOT NULL,
    PRIMARY KEY(owner,part_id));
CREATE TABLE assistant_journal_part_versions(
    owner TEXT NOT NULL, part_id TEXT NOT NULL, sequence INTEGER NOT NULL,
    head_json TEXT, PRIMARY KEY(owner,part_id,sequence));
CREATE INDEX assistant_journal_versions_by_sequence ON assistant_journal_part_versions(owner,sequence,part_id);
CREATE TABLE assistant_journal_chunks(
    digest TEXT PRIMARY KEY, bytes BLOB NOT NULL CHECK(length(bytes) BETWEEN 1 AND 65536));
CREATE TABLE assistant_journal_manifests(
    root TEXT PRIMARY KEY, owner TEXT NOT NULL, part_id TEXT NOT NULL,
    field TEXT NOT NULL, generation INTEGER NOT NULL CHECK(generation>0),
    byte_count INTEGER NOT NULL CHECK(byte_count>=0),
    chunk_count INTEGER NOT NULL CHECK(chunk_count>=0),
    previous_root TEXT, chunk_digest TEXT REFERENCES assistant_journal_chunks(digest));
CREATE TABLE assistant_journal_manifest_chunks(
    owner TEXT NOT NULL, part_id TEXT NOT NULL, field TEXT NOT NULL,
    generation INTEGER NOT NULL, chunk_index INTEGER NOT NULL,
    digest TEXT NOT NULL REFERENCES assistant_journal_chunks(digest),
    byte_count INTEGER NOT NULL, result_root TEXT NOT NULL,
    PRIMARY KEY(owner,part_id,field,generation,chunk_index));
CREATE INDEX assistant_journal_chunk_owner ON assistant_journal_manifest_chunks(owner,digest);
CREATE TABLE assistant_journal_snapshots(
    id TEXT PRIMARY KEY, owner TEXT NOT NULL, sequence INTEGER NOT NULL,
    digest TEXT NOT NULL);
