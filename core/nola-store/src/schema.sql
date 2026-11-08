CREATE TABLE conversations(
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    revision INTEGER NOT NULL,
    configuration_revision INTEGER NOT NULL DEFAULT 1 CHECK(configuration_revision > 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE conversation_settings(
    conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
    settings_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE code_cell_observations(
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    cell_id TEXT NOT NULL,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    parent_part_id TEXT NOT NULL REFERENCES message_parts(id) ON DELETE CASCADE,
    core_revision INTEGER NOT NULL CHECK(core_revision > 0),
    record_json TEXT NOT NULL,
    PRIMARY KEY(conversation_id, cell_id)
);
CREATE TABLE messages(
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    run_id TEXT,
    role TEXT NOT NULL CHECK(role IN ('user', 'assistant', 'system')),
    status TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    UNIQUE(conversation_id, ordinal)
);
CREATE TABLE message_parts(
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    state TEXT NOT NULL,
    content_text TEXT NOT NULL DEFAULT '',
    content_json TEXT,
    content_digest TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(message_id, ordinal)
);
-- Immutable bodies may be prepared before their message publication. They have
-- no lifecycle cascade: reclaiming unreferenced content requires explicit GC.
CREATE TABLE canonical_text_objects(
    id TEXT PRIMARY KEY,
    reference_json TEXT NOT NULL CHECK(length(CAST(reference_json AS BLOB)) <= 4096)
);
CREATE TABLE canonical_text_chunks(
    content_id TEXT NOT NULL REFERENCES canonical_text_objects(id) ON DELETE CASCADE,
    offset_bytes INTEGER NOT NULL CHECK(offset_bytes >= 0),
    text TEXT NOT NULL CHECK(length(CAST(text AS BLOB)) BETWEEN 1 AND 32768),
    PRIMARY KEY(content_id, offset_bytes)
);
CREATE TABLE message_part_body_references(
    part_id TEXT NOT NULL REFERENCES message_parts(id) ON DELETE CASCADE,
    field TEXT NOT NULL CHECK(field IN ('content_text','content_json')),
    reference_json TEXT NOT NULL CHECK(length(CAST(reference_json AS BLOB)) <= 4096),
    PRIMARY KEY(part_id,field)
);
-- Exact original AgentItems, not a second visible Part or a checkpoint copy.
-- The source identity and owner cannot be rebound when a Message is handed off.
CREATE TABLE canonical_tool_sources(
    source_item_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    run_id TEXT REFERENCES runs(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    part_id TEXT NOT NULL REFERENCES message_parts(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('tool_call','tool_result')),
    binding_json TEXT NOT NULL CHECK(length(CAST(binding_json AS BLOB)) <= 16384),
    PRIMARY KEY(conversation_id,source_item_id),
    UNIQUE(part_id,kind)
);
CREATE TRIGGER immutable_tool_source BEFORE UPDATE ON canonical_tool_sources
BEGIN SELECT RAISE(ABORT,'tool source binding is immutable'); END;
CREATE INDEX canonical_tool_source_identity ON canonical_tool_sources(source_item_id);
-- Branch-owned provenance is retained with the target, independently of source lifetime.
CREATE TABLE message_part_derivations(
    target_part_id TEXT PRIMARY KEY REFERENCES message_parts(id) ON DELETE CASCADE,
    operation_id TEXT NOT NULL CHECK(length(CAST(operation_id AS BLOB)) BETWEEN 1 AND 128),
    derivation_json TEXT NOT NULL CHECK(length(CAST(derivation_json AS BLOB)) <= 16384),
    provenance TEXT NOT NULL CHECK(provenance IN ('local_branch','replica','archive'))
);
CREATE TRIGGER immutable_part_derivation BEFORE UPDATE ON message_part_derivations
BEGIN SELECT RAISE(ABORT,'branch Part derivation is immutable'); END;
CREATE TABLE branch_fact_receipts(
    operation_id TEXT PRIMARY KEY CHECK(length(CAST(operation_id AS BLOB)) BETWEEN 1 AND 200),
    entity_type TEXT NOT NULL CHECK(entity_type IN ('part_derivation','conversation_branch_lineage')),
    entity_id TEXT NOT NULL CHECK(length(CAST(entity_id AS BLOB)) BETWEEN 1 AND 128),
    canonical_sha256 TEXT NOT NULL CHECK(length(canonical_sha256)=64)
);
CREATE TABLE canonical_text_replica_chunks(
    content_id TEXT NOT NULL,
    offset_bytes INTEGER NOT NULL CHECK(offset_bytes >= 0),
    reference_json TEXT NOT NULL CHECK(length(CAST(reference_json AS BLOB)) <= 4096),
    text TEXT NOT NULL CHECK(length(CAST(text AS BLOB)) BETWEEN 1 AND 32768),
    PRIMARY KEY(content_id, offset_bytes)
);
CREATE TABLE transcript_import_publications(
    import_id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    manifest_json TEXT NOT NULL,
    result_json TEXT NOT NULL
);
CREATE TABLE context_checkpoints(
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    run_id TEXT,
    model_profile_id TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    covered_through_message_id TEXT,
    covered_through_sequence INTEGER,
    summary_part_id TEXT,
    retained_tail_start_message_id TEXT,
    estimated_tokens INTEGER NOT NULL,
    measured_tokens INTEGER,
    context_window_tokens INTEGER NOT NULL,
    prompt_limit_tokens INTEGER NOT NULL,
    payload_digest TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE TABLE context_components(
    id TEXT PRIMARY KEY,
    checkpoint_id TEXT NOT NULL REFERENCES context_checkpoints(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    source_id TEXT,
    source_uri TEXT,
    title TEXT,
    mime_type TEXT,
    authorization TEXT NOT NULL CHECK(authorization IN(
        'not_determined','authorized','denied','restricted','unavailable','not_required'
    )),
    observed_at_ms INTEGER,
    maximum_age_minutes INTEGER,
    freshness TEXT NOT NULL CHECK(freshness IN('fresh','stale','unknown')),
    content_reference_json TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    estimated_tokens INTEGER NOT NULL,
    provenance_json TEXT NOT NULL
);
CREATE TABLE runs(
    id TEXT PRIMARY KEY,
    active_attempt_id TEXT REFERENCES run_attempts(id) DEFERRABLE INITIALLY DEFERRED,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN('conversation','maintenance')),
    user_message_id TEXT REFERENCES messages(id),
    assistant_message_id TEXT REFERENCES messages(id),
    context_checkpoint_id TEXT REFERENCES context_checkpoints(id),
    invocation_config_digest TEXT,
    status TEXT NOT NULL,
    next_event_sequence INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER
);
CREATE TABLE run_invocation_payloads(
    run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    config_json TEXT NOT NULL
);
CREATE TABLE delegated_invocations(
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    effect_id TEXT NOT NULL UNIQUE,
    backend_id TEXT NOT NULL,
    backend_session_id TEXT,
    text_part_id TEXT NOT NULL,
    reasoning_part_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('prepared','running','waiting_permission','interrupted','cancelling','succeeded','failed','cancelled')),
    revision INTEGER NOT NULL CHECK(revision > 0),
    record_json TEXT NOT NULL CHECK(length(CAST(record_json AS BLOB)) <= 65536),
    UNIQUE(run_id,text_part_id),
    UNIQUE(run_id,reasoning_part_id),
    CHECK(text_part_id != reasoning_part_id)
);
CREATE INDEX delegated_invocation_run_state ON delegated_invocations(run_id,state);
CREATE TABLE delegated_invocation_deliveries(
    request_id TEXT PRIMARY KEY,
    invocation_id TEXT NOT NULL REFERENCES delegated_invocations(id) ON DELETE CASCADE
);
CREATE TABLE delegated_permission_deliveries(
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    invocation_id TEXT NOT NULL REFERENCES delegated_invocations(id) ON DELETE CASCADE,
    target_effect_id TEXT NOT NULL,
    occurrence_id TEXT NOT NULL,
    control_effect_id TEXT UNIQUE,
    control_request_id TEXT UNIQUE,
    state TEXT NOT NULL CHECK(state IN ('awaiting_decision','prepared','claimed','transport_written','outcome_unknown','interrupted')),
    revision INTEGER NOT NULL CHECK(revision > 0),
    record_json TEXT NOT NULL CHECK(length(CAST(record_json AS BLOB)) <= 65536),
    UNIQUE(invocation_id,occurrence_id),
    CHECK((control_effect_id IS NULL) = (control_request_id IS NULL))
);
CREATE INDEX delegated_permission_open_target ON delegated_permission_deliveries(target_effect_id,id) WHERE state IN ('awaiting_decision','prepared','claimed');
CREATE INDEX delegated_permission_run_state ON delegated_permission_deliveries(run_id,state);
CREATE TRIGGER delegated_permission_target_owner BEFORE INSERT ON delegated_permission_deliveries
WHEN NOT EXISTS(SELECT 1 FROM delegated_invocations d WHERE d.id=NEW.invocation_id AND d.run_id=NEW.run_id AND d.effect_id=NEW.target_effect_id)
BEGIN SELECT RAISE(ABORT,'delegated permission target ownership mismatch'); END;
CREATE TABLE auxiliary_operations(
    id TEXT PRIMARY KEY,
    record_json TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0)
);
CREATE INDEX auxiliary_operations_conversation_owner ON auxiliary_operations(
    json_extract(record_json,'$.owner.conversation_id'))
    WHERE json_extract(record_json,'$.owner.kind')='conversation';

-- Observed source receipts only: never insert these into run_attempts again.
CREATE TABLE auxiliary_source_observations(
    local_owner_json TEXT NOT NULL,
    source_host_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    record_json TEXT NOT NULL,
    revision INTEGER NOT NULL,
    PRIMARY KEY(local_owner_json, source_host_id, operation_id),
    UNIQUE(source_host_id, operation_id)
);
CREATE INDEX auxiliary_source_conversation_owner ON auxiliary_source_observations(
    json_extract(local_owner_json,'$.conversation_id'))
    WHERE json_extract(local_owner_json,'$.kind')='conversation';
CREATE INDEX auxiliary_source_conversation_unreported ON auxiliary_source_observations(
    json_extract(local_owner_json,'$.conversation_id'),source_host_id,operation_id)
    WHERE json_extract(local_owner_json,'$.kind')='conversation'
      AND (json_extract(record_json,'$.receipt.operation.state') IN ('ready','running')
        OR COALESCE(json_extract(record_json,'$.receipt.attempt.usage.coverage'),'unknown')<>'known');
-- One unresolved worker-scope slot, not content deduplication. Different inputs
-- cannot bypass an uncertain earlier physical request by inventing a new UUID.
CREATE UNIQUE INDEX auxiliary_source_unresolved_slot ON auxiliary_source_observations(
    local_owner_json,json_extract(record_json,'$.receipt.operation.scenario'))
    WHERE json_extract(record_json,'$.receipt.operation.state') IN ('ready','running');
CREATE TABLE run_attempts(
    id TEXT PRIMARY KEY,
    run_id TEXT REFERENCES runs(id) ON DELETE CASCADE,
    operation_id TEXT REFERENCES auxiliary_operations(id) ON DELETE CASCADE,
    invocation_id TEXT NOT NULL,
    effect_id TEXT NOT NULL,
    text_part_id TEXT,
    reasoning_part_id TEXT,
    predecessor_id TEXT UNIQUE REFERENCES run_attempts(id),
    state TEXT NOT NULL CHECK(state IN ('prepared','running','succeeded','failed','cancelled','superseded','interrupted')),
    revision INTEGER NOT NULL CHECK(revision > 0),
    record_json TEXT NOT NULL CHECK(length(CAST(record_json AS BLOB)) <= 65536),
    UNIQUE(run_id,text_part_id),
    UNIQUE(run_id,reasoning_part_id),
    CHECK((run_id IS NOT NULL AND operation_id IS NULL AND text_part_id IS NOT NULL AND reasoning_part_id IS NOT NULL AND text_part_id != reasoning_part_id)
       OR (run_id IS NULL AND operation_id IS NOT NULL AND text_part_id IS NULL AND reasoning_part_id IS NULL))
);
CREATE INDEX auxiliary_attempt_operation ON run_attempts(operation_id,id) WHERE operation_id IS NOT NULL;
CREATE UNIQUE INDEX run_attempt_current_effect ON run_attempts(effect_id)
    WHERE state IN ('prepared','running','interrupted');
CREATE INDEX run_attempt_effect_chain ON run_attempts(effect_id,predecessor_id);
CREATE TABLE run_attempt_deliveries(
    request_id TEXT PRIMARY KEY,
    attempt_id TEXT NOT NULL REFERENCES run_attempts(id) ON DELETE CASCADE
);
CREATE TRIGGER delegated_delivery_excludes_attempt BEFORE INSERT ON delegated_invocation_deliveries
WHEN EXISTS (SELECT 1 FROM run_attempt_deliveries WHERE request_id=NEW.request_id)
BEGIN SELECT RAISE(ABORT,'delivery already owns a model Attempt'); END;
CREATE TRIGGER attempt_delivery_excludes_delegated BEFORE INSERT ON run_attempt_deliveries
WHEN EXISTS (SELECT 1 FROM delegated_invocation_deliveries WHERE request_id=NEW.request_id)
BEGIN SELECT RAISE(ABORT,'delivery already owns a delegated invocation'); END;
CREATE TRIGGER run_active_attempt_owner BEFORE UPDATE OF active_attempt_id ON runs
WHEN NEW.active_attempt_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM run_attempts WHERE id=NEW.active_attempt_id AND run_id=NEW.id
    AND state IN ('prepared','running','interrupted'))
BEGIN SELECT RAISE(ABORT,'active Attempt does not belong to Run'); END;
CREATE TABLE run_events(
    event_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    event_kind TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    UNIQUE(run_id, sequence)
);
CREATE TABLE host_event_inbox(
    event_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    effect_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    payload_json TEXT NOT NULL,
    reduction_json TEXT,
    received_at_ms INTEGER NOT NULL,
    UNIQUE(run_id, effect_id, sequence)
);
CREATE TABLE effect_outbox(
    effect_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    request_id TEXT NOT NULL UNIQUE,
    effect_kind TEXT NOT NULL,
    binding_json TEXT,
    payload_digest TEXT NOT NULL,
    recovery_required INTEGER NOT NULL DEFAULT 0,
    target_effect_id TEXT,
    status TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    acknowledged_at_ms INTEGER
);
CREATE INDEX effect_outbox_pending_delivery
    ON effect_outbox(run_id,status,created_at_ms,effect_id);
CREATE INDEX effect_outbox_runtime_work
    ON effect_outbox(status,created_at_ms,run_id,effect_id)
    WHERE status='pending';
CREATE TABLE effect_outbox_payloads(
    effect_id TEXT PRIMARY KEY REFERENCES effect_outbox(effect_id) ON DELETE CASCADE,
    payload_json TEXT NOT NULL
);
CREATE TABLE effect_delivery_attempts(
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    effect_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    predecessor_id TEXT UNIQUE REFERENCES effect_delivery_attempts(id),
    attempt_number INTEGER NOT NULL CHECK(attempt_number > 0),
    replay_safety TEXT NOT NULL CHECK(replay_safety IN ('idempotent','recoverable','requires_recovery_decision')),
    state TEXT NOT NULL CHECK(state IN ('prepared','leased','execution_started','settled','interrupted','outcome_unknown')),
    recovery_plan TEXT CHECK(recovery_plan IN ('retry','resume')),
    worker_generation TEXT,
    lease_owner TEXT,
    lease_token TEXT,
    lease_expires_at_ms INTEGER,
    revision INTEGER NOT NULL CHECK(revision > 0),
    record_json TEXT NOT NULL CHECK(length(CAST(record_json AS BLOB)) <= 65536),
    UNIQUE(effect_id,attempt_number)
);
CREATE UNIQUE INDEX effect_delivery_one_active_per_effect
    ON effect_delivery_attempts(effect_id)
    WHERE state IN ('prepared','leased','execution_started');
CREATE INDEX effect_delivery_active_owner
    ON effect_delivery_attempts(lease_owner,lease_expires_at_ms,id)
    WHERE state IN ('leased','execution_started');
CREATE INDEX effect_delivery_run_recovery
    ON effect_delivery_attempts(run_id,state,id)
    WHERE state IN ('prepared','leased','execution_started','interrupted','outcome_unknown');
CREATE INDEX effect_delivery_interrupted_candidate
    ON effect_delivery_attempts(run_id,state,recovery_plan,id)
    WHERE state='interrupted';
CREATE TABLE core_checkpoints(
    conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL,
    event_cursor INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE core_checkpoint_bodies(
    conversation_id TEXT PRIMARY KEY REFERENCES core_checkpoints(conversation_id) ON DELETE CASCADE,
    checkpoint_json TEXT NOT NULL
);
CREATE TABLE committed_transitions(
    transition_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    checkpoint_revision INTEGER NOT NULL,
    next_event_sequence INTEGER NOT NULL,
    committed_at_ms INTEGER NOT NULL
);
CREATE TABLE attachments(
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    display_name TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    blob_digest TEXT NOT NULL,
    byte_count INTEGER NOT NULL,
    source_uri TEXT,
    created_at_ms INTEGER NOT NULL
);
CREATE TABLE conversation_pdf_resources(
    resource_id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    display_name TEXT NOT NULL,
    mime_type TEXT NOT NULL CHECK(mime_type='application/pdf'),
    byte_count INTEGER NOT NULL CHECK(byte_count>0 AND byte_count<=8388608),
    sha256 TEXT NOT NULL,
    body BLOB NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX conversation_pdf_resources_owner ON conversation_pdf_resources(conversation_id,created_at_ms);
CREATE TABLE artifacts(
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    message_id TEXT REFERENCES messages(id) ON DELETE SET NULL,
    display_name TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    blob_digest TEXT NOT NULL,
    byte_count INTEGER NOT NULL,
    provenance_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_messages_conversation_ordinal
    ON messages(conversation_id, ordinal);
CREATE INDEX idx_run_events_run_sequence
    ON run_events(run_id, sequence);
CREATE INDEX idx_outbox_pending
    ON effect_outbox(status, created_at_ms) WHERE status = 'pending';
CREATE INDEX idx_outbox_pending_run
    ON effect_outbox(run_id,effect_id) WHERE status = 'pending';
CREATE INDEX idx_context_checkpoints_conversation
    ON context_checkpoints(conversation_id, created_at_ms);
CREATE INDEX idx_context_components_checkpoint
    ON context_components(checkpoint_id, id);
CREATE TABLE remote_session_mirrors(
    conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
    run_id TEXT REFERENCES runs(id) ON DELETE CASCADE,
    source_host_id TEXT NOT NULL,
    core_revision INTEGER NOT NULL,
    event_cursor INTEGER NOT NULL,
    snapshot_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE remote_conversation_projections(
    conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
    source_host_id TEXT NOT NULL,
    contract_version INTEGER NOT NULL,
    revision INTEGER NOT NULL,
    event_cursor INTEGER NOT NULL,
    projection_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE committed_remote_transitions(
    transition_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    next_event_sequence INTEGER NOT NULL,
    committed_at_ms INTEGER NOT NULL
);
CREATE TABLE permission_grants(
    id TEXT PRIMARY KEY,
    scope TEXT NOT NULL CHECK(scope IN ('user_global', 'host', 'conversation')),
    conversation_id TEXT REFERENCES conversations(id) ON DELETE CASCADE,
    host_id TEXT,
    capability_pattern TEXT NOT NULL,
    resource_pattern TEXT NOT NULL,
    decision TEXT NOT NULL CHECK(decision IN ('allow', 'deny')),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    revoked_at_ms INTEGER,
    CHECK(
        (scope = 'user_global' AND conversation_id IS NULL AND host_id IS NULL) OR
        (scope = 'host' AND conversation_id IS NULL AND host_id IS NOT NULL) OR
        (scope = 'conversation' AND conversation_id IS NOT NULL AND host_id IS NULL)
    )
);
CREATE INDEX idx_permission_grants_effective
    ON permission_grants(scope, conversation_id, host_id, updated_at_ms)
    WHERE revoked_at_ms IS NULL;
CREATE TABLE skill_definitions(
    id TEXT PRIMARY KEY,
    source TEXT NOT NULL CHECK(source IN ('built_in', 'user')),
    display_name TEXT NOT NULL,
    summary TEXT NOT NULL,
    category TEXT NOT NULL,
    current_version TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    deleted_at_ms INTEGER
, revision INTEGER NOT NULL DEFAULT 1);
CREATE TABLE skill_versions(
    skill_id TEXT NOT NULL REFERENCES skill_definitions(id) ON DELETE CASCADE,
    version TEXT NOT NULL,
    instructions_ref TEXT NOT NULL,
    instructions_digest TEXT NOT NULL,
    instructions_text TEXT NOT NULL,
    required_tools_json TEXT,
    default_mode TEXT CHECK(default_mode IS NULL OR default_mode IN ('inline', 'fork')),
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(skill_id, version)
);
CREATE INDEX idx_skill_definitions_active
    ON skill_definitions(enabled, category, display_name)
    WHERE deleted_at_ms IS NULL;
CREATE TABLE skill_instruction_selections(
    skill_id TEXT PRIMARY KEY REFERENCES skill_definitions(id) ON DELETE CASCADE,
    package_version TEXT NOT NULL,
    override_version TEXT,
    FOREIGN KEY(skill_id, package_version) REFERENCES skill_versions(skill_id, version),
    FOREIGN KEY(skill_id, override_version) REFERENCES skill_versions(skill_id, version)
);
CREATE TABLE durable_tasks(
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    title TEXT NOT NULL,
    project_root TEXT NOT NULL,
    owner_session_id TEXT,
    parent_task_id TEXT REFERENCES durable_tasks(id) ON DELETE SET NULL,
    parent_tool_call_id TEXT,
    child_session_id TEXT,
    agent_type TEXT,
    prompt TEXT,
    progress_summary TEXT,
    last_activity TEXT,
    result_summary TEXT,
    error TEXT,
    output_path TEXT,
    unread INTEGER NOT NULL DEFAULT 0 CHECK(unread IN (0,1)),
    cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK(cancel_requested IN (0,1)),
    cancel_requested_at_ms INTEGER,
    worker_id TEXT,
    ownership_token TEXT,
    lease_until_ms INTEGER,
    heartbeat_at_ms INTEGER,
    retry_count INTEGER NOT NULL DEFAULT 0,
    retry_at_ms INTEGER,
    last_error TEXT,
    input_watermark INTEGER,
    completion_watermark INTEGER,
    created_at_ms INTEGER NOT NULL,
    started_at_ms INTEGER,
    ended_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL,
    metadata_json TEXT NOT NULL
);
CREATE INDEX idx_durable_tasks_project ON durable_tasks(project_root,updated_at_ms DESC);
CREATE INDEX idx_durable_tasks_claim ON durable_tasks(status,retry_at_ms,lease_until_ms);
CREATE INDEX idx_durable_tasks_session ON durable_tasks(owner_session_id,updated_at_ms DESC);
CREATE TABLE runtime_task_attempts(
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES durable_tasks(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL CHECK(attempt_number > 0),
    worker_id TEXT NOT NULL,
    worker_generation TEXT NOT NULL,
    lease_token TEXT NOT NULL,
    lease_expires_at_ms INTEGER NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('leased','permission_suspended','superseded')),
    revision INTEGER NOT NULL CHECK(revision > 0),
    claimed_at_ms INTEGER NOT NULL,
    UNIQUE(task_id,attempt_number)
);
CREATE UNIQUE INDEX idx_runtime_task_attempts_active
    ON runtime_task_attempts(task_id) WHERE state='leased';
CREATE TABLE runtime_task_permission_suspensions(
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES durable_tasks(id) ON DELETE CASCADE,
    attempt_id TEXT NOT NULL UNIQUE REFERENCES runtime_task_attempts(id),
    permission_request_id TEXT NOT NULL UNIQUE REFERENCES permission_requests(request_id),
    continuation_id TEXT NOT NULL,
    continuation_json TEXT NOT NULL CHECK(json_valid(continuation_json) AND length(CAST(continuation_json AS BLOB)) <= 65536),
    state TEXT NOT NULL CHECK(state IN ('pending','resumed','settled','cancelled')),
    revision INTEGER NOT NULL CHECK(revision > 0),
    suspended_at_ms INTEGER NOT NULL,
    resumed_at_ms INTEGER
);
CREATE UNIQUE INDEX idx_runtime_task_permission_pending
    ON runtime_task_permission_suspensions(task_id) WHERE state='pending';
CREATE TABLE durable_task_events(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL REFERENCES durable_tasks(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    summary TEXT,
    detail_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_durable_task_events_task ON durable_task_events(task_id,id DESC);
CREATE TABLE memory_records(
    id TEXT PRIMARY KEY,
    owner_json TEXT NOT NULL CHECK(json_valid(owner_json)),
    namespace_json TEXT NOT NULL,
    scope TEXT NOT NULL,
    kind TEXT NOT NULL,
    content TEXT NOT NULL,
    summary TEXT,
    tags_json TEXT NOT NULL,
    confidence REAL NOT NULL CHECK(confidence>=0 AND confidence<=1),
    importance REAL NOT NULL CHECK(importance>=0 AND importance<=1),
    sensitivity TEXT,
    retention_json TEXT NOT NULL,
    expires_at TEXT,
    supersedes_json TEXT NOT NULL,
    provenance_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    last_used_at_ms INTEGER,
    deleted_at_ms INTEGER
, revision INTEGER NOT NULL DEFAULT 1);
CREATE INDEX idx_memory_records_recall
    ON memory_records(owner_json,namespace_json,scope,kind,importance DESC,confidence DESC,updated_at_ms DESC)
    WHERE deleted_at_ms IS NULL;
CREATE INDEX idx_memory_records_expiry ON memory_records(expires_at)
    WHERE deleted_at_ms IS NULL AND expires_at IS NOT NULL;
CREATE TABLE memory_usage_ledger(
    usage_id TEXT PRIMARY KEY,
    memory_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    used_at_ms INTEGER NOT NULL CHECK(used_at_ms>=0),
    context_payload_digest TEXT NOT NULL,
    citation_ids_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    FOREIGN KEY(memory_id) REFERENCES memory_records(id) ON DELETE CASCADE
);
CREATE INDEX idx_memory_usage_record
    ON memory_usage_ledger(memory_id,used_at_ms DESC,usage_id DESC);
CREATE TABLE automation_definitions(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    schedule_format TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
    revision INTEGER NOT NULL,
    metadata_json TEXT NOT NULL,
    execution_spec_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    deleted_at_ms INTEGER
);
CREATE INDEX idx_automation_definitions_active
    ON automation_definitions(enabled,updated_at_ms DESC) WHERE deleted_at_ms IS NULL;
CREATE TABLE automation_results(
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL,
    status TEXT NOT NULL,
    metadata_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_automation_results_source
    ON automation_results(source_id,created_at_ms DESC);
CREATE TABLE data_sources(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    location TEXT NOT NULL,
    access_policy TEXT NOT NULL,
    validation_contract_json TEXT NOT NULL,
    capabilities_json TEXT NOT NULL,
    privacy TEXT NOT NULL,
    revision INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    deleted_at_ms INTEGER
, enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN(0,1)), metadata_json TEXT NOT NULL DEFAULT '{}');
CREATE INDEX idx_data_sources_active ON data_sources(updated_at_ms DESC)
    WHERE deleted_at_ms IS NULL;
CREATE TABLE data_source_overrides(
    conversation_id TEXT NOT NULL,
    source_id TEXT NOT NULL REFERENCES data_sources(id) ON DELETE CASCADE,
    enabled INTEGER NOT NULL CHECK(enabled IN(0,1)),
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(conversation_id,source_id)
);
CREATE TABLE data_source_usage(
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL,
    source_title TEXT NOT NULL,
    consumer TEXT NOT NULL,
    purpose TEXT NOT NULL,
    conversation_id TEXT,
    disclosed_to_cloud INTEGER CHECK(disclosed_to_cloud IN(0,1)),
    metadata_json TEXT NOT NULL,
    used_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_data_source_usage_source ON data_source_usage(source_id,used_at_ms DESC);
CREATE INDEX idx_data_source_usage_conversation ON data_source_usage(conversation_id,used_at_ms DESC);
CREATE TABLE mcp_servers(
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK(enabled IN(0,1)),
    configuration_json TEXT NOT NULL,
    revision INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    deleted_at_ms INTEGER
);
CREATE INDEX idx_mcp_servers_active
    ON mcp_servers(enabled,display_name,id) WHERE deleted_at_ms IS NULL;
CREATE TABLE automation_runs(
    definition_revision INTEGER NOT NULL CHECK(definition_revision>0),
    id TEXT PRIMARY KEY,
    automation_id TEXT NOT NULL,
    occurrence_key TEXT NOT NULL,
    trigger TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN (
        'queued','running','retry_wait','reconciled','launch_failed'
    )),
    attempt INTEGER NOT NULL DEFAULT 0 CHECK(attempt>=0),
    worker_id TEXT,
    lease_token TEXT,
    lease_until_ms INTEGER,
    scheduled_for_ms INTEGER NOT NULL,
    started_at_ms INTEGER,
    finished_at_ms INTEGER,
    next_retry_at_ms INTEGER,
    result_json TEXT,
    error_json TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(automation_id,occurrence_key)
);
CREATE INDEX idx_automation_runs_claim
    ON automation_runs(status,next_retry_at_ms,lease_until_ms,scheduled_for_ms);
CREATE INDEX idx_automation_runs_definition
    ON automation_runs(automation_id,scheduled_for_ms DESC);
CREATE TABLE unknown_effect_recovery_receipts(
    resolution_id TEXT PRIMARY KEY NOT NULL,
    request_digest TEXT NOT NULL,
    receipt_json TEXT NOT NULL
);
CREATE TABLE automation_execution_bindings(
    occurrence_id TEXT PRIMARY KEY REFERENCES automation_runs(id),
    conversation_id TEXT NOT NULL UNIQUE REFERENCES conversations(id),
    run_id TEXT NOT NULL UNIQUE REFERENCES runs(id),
    binding_json TEXT NOT NULL,
    request_digest TEXT NOT NULL,
    accepted_json TEXT NOT NULL
);
CREATE TABLE automation_execution_receipts(
    occurrence_id TEXT PRIMARY KEY REFERENCES automation_execution_bindings(occurrence_id),
    notification_id TEXT NOT NULL UNIQUE,
    receipt_json TEXT NOT NULL,
    delivered_at_ms INTEGER
);
CREATE TABLE automation_worker_observations(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    occurrence_id TEXT NOT NULL REFERENCES automation_runs(id),
    worker_id TEXT NOT NULL,
    lease_token TEXT NOT NULL,
    observation_json TEXT NOT NULL,
    observed_at_ms INTEGER NOT NULL
);
CREATE TABLE conversation_projection_operations(
    operation_id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    operation_kind TEXT NOT NULL,
    parent_message_id TEXT,
    resulting_revision INTEGER NOT NULL,
    ledger_message_count INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_conversation_projection_revision
    ON conversation_projection_operations(conversation_id,resulting_revision DESC);
CREATE TABLE message_projection_events(
    event_id TEXT PRIMARY KEY,
    operation_id TEXT NOT NULL REFERENCES conversation_projection_operations(operation_id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    visible INTEGER NOT NULL CHECK(visible IN(0,1)),
    reason TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_message_projection_latest
    ON message_projection_events(conversation_id,message_id,created_at_ms DESC);
CREATE TABLE model_profiles(
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    provider TEXT NOT NULL,
    model_name TEXT NOT NULL,
    runtime TEXT NOT NULL,
    source_json TEXT NOT NULL,
    parameters_json TEXT NOT NULL,
    selected INTEGER NOT NULL CHECK(selected IN(0,1)),
    enabled INTEGER NOT NULL CHECK(enabled IN(0,1)),
    revision INTEGER NOT NULL CHECK(revision>0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    deleted_at_ms INTEGER
);
CREATE UNIQUE INDEX idx_model_profiles_selected
    ON model_profiles(provider,runtime) WHERE selected=1 AND enabled=1 AND deleted_at_ms IS NULL;
CREATE TABLE model_runtimes(
    host_id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL REFERENCES model_profiles(id),
    phase TEXT NOT NULL CHECK(phase IN('starting','ready','failed','stopping','stopped')),
    endpoint_json TEXT,
    process_id INTEGER,
    error TEXT,
    started_at_ms INTEGER,
    stopped_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_model_runtimes_profile ON model_runtimes(profile_id,updated_at_ms DESC);
CREATE TABLE conversation_plans(
    conversation_id TEXT PRIMARY KEY,
    sop_id TEXT NOT NULL CHECK(length(trim(sop_id))>0),
    sop_version INTEGER NOT NULL CHECK(sop_version>0),
    run_id TEXT NOT NULL UNIQUE CHECK(length(trim(run_id))>0),
    todos_json TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision>0),
    frozen_revision INTEGER CHECK(frozen_revision IS NULL OR frozen_revision>0),
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE conversation_plan_revisions(
    conversation_id TEXT NOT NULL REFERENCES conversation_plans(conversation_id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK(revision>0),
    sop_id TEXT NOT NULL CHECK(length(trim(sop_id))>0),
    sop_version INTEGER NOT NULL CHECK(sop_version>0),
    run_id TEXT NOT NULL CHECK(length(trim(run_id))>0),
    todos_json TEXT NOT NULL,
    change_kind TEXT NOT NULL CHECK(change_kind IN('save','approval','edit','freeze','replan','execution')),
    change_summary TEXT NOT NULL DEFAULT '',
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(conversation_id,revision)
);
CREATE INDEX idx_conversation_plan_revisions_updated
    ON conversation_plan_revisions(conversation_id,revision DESC);
CREATE TABLE document_sources(
    id TEXT PRIMARY KEY,
    scope TEXT NOT NULL,
    uri TEXT NOT NULL,
    kind TEXT NOT NULL,
    mime_type TEXT,
    sha256 TEXT,
    byte_count INTEGER,
    title TEXT,
    text_content TEXT,
    metadata_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_document_sources_scope_kind
    ON document_sources(scope,kind,updated_at_ms DESC,id);
CREATE TABLE permission_requests(
    request_id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES durable_tasks(id) ON DELETE CASCADE,
    child_session_id TEXT NOT NULL,
    capability TEXT NOT NULL,
    input_json TEXT NOT NULL,
    scope_rule TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN('pending','resolved','withdrawn')),
    decision TEXT CHECK(decision IS NULL OR decision IN(
        'allow_once','allow_conversation','return_to_composer','provide_instruction','deny'
    )),
    custom_response TEXT,
    core_permission_request_id TEXT,
    permission_instructions_json TEXT NOT NULL DEFAULT '[]' CHECK(length(CAST(permission_instructions_json AS BLOB)) <= 131072),
    created_at_ms INTEGER NOT NULL,
    resolved_at_ms INTEGER,
    closure_json TEXT,
    CHECK((status='withdrawn')=(closure_json IS NOT NULL)),
    CHECK(closure_json IS NULL OR (json_valid(closure_json) AND json_type(closure_json)='object')),
    CHECK(status!='pending' OR (decision IS NULL AND resolved_at_ms IS NULL AND custom_response IS NULL)),
    CHECK(status!='resolved' OR (decision IS NOT NULL AND resolved_at_ms IS NOT NULL)),
    CHECK(status!='withdrawn' OR ((decision IS NULL AND resolved_at_ms IS NULL AND custom_response IS NULL)
        OR (decision IS NOT NULL AND resolved_at_ms IS NOT NULL)))
);
CREATE TRIGGER permission_closed_immutable BEFORE UPDATE ON permission_requests
WHEN OLD.closure_json IS NOT NULL
BEGIN SELECT RAISE(ABORT,'closed permission is immutable'); END;
CREATE INDEX idx_permission_requests_task_pending
    ON permission_requests(task_id,status,created_at_ms,request_id);
CREATE UNIQUE INDEX idx_permission_core_identity ON permission_requests(task_id,core_permission_request_id) WHERE core_permission_request_id IS NOT NULL;

-- Historical permission authority survives delivery queue garbage collection.
CREATE TABLE permission_request_sources(
    request_id TEXT PRIMARY KEY REFERENCES permission_requests(request_id) ON DELETE CASCADE,
    proof_json TEXT NOT NULL CHECK(json_valid(proof_json) AND json_type(proof_json)='object')
);
CREATE UNIQUE INDEX idx_permission_source_effect
    ON permission_request_sources(json_extract(proof_json,'$.permission.run_id'),json_extract(proof_json,'$.permission.effect_id'));
CREATE TRIGGER permission_source_immutable BEFORE UPDATE ON permission_request_sources
BEGIN SELECT RAISE(ABORT,'permission source is immutable'); END;

CREATE TABLE agent_commands(
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    command_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    lease_owner TEXT,
    lease_until_ms INTEGER,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    UNIQUE(session_id,command_id)
);
CREATE INDEX idx_agent_commands_claim
    ON agent_commands(session_id,sequence,lease_until_ms);
CREATE TABLE trace_journal_runs(
    trace_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL UNIQUE,
    conversation_id TEXT NOT NULL,
    message_id TEXT,
    turn_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'running',
    last_sequence INTEGER NOT NULL DEFAULT 0,
    terminal_reason TEXT,
    started_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    finished_at TEXT
);
CREATE INDEX idx_trace_journal_runs_conversation
    ON trace_journal_runs(conversation_id,updated_at DESC);
CREATE TABLE trace_journal_events(
    event_id TEXT PRIMARY KEY,
    trace_id TEXT NOT NULL REFERENCES trace_journal_runs(trace_id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    visibility TEXT NOT NULL,
    span_id TEXT,
    parent_span_id TEXT,
    emitted_at TEXT NOT NULL,
    event_json TEXT NOT NULL,
    envelope_json TEXT,
    UNIQUE(trace_id,sequence)
);
CREATE INDEX idx_trace_journal_events_conversation
    ON trace_journal_events(conversation_id,trace_id,sequence);
CREATE TABLE workspace_checkpoints(
    id TEXT PRIMARY KEY,
    root TEXT NOT NULL,
    git_root TEXT NOT NULL,
    revision TEXT NOT NULL,
    source TEXT NOT NULL,
    label TEXT NOT NULL DEFAULT '',
    created_at_ms INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT '',
    untracked_json TEXT NOT NULL DEFAULT '[]'
);
CREATE INDEX idx_workspace_checkpoints_root_created
    ON workspace_checkpoints(root,created_at_ms DESC,id);
CREATE TABLE work_step_transactions(
    id TEXT PRIMARY KEY,
    work_id TEXT NOT NULL,
    step_id TEXT NOT NULL,
    tool_call_id TEXT NOT NULL UNIQUE,
    root TEXT NOT NULL,
    effect_kind TEXT NOT NULL,
    phase TEXT NOT NULL CHECK(phase IN ('prepared','checkpointed','effect_observed','committed','failed')),
    workspace_checkpoint_id TEXT,
    request_json TEXT NOT NULL,
    result_json TEXT,
    result_ref TEXT,
    error TEXT,
    revision INTEGER NOT NULL CHECK(revision > 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    FOREIGN KEY(workspace_checkpoint_id) REFERENCES workspace_checkpoints(id) ON DELETE SET NULL
);
CREATE INDEX idx_work_step_transactions_work
    ON work_step_transactions(work_id,step_id,created_at_ms,id);
CREATE INDEX idx_work_step_transactions_phase
    ON work_step_transactions(phase,updated_at_ms,id);
CREATE TABLE session_run_leases(
    session_id TEXT PRIMARY KEY,
    owner TEXT NOT NULL,
    pid INTEGER NOT NULL,
    process_start TEXT NOT NULL DEFAULT '',
    acquired_at_ms INTEGER NOT NULL
);
CREATE TABLE product_settings(
    key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE sop_runs(
    run_id TEXT PRIMARY KEY,
    sop_id TEXT NOT NULL,
    sop_version INTEGER NOT NULL CHECK(sop_version > 0),
    owner_session_id TEXT,
    project_root TEXT NOT NULL,
    status TEXT NOT NULL,
    definition_json TEXT NOT NULL,
    record_json TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_sop_runs_project_updated
    ON sop_runs(project_root,updated_at_ms DESC,run_id);
CREATE INDEX idx_sop_runs_session_updated
    ON sop_runs(owner_session_id,updated_at_ms DESC,run_id);
CREATE TABLE sync_outbox(
    local_sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id TEXT NOT NULL UNIQUE,
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    deleted INTEGER NOT NULL CHECK(deleted IN(0,1)),
    payload_json TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN(
        'pending','acknowledged','unchanged','conflict'
    )),
    remote_cursor INTEGER,
    created_at_ms INTEGER NOT NULL,
    resolved_at_ms INTEGER,
    UNIQUE(entity_type,entity_id,revision)
);
CREATE INDEX idx_sync_outbox_pending ON sync_outbox(status,local_sequence);
CREATE TABLE sync_cursors(
    replica_id TEXT PRIMARY KEY,
    cursor INTEGER NOT NULL CHECK(cursor>=0),
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE sync_entity_revisions(
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision>0),
    deleted INTEGER NOT NULL DEFAULT 0 CHECK(deleted IN (0,1)),
    operation_id TEXT,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(entity_type,entity_id)
);
CREATE TABLE store_metadata(
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
INSERT INTO store_metadata(key,value) VALUES ('schema_epoch','8');
CREATE TABLE command_receipts(
    client_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    command_type TEXT NOT NULL,
    command_json TEXT NOT NULL,
    expected_revision INTEGER,
    status TEXT NOT NULL CHECK(status IN ('accepted','committed','rejected','conflict')),
    committed_revision INTEGER CHECK(committed_revision IS NULL OR committed_revision > 0),
    result_json TEXT,
    error_json TEXT,
    conflict_json TEXT,
    received_at TEXT NOT NULL,
    completed_at TEXT,
    PRIMARY KEY(client_id,request_id),
    CHECK(
        (status='accepted' AND completed_at IS NULL AND result_json IS NULL AND error_json IS NULL AND conflict_json IS NULL)
        OR (status='committed' AND completed_at IS NOT NULL AND result_json IS NOT NULL AND error_json IS NULL AND conflict_json IS NULL)
        OR (status='rejected' AND completed_at IS NOT NULL AND result_json IS NULL AND error_json IS NOT NULL AND conflict_json IS NULL)
        OR (status='conflict' AND completed_at IS NOT NULL AND result_json IS NULL AND error_json IS NULL AND conflict_json IS NOT NULL)
    )
);
CREATE INDEX idx_command_receipts_received
    ON command_receipts(received_at,client_id,request_id);
CREATE TABLE projection_cursors(
    scope_type TEXT NOT NULL,
    scope_id TEXT NOT NULL DEFAULT '',
    current_sequence INTEGER NOT NULL CHECK(current_sequence >= 0),
    revision INTEGER NOT NULL CHECK(revision > 0),
    updated_at TEXT NOT NULL,
    PRIMARY KEY(scope_type,scope_id)
);
CREATE TABLE projection_events(
    event_id TEXT PRIMARY KEY,
    scope_type TEXT NOT NULL,
    scope_id TEXT NOT NULL DEFAULT '',
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    payload_json TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    UNIQUE(scope_type,scope_id,sequence),
    FOREIGN KEY(scope_type,scope_id)
        REFERENCES projection_cursors(scope_type,scope_id) ON DELETE CASCADE
);
CREATE INDEX idx_projection_events_suffix
    ON projection_events(scope_type,scope_id,sequence);

-- Durable branch identity and lineage survive source deletion; exact replay is
-- checked against the original metadata-only intent.
CREATE TABLE conversation_branch_operations(
    operation_id TEXT PRIMARY KEY,
    intent_json TEXT NOT NULL,
    result_json TEXT NOT NULL
);

-- Canonical directory CAS is a write-side fact, independent of UI cursors.
CREATE TABLE archive_directory_revision(singleton INTEGER PRIMARY KEY CHECK(singleton=1),revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>=0));
INSERT INTO archive_directory_revision VALUES(1,0);
CREATE TRIGGER archive_directory_insert AFTER INSERT ON conversations BEGIN UPDATE archive_directory_revision SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER archive_directory_update AFTER UPDATE ON conversations BEGIN UPDATE archive_directory_revision SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER archive_directory_delete AFTER DELETE ON conversations BEGIN UPDATE archive_directory_revision SET revision=revision+1 WHERE singleton=1; END;
CREATE TABLE archive_restore_sessions(
    restore_id TEXT PRIMARY KEY,begin_json TEXT NOT NULL,expected_directory_revision INTEGER NOT NULL,
    next_sequence INTEGER NOT NULL,manifest_digest TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN('staging','committed')),
    commit_json TEXT,result_json TEXT
);
CREATE TABLE archive_restore_entries(
    restore_id TEXT NOT NULL REFERENCES archive_restore_sessions(restore_id),sequence INTEGER NOT NULL,
    entry_json TEXT NOT NULL CHECK(length(CAST(entry_json AS BLOB))<=131072),PRIMARY KEY(restore_id,sequence)
);
CREATE TABLE archive_restore_profiles(
    restore_id TEXT NOT NULL REFERENCES archive_restore_sessions(restore_id),conversation_id TEXT NOT NULL,
    profile_json TEXT NOT NULL,PRIMARY KEY(restore_id,conversation_id)
);
CREATE TABLE archive_restore_messages(
    restore_id TEXT NOT NULL REFERENCES archive_restore_sessions(restore_id),message_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,ordinal INTEGER NOT NULL,sequence INTEGER NOT NULL,
    PRIMARY KEY(restore_id,message_id),UNIQUE(restore_id,conversation_id,ordinal)
);
CREATE TABLE archive_restore_parts(
    restore_id TEXT NOT NULL REFERENCES archive_restore_sessions(restore_id),part_id TEXT NOT NULL,
    message_id TEXT NOT NULL,PRIMARY KEY(restore_id,part_id)
);
CREATE TABLE archive_restore_derivations(
    restore_id TEXT NOT NULL REFERENCES archive_restore_sessions(restore_id),
    target_part_id TEXT NOT NULL CHECK(length(CAST(target_part_id AS BLOB)) BETWEEN 1 AND 128),
    sequence INTEGER NOT NULL CHECK(sequence>=0),
    PRIMARY KEY(restore_id,target_part_id),
    UNIQUE(restore_id,sequence),
    FOREIGN KEY(restore_id,target_part_id) REFERENCES archive_restore_parts(restore_id,part_id)
);
CREATE TABLE archive_restore_prepared(
    restore_id TEXT NOT NULL REFERENCES archive_restore_sessions(restore_id),content_id TEXT NOT NULL,
    reference_json TEXT NOT NULL,PRIMARY KEY(restore_id,content_id)
);
CREATE TABLE archive_restore_targets(
    restore_id TEXT NOT NULL REFERENCES archive_restore_sessions(restore_id),conversation_id TEXT NOT NULL,
    target_json TEXT NOT NULL,PRIMARY KEY(restore_id,conversation_id)
);

CREATE TRIGGER immutable_runtime_task_identity BEFORE UPDATE ON durable_tasks
WHEN OLD.kind='agent' AND (NEW.id IS NOT OLD.id OR NEW.kind IS NOT OLD.kind
    OR NEW.owner_session_id IS NOT OLD.owner_session_id OR NEW.child_session_id IS NOT OLD.child_session_id
    OR NEW.parent_tool_call_id IS NOT OLD.parent_tool_call_id
    OR NEW.input_watermark IS NOT OLD.input_watermark)
BEGIN SELECT RAISE(ABORT, 'agent task identity is immutable'); END;

CREATE TABLE goal_execution_bindings(
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    goal_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    source_host_id TEXT NOT NULL,
    binding_json TEXT NOT NULL,
    binding_sha256 TEXT NOT NULL CHECK(length(binding_sha256)=64),
    created_at_ms INTEGER NOT NULL CHECK(created_at_ms>=0),
    PRIMARY KEY(conversation_id,goal_id),
    UNIQUE(conversation_id,operation_id)
);
