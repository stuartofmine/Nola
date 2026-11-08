-- SPDX-License-Identifier: Apache-2.0
-- This is a derived admission witness, never a recoverable Core checkpoint.
-- All checkpoint writers, including restore/import and transaction rollback,
-- maintain it on the same connection and in the same SQLite transaction.
CREATE TABLE core_delivery_witnesses (
    conversation_id TEXT PRIMARY KEY REFERENCES core_checkpoints(conversation_id) ON DELETE CASCADE,
    checkpoint_revision INTEGER NOT NULL,
    event_cursor INTEGER NOT NULL,
    witness_json TEXT NOT NULL
);


CREATE TRIGGER core_delivery_witness_insert AFTER INSERT ON core_checkpoint_bodies
BEGIN
    INSERT INTO core_delivery_witnesses
    SELECT * FROM core_delivery_witness_projection WHERE conversation_id=NEW.conversation_id
    ON CONFLICT(conversation_id) DO UPDATE SET checkpoint_revision=excluded.checkpoint_revision,
        event_cursor=excluded.event_cursor,witness_json=excluded.witness_json;
END;
CREATE TRIGGER core_delivery_witness_update
AFTER UPDATE OF checkpoint_json ON core_checkpoint_bodies
BEGIN
    INSERT INTO core_delivery_witnesses
    SELECT * FROM core_delivery_witness_projection WHERE conversation_id=NEW.conversation_id
    ON CONFLICT(conversation_id) DO UPDATE SET checkpoint_revision=excluded.checkpoint_revision,
        event_cursor=excluded.event_cursor,witness_json=excluded.witness_json;
END;

INSERT INTO core_delivery_witnesses SELECT * FROM core_delivery_witness_projection;
