// SPDX-License-Identifier: Apache-2.0
//! Real SQLite atomic admission and cold reads; no provider or plaintext secrets.
mod support;
use nola_agent_engine::agent::{
    protocol::AgentCoreConfig, AgentSubmission, GoalRuntimeCommand, GoalRuntimeCommandEnvelope,
};
use nola_application::goal_execution::*;
use nola_domain::goal_execution::*;
use nola_store::NolaStore;
use nola_store_port::goal_execution::GoalExecutionRepository;
use serde_json::json;
fn config() -> AgentCoreConfig {
    let route = json!({"kind":"http","protocol":"chat_completions","output_token_field":"max_tokens","provider":"fixture", "endpoint":"https://fixture.invalid/v1",
        "endpoint_query_sha256":null,"model":"original-model","credential_sha256":"a".repeat(64),
        "credential_source":{"kind":"ephemeral"},"endpoint_query_source":{"kind":"none"}});
    let mut config = support::model_identity::config();
    config.host_catalog =
        Some(nola_agent_engine::agent::host::HostCapabilityCatalog::empty("host"));
    config.session_id = Some(nola_agent_engine::agent::protocol::SessionId::from_string(
        "core-c".into(),
    ));
    config.tools = vec![json!({
        "type": "function",
        "function": {"name": "fixture_tool", "parameters": {"type": "object", "properties": {}}},
        "private_context_marker": "must-not-be-disclosed"
    })];
    config.context_window.context_window_tokens = 4096;
    config.context_window.reserved_output_tokens = 64;
    config.context_window.prompt_token_limit = Some(2048);
    config.context_window.auto_compact_tokens = Some(3000);
    config.context_window.compaction_target_tokens = Some(1500);
    config.context_window.auto_compact_percent = 75;
    config.context_window.compaction_target_percent = 40;
    config.turn_identity.remote_tool_plan = Some(serde_json::from_value(json!({
        "schema_version":1,"reference":{"manifest_id":"frozen-plan","host_id":"host","client_id":"client",
            "session_id":"c","catalog_revision":"a".repeat(64)},
        "issued_at_ms":1,"expires_at_ms":300001,"tools":[]
    })).unwrap());
    config.turn_identity.host_invocation = Some(serde_json::from_value(json!({
        "version":nola_host_protocol::HOST_INVOCATION_BINDING_VERSION,
        "primary":route,"fallbacks":[],
        "generation":{"temperature":0,"top_p":0,"max_output_tokens":16384,
            "max_retries":2,"parallel_tool_calls":null,
            "reasoning_selection":null,"sampling_override":false,"generation_options":{},
            "reasoning_tier":"balanced",
            "thinking_enabled":false,"prompt_cache":false,"prompt_cache_key_sha256":null,
            "prompt_cache_key_prefix_sha256":null,"prompt_cache_retention":null},
        "output_admission":{"version":nola_host_protocol::HOST_OUTPUT_ADMISSION_VERSION,
            "routes":[{"route":route,"max_input_tokens":null,"max_output_tokens":null,"max_total_tokens":null}],
            "preference":"default","resolved":{"tokens":16384,"source":"product_default"}},
        "execution_policy_sha256":"b".repeat(64)
    })).unwrap());
    config.turn_identity.host_recovery = Some(nola_host_protocol::HostRunRecoveryReference {
        adapter: "fixture.host".into(),
        handle: "opaque-descriptor".into(),
        sha256: "c".repeat(64),
    });
    config
}

fn input() -> StartGoalExecution {
    StartGoalExecution {
        identity:GoalExecutionBindingIdentity {conversation_id:"c".into(),goal_id:"g".into(),operation_id:"start-g".into(),source_host_id:"host".into()},
        template:GoalExecutionTemplate {model_selection:GoalModelSelection::Profile {profile_id:"chosen".into(),revision:1},core_config:config()},
        envelope:GoalRuntimeCommandEnvelope {operation_id:"start-g".into(),command:GoalRuntimeCommand::Start {
            id:"g".into(),objective:"verify result".into(),constraints:vec![],now_epoch_ms:100,
            requirements:vec![serde_json::from_value(json!({"id":"r","semantic_revision":1,"description":"verify result","dependencies":[],"status":"pending","evidence_refs":[],"acceptance_policy":{"kind":"model_judgment"}})).unwrap()],
        }},
    }
}
fn seed(path: &std::path::Path) -> NolaStore {
    let mut store = NolaStore::open(path).unwrap();
    use nola_store_port::model::ModelRepository;
    store
        .model_repository()
        .upsert_model_profile(&nola_store_port::model::StoredModelProfile {
            id: "chosen".into(),
            display_name: "Chosen".into(),
            provider: "fixture".into(),
            model_name: "original-model".into(),
            runtime: "remote".into(),
            source_json: "{}".into(),
            parameters_json: "{}".into(),
            selected: false,
            enabled: true,
            revision: 1,
            created_at_ms: 1,
            updated_at_ms: 1,
            deleted_at_ms: None,
        })
        .unwrap();
    // The Store allocates revisions; caller-supplied numbers cannot invent one.
    assert_eq!(store.model_repository().model_profile("chosen").unwrap().unwrap().revision, 1);
    rusqlite::Connection::open(path).unwrap().execute("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','test',1,1,1)",[]).unwrap();
    store
}
#[test]
fn goal_binding_is_atomic_immutable_and_survives_cold_open_before_any_run() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("goal.sqlite");
    let mut store = seed(&path);
    start_goal_execution(&mut store, input()).unwrap();
    let original = load_goal_execution_binding(&store, "c", "g")
        .unwrap()
        .unwrap();
    start_goal_execution(&mut store, input()).unwrap();
    let mut changed = input();
    changed.template.model_selection = GoalModelSelection::DirectRoute;
    assert!(start_goal_execution(&mut store, changed)
        .unwrap_err()
        .contains("immutable"));
    drop(store);
    let reopened = NolaStore::open(&path).unwrap();
    let restored = load_goal_execution_binding(&reopened, "c", "g")
        .unwrap()
        .unwrap();
    assert_eq!(restored.binding_sha256, original.binding_sha256);
    assert_eq!(
        restored.template.model_selection,
        GoalModelSelection::Profile {
            profile_id: "chosen".into(),
            revision: 1
        }
    );
    assert!(load_goal_execution_binding(&reopened, "foreign", "g")
        .unwrap()
        .is_none());
    let sql = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        sql.query_row("SELECT count(*) FROM runs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    sql.execute("UPDATE goal_execution_bindings SET binding_json='{}'", [])
        .unwrap();
    assert!(load_goal_execution_binding(&reopened, "c", "g")
        .unwrap_err()
        .contains("digest"));
}
#[test]
fn failed_goal_reduction_rolls_back_binding_and_unbound_start_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollback.sqlite");
    let mut store = seed(&path);
    let mut bad = input();
    if let GoalRuntimeCommand::Start { objective, .. } = &mut bad.envelope.command {
        *objective = String::new();
    }
    assert!(start_goal_execution(&mut store, bad).is_err());
    assert!(store
        .load_goal_execution_binding("c", "g")
        .unwrap()
        .is_none());
    let i = input();
    assert!(nola_application::reduce_and_commit_idle_submission(
        &mut store,
        nola_application::ReduceAndCommitIdleSubmission {
            conversation_id: "c".into(),
            core_config: i.template.core_config,
            submission: AgentSubmission::GoalRuntime {
                envelope: i.envelope
            },
            committed_at_ms: 100,
        }
    )
    .unwrap_err()
    .contains("atomic execution binding"));
    start_goal_execution(&mut store, input()).unwrap();
}

#[test]
fn goal_reconstruction_reuses_exact_run_policy_validator() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("authority.sqlite");
    let mut store = seed(&path);
    start_goal_execution(&mut store, input()).unwrap();
    let frozen = load_goal_execution_binding(&store, "c", "g")
        .unwrap()
        .unwrap();
    let config = &frozen.template.core_config;
    let current = nola_application::RunRecoveryAuthority {
        host_catalog: config.host_catalog.clone(),
        tools: config.tools.clone(),
        permission_rules: config.permission_rules.clone(),
        context_window: config.context_window.clone(),
        host_invocation: config.turn_identity.host_invocation.clone().unwrap(),
    };
    validate_goal_execution_authority(
        &store,
        "c",
        "g",
        "host",
        &frozen.binding_sha256,
        &current,
        101,
    )
    .unwrap();
    assert!(validate_goal_execution_authority(
        &store,
        "c",
        "g",
        "foreign",
        &frozen.binding_sha256,
        &current,
        101
    )
    .is_err());
    let mut changed = current.clone();
    changed.tools.clear();
    assert!(validate_goal_execution_authority(
        &store,
        "c",
        "g",
        "host",
        &frozen.binding_sha256,
        &changed,
        101
    )
    .is_err());
    let mut changed = current.clone();
    changed.context_window.context_window_tokens += 1;
    assert!(validate_goal_execution_authority(
        &store,
        "c",
        "g",
        "host",
        &frozen.binding_sha256,
        &changed,
        101
    )
    .is_err());
}
#[test]
fn goal_binding_requires_recoverable_template_and_canonical_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.sqlite");
    let mut store = seed(&path);
    let mut missing = input();
    missing.template.core_config.turn_identity.host_recovery = None;
    assert!(start_goal_execution(&mut store, missing)
        .unwrap_err()
        .contains("recovery reference"));
    assert!(store
        .load_goal_execution_binding("c", "g")
        .unwrap()
        .is_none());
    start_goal_execution(&mut store, input()).unwrap();
    let row = store
        .load_goal_execution_binding("c", "g")
        .unwrap()
        .unwrap();
    assert!(store
        .insert_goal_execution_binding(&row)
        .unwrap_err()
        .to_string()
        .contains("transaction"));
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute(
        "UPDATE goal_execution_bindings SET source_host_id='foreign'",
        [],
    )
    .unwrap();
    assert!(load_goal_execution_binding(&store, "c", "g")
        .unwrap_err()
        .contains("sealed payload"));
}

#[test]
fn checkpoint_write_failure_rolls_back_goal_binding_in_the_same_sqlite_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("commit-failure.sqlite");
    let mut store = seed(&path);
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch("CREATE TRIGGER reject_goal_checkpoint BEFORE INSERT ON core_checkpoint_bodies BEGIN SELECT RAISE(ABORT,'fixture checkpoint failure'); END;").unwrap();
    assert!(start_goal_execution(&mut store, input()).is_err());
    assert!(store
        .load_goal_execution_binding("c", "g")
        .unwrap()
        .is_none());
    assert_eq!(
        sql.query_row("SELECT count(*) FROM core_checkpoints", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    sql.execute_batch("DROP TRIGGER reject_goal_checkpoint")
        .unwrap();
    start_goal_execution(&mut store, input()).unwrap();
}

#[test]
fn disabled_or_revised_profile_blocks_execution_but_not_inspection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("revocation.sqlite");
    let mut store = seed(&path);
    start_goal_execution(&mut store, input()).unwrap();
    prepare_goal_execution(&mut store, "c", "g").unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute("UPDATE model_profiles SET enabled=0 WHERE id='chosen'", [])
        .unwrap();
    assert!(prepare_goal_execution(&mut store, "c", "g")
        .unwrap_err()
        .contains("disabled"));
    assert!(load_goal_execution_binding(&store, "c", "g")
        .unwrap()
        .is_some());
    sql.execute(
        "UPDATE model_profiles SET enabled=1,revision=2 WHERE id='chosen'",
        [],
    )
    .unwrap();
    assert!(prepare_goal_execution(&mut store, "c", "g")
        .unwrap_err()
        .contains("changed"));
}
