-- Review facts are derived within publication transactions, never at read time.
CREATE TABLE IF NOT EXISTS canonical_part_disclosures(
    part_id TEXT PRIMARY KEY REFERENCES message_parts(id) ON DELETE CASCADE,
    generation INTEGER NOT NULL CHECK(generation>0),
    evaluated_generation INTEGER,
    conversation_id TEXT,
    message_id TEXT,
    backing_json TEXT CHECK(backing_json IS NULL OR length(CAST(backing_json AS BLOB))<=4096),
    decision TEXT CHECK(decision IN ('allow','withheld','invalid')),
    CHECK(evaluated_generation IS NULL OR evaluated_generation=generation)
);
CREATE INDEX IF NOT EXISTS canonical_part_disclosures_dirty ON canonical_part_disclosures(part_id)
    WHERE evaluated_generation IS NULL;
-- Exact provider role classification shares the canonical content-generation
-- invalidation boundary, but is a separate derived fact from text disclosure.
CREATE TABLE canonical_provider_classifications(
    part_id TEXT PRIMARY KEY REFERENCES message_parts(id) ON DELETE CASCADE,
    generation INTEGER NOT NULL,
    message_state TEXT NOT NULL,
    part_state TEXT NOT NULL,
    has_tool_result INTEGER NOT NULL CHECK(has_tool_result IN (0,1))
);
CREATE TRIGGER IF NOT EXISTS disclosure_part_insert AFTER INSERT ON message_parts BEGIN
    INSERT INTO canonical_part_disclosures(part_id,generation) VALUES(NEW.id,1);
END;
CREATE TRIGGER IF NOT EXISTS disclosure_part_update AFTER UPDATE OF message_id,kind,content_json ON message_parts
WHEN OLD.message_id IS NOT NEW.message_id OR OLD.kind IS NOT NEW.kind OR OLD.content_json IS NOT NEW.content_json BEGIN
    UPDATE canonical_part_disclosures SET generation=generation+1,evaluated_generation=NULL,decision=NULL
        WHERE part_id=NEW.id;
END;
CREATE TRIGGER IF NOT EXISTS disclosure_message_owner AFTER UPDATE OF conversation_id ON messages
WHEN OLD.conversation_id IS NOT NEW.conversation_id BEGIN
    UPDATE canonical_part_disclosures SET generation=generation+1,evaluated_generation=NULL,decision=NULL
        WHERE part_id IN (SELECT id FROM message_parts WHERE message_id=NEW.id);
END;
CREATE TRIGGER IF NOT EXISTS disclosure_reference_insert AFTER INSERT ON message_part_body_references
WHEN NEW.field='content_json' BEGIN
    UPDATE canonical_part_disclosures SET generation=generation+1,evaluated_generation=NULL,decision=NULL WHERE part_id=NEW.part_id;
END;
CREATE TRIGGER IF NOT EXISTS disclosure_reference_delete AFTER DELETE ON message_part_body_references
WHEN OLD.field='content_json' BEGIN
    UPDATE canonical_part_disclosures SET generation=generation+1,evaluated_generation=NULL,decision=NULL WHERE part_id=OLD.part_id;
END;
CREATE TRIGGER IF NOT EXISTS disclosure_reference_update AFTER UPDATE ON message_part_body_references
WHEN (OLD.field='content_json' OR NEW.field='content_json')
 AND (OLD.part_id IS NOT NEW.part_id OR OLD.field IS NOT NEW.field OR OLD.reference_json IS NOT NEW.reference_json) BEGIN
    UPDATE canonical_part_disclosures SET generation=generation+1,evaluated_generation=NULL,decision=NULL
        WHERE part_id IN (OLD.part_id,NEW.part_id);
END;
INSERT OR IGNORE INTO canonical_part_disclosures(part_id,generation) SELECT id,1 FROM message_parts;
CREATE INDEX IF NOT EXISTS disclosure_json_backing ON message_part_body_references(
    CASE WHEN json_valid(reference_json) THEN json_extract(reference_json,'$.content_id') END)
    WHERE field='content_json';
CREATE TRIGGER IF NOT EXISTS disclosure_canonical_text_chunks_insert AFTER INSERT ON canonical_text_chunks BEGIN
    UPDATE canonical_part_disclosures SET generation=generation+1,evaluated_generation=NULL,decision=NULL
    WHERE part_id IN (SELECT part_id FROM message_part_body_references WHERE field='content_json'
        AND (CASE WHEN json_valid(reference_json) THEN json_extract(reference_json,'$.content_id') END) IN (NEW.content_id));
END;
CREATE TRIGGER IF NOT EXISTS disclosure_canonical_text_chunks_update AFTER UPDATE ON canonical_text_chunks BEGIN
    UPDATE canonical_part_disclosures SET generation=generation+1,evaluated_generation=NULL,decision=NULL
    WHERE part_id IN (SELECT part_id FROM message_part_body_references WHERE field='content_json'
        AND (CASE WHEN json_valid(reference_json) THEN json_extract(reference_json,'$.content_id') END) IN (OLD.content_id,NEW.content_id));
END;
CREATE TRIGGER IF NOT EXISTS disclosure_canonical_text_chunks_delete AFTER DELETE ON canonical_text_chunks BEGIN
    UPDATE canonical_part_disclosures SET generation=generation+1,evaluated_generation=NULL,decision=NULL
    WHERE part_id IN (SELECT part_id FROM message_part_body_references WHERE field='content_json'
        AND (CASE WHEN json_valid(reference_json) THEN json_extract(reference_json,'$.content_id') END) IN (OLD.content_id));
END;
CREATE TRIGGER IF NOT EXISTS disclosure_canonical_text_objects_insert AFTER INSERT ON canonical_text_objects BEGIN
    UPDATE canonical_part_disclosures SET generation=generation+1,evaluated_generation=NULL,decision=NULL
    WHERE part_id IN (SELECT part_id FROM message_part_body_references WHERE field='content_json'
        AND (CASE WHEN json_valid(reference_json) THEN json_extract(reference_json,'$.content_id') END) IN (NEW.id));
END;
CREATE TRIGGER IF NOT EXISTS disclosure_canonical_text_objects_update AFTER UPDATE ON canonical_text_objects BEGIN
    UPDATE canonical_part_disclosures SET generation=generation+1,evaluated_generation=NULL,decision=NULL
    WHERE part_id IN (SELECT part_id FROM message_part_body_references WHERE field='content_json'
        AND (CASE WHEN json_valid(reference_json) THEN json_extract(reference_json,'$.content_id') END) IN (OLD.id,NEW.id));
END;
CREATE TRIGGER IF NOT EXISTS disclosure_canonical_text_objects_delete AFTER DELETE ON canonical_text_objects BEGIN
    UPDATE canonical_part_disclosures SET generation=generation+1,evaluated_generation=NULL,decision=NULL
    WHERE part_id IN (SELECT part_id FROM message_part_body_references WHERE field='content_json'
        AND (CASE WHEN json_valid(reference_json) THEN json_extract(reference_json,'$.content_id') END) IN (OLD.id));
END;
