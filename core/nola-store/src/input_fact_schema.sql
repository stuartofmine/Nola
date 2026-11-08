-- SPDX-License-Identifier: Apache-2.0
-- Exact source-event indices. Turn sequence is deliberately NOT a Run watermark.
CREATE TABLE input_admissions (
    core_session_id TEXT NOT NULL,
    admission_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    submission_id TEXT,
    accepted_sequence INTEGER NOT NULL CHECK(accepted_sequence>0),
    source_event_id TEXT NOT NULL REFERENCES run_events(event_id) ON DELETE CASCADE,
    source_run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    core_revision INTEGER NOT NULL CHECK(core_revision>=0),
    core_cursor INTEGER NOT NULL CHECK(core_cursor>0),
    delivery TEXT NOT NULL CHECK(delivery IN ('current_turn','next_turn')),
    anchor_turn_id TEXT,
    origin TEXT NOT NULL,
    payload_digest TEXT NOT NULL,
    source_digest TEXT NOT NULL,
    provenance TEXT NOT NULL CHECK(provenance = 'current_frontier'),
    PRIMARY KEY(core_session_id,admission_id),
    UNIQUE(core_session_id,accepted_sequence),
    UNIQUE(core_session_id,submission_id)
);
CREATE TABLE input_turn_applications (
    core_session_id TEXT NOT NULL,
    admission_id TEXT NOT NULL,
    destination_turn_id TEXT NOT NULL,
    turn_applied_sequence INTEGER NOT NULL CHECK(turn_applied_sequence>0),
    source_digest TEXT NOT NULL,
    source_run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    source_event_id TEXT NOT NULL REFERENCES run_events(event_id) ON DELETE CASCADE,
    core_revision INTEGER NOT NULL CHECK(core_revision>=0),
    core_cursor INTEGER NOT NULL CHECK(core_cursor>0),
    provenance TEXT NOT NULL CHECK(provenance = 'current_frontier'),
    PRIMARY KEY(core_session_id,admission_id),
    UNIQUE(core_session_id,destination_turn_id,turn_applied_sequence),
    FOREIGN KEY(core_session_id,admission_id) REFERENCES input_admissions(core_session_id,admission_id) ON DELETE CASCADE
);
CREATE TABLE input_dispositions (
    core_session_id TEXT NOT NULL,
    admission_id TEXT NOT NULL,
    source_digest TEXT NOT NULL,
    source_event_id TEXT NOT NULL REFERENCES run_events(event_id) ON DELETE CASCADE,
    core_revision INTEGER NOT NULL CHECK(core_revision>=0),
    core_cursor INTEGER NOT NULL CHECK(core_cursor>0),
    PRIMARY KEY(core_session_id,admission_id),
    FOREIGN KEY(core_session_id,admission_id) REFERENCES input_admissions(core_session_id,admission_id) ON DELETE CASCADE
);
CREATE TABLE run_successor_admissions (
    core_session_id TEXT NOT NULL,
    admission_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    previous_run_id TEXT NOT NULL UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    originating_submission_id TEXT NOT NULL,
    request_digest TEXT NOT NULL,
    result_json TEXT NOT NULL,
    PRIMARY KEY(core_session_id,admission_id),
    FOREIGN KEY(core_session_id,admission_id) REFERENCES input_admissions(core_session_id,admission_id) ON DELETE CASCADE
);
CREATE TRIGGER run_successor_admissions_immutable_update BEFORE UPDATE ON run_successor_admissions
BEGIN SELECT RAISE(ABORT,'successor admission is immutable'); END;
CREATE TRIGGER run_successor_admissions_immutable_delete BEFORE DELETE ON run_successor_admissions
WHEN EXISTS(SELECT 1 FROM conversations WHERE id=OLD.conversation_id)
 AND EXISTS(SELECT 1 FROM runs WHERE id=OLD.previous_run_id)
 AND EXISTS(SELECT 1 FROM runs WHERE id=OLD.run_id)
 AND EXISTS(SELECT 1 FROM input_admissions WHERE core_session_id=OLD.core_session_id AND admission_id=OLD.admission_id)
BEGIN SELECT RAISE(ABORT,'successor admission is immutable'); END;
