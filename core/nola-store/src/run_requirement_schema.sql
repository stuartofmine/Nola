-- SPDX-License-Identifier: Apache-2.0
-- Derived source pointers, never a second writable requirements ledger.
CREATE TABLE run_requirement_sources (
    source_event_id TEXT PRIMARY KEY REFERENCES run_events(event_id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    core_session_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    scope_id TEXT NOT NULL,
    requirement_revision INTEGER NOT NULL CHECK(requirement_revision>=0),
    core_revision INTEGER NOT NULL CHECK(core_revision>0),
    core_cursor INTEGER NOT NULL CHECK(core_cursor>0),
    source_digest TEXT NOT NULL,
    source_transition_id TEXT REFERENCES committed_transitions(transition_id) ON DELETE CASCADE,
    source_host_event_id TEXT REFERENCES host_event_inbox(event_id) ON DELETE CASCADE,
    source_host_digest TEXT,
    CHECK((source_host_event_id IS NULL AND source_host_digest IS NULL)
       OR (source_host_event_id IS NOT NULL AND source_host_digest IS NOT NULL AND source_transition_id IS NOT NULL)),
    UNIQUE(run_id,kind,entity_id),
    UNIQUE(run_id,core_cursor)
);
CREATE UNIQUE INDEX one_scope_revision ON run_requirement_sources(run_id,requirement_revision)
 WHERE kind='requirement_scope_changed';
CREATE UNIQUE INDEX one_result_per_run ON run_requirement_sources(run_id)
 WHERE kind='run_result_decided';
CREATE INDEX requirement_sources_frontier ON run_requirement_sources(run_id,kind,core_cursor);
CREATE TABLE run_results (
    run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    source_event_id TEXT NOT NULL UNIQUE REFERENCES run_requirement_sources(source_event_id) ON DELETE CASCADE,
    result_json TEXT NOT NULL,
    result_digest TEXT NOT NULL
);
CREATE TRIGGER requirement_sources_immutable_update BEFORE UPDATE ON run_requirement_sources
 BEGIN SELECT RAISE(ABORT,'requirement source is immutable'); END;
CREATE TRIGGER requirement_sources_immutable_delete BEFORE DELETE ON run_requirement_sources
 WHEN EXISTS(SELECT 1 FROM runs r JOIN conversations c ON c.id=r.conversation_id
             WHERE r.id=OLD.run_id AND c.id=OLD.conversation_id)
 BEGIN SELECT RAISE(ABORT,'requirement source is immutable'); END;
CREATE TRIGGER run_results_immutable_update BEFORE UPDATE ON run_results
 BEGIN SELECT RAISE(ABORT,'Run result is immutable'); END;
CREATE TRIGGER run_results_immutable_delete BEFORE DELETE ON run_results
 WHEN EXISTS(SELECT 1 FROM runs r JOIN conversations c ON c.id=r.conversation_id WHERE r.id=OLD.run_id)
 BEGIN SELECT RAISE(ABORT,'Run result is immutable'); END;
