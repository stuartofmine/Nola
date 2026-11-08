-- SPDX-License-Identifier: Apache-2.0
-- A derived Run-wide sequence, never an accepted or Turn-local sequence.
CREATE TABLE run_input_applications (
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    run_sequence INTEGER NOT NULL CHECK(run_sequence > 0),
    core_session_id TEXT NOT NULL,
    admission_id TEXT NOT NULL,
    destination_turn_id TEXT NOT NULL,
    turn_applied_sequence INTEGER NOT NULL CHECK(turn_applied_sequence > 0),
    source_event_id TEXT NOT NULL REFERENCES run_events(event_id) ON DELETE CASCADE,
    core_revision INTEGER NOT NULL CHECK(core_revision >= 0),
    core_cursor INTEGER NOT NULL CHECK(core_cursor > 0),
    event_application_index INTEGER NOT NULL CHECK(event_application_index >= 0),
    source_digest TEXT NOT NULL,
    admission_source_digest TEXT NOT NULL,
    admission_payload_digest TEXT NOT NULL,
    provenance TEXT NOT NULL CHECK(provenance = 'current_frontier'),
    chain_digest TEXT NOT NULL,
    admission_source_event_id TEXT NOT NULL REFERENCES run_events(event_id) ON DELETE CASCADE,
    PRIMARY KEY(run_id,run_sequence),
    UNIQUE(core_session_id,admission_id),
    UNIQUE(run_id,core_cursor,event_application_index),
    FOREIGN KEY(core_session_id,admission_id)
        REFERENCES input_turn_applications(core_session_id,admission_id) ON DELETE CASCADE
);
CREATE TRIGGER run_input_applications_immutable_update BEFORE UPDATE ON run_input_applications
BEGIN SELECT RAISE(ABORT,'Run input applications are immutable'); END;
CREATE TRIGGER run_input_applications_immutable_delete BEFORE DELETE ON run_input_applications
WHEN EXISTS(SELECT 1 FROM runs r JOIN conversations c ON c.id=r.conversation_id WHERE r.id=OLD.run_id)
 AND EXISTS(SELECT 1 FROM input_admissions WHERE core_session_id=OLD.core_session_id AND admission_id=OLD.admission_id)
 AND EXISTS(SELECT 1 FROM run_events WHERE event_id=OLD.admission_source_event_id)
 AND EXISTS(SELECT 1 FROM run_events WHERE event_id=OLD.source_event_id)
BEGIN SELECT RAISE(ABORT,'Run input applications are immutable'); END;
