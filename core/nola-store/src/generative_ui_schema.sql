CREATE TABLE ui_surfaces(
    identity_key TEXT PRIMARY KEY,
    authority_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    part_id TEXT NOT NULL REFERENCES message_parts(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK(revision > 0),
    deleted INTEGER NOT NULL CHECK(deleted IN (0,1)),
    state_json TEXT NOT NULL CHECK(length(CAST(state_json AS BLOB)) <= 1048576)
);
CREATE INDEX ui_surfaces_conversation ON ui_surfaces(authority_id,conversation_id,deleted);

-- Immutable versions are owned by the same Surface and disappear only when
-- its canonical conversation/part is explicitly deleted by the user.
CREATE TABLE ui_surface_revisions(
    identity_key TEXT NOT NULL REFERENCES ui_surfaces(identity_key) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK(revision > 0),
    state_json TEXT NOT NULL CHECK(length(CAST(state_json AS BLOB)) <= 1048576),
    PRIMARY KEY(identity_key, revision)
);

CREATE TABLE ui_tool_publications(
    effect_id TEXT PRIMARY KEY REFERENCES effect_outbox(effect_id) ON DELETE CASCADE,
    request_digest TEXT NOT NULL,
    result_json TEXT NOT NULL CHECK(length(CAST(result_json AS BLOB)) <= 8192)
);
