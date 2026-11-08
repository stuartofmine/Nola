-- One immutable publication receipt per actual source commit, never per digest.
CREATE TABLE assistant_journal_publications(
    source_key TEXT PRIMARY KEY,
    source_store_epoch TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK(source_kind IN ('admission','transition','idle_checkpoint','ui_surface','task_run_binding')),
    source_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    run_id TEXT REFERENCES runs(id) ON DELETE CASCADE,
    source_revision INTEGER NOT NULL CHECK(source_revision>0),
    set_digest TEXT NOT NULL,
    group_count INTEGER NOT NULL CHECK(group_count>0),
    frame_count INTEGER NOT NULL CHECK(frame_count>0),
    operation_count INTEGER NOT NULL CHECK(operation_count>0),
    supplied_chunk_count INTEGER NOT NULL CHECK(supplied_chunk_count>=0),
    supplied_utf8_bytes INTEGER NOT NULL CHECK(supplied_utf8_bytes>=0),
    result_json TEXT NOT NULL,
    UNIQUE(source_store_epoch,source_kind,source_id),
    CHECK((source_kind='idle_checkpoint' AND run_id IS NULL) OR (source_kind!='idle_checkpoint' AND run_id IS NOT NULL)));
CREATE TABLE assistant_journal_publication_frames(
    source_key TEXT NOT NULL REFERENCES assistant_journal_publications(source_key) ON DELETE CASCADE,
    group_ordinal INTEGER NOT NULL CHECK(group_ordinal>=0),
    frame_ordinal INTEGER NOT NULL CHECK(frame_ordinal>=0),
    owner TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK(sequence>0),
    record_digest TEXT NOT NULL,
    PRIMARY KEY(source_key,group_ordinal,frame_ordinal),
    UNIQUE(owner,sequence),
    FOREIGN KEY(owner,sequence) REFERENCES assistant_journal_records(owner,sequence) ON DELETE CASCADE);
