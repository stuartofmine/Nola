-- Exact accepted Turn evidence, never reconstructed from checkpoint/Part bodies.
CREATE TABLE assistant_turn_message_bindings(
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    turn_id TEXT NOT NULL,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    admission INTEGER NOT NULL CHECK(admission IN (0,1)),
    PRIMARY KEY(run_id,turn_id,message_id)
);
CREATE UNIQUE INDEX assistant_turn_single_admission
    ON assistant_turn_message_bindings(run_id) WHERE admission=1;
CREATE INDEX assistant_turn_exact_owner
    ON assistant_turn_message_bindings(conversation_id,run_id,turn_id,admission DESC,message_id);
CREATE TABLE assistant_turn_part_bindings(
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    part_id TEXT NOT NULL REFERENCES message_parts(id) ON DELETE CASCADE,
    turn_id TEXT NOT NULL,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    PRIMARY KEY(run_id,part_id)
);
-- Allocation is independent of live heads; removal never returns a position.
CREATE TABLE assistant_journal_order_counters(
    owner TEXT NOT NULL,
    section INTEGER NOT NULL CHECK(section BETWEEN 0 AND 3),
    next_position INTEGER NOT NULL CHECK(next_position>0),
    PRIMARY KEY(owner,section)
);
CREATE TABLE assistant_journal_order_reservations(
    owner TEXT NOT NULL,
    part_id TEXT NOT NULL,
    section INTEGER NOT NULL CHECK(section BETWEEN 0 AND 3),
    position INTEGER NOT NULL CHECK(position>0),
    PRIMARY KEY(owner,part_id),
    UNIQUE(owner,section,position)
);

-- Exact Core declaration, retained across delivery rebinding and model attempts.
CREATE TABLE assistant_effect_message_bindings(
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    effect_id TEXT NOT NULL PRIMARY KEY,
    turn_id TEXT NOT NULL,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE
);

CREATE TABLE assistant_message_segments(
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    turn_id TEXT NOT NULL,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    source_sequence INTEGER NOT NULL CHECK(source_sequence>0),
    PRIMARY KEY(run_id,message_id), UNIQUE(run_id,source_sequence)
);

CREATE TABLE assistant_activity_message_bindings(
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    turn_id TEXT NOT NULL,
    activity_kind TEXT NOT NULL CHECK(activity_kind IN ('task','skill')),
    activity_id TEXT NOT NULL,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    PRIMARY KEY(run_id,activity_kind,activity_id)
);
