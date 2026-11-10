// SPDX-License-Identifier: Apache-2.0
// Real Application admission, terminal redaction and exact recovery boundaries.
use super::*;
use crate::application_port::CliApplicationPort;
use nola_core::agent::host::*;

fn checkpoint(port: &mut CliApplicationPort) -> CoreCheckpoint<AgentThreadState> {
    serde_json::from_str(
        &port
            .load_checkpoint("source-conversation")
            .unwrap()
            .unwrap()
            .checkpoint_json,
    )
    .unwrap()
}

fn owned_config(checkpoint: &CoreCheckpoint<AgentThreadState>) -> AgentCoreConfig {
    let state = &checkpoint.state;
    AgentCoreConfig {
        automation_execution_spec: None,
        session_id: Some(state.session_id.clone()),
        model: state.configuration.model.clone(),
        max_model_iterations: state.configuration.max_model_iterations,
        max_strategy_recovery_attempts: state.configuration.max_strategy_recovery_attempts,
        initial_items: vec![],
        tools: state.configuration.tools.clone(),
        context_window: state.configuration.context_window.clone(),
        host_catalog: Some(state.host.catalog.clone()),
        permission_rules: state.execution.permission_rules.clone(),
        memory_policy: state.configuration.memory_policy.clone(),
        turn_identity: state.configuration.turn_identity.clone(),
    }
}

fn captured_surface(
    base: &HostCapabilityCatalog,
    name: Option<&str>,
) -> (Vec<Value>, HostCapabilityCatalog) {
    let mut catalog = base.clone();
    catalog
        .capabilities
        .retain(|_, descriptor| descriptor.route != HostCapabilityRoute::Tool);
    catalog.revision += 1;
    let tools = name
        .map(|name| {
            catalog.capabilities.insert(
                name.into(),
                HostCapabilityDescriptor {
                    id: name.into(),
                    title: name.into(),
                    route: HostCapabilityRoute::Tool,
                    availability: CapabilityAvailability::Available,
                    exposure: HostCapabilityExposure::Visible,
                    permission_domain: None,
                    input_schema: json!({"type":"object"}),
                    input_normalization: Default::default(),
                    output_policy: HostOutputPolicy {
                        preview_char_limit: 1000,
                        persist_full_output: true,
                        allow_artifacts: true,
                    },
                    supports_streaming: false,
                    supports_argument_streaming: false,
                    parallelism: HostCapabilityParallelism::Serial,
                    cancellation: HostCancellationMode::Cooperative,
                    replay_safety: HostReplaySafety::Recoverable,
                },
            );
            vec![json!({"type":"function","function":{"name":name,"parameters":{"type":"object"}}})]
        })
        .unwrap_or_default();
    (tools, catalog)
}

fn begin_successor(
    port: &mut CliApplicationPort,
    core_config: AgentCoreConfig,
) -> nola_application::AcceptedAssistantTurn {
    let frontier = port.load_conversation("source-conversation").unwrap().unwrap();
    port.begin_assistant_turn(BeginAssistantTurn {
        expected_configuration_revision: Some(frontier.configuration_revision),
        expected_parent_message_id: frontier.messages.last().map(|message| message.id.clone()),
        conversation_id: "source-conversation".into(),
        conversation_title: "Source".into(),
        user_message_id: "successor-user".into(),
        user_part_id: "successor-user:text".into(),
        user_text: "A genuinely new invocation".into(),
        user_content_json: None,
        assistant_message_id: "successor-root".into(),
        context_checkpoint_id: None,
        core_config,
        submission: AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("successor-user".into()),
                "A genuinely new invocation",
            )],
            turn_context_items: vec![],
            turn_context_platform: Default::default(),
            turn_context_components: vec![],
            submission_id: Some("successor-run".into()),
            submitted_at_epoch_ms: Some(5000),
            delivery: AgentInputDelivery::CurrentTurn,
        },
        created_at_ms: 5000,
    })
    .unwrap()
}

fn first_model_tools(port: &mut CliApplicationPort) -> Vec<String> {
    let requests = port.load_committed_host_requests("successor-run").unwrap();
    requests
        .requests
        .into_iter()
        .find_map(|request| match request.payload {
            HostRequest::ModelStream { tools, .. } => Some(
                tools
                    .into_iter()
                    .filter_map(|tool| {
                        tool.pointer("/function/name")
                            .or_else(|| tool.get("name"))
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .collect(),
            ),
            _ => None,
        })
        .expect("actual newly committed model request")
}

fn check_terminal_to_new_run(cold: bool, available: bool) {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("source.sqlite");
    let (mut port, source) =
        crate::source_journal::source_tests::completed_source_with_port(
            CliApplicationPort::persistent(&database).unwrap(),
            &[("read_file", json!({"observed":true}))],
            "prior answer",
        );
    let previous = checkpoint(&mut port);
    assert!(
        previous.state.configuration.tools.is_empty(),
        "terminal discovery schemas remain redacted"
    );
    assert!(previous
        .state
        .host
        .catalog
        .capabilities
        .values()
        .all(|cap| cap.route != HostCapabilityRoute::Tool));
    assert!(source.seal.is_some());
    drop(source);
    if cold {
        drop(port);
        port = CliApplicationPort::persistent(&database).unwrap();
    }
    let saved = checkpoint(&mut port);
    let (schemas, catalog) = captured_surface(
        &saved.state.host.catalog,
        available.then_some("successor.read"),
    );
    let current =
        new_run_admission_configuration(&owned_config(&saved), saved.state.permits_new_turn_execution_configuration(), &schemas, catalog.clone())
            .unwrap();
    let idle = port
        .reduce_and_commit_idle_submission(nola_application::ReduceAndCommitIdleSubmission {
            conversation_id: "source-conversation".into(),
            core_config: current,
            submission: AgentSubmission::UpdateSessionConfiguration {
                patch: nola_core::agent::protocol::AgentSessionConfigurationPatch {
                    max_model_iterations: Some(0),
                    ..Default::default()
                },
            },
            committed_at_ms: 4000,
        })
        .unwrap();
    assert!(
        idle.execution.configuration.tools.is_empty(),
        "idle commit must not leak live schemas to history"
    );
    let candidate = new_run_admission_configuration(
        &idle.execution.configuration,
        idle.execution.permits_new_turn_configuration,
        &schemas,
        catalog,
    )
    .unwrap();
    assert_eq!(
        candidate.session_id,
        Some(previous.state.session_id.clone()),
        "actual Core identity does not become the canonical conversation/Run"
    );
    assert_eq!(
        candidate.permission_rules,
        idle.execution.configuration.permission_rules
    );
    let accepted = begin_successor(&mut port, candidate);
    assert_eq!(accepted.begin.run_id, "successor-run");
    let actual_tools = first_model_tools(&mut port);
    assert_eq!(actual_tools.iter().any(|name| name == "successor.read"), available,
        "fresh admission uses captured current availability, including an intentionally empty surface");
    assert!(
        !actual_tools.iter().any(|name| name == "read_file"),
        "prior history cannot restore a revoked tool"
    );
    let active = checkpoint(&mut port);
    let before = serde_json::to_value(&active).unwrap();
    let (unrelated, newer_catalog) =
        captured_surface(&active.state.host.catalog, Some("unrelated.scope"));
    assert!(
        AgentCore::restore(owned_config(&active), active.clone()).is_ok(),
        "exact same-Run checkpoint remains recoverable with its frozen configuration"
    );
    let mut wrong_recovery = owned_config(&active);
    wrong_recovery.tools = unrelated.clone();
    wrong_recovery.host_catalog = Some(newer_catalog.clone());
    assert!(
        AgentCore::restore(wrong_recovery, active.clone()).is_err(),
        "Core itself rejects current-scope replacement of an active frozen Run"
    );
    assert!(
        new_run_admission_configuration(&owned_config(&active), active.state.permits_new_turn_execution_configuration(), &unrelated, newer_catalog)
            .is_err(),
        "same-Run active recovery cannot substitute fresh schemas/catalog"
    );
    assert_eq!(
        serde_json::to_value(&active).unwrap(),
        before,
        "rejected transfer does not mutate its owner"
    );
    if cold {
        drop(port);
        port = CliApplicationPort::persistent(&database).unwrap();
    }
    assert_eq!(
        first_model_tools(&mut port),
        actual_tools,
        "same-Run cold reads retain the exact admitted tool projection"
    );
}

#[test]
fn warm_new_admission_rehydrates_current_tools_after_idle_terminal_redaction() {
    check_terminal_to_new_run(false, true);
}

#[test]
fn cold_new_admission_rehydrates_current_tools_after_idle_terminal_redaction() {
    check_terminal_to_new_run(true, true);
}

#[test]
fn new_admission_with_revoked_availability_stays_tool_free_instead_of_falling_back() {
    check_terminal_to_new_run(true, false);
}
