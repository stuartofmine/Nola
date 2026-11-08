-- SPDX-License-Identifier: Apache-2.0
CREATE TABLE core_checkpoint_replay_bases (
    conversation_id TEXT PRIMARY KEY REFERENCES core_checkpoints(conversation_id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    checkpoint_revision INTEGER NOT NULL,
    through_ordinal INTEGER NOT NULL CHECK(through_ordinal>=0),
    through_reduction_bytes INTEGER NOT NULL DEFAULT 0 CHECK(typeof(through_reduction_bytes)='integer' AND through_reduction_bytes>=0)
);
-- Unscoped replacement/import/idle updates must not inherit an old Run fence.
CREATE TRIGGER checkpoint_replay_base_invalidate AFTER UPDATE OF checkpoint_json ON core_checkpoint_bodies
BEGIN
    DELETE FROM core_checkpoint_replay_bases WHERE conversation_id=OLD.conversation_id;
END;

CREATE TRIGGER checkpoint_replay_base_commit AFTER INSERT ON committed_transitions
BEGIN
    SELECT CASE WHEN NOT EXISTS(
        SELECT 1 FROM runs r JOIN core_checkpoints c ON c.conversation_id=r.conversation_id
        WHERE r.id=NEW.run_id AND c.revision=NEW.checkpoint_revision
    ) THEN RAISE(ABORT,'checkpoint replay base has no exact transition owner') END;
    INSERT OR REPLACE INTO core_checkpoint_replay_bases
    SELECT r.conversation_id,NEW.run_id,NEW.checkpoint_revision,
        COALESCE(h.ordinal,0),COALESCE(h.reduction_bytes,0)
    FROM runs r LEFT JOIN host_event_replay_heads h ON h.run_id=r.id WHERE r.id=NEW.run_id;
END;

CREATE TRIGGER checkpoint_body_delete AFTER DELETE ON core_checkpoint_bodies
BEGIN
    DELETE FROM core_delivery_witnesses WHERE conversation_id=OLD.conversation_id;
    DELETE FROM core_checkpoint_replay_bases WHERE conversation_id=OLD.conversation_id;
END;
