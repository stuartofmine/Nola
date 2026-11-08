-- SPDX-License-Identifier: Apache-2.0
-- Presentation and answer CAS indices of exact committed Core event facts.
CREATE TABLE user_input_wait_heads (
    wait_id TEXT PRIMARY KEY,
    core_session_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    turn_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision>0),
    state TEXT NOT NULL CHECK(state IN ('open','resolved','withdrawn')),
    response_schema_digest TEXT NOT NULL,
    source_event_id TEXT NOT NULL REFERENCES run_events(event_id),
    source_digest TEXT NOT NULL,
    core_revision INTEGER NOT NULL CHECK(core_revision>=0),
    core_cursor INTEGER NOT NULL CHECK(core_cursor>0)
);
CREATE INDEX user_input_wait_open_by_owner ON user_input_wait_heads(conversation_id,run_id,state,wait_id);
CREATE TABLE user_input_wait_responses (
    wait_id TEXT PRIMARY KEY REFERENCES user_input_wait_heads(wait_id) ON DELETE CASCADE,
    command_id TEXT NOT NULL,
    response_digest TEXT NOT NULL,
    resolved_revision INTEGER NOT NULL CHECK(resolved_revision>1),
    source_event_id TEXT NOT NULL REFERENCES run_events(event_id),
    UNIQUE(wait_id,command_id)
);
