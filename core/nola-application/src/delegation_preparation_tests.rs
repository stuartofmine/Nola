// SPDX-License-Identifier: Apache-2.0
//! Real SQLite/Engine preparation fences. No model/provider is simulated: the
//! explicit user command asks Core to dispatch its own spawn capability.
use super::*;
use nola_agent_engine::agent::{AgentCoreConfig, AgentSubmission};
use nola_host_protocol::delegation::*;
use serde_json::json;

fn parent_snapshot() -> ValidatedDelegationParent {
    let endpoint = "https://fixture.invalid/v1";
    ValidatedDelegationParent::new(DelegationParentSnapshot {
        process_sandbox: None,
        bounded_file_tools: Default::default(),
        project_repository_identity:Some(nola_domain::task_workspace::TaskRepositoryIdentity {
            common_directory:"/fixture/.git".into(),common_directory_identity:"common:3".into()}),
        task_worktree: None, task_sandbox_requirement: None,
        authorized_workspace_roots: vec!["/fixture".into()], requested_native_sandbox: Some("workspace-write".into()),
        model: DelegatedModel {
            endpoint: endpoint.into(),
            model: "fixture".into(),
            swe_mode: false,
        },
        runtime: DelegatedRuntimeConfiguration {
            provider: Some("chat_completions".into()),
            ..Default::default()
        },
        credential: Some(CredentialReference::inherited(
            route_identity(endpoint, Some("chat_completions")).unwrap(),
        )),
        fallback_credentials: vec![],
        profiles: nola_domain::delegation::effective_agent_profiles(&Default::default()),
        project_root: "/fixture".into(),
        read_only_tools: nola_domain::delegation::effective_agent_profiles(&Default::default())["explore"].tools.iter().cloned().collect(),
        profile_credentials: Default::default(),
        execution_backend: "nola".into(),
        external_execution_model: None,
        external_execution_model_provider: None,
        max_iterations: 4,
    })
    .unwrap()
}
fn response(
    request: &HostRequestEnvelope,
    run: &str,
    digest: &str,
    sequence: u64,
    payload: HostResponse,
) -> ReduceAndAcceptHostResponse {
    ReduceAndAcceptHostResponse {
        conversation_id: "conversation".into(),
        run_id: run.into(),
        invocation_config_digest: digest.into(),
        received_at_ms: 1100 + sequence as i64,
        response: HostResponseEnvelope {
            request_id: request.request_id.clone(),
            attempt_id: request.attempt_id.clone(),
            effect_id: request.effect_id.clone(),
            delegated_invocation_id: request.delegated_invocation_id.clone(),
            host_id: request.host_id.clone(),
            lease_id: request.lease_id.clone(),
            session_id: request.session_id.clone(),
            turn_id: request.turn_id.clone(),
            catalog_revision: request.catalog_revision,
            availability_epoch: request.availability_epoch,
            delivery: request.delivery.clone(),
            sequence,
            payload,
        },
    }
}
fn prepared_input(
    request: &HostRequestEnvelope,
) -> crate::delegation_preparation::PrepareDelegatedTask {
    let HostRequest::ExecuteTool { call_id, .. } = &request.payload else {
        panic!("Core must produce actual spawn effect")
    };
    crate::delegation_preparation::PrepareDelegatedTask {
        observed_worktree: None,
        request: request.clone(),
        now_ms: 1200,
        input: CreateRuntimeTask {
            id: "prepared-task".into(),
            kind: "agent".into(),
            title: "Delegation".into(),
            project_root: "/fixture".into(),
            owner_session_id: Some("conversation".into()),
            parent_task_id: None,
            parent_tool_call_id: Some(call_id.clone()),
            child_session_id: Some("child".into()),
            agent_type: Some("explore".into()),
            prompt: Some("Inspect without edits".into()),
            initial_progress_summary: Some("queued".into()),
            initial_activity: None,
            output_path: None,
            initially_unread: false,
            input_watermark: Some(1200),
            metadata: json!({}),
            scheduling: RuntimeTaskScheduling {
                priority: RuntimeTaskPriority::Normal,
                queued_at_ms: 1200,
                resource_budget: Default::default(),
                output_bytes: 0,
                resource_usage: Default::default(),
                budget_exhaustion: None,
            },
            created_at_ms: 1200,
        },
    }
}

#[test]
fn prepared_delegation_requires_started_exact_effect_and_preserves_original_after_cold_replay() {
    check_prepared_delegation_cold_replay(false, false);
}
#[test]
fn bound_worktree_survives_exact_delegation_cold_replay() {
    check_prepared_delegation_cold_replay(true, false);
}
#[test]
fn bound_worktree_cannot_bypass_writer_admission_with_a_nested_child() {
    check_prepared_delegation_cold_replay(true, true);
}
fn check_prepared_delegation_cold_replay(bound: bool, reject_nested_writer: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delegation.sqlite");
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    let mut config: AgentCoreConfig = crate::user_input_wait_tests::config();
    config.turn_identity.host_invocation = Some(crate::run_recovery_port_tests::recovery_host_binding("fixture"));
    let mut parent=parent_snapshot().snapshot().unwrap();
    if !bound {parent.profiles.get_mut("explore").unwrap().tools.push("delegate_expert".into());parent.read_only_tools.insert("delegate_expert".into());}
    if bound {
        if reject_nested_writer {
            // Deliberately withhold read-only evidence for this negative fixture.
            parent.read_only_tools.clear();
        }
        parent.task_worktree=Some(nola_domain::task_workspace::TaskWorktreeBinding {
            selection:nola_domain::task_workspace::TaskWorktreeSelection { source_snapshot:None, expected_directory_identity:None,root:"/fixture".into(), expected_head:"a".repeat(40), expected_common_directory:"/fixture/.git".into()},
            git_directory:"/fixture/.git/worktrees/child".into(), root_identity:"synthetic-root".into(),
            git_directory_identity:"synthetic-git".into(), common_directory_identity:"synthetic-common".into(),
        });
    }
    if let Some(binding)=&parent.task_worktree {parent.project_repository_identity=Some(binding.repository_identity());}
    let binding=parent.task_worktree.clone();
    config.turn_identity.delegation = Some(ValidatedDelegationParent::new(parent).unwrap());
    config.turn_identity.host_dependencies = Some(json!([{"id":"frozen-skill","instructions":"original revision"}]));
    let mut spawn = config.host_catalog.as_ref().unwrap().capabilities["read_file"].clone();
    spawn.id = "spawn_agent".into();
    config
        .host_catalog
        .as_mut()
        .unwrap()
        .capabilities
        .insert("spawn_agent".into(), spawn);
    if !bound {
        let mut inherited=config.host_catalog.as_ref().unwrap().capabilities["spawn_agent"].clone();
        inherited.id="delegate_expert".into();inherited.exposure=nola_agent_engine::agent::host::HostCapabilityExposure::Hidden;
        config.host_catalog.as_mut().unwrap().capabilities.insert("delegate_expert".into(),inherited);
    }
    let submission = AgentSubmission::UserDelegation {
        agent_type: "explore".into(),
        prompt: "Inspect without edits".into(),
        submission_id: "command".into(),
        submitted_at_epoch_ms: 1000,
    };
    let accepted = crate::assistant_turn::admit_with_core(
        &mut store,
        BeginAssistantTurn {
            conversation_id: "conversation".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_title: "Delegate".into(),
            user_message_id: "command:user".into(),
            user_part_id: "command:user:body".into(),
            user_text: "/agent explore Inspect without edits".into(),
            user_content_json: None,
            assistant_message_id: "assistant".into(),
            context_checkpoint_id: None,
            core_config: config,
            submission,
            created_at_ms: 1000,
        },
        None,
    )
    .unwrap()
    .0;
    let request = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: accepted.begin.run_id.clone(),
            worker_generation: "worker".into(),
            lease_owner: "worker".into(),
            claimed_at_ms: 1100,
            lease_expires_at_ms: 9000,
            limit: 4,
        },
    )
    .unwrap()
    .requests
    .remove(0);
    let mut input = prepared_input(&request);
    input.observed_worktree=binding.clone();
    let mut forged = input.input.clone();
    forged.metadata = json!({"delegation_origin":{"parent_run_id":"forged"}});
    assert!(
        crate::runtime_task::execute_runtime_task_command(
            &mut store,
            RuntimeTaskCommand::Create { input: forged }
        )
        .is_err(),
        "public task creation must not mint delegation authority"
    );
    assert!(
        crate::delegation_preparation::prepare(&mut store, input.clone()).is_err(),
        "claim alone is not execution authority"
    );
    reduce_and_accept_host_response(
        &mut store,
        response(
            &request,
            &accepted.begin.run_id,
            &accepted.invocation_config_digest,
            0,
            HostResponse::ExecutionStarted,
        ),
    )
    .unwrap();
    let mut foreign = input.clone();
    foreign.request.delivery.as_mut().unwrap().lease_token = "foreign".into();
    assert!(crate::delegation_preparation::prepare(&mut store, foreign).is_err());
    assert!(crate::mobile_delegation::guide(&mut store, crate::mobile_delegation::PrepareMobileDelegatedTask {
        request: request.clone(), now_ms: 1200,
    }).is_err(), "A started spawn effect cannot authorize guidance");
    if reject_nested_writer {
        let error=crate::delegation_preparation::prepare(&mut store,input.clone()).err().unwrap();
        assert!(error.contains("cannot admit another write/command child"),"{error}");
        assert!(store.runtime_task_for_delegation("conversation",&accepted.begin.run_id,&request.effect_id.0).unwrap().is_none());
        return;
    }
    let first = crate::delegation_preparation::prepare(&mut store, input.clone()).unwrap();
    let run = match &request.payload { HostRequest::ExecuteTool { .. } => &accepted.begin.run_id, _ => unreachable!() };
    assert_eq!(store.runtime_task_for_delegation("conversation", run, &request.effect_id.0).unwrap().unwrap().id, first.task.id);
    assert!(store.runtime_task_for_delegation("other", run, &request.effect_id.0).unwrap().is_none());
    assert!(store.runtime_task_for_delegation("conversation", "other", &request.effect_id.0).unwrap().is_none());
    assert!(store.runtime_task_for_delegation("conversation", run, "other").unwrap().is_none());

    if !bound {
    let mobile = crate::mobile_delegation::prepare(&mut store,
        crate::mobile_delegation::PrepareMobileDelegatedTask {
            request: request.clone(), now_ms: 1200,
        }).unwrap();
    assert_eq!(mobile.prepared.task.id, first.task.id,
        "mobile and CLI consume the same exact parent effect, not parallel task ledgers");
    assert!(mobile.prepared.replayed);
    assert!(matches!(mobile.response, HostResponse::TaskPromoted { ref task_id, .. } if task_id == &first.task.id));
    assert!(crate::mobile_delegation::admit_source(&mut store, "conversation", &first.task.id).is_err(),
        "preparation cannot manufacture committed TaskPromoted source authority");
    }
    assert!(
        !first.source_admitted,
        "prepared is not TaskPromoted admission"
    );
    assert!(first.snapshot.runtime.agents.is_empty());
    let mut tampered = input.clone();
    tampered.input.prompt = Some("Substitute instructions".into());
    assert!(crate::delegation_preparation::prepare(&mut store, tampered).is_err());
    drop(store);
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    let mut replay = input.clone();
    replay.input.id = "new-candidate-must-not-be-created".into();
    let again = crate::delegation_preparation::prepare(&mut store, replay).unwrap();
    assert!(again.replayed);
    assert_eq!(again.task.id, first.task.id);
    assert!(!again.source_admitted);
    assert_eq!(again.task.metadata_json, first.task.metadata_json);
    let connection = rusqlite::Connection::open(&path).unwrap();
    assert!(connection.execute("UPDATE durable_tasks SET metadata_json=json_set(metadata_json,'$.delegation_origin.parent_run_id','other') WHERE id=?1",[&first.task.id]).is_err());
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM durable_tasks", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(connection);
    if bound {
        assert_eq!(again.snapshot.worktree,binding);
        assert!(crate::mobile_delegation_configuration::child_config(&store,&first.task.id).is_err());
        return;
    }
    let HostRequest::ExecuteTool { call_id, .. } = &request.payload else {
        unreachable!()
    };
    let promoted = HostResponse::TaskPromoted {
        task_id: first.task.id.clone(),
        input_watermark: first.task.input_watermark,
        kind: nola_agent_engine::agent::TaskKind::Agent,
        state: nola_agent_engine::agent::TaskState::Queued,
        progress: Some(0.0),
        current_step: Some("queued".into()),
        lineage: nola_agent_engine::agent::TaskLineage {
            child_session_id: Some(nola_agent_engine::agent::SessionId::from_string(
                "child".into(),
            )),
            parent_call_id: Some(nola_agent_engine::agent::CallId::from_string(
                call_id.clone(),
            )),
            permission_scope_id: Some(format!("task:{}", first.task.id)),
            cancellation_scope_id: Some(format!("task:{}", first.task.id)),
            ..Default::default()
        },
    };
    let promotion_receipt = reduce_and_accept_host_response(
        &mut store,
        response(
            &request,
            &accepted.begin.run_id,
            &accepted.invocation_config_digest,
            1,
            promoted,
        ),
    )
    .unwrap();
    let cursor = promotion_receipt
        .events
        .iter()
        .find_map(|event| match &event.event {
            nola_agent_engine::agent::AgentEvent::TaskUpdated { task, .. }
                if task.id == first.task.id =>
            {
                Some(event.cursor.sequence)
            }
            _ => None,
        })
        .expect("actual committed promotion source");
    let admission = crate::agent_task_admission::export(
        &store,
        "conversation",
        &accepted.begin.run_id,
        &first.task.id,
        cursor,
    )
    .unwrap();
    store.begin_write_unit_of_work().unwrap();
    crate::agent_task_admission::import(&mut store, admission).unwrap();
    assert!(crate::mobile_delegation::admit_source(&mut store, "foreign-parent", &first.task.id).is_err());
    let mobile_authority = crate::mobile_delegation::admit_source(&mut store, "conversation", &first.task.id).unwrap();
    assert_eq!(mobile_authority.admission.as_ref().unwrap().parent_run_id, accepted.begin.run_id);
    store.commit_write_unit_of_work().unwrap();
    let candidate = crate::mobile_delegation_configuration::child_config(&store, &first.task.id).unwrap();
    assert!(!candidate.tools.iter().any(|tool|nola_domain::expert_task_target::schema_tool_name(tool)==Some("delegate_expert")));
    assert!(!candidate.host_catalog.as_ref().unwrap().capabilities.contains_key("delegate_expert"),"A temporary child must not inherit Main-only expert controls, including hidden activation entries");
    assert_eq!(candidate.turn_identity.host_dependencies, Some(json!([{"id":"frozen-skill","instructions":"original revision"}])));
    let worker = nola_host_protocol::HostWorkerIdentity::Unobservable { nonce: "mobile-child-fixture".into() };
    let mobile_input = crate::mobile_delegation::AdmitMobileTaskChild {
        task_id: first.task.id.clone(), parent_conversation_id: "conversation".into(),
        worker, now_ms: 1300, lease_until_ms: 9000, current_tools: candidate.tools.clone(), candidate_config: candidate,
    };
    let mut foreign_candidate = mobile_input.clone();
    foreign_candidate.candidate_config.tools.push(json!({"type":"function","function":{"name":"unadmitted_write","parameters":{"type":"object"}}}));
    assert!(crate::mobile_delegation::admit(&mut store, foreign_candidate).is_err());
    assert!(store.runtime_task_run_binding(&first.task.id).unwrap().is_none(),
        "rejected candidate must not bind a child Run");
    let wake = crate::mobile_delegation::wake_plan(&store, 1300).unwrap();
    assert_eq!(wake.candidates.len(), 1);
    assert_eq!(wake.candidates[0].parent_invocation.run_id, accepted.begin.run_id);
    assert_eq!(wake.candidates[0].task_id, first.task.id);
    let mobile_child = crate::mobile_delegation::admit(&mut store, mobile_input.clone()).unwrap().unwrap();
    assert!(crate::mobile_delegation::admit(&mut store, mobile_input.clone()).unwrap().is_none(),
        "a second native owner cannot steal a live lease");
    let sleeping = crate::mobile_delegation::wake_plan(&store, 1301).unwrap();
    assert!(sleeping.candidates.is_empty());
    assert_eq!(sleeping.next_wake_at_ms, Some(9000));
    let due = crate::mobile_delegation::wake_plan(&store, 9001).unwrap();
    assert_eq!(due.candidates.len(), 1);
    let mut takeover = mobile_input;
    takeover.now_ms = 9001;
    takeover.lease_until_ms = 18000;
    takeover.worker = nola_host_protocol::HostWorkerIdentity::Unobservable { nonce: "reopened-mobile-worker".into() };
    let recovered = crate::mobile_delegation::admit(&mut store, takeover.clone()).unwrap().unwrap();
    assert_eq!(recovered.invocation.run_id, mobile_child.invocation.run_id,
        "expired lease recovery must retain the exact child Run");
    assert_ne!(recovered.attempt.id, mobile_child.attempt.id);
    assert_eq!(mobile_child.invocation.conversation_id, "child");
    assert_eq!(store.runtime_task_run_binding(&first.task.id).unwrap().unwrap().run_id,
        mobile_child.invocation.run_id);
    assert_eq!(mobile_child.attempt.task_id, first.task.id);
    assert!(!recovered.cancellation_only);
    let child_request = crate::claim_committed_host_requests(&mut store, crate::ClaimCommittedHostRequests {
        run_id: recovered.invocation.run_id.clone(), worker_generation: "cancel-test-worker".into(),
        lease_owner: "cancel-test-worker".into(), claimed_at_ms: 9100, lease_expires_at_ms: 17000, limit: 4,
    }).unwrap().requests.remove(0);
    let mut child_started = response(&child_request, &recovered.invocation.run_id,
        &recovered.invocation.invocation_config_digest, 0, HostResponse::ExecutionStarted);
    child_started.conversation_id = recovered.invocation.conversation_id.clone();
    child_started.received_at_ms = 9101;
    reduce_and_accept_host_response(&mut store, child_started).unwrap();
    store.request_runtime_task_cancel(&first.task.id, 9200).unwrap();
    takeover.now_ms = 9300;
    assert!(crate::mobile_delegation::admit(&mut store, takeover.clone()).unwrap().is_none(),
        "cancellation recovery cannot steal a live task lease");
    takeover.now_ms = 18001;
    takeover.lease_until_ms = 28000;
    takeover.current_tools.clear(); // cancellation never grants new tool authority
    let cancellation = crate::mobile_delegation::admit(&mut store, takeover.clone()).unwrap().unwrap();
    assert!(cancellation.cancellation_only);
    assert_eq!(cancellation.attempt, recovered.attempt, "cancel-only does not reclaim or renew a normal execution lease");
    assert_eq!(cancellation.invocation.run_id, recovered.invocation.run_id);
    assert_ne!(store.runtime_task(&first.task.id).unwrap().unwrap().status, "cancelled",
        "an expired worker is not a canonical child terminal seal");
    let controls = crate::load_committed_host_requests(&mut store, &cancellation.invocation.run_id).unwrap();
    assert!(!controls.requests.is_empty());
    assert!(controls.requests.iter().all(|request| matches!(request.payload, HostRequest::Cancel { .. })),
        "cancel-only recovery must not expose model/tool execution");
    let replay = crate::mobile_delegation::admit(&mut store, takeover).unwrap().unwrap();
    assert!(replay.cancellation_only);
    assert_eq!(replay.attempt, cancellation.attempt);
    drop(store);
    let store = nola_store::NolaStore::open(&path).unwrap();
    let authority = store
        .runtime_agent_task_authority(&first.task.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        authority.phase,
        nola_domain::agent_task_admission::RuntimeAgentTaskAuthorityPhase::Admitted
    );
    assert_eq!(
        authority.admission.unwrap().parent_run_id,
        accepted.begin.run_id
    );
    let connection = rusqlite::Connection::open(&path).unwrap();
    let (status,result):(String,String)=connection.query_row("SELECT runs.status,run_results.result_json FROM runs JOIN run_results ON run_results.run_id=runs.id WHERE runs.id=?1",[&accepted.begin.run_id],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(status, "sealed");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap()["value"]["kind"],
        "delegation_accepted"
    );
}

#[test]
fn mobile_permission_unstarted_question_cold_restore_keeps_user_decision_pending() {
    permission_recovery_case_inner(false, false, false, false, false);
}

#[test]
fn mobile_permission_cold_allow_preserves_frontier_for_retry() {
    permission_recovery_case(false, true, false, false);
}

#[test]
fn mobile_permission_cold_allow_preserves_frontier_for_resume() {
    permission_recovery_case(true, true, false, false);
}

#[test]
fn mobile_permission_runtime_cold_allow_preserves_frontier_for_retry() {
    permission_recovery_case(false, true, true, false);
}

#[test]
fn mobile_permission_runtime_cold_allow_preserves_frontier_for_resume() {
    permission_recovery_case(true, true, true, false);
}

#[test]
fn mobile_permission_runtime_model_tool_allow_retry() {
    permission_recovery_case(false, true, true, true);
}

#[test]
fn mobile_permission_runtime_model_tool_allow_resume() {
    permission_recovery_case(true, true, true, true);
}

fn permission_recovery_case(recoverable: bool, allow: bool, hot_runtime: bool, model_tool: bool) {
    permission_recovery_case_inner(recoverable, allow, hot_runtime, model_tool, true);
}

fn permission_recovery_case_inner(recoverable: bool, allow: bool, hot_runtime: bool, model_tool: bool, started: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delegation.sqlite");
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    let mut config: AgentCoreConfig = crate::user_input_wait_tests::config();
    config.turn_identity.host_invocation = Some(crate::run_recovery_port_tests::recovery_host_binding("fixture"));
    config.turn_identity.delegation = Some(parent_snapshot());
    config.turn_identity.host_dependencies = Some(json!([{"id":"frozen-skill","instructions":"original revision"}]));
    let mut spawn = config.host_catalog.as_ref().unwrap().capabilities["read_file"].clone();
    spawn.id = "spawn_agent".into();
    config
        .host_catalog
        .as_mut()
        .unwrap()
        .capabilities
        .insert("spawn_agent".into(), spawn);
    config.permission_rules.rules.push(nola_domain::PermissionRule {
        capability_pattern: "*".into(), resource_pattern: "*".into(),
        decision: nola_domain::PermissionPolicyDecision::Ask,
        source: nola_domain::PermissionRuleSource::UserGlobal,
    });
    if recoverable {
        let mut prompt = config.host_catalog.as_ref().unwrap().capabilities["spawn_agent"].clone();
        prompt.id = "permission.ask".into();
        prompt.replay_safety = nola_agent_engine::agent::host::HostReplaySafety::Recoverable;
        config.host_catalog.as_mut().unwrap().capabilities.insert(prompt.id.clone(), prompt);
    }
    let submission = if model_tool { AgentSubmission::UserInput {
        items: vec![nola_agent_engine::agent::AgentItem::user_message(nola_agent_engine::agent::ItemId::from_string("command:user".into()), "Inspect without edits")],
        turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
        submission_id: Some("command".into()), submitted_at_epoch_ms: Some(1000),
        delivery: nola_agent_engine::agent::AgentInputDelivery::CurrentTurn,
    } } else { AgentSubmission::UserDelegation {
        agent_type: "explore".into(),
        prompt: "Inspect without edits".into(),
        submission_id: "command".into(),
        submitted_at_epoch_ms: 1000,
    } };
    let accepted = crate::assistant_turn::admit_with_core(
        &mut store,
        BeginAssistantTurn {
            conversation_id: "conversation".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_title: "Delegate".into(),
            user_message_id: "command:user".into(),
            user_part_id: "command:user:body".into(),
            user_text: "/agent explore Inspect without edits".into(),
            user_content_json: None,
            assistant_message_id: "assistant".into(),
            context_checkpoint_id: None,
            core_config: config,
            submission,
            created_at_ms: 1000,
        },
        None,
    )
    .unwrap()
    .0;
    if model_tool {
        let model = claim_committed_host_requests(&mut store, ClaimCommittedHostRequests {
            run_id: accepted.begin.run_id.clone(), worker_generation: "model".into(), lease_owner: "model".into(),
            claimed_at_ms: 1001, lease_expires_at_ms: 9000, limit: 4,
        }).unwrap().requests.remove(0);
        let mut model_runtime = crate::ApplicationRuntime::open(&path).unwrap();
        for (sequence, payload) in [
            (0, HostResponse::ExecutionStarted),
            (1, HostResponse::ModelAttemptStarted { provider: "fixture".into(), model: "fixture".into(), reason: "initial".into() }),
            (2, HostResponse::ModelToolCallDelta { part_id: "tool:read-call".into(), call_id: "read-call".into(),
                capability_id: "read_file".into(), arguments_delta: "{\"path\":".into() }),
            (3, HostResponse::ModelToolCallDelta { part_id: "tool:read-call".into(), call_id: "read-call".into(),
                capability_id: "read_file".into(), arguments_delta: "\"fixture".into() }),
            (4, HostResponse::ModelToolCallDelta { part_id: "tool:read-call".into(), call_id: "read-call".into(),
                capability_id: "read_file".into(), arguments_delta: ".txt\"}".into() }),
            (5, HostResponse::ModelCompleted { items: vec![], tool_calls: vec![json!({
                "call_id":"read-call", "name":"read_file", "arguments":{"path":"fixture.txt"},
                "permission_scope":null, "permission_rationale":null})], usage: json!({"input_tokens":1,"output_tokens":1}) }),
        ] {
            let mut input = response(&model, &accepted.begin.run_id, &accepted.invocation_config_digest, sequence, payload);
            input.received_at_ms = 1002 + sequence as i64;
            model_runtime.execute(crate::ApplicationCommand::ReduceAndAcceptHostResponse { input }).unwrap();
        }
    }
    if model_tool {
        let connection = rusqlite::Connection::open(&path).unwrap();
        let (store_revision, core_revision): (i64, i64) = connection.query_row(
            "SELECT c.revision,json_extract(w.witness_json,'$.core_revision') FROM core_checkpoints c
             JOIN core_delivery_witnesses w ON w.conversation_id=c.conversation_id
               AND w.checkpoint_revision=c.revision AND w.event_cursor=c.event_cursor
             WHERE c.conversation_id='conversation'", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert!(core_revision > store_revision,
            "streamed reductions must separate Core revision from durable Store CAS revision");
    }
    let request = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: accepted.begin.run_id.clone(),
            worker_generation: "worker".into(),
            lease_owner: "worker".into(),
            claimed_at_ms: 1100,
            lease_expires_at_ms: 9000,
            limit: 4,
        },
    )
    .unwrap()
    .requests
    .remove(0);

    assert!(matches!(request.payload, HostRequest::RequestPermission { .. }));
    assert!(crate::mobile_delegation::inspect_permission(&mut store, request.clone(), 1100).is_ok());
    assert_eq!(store.active_effect_delivery_attempt(&request.effect_id.0).unwrap().unwrap().state,
        nola_domain::EffectDeliveryAttemptState::Leased);
    if started {
        reduce_and_accept_host_response(&mut store, response(&request, &accepted.begin.run_id,
            &accepted.invocation_config_digest, 0, HostResponse::ExecutionStarted)).unwrap();
    }
    let proof = crate::mobile_delegation::inspect_permission(&mut store, request.clone(), 1200).unwrap();
    assert_eq!(proof["conversation_id"], "conversation");
    assert_eq!(proof["payload"], serde_json::to_value(&request.payload).unwrap());
    assert_eq!(proof["capability"]["id"], if model_tool { "read_file" } else { "spawn_agent" });
    assert!(proof["permission_domain"].is_null(), "no OS domain must not mean permission is allowed");
    let mut foreign = request.clone();
    foreign.delivery.as_mut().unwrap().lease_token = "other-worker".into();
    assert!(crate::mobile_delegation::inspect_permission(&mut store, foreign, 1200).is_err());
    let mut altered = request.clone();
    if let HostRequest::RequestPermission { allow_conversation, .. } = &mut altered.payload {
        *allow_conversation = !*allow_conversation;
    }
    assert!(crate::mobile_delegation::inspect_permission(&mut store, altered, 1200).is_err());
    assert!(crate::mobile_delegation::inspect_permission(&mut store, request.clone(), 9001).is_err());
    let pending = crate::load_committed_host_requests(&mut store, &accepted.begin.run_id).unwrap();
    assert!(pending.requests.iter().any(|p| p.effect_id == request.effect_id),
        "reading approval facts must not settle the permission");

    let binding = crate::run_recovery_port_tests::recovery_host_binding("fixture");
    let prepare = |now_ms| crate::effect_delivery::PrepareRunEffectRecovery {
        conversation_id: "conversation".into(),
        run_id: accepted.begin.run_id.clone(),
        invocation_config_digest: accepted.invocation_config_digest.clone(),
        now_ms,
    };
    crate::run_recovery::recover_expired(&mut store, "conversation", &accepted.begin.run_id,
        &binding, "1970-01-01T00:00:08.999Z", 8999).unwrap();
    assert_eq!(crate::effect_delivery::prepare_run_effect_recovery(&mut store, prepare(8999)).unwrap().rebound_count, 0,
        "a live permission lease cannot be stolen");
    assert!(crate::mobile_delegation::inspect_permission(&mut store, request.clone(), 8999).is_ok());

    if !recoverable && started {
        let original_delivery = store.active_effect_delivery_attempt(&request.effect_id.0).unwrap().unwrap();
        let unauthorized = nola_kernel::authorize_pending_permission_redisplay(&original_delivery,
            proof["request_id"].as_str().unwrap(), 9001).unwrap();
        assert!(store.commit_effect_delivery_attempt_mutations(&[unauthorized]).is_err(),
            "a public mutation cannot manufacture same-transaction approval redisplay authority");
        let foreign = nola_kernel::authorize_pending_permission_redisplay(&original_delivery,
            "foreign-permission", 9001).unwrap();
        assert!(nola_store_port::store_maintenance::StoreMaintenanceRepository::with_run_recovery_plan(
            &mut store, Some("conversation"), "fixture", "1970-01-01T00:00:09.001Z", 9001,
            |_| Ok(nola_store_port::store_maintenance::RunRecoveryPlan {
                effect_delivery_attempt_mutations: vec![foreign.clone()],
                preserve_all_traces: true, ..Default::default()
            })).is_err(), "even the recovery transaction must reject a foreign permission proof");
    }
    // Reopen actual SQLite to exercise process death, not an in-memory mock.
    drop(store);
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    crate::run_recovery::recover_expired(&mut store, "conversation", &accepted.begin.run_id,
        &binding, "1970-01-01T00:00:09.001Z", 9001).unwrap();
    let recovered = crate::effect_delivery::prepare_run_effect_recovery(&mut store, prepare(9001)).unwrap();
    assert_eq!(recovered.rebound_count, usize::from(recoverable));
    assert_eq!(crate::effect_delivery::prepare_run_effect_recovery(&mut store, prepare(9001)).unwrap().rebound_count, 0,
        "repeated preparation must not create another permission occurrence");
    let successor = claim_committed_host_requests(&mut store, ClaimCommittedHostRequests {
        run_id: accepted.begin.run_id.clone(), worker_generation: "successor".into(),
        lease_owner: "successor".into(), claimed_at_ms: 9002, lease_expires_at_ms: 18000, limit: 4,
    }).unwrap().requests.remove(0);
    assert_eq!(successor.effect_id, request.effect_id);
    if recoverable { assert_ne!(successor.request_id, request.request_id); }
    else { assert_eq!(successor.request_id, request.request_id); }
    assert_ne!(successor.delivery.as_ref().unwrap().attempt_id, request.delivery.as_ref().unwrap().attempt_id);
    assert_ne!(successor.delivery.as_ref().unwrap().lease_token, request.delivery.as_ref().unwrap().lease_token);
    assert_eq!(serde_json::to_value(&successor.payload).unwrap(), serde_json::to_value(&request.payload).unwrap());
    assert!(matches!(successor.payload, HostRequest::RequestPermission { .. }));
    assert!(crate::mobile_delegation::inspect_permission(&mut store, successor.clone(), 9002).is_ok(),
        "redisplaying a question must not start the requested operation");
    let mut start = response(&successor, &accepted.begin.run_id, &accepted.invocation_config_digest,
        successor.resume_after_sequence.unwrap_or(0) + 1, HostResponse::ExecutionStarted);
    start.received_at_ms = 9003;
    let mut runtime = hot_runtime.then(|| crate::ApplicationRuntime::open(&path).unwrap());
    if let Some(runtime) = &mut runtime {
        runtime.execute(crate::ApplicationCommand::ReduceAndAcceptHostResponse { input: start }).unwrap();
    } else { reduce_and_accept_host_response(&mut store, start).unwrap(); }
    let proof = crate::mobile_delegation::inspect_permission(&mut store, successor.clone(), 9004).unwrap();
    assert_eq!(proof["capability"]["id"], if model_tool { "read_file" } else { "spawn_agent" });
    assert_eq!(proof["payload"], serde_json::to_value(&successor.payload).unwrap());
    assert!(crate::mobile_delegation::inspect_permission(&mut store, request.clone(), 9004).is_err());
    let mut late = response(&request, &accepted.begin.run_id, &accepted.invocation_config_digest,
        2, HostResponse::PermissionResolved { request_id: proof["request_id"].as_str().unwrap().into(),
            resolution: nola_domain::host_effect::HostPermissionResolution::allow_once() });
    late.received_at_ms = 9005;
    assert!(reduce_and_accept_host_response(&mut store, late).is_err(),
        "same permission ID cannot authorize a late answer from the expired owner");
    let pending = crate::load_committed_host_requests(&mut store, &accepted.begin.run_id).unwrap();
    assert_eq!(pending.requests.len(), 1);
    assert_eq!(pending.requests[0].request_id, successor.request_id,
        "cold recovery reissues approval but never manufactures the user's decision");
    let mut decision = response(&successor, &accepted.begin.run_id, &accepted.invocation_config_digest,
        successor.resume_after_sequence.unwrap_or(0) + 2,
        HostResponse::PermissionResolved { request_id: proof["request_id"].as_str().unwrap().into(),
            resolution: if allow { nola_domain::host_effect::HostPermissionResolution::allow_once() }
                else { nola_domain::host_effect::HostPermissionResolution::deny() } });
    decision.received_at_ms = 9006;
    if let Some(runtime) = &mut runtime {
        runtime.execute(crate::ApplicationCommand::ReduceAndAcceptHostResponse { input: decision }).unwrap();
    } else { reduce_and_accept_host_response(&mut store, decision).unwrap(); }
    if model_tool {
        let connection = rusqlite::Connection::open(&path).unwrap();
        let (store_revision, source_revision): (i64, i64) = connection.query_row(
            "SELECT c.revision,json_extract(e.payload_json,'$.core_envelope.revision')
             FROM core_checkpoints c JOIN runs r ON r.conversation_id=c.conversation_id
             JOIN run_events e ON e.run_id=r.id
             WHERE c.conversation_id='conversation' AND e.event_kind='permission_settled'
             ORDER BY e.sequence DESC LIMIT 1", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert!(source_revision > store_revision,
            "the former Store-CAS comparison must reject this valid streamed settlement");
    }
    assert!(crate::mobile_delegation::inspect_permission(&mut store, successor, 9007).is_err(),
        "a committed decision is not a pending question to redisplay");
    let after = crate::load_committed_host_requests(&mut store, &accepted.begin.run_id).unwrap();
    assert!(!after.requests.iter().any(|r| matches!(r.payload, HostRequest::RequestPermission { .. })));
    if allow { assert!(after.requests.iter().any(|r| matches!(r.payload, HostRequest::ExecuteTool { .. }))); }
    else { assert!(after.requests.is_empty()); }
}

#[test]
fn native_shell_policy_is_frozen_and_cannot_be_dropped_for_an_external_child() {
    let dir=tempfile::tempdir().unwrap();let mut store=nola_store::NolaStore::open(&dir.path().join("policy.sqlite")).unwrap();
    let mut parent=parent_snapshot().snapshot().unwrap();
    parent.process_sandbox=Some(json!({"scope":"native_shell","fixture":"frozen-host-policy"}));
    let snapshot=crate::delegation_preparation::capture_delegated_invocation(&mut store,&parent,"owner","workspace").unwrap();
    assert_eq!(snapshot.process_sandbox,parent.process_sandbox);
    parent.process_sandbox.as_mut().unwrap()["fixture"]=json!("later-host-default");
    assert_eq!(snapshot.process_sandbox.as_ref().unwrap()["fixture"],"frozen-host-policy");
    assert!(crate::delegation_preparation::capture_delegated_invocation(&mut store,&parent,"owner","codex").unwrap_err().contains("native process sandbox"));
}

#[test]
fn worktree_binding_and_required_sandbox_are_frozen_by_core() {
    use nola_domain::task_workspace::*;
    let dir = tempfile::tempdir().unwrap();
    let mut store = nola_store::NolaStore::open(&dir.path().join("workspace.sqlite")).unwrap();
    let parent = parent_snapshot().snapshot().unwrap();
    let original = crate::delegation_preparation::capture_delegated_invocation(&mut store, &parent, "owner", "explore").unwrap();
    let binding = TaskWorktreeBinding {
        selection: TaskWorktreeSelection { source_snapshot:None, expected_directory_identity:None, root:"/fixture/child".into(), expected_head:"a".repeat(40), expected_common_directory:"/fixture/.git".into() },
        git_directory:"/fixture/.git/worktrees/child".into(), root_identity:"root:1".into(), git_directory_identity:"git:2".into(), common_directory_identity:"common:3".into(),
    };
    let args = json!({"worktree":binding.selection});
    let bind = crate::delegation_preparation::bind_task_workspace;
    assert!(bind(args.as_object().unwrap(), &parent, None, original.clone(), false).is_err());
    assert!(bind(json!({}).as_object().unwrap(), &parent, Some(&binding), original.clone(), false).is_err());
    let frozen = bind(args.as_object().unwrap(), &parent, Some(&binding), original.clone(), false).unwrap();
    assert_eq!(frozen.worktree.as_ref(),Some(&binding));
    for backend in ["codex","claude_code","opencode"] {
        let mut external=original.clone(); external.execution_backend=backend.into(); external.profile.tools.clear();
        let external=bind(args.as_object().unwrap(),&parent,Some(&binding),external,false).unwrap();
        assert_eq!(external.worktree.as_ref(),Some(&binding));
        assert!(!external.bounded_file_writes,"harness tools cannot inherit Nola's file-only guarantee");
        assert!(external.sandbox_requirement.is_none(),"a worktree does not attest an OS sandbox");
        let restored=serde_json::from_value(serde_json::to_value(external).unwrap()).unwrap();
        assert!(bind(json!({}).as_object().unwrap(),&parent,Some(&binding),restored,true).is_ok());
    }
    let persisted = serde_json::to_string(&frozen).unwrap();
    let restored = serde_json::from_str(&persisted).unwrap();
    assert!(bind(json!({}).as_object().unwrap(), &parent, Some(&binding), restored, true).is_ok());
    let mut replaced = binding.clone(); replaced.root_identity="replacement:4".into();
    assert!(bind(args.as_object().unwrap(), &parent, Some(&replaced), frozen, true).is_err());
    let mut foreign=binding.clone();
    foreign.selection.expected_common_directory="/fixture/other/.git".into();
    foreign.git_directory="/fixture/other/.git/worktrees/child".into();
    let foreign_args=json!({"worktree":foreign.selection});
    assert!(bind(foreign_args.as_object().unwrap(),&parent,Some(&foreign),original.clone(),false).unwrap_err().contains("different repository"));
    let mut replaced_repository=binding.clone();replaced_repository.common_directory_identity="another-repository-at-same-path".into();
    assert!(bind(args.as_object().unwrap(),&parent,Some(&replaced_repository),original.clone(),false).unwrap_err().contains("different repository"));
    let mut unobserved=parent.clone();unobserved.project_repository_identity=None;
    assert!(bind(args.as_object().unwrap(),&unobserved,Some(&binding),original.clone(),false).is_err());
    let mut wrong_source=binding.clone();
    wrong_source.selection.source_snapshot=Some(nola_domain::task_workspace::SourceSnapshotBinding {
        source_root:"/fixture/other-source".into(),tree:"d".repeat(40),
        plan:nola_domain::task_workspace::SourceSnapshotPlan {source_head:"a".repeat(40),state_sha256:"b".repeat(64)},
    });
    assert!(bind(json!({"worktree":wrong_source.selection}).as_object().unwrap(),&parent,Some(&wrong_source),original.clone(),false)
        .unwrap_err().contains("frozen parent checkout"));
    let mut inherited=wrong_source.clone();
    inherited.selection.source_snapshot.as_mut().unwrap().source_root=parent.project_root.clone();
    let mut child_parent=parent.clone();child_parent.project_root=inherited.selection.root.clone();
    child_parent.authorized_workspace_roots=vec![inherited.selection.root.clone()];
    child_parent.task_worktree=Some(inherited.clone());
    assert!(bind(json!({}).as_object().unwrap(),&child_parent,Some(&inherited),original.clone(),false).is_ok(),
        "Read-only inheritance keeps the original source snapshot without granting source access");
    let mut revoked=parent.clone(); revoked.authorized_workspace_roots=vec!["/other".into()];
    assert!(bind(args.as_object().unwrap(), &revoked, Some(&binding), original.clone(), false).is_err());
    let required=json!({"sandbox_requirement":"codex_workspace_write"});
    assert!(bind(required.as_object().unwrap(), &parent, None, original.clone(), false).is_err());
    let mut codex=original; codex.execution_backend="codex".into();
    assert!(bind(required.as_object().unwrap(), &parent, None, codex.clone(), false).is_ok());
    let mut unrestricted=parent; unrestricted.requested_native_sandbox=Some("danger-full-access".into());
    assert!(bind(required.as_object().unwrap(), &unrestricted, None, codex, false).is_err());
}

#[test]
fn bounded_file_scope_requires_exact_native_capability_facts() {
    let dir=tempfile::tempdir().unwrap();let mut store=nola_store::NolaStore::open(&dir.path().join("scope.db")).unwrap();
    let mut parent=parent_snapshot().snapshot().unwrap();
    parent.bounded_file_tools=["write_file","edit_file","edit_file_lines","delete_file"].into_iter().map(str::to_string).collect();
    parent.read_only_tools=parent.profiles["workspace"].tools.iter().filter(|name|!parent.bounded_file_tools.contains(*name)).cloned().collect();
    let mut snapshot=crate::delegation_preparation::capture_delegated_invocation(&mut store,&parent,"owner","workspace").unwrap();
    snapshot.worktree=Some(nola_domain::task_workspace::TaskWorktreeBinding {
        selection:nola_domain::task_workspace::TaskWorktreeSelection { source_snapshot:None,expected_directory_identity:None,root:"/fixture/child".into(),expected_head:"a".repeat(40),expected_common_directory:"/fixture/.git".into()},
        git_directory:"/fixture/.git/worktrees/child".into(),root_identity:"root".into(),git_directory_identity:"git".into(),common_directory_identity:"common:3".into(),
    });
    assert!(crate::delegation_preparation::bounded_file_profile(&parent,&snapshot));
    let mut broad=snapshot.clone();broad.profile.tools.clear();
    assert!(!crate::delegation_preparation::bounded_file_profile(&parent,&broad));
    broad=snapshot.clone();broad.profile.tools.push("bash".into());
    assert!(!crate::delegation_preparation::bounded_file_profile(&parent,&broad));
    broad=snapshot.clone();broad.execution_backend="codex".into();
    assert!(!crate::delegation_preparation::bounded_file_profile(&parent,&broad));
    snapshot.worktree=None;
    assert!(!crate::delegation_preparation::bounded_file_profile(&parent,&snapshot));
}
