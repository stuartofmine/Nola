-- SPDX-License-Identifier: Apache-2.0
-- Derived pointers only; Core and normalized Host events own all receipt data.
CREATE TABLE execution_evidence_sources (
    evidence_id TEXT NOT NULL,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    core_session_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    call_id TEXT NOT NULL,
    core_revision INTEGER NOT NULL CHECK(core_revision>0),
    core_cursor INTEGER NOT NULL CHECK(core_cursor>0),
    source_event_id TEXT NOT NULL UNIQUE REFERENCES run_events(event_id) ON DELETE CASCADE,
    source_digest TEXT NOT NULL,
    host_event_id TEXT NOT NULL UNIQUE REFERENCES host_event_inbox(event_id) ON DELETE CASCADE,
    host_reduction_digest TEXT NOT NULL,
    PRIMARY KEY(run_id,evidence_id)
);
CREATE INDEX execution_evidence_by_owner ON execution_evidence_sources(conversation_id,run_id,core_cursor);
