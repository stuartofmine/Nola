-- No lease or worker data: runtime_task_attempts is the sole dynamic authority.
CREATE TABLE IF NOT EXISTS runtime_agent_task_authorities (
    -- Retain historical admission after task deletion, like Run bindings.
    task_id TEXT PRIMARY KEY,
    phase TEXT NOT NULL CHECK(phase IN ('prepared','admitted')),
    authority_revision INTEGER NOT NULL CHECK(authority_revision>=1),
    admission_json TEXT,
    CHECK((phase='prepared' AND authority_revision=1 AND admission_json IS NULL)
       OR (phase='admitted' AND authority_revision>=2 AND admission_json IS NOT NULL
           AND json_valid(admission_json) AND length(CAST(admission_json AS BLOB))<=32768))
);
CREATE TRIGGER IF NOT EXISTS immutable_admitted_agent_task_authority
BEFORE UPDATE ON runtime_agent_task_authorities
WHEN (OLD.phase='admitted' AND NEW.admission_json IS NOT OLD.admission_json) OR NEW.task_id<>OLD.task_id
    OR NEW.phase<>'admitted' OR NEW.authority_revision<>OLD.authority_revision+1
BEGIN SELECT RAISE(ABORT,'agent Task admission is immutable'); END;
-- The revision witnesses changes in the existing dynamic authority. No lease,
-- progress, permission or result data is copied into the immutable admission.
CREATE TRIGGER IF NOT EXISTS advance_agent_task_authority_revision
AFTER UPDATE ON durable_tasks
BEGIN
    UPDATE runtime_agent_task_authorities
    SET authority_revision=authority_revision+1
    WHERE task_id=NEW.id AND phase='admitted';
END;
CREATE TRIGGER IF NOT EXISTS retain_agent_task_authority
BEFORE DELETE ON runtime_agent_task_authorities
BEGIN SELECT RAISE(ABORT,'agent Task admission must be retained'); END;

-- The preparation source and execution snapshot are Core-owned facts. Worker
-- progress may update scheduling/observations, never rebind the originating call.
CREATE TRIGGER IF NOT EXISTS immutable_delegation_preparation
BEFORE UPDATE OF metadata_json ON durable_tasks
WHEN json_extract(OLD.metadata_json,'$.delegation_origin') IS NOT NULL
 AND (json_extract(NEW.metadata_json,'$.delegation_origin') IS NOT json_extract(OLD.metadata_json,'$.delegation_origin')
   OR json_extract(NEW.metadata_json,'$.delegated_invocation') IS NOT json_extract(OLD.metadata_json,'$.delegated_invocation'))
BEGIN SELECT RAISE(ABORT,'delegation preparation is immutable'); END;
