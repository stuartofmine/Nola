CREATE TABLE IF NOT EXISTS run_message_admission_records (
    owner TEXT NOT NULL, sequence INTEGER NOT NULL, record_json TEXT NOT NULL,
    PRIMARY KEY(owner,sequence)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS run_message_admission_owners (
    owner TEXT NOT NULL, message_id TEXT NOT NULL, source_ordinal INTEGER NOT NULL,
    local_ordinal INTEGER NOT NULL, declaration_digest TEXT NOT NULL,
    source_created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(owner,message_id), UNIQUE(owner,source_ordinal)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS run_message_admission_stages (
    owner TEXT PRIMARY KEY, message_id TEXT NOT NULL, total_bytes INTEGER NOT NULL,
    declaration_digest TEXT NOT NULL, next_offset INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS run_message_admission_chunks (
    owner TEXT NOT NULL REFERENCES run_message_admission_stages(owner) ON DELETE CASCADE,
    offset INTEGER NOT NULL, text TEXT NOT NULL, PRIMARY KEY(owner,offset)
) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS run_message_admission_record_immutable
BEFORE UPDATE ON run_message_admission_records BEGIN
    SELECT RAISE(ABORT,'message admission record is immutable');
END;
CREATE TRIGGER IF NOT EXISTS run_message_admission_owner_immutable
BEFORE UPDATE ON run_message_admission_owners BEGIN
    SELECT RAISE(ABORT,'message admission source order is immutable');
END;
CREATE TRIGGER IF NOT EXISTS delete_run_message_admissions AFTER DELETE ON runs BEGIN
    DELETE FROM run_message_admission_records WHERE json_extract(owner,'$.run_id')=OLD.id;
    DELETE FROM run_message_admission_owners WHERE json_extract(owner,'$.run_id')=OLD.id;
    DELETE FROM run_message_admission_stages WHERE json_extract(owner,'$.run_id')=OLD.id;
END;

CREATE TABLE IF NOT EXISTS remote_conversation_execution_owners (
    run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    source_host_id TEXT NOT NULL,
    source_store_epoch TEXT NOT NULL,
    ownership_json TEXT NOT NULL CHECK(json_valid(ownership_json))
);
CREATE TRIGGER IF NOT EXISTS remote_conversation_execution_owner_immutable
BEFORE UPDATE ON remote_conversation_execution_owners BEGIN
    SELECT RAISE(ABORT,'remote execution ownership is immutable per Run');
END;
