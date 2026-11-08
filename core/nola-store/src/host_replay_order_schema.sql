-- SPDX-License-Identifier: Apache-2.0
-- Explicit durable order, independent of per-effect sequence and SQLite rowid.
CREATE TABLE host_event_replay_heads (
    run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(typeof(ordinal)='integer' AND ordinal>0),
    reduction_bytes INTEGER NOT NULL DEFAULT 0 CHECK(typeof(reduction_bytes)='integer' AND reduction_bytes>=0)
);
CREATE TABLE host_event_replay_order (
    event_id TEXT UNIQUE REFERENCES host_event_inbox(event_id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(typeof(ordinal)='integer' AND ordinal>0),
    submission_id TEXT UNIQUE REFERENCES committed_transitions(transition_id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED,
    submission_json TEXT,
    decision_time_ms INTEGER,
    PRIMARY KEY(run_id, ordinal),
    CHECK ((event_id IS NOT NULL AND submission_id IS NULL AND submission_json IS NULL AND decision_time_ms IS NULL)
        OR (event_id IS NULL AND submission_id IS NOT NULL AND submission_json IS NOT NULL
            AND decision_time_ms IS NOT NULL AND decision_time_ms>=0))
);

CREATE TRIGGER host_event_replay_order_insert AFTER INSERT ON host_event_inbox
BEGIN
    INSERT INTO host_event_replay_heads(run_id,ordinal,reduction_bytes)
    VALUES(NEW.run_id,1,COALESCE(length(CAST(NEW.reduction_json AS BLOB)),0))
    ON CONFLICT(run_id) DO UPDATE SET ordinal=ordinal+1,
        reduction_bytes=reduction_bytes+excluded.reduction_bytes;
    INSERT INTO host_event_replay_order(event_id,run_id,ordinal)
    SELECT NEW.event_id,NEW.run_id,ordinal FROM host_event_replay_heads WHERE run_id=NEW.run_id;
END;
