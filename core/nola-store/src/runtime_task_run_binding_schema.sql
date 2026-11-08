-- Intentionally no Run/conversation FK: deletion must not erase the fact that
-- this task has already been admitted and authorize repeating its execution.
CREATE TABLE IF NOT EXISTS runtime_task_run_bindings (
    task_id TEXT PRIMARY KEY,
    owner_session_id TEXT NOT NULL,
    child_session_id TEXT NOT NULL,
    run_id TEXT NOT NULL UNIQUE,
    admission_attempt_id TEXT NOT NULL,
    input_watermark INTEGER,
    bound_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS runtime_task_run_bindings_child ON runtime_task_run_bindings(child_session_id);
CREATE TRIGGER IF NOT EXISTS immutable_runtime_task_run_binding BEFORE UPDATE ON runtime_task_run_bindings
BEGIN SELECT RAISE(ABORT, 'task Run admission is immutable'); END;
CREATE TRIGGER IF NOT EXISTS retain_runtime_task_run_binding BEFORE DELETE ON runtime_task_run_bindings
BEGIN SELECT RAISE(ABORT, 'task Run admission must be retained'); END;

CREATE TABLE IF NOT EXISTS runtime_task_terminal_receipts (
    task_id TEXT PRIMARY KEY,
    receipt_id TEXT NOT NULL UNIQUE,
    receipt_json TEXT NOT NULL CHECK(json_valid(receipt_json))
);
CREATE TRIGGER IF NOT EXISTS immutable_runtime_task_terminal_receipt BEFORE UPDATE ON runtime_task_terminal_receipts
BEGIN SELECT RAISE(ABORT, 'task terminal receipt is immutable'); END;
CREATE TRIGGER IF NOT EXISTS retain_runtime_task_terminal_receipt BEFORE DELETE ON runtime_task_terminal_receipts
BEGIN SELECT RAISE(ABORT, 'task terminal receipt must be retained'); END;

CREATE TABLE IF NOT EXISTS runtime_task_guidance (
    task_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    record_json TEXT NOT NULL CHECK(json_valid(record_json)),
    PRIMARY KEY(task_id,message_id)
);
CREATE TRIGGER IF NOT EXISTS immutable_runtime_task_guidance_identity BEFORE UPDATE ON runtime_task_guidance
WHEN NEW.task_id IS NOT OLD.task_id OR NEW.message_id IS NOT OLD.message_id
    OR json_extract(NEW.record_json,'$.content') IS NOT json_extract(OLD.record_json,'$.content')
    OR json_extract(NEW.record_json,'$.run_id') IS NOT json_extract(OLD.record_json,'$.run_id')
    OR json_extract(NEW.record_json,'$.owner_session_id') IS NOT json_extract(OLD.record_json,'$.owner_session_id')
    OR json_extract(NEW.record_json,'$.parent_core_session_id') IS NOT json_extract(OLD.record_json,'$.parent_core_session_id')
    OR json_extract(NEW.record_json,'$.parent_turn_id') IS NOT json_extract(OLD.record_json,'$.parent_turn_id')
    OR json_extract(NEW.record_json,'$.child_session_id') IS NOT json_extract(OLD.record_json,'$.child_session_id')
    OR json_extract(NEW.record_json,'$.input_watermark') IS NOT json_extract(OLD.record_json,'$.input_watermark')
    OR json_extract(NEW.record_json,'$.submitted_at_ms') IS NOT json_extract(OLD.record_json,'$.submitted_at_ms')
    OR json_extract(OLD.record_json,'$.status') IN ('delivered','rejected')
BEGIN SELECT RAISE(ABORT, 'guidance identity and final result are immutable'); END;
CREATE TRIGGER IF NOT EXISTS retain_runtime_task_guidance BEFORE DELETE ON runtime_task_guidance
BEGIN SELECT RAISE(ABORT, 'guidance result must be retained'); END;
