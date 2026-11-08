-- A Run seal freezes its completion frontier; later background presentation
-- records may advance live heads without changing these immutable facts.
CREATE TABLE IF NOT EXISTS assistant_run_terminal_seals(
 run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
 conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
 transition_id TEXT NOT NULL UNIQUE REFERENCES committed_transitions(transition_id) ON DELETE CASCADE,
 seal_json TEXT NOT NULL, seal_digest TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS assistant_run_terminal_manifest(
 run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
 ordinal INTEGER NOT NULL CHECK(ordinal>=0), entry_json TEXT NOT NULL,
 PRIMARY KEY(run_id,ordinal)
);
CREATE TRIGGER IF NOT EXISTS assistant_run_seal_immutable BEFORE UPDATE ON assistant_run_terminal_seals
BEGIN SELECT RAISE(ABORT,'assistant Run terminal seal is immutable'); END;
CREATE TRIGGER IF NOT EXISTS assistant_run_manifest_immutable BEFORE UPDATE ON assistant_run_terminal_manifest
BEGIN SELECT RAISE(ABORT,'assistant Run terminal manifest is immutable'); END;
CREATE TRIGGER IF NOT EXISTS assistant_run_manifest_closed BEFORE INSERT ON assistant_run_terminal_manifest
WHEN EXISTS(SELECT 1 FROM assistant_run_terminal_seals WHERE run_id=NEW.run_id)
BEGIN SELECT RAISE(ABORT,'assistant Run terminal manifest is sealed'); END;
CREATE TRIGGER IF NOT EXISTS assistant_run_terminal_status_immutable BEFORE UPDATE OF status,completed_at_ms ON runs
WHEN EXISTS(SELECT 1 FROM assistant_run_terminal_seals WHERE run_id=OLD.id)
 AND (NEW.status IS NOT OLD.status OR NEW.completed_at_ms IS NOT OLD.completed_at_ms)
BEGIN SELECT RAISE(ABORT,'sealed Run terminal cannot change'); END;

CREATE TABLE IF NOT EXISTS remote_assistant_run_terminal_stages (
 run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
 source_host_id TEXT NOT NULL, seal_json TEXT NOT NULL,
 next_entry INTEGER NOT NULL DEFAULT 0 CHECK(next_entry>=0),
 manifest_complete INTEGER NOT NULL DEFAULT 0 CHECK(manifest_complete IN (0,1)),
 committed INTEGER NOT NULL DEFAULT 0 CHECK(committed IN (0,1))
);
CREATE TABLE IF NOT EXISTS remote_assistant_run_terminal_entries (
 run_id TEXT NOT NULL REFERENCES remote_assistant_run_terminal_stages(run_id) ON DELETE CASCADE,
 ordinal INTEGER NOT NULL CHECK(ordinal>=0), entry_json TEXT NOT NULL,
 PRIMARY KEY(run_id,ordinal)
);
CREATE TRIGGER IF NOT EXISTS remote_run_terminal_seal_immutable BEFORE UPDATE OF source_host_id,seal_json ON remote_assistant_run_terminal_stages
BEGIN SELECT RAISE(ABORT,'remote terminal seal is immutable'); END;
CREATE TRIGGER IF NOT EXISTS remote_run_terminal_entries_immutable BEFORE UPDATE ON remote_assistant_run_terminal_entries
BEGIN SELECT RAISE(ABORT,'remote terminal manifest is immutable'); END;
CREATE TRIGGER IF NOT EXISTS remote_run_terminal_entries_closed BEFORE INSERT ON remote_assistant_run_terminal_entries
WHEN EXISTS(SELECT 1 FROM remote_assistant_run_terminal_stages WHERE run_id=NEW.run_id AND manifest_complete=1)
BEGIN SELECT RAISE(ABORT,'remote terminal manifest is sealed'); END;
CREATE TRIGGER IF NOT EXISTS remote_run_terminal_status_immutable BEFORE UPDATE OF status,completed_at_ms ON runs
WHEN EXISTS(SELECT 1 FROM remote_assistant_run_terminal_stages WHERE run_id=OLD.id AND committed=1)
 AND (NEW.status IS NOT OLD.status OR NEW.completed_at_ms IS NOT OLD.completed_at_ms)
BEGIN SELECT RAISE(ABORT,'received sealed Run terminal cannot change'); END;
