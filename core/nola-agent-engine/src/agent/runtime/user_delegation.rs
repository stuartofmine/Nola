// SPDX-License-Identifier: Apache-2.0
//! Explicit delegation commands share the normal tool permission/resource lifecycle.
use super::*;

impl AgentRuntime {
    pub(super) fn submit_user_delegation(
        &mut self, agent_type: String, prompt: String, submission_id: String, submitted_at_epoch_ms: i64,
        events: &mut Vec<RuntimeEvent>, effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        if self.session.execution.active_turn.as_ref().is_some_and(|turn| !turn.status.is_terminal())
            || !self.session.host.pending_effects.is_quiescent()
            || self.session.execution.input_queue.has_next_turn_input()
            || self.session.execution.input_queue.has_current_turn_input() {
            return Err("explicit delegation requires an idle settled conversation".into());
        }
        if self.session.configuration.turn_identity.delegation.is_none() {
            return Err("Host has no admitted delegation execution capability".into());
        }
        let item = crate::agent::protocol::user_delegation_item(&agent_type, &prompt, &submission_id)?;
        let items = vec![item];
        let origin = crate::agent::protocol::AgentInputOrigin::InteractiveUser;
        let delivery = AgentInputDelivery::CurrentTurn;
        let digest = crate::agent::input_queue::input_digest(&json!({
            "kind":"user_delegation", "agent_type":agent_type, "prompt":prompt,
            "items":items, "submitted_at_epoch_ms":submitted_at_epoch_ms,
        }));
        let admission_id = self.session.allocate_id("input-admission");
        let batch = self.session.execution.input_queue.admit(admission_id, Some(submission_id.clone()),
            origin, delivery, None, digest, items.clone())?;
        events.push(RuntimeEvent::InputAccepted { admission: batch.admission.clone() });
        let call = crate::agent::protocol::ModelToolCall {
            call_id: CallId::from_string(self.session.allocate_id("user-delegation-call")),
            name: "spawn_agent".into(),
            arguments: json!({"agent_type":agent_type,"description":prompt.chars().take(72).collect::<String>(),"prompt":prompt}),
            permission_scope: None, permission_rationale: None,
        };
        self.start_turn(origin, items, vec![batch], Vec::new(),
            crate::context_engine::ContextPlatform::Shared, Vec::new(), Some(submission_id),
            Some(submitted_at_epoch_ms), Some(call), events, effects)
    }
}

impl AgentRuntime {
    pub(super) fn finish_user_delegation(&mut self, turn_id: &TurnId,
        events: &mut Vec<RuntimeEvent>, effects: &mut Vec<AgentEffect>) -> Result<(), String> {
        let turn = self.session.execution.active_turn.as_ref().ok_or("no delegation command Turn")?;
        if &turn.turn_id != turn_id { return Err("delegation command Turn differs".into()); }
        let call = turn.user_delegation_call.as_ref().ok_or("missing delegation command call")?;
        let tool = turn.tools.iter().find(|tool| &tool.call_id == call && tool.name == "spawn_agent")
            .ok_or("delegation command lost its exact tool")?;
        if !tool.status.is_terminal() { return Ok(()); }
        let task_id = tool.output.as_ref().and_then(|value| value.get("task_id")).and_then(Value::as_str);
        let admitted = task_id.and_then(|id| self.session.task_projection.tasks.get(id))
            .is_some_and(|task| task.owner_session_id == self.session.session_id && &task.owner_turn_id == turn_id
                && task.lineage.parent_call_id.as_ref() == Some(call));
        if tool.is_error || !admitted {
            let error = "delegation command did not produce an exact admitted Task".to_owned();
            let turn = self.session.execution.active_turn.as_mut().ok_or("no delegation Turn")?;
            turn.status = AgentTurnStatus::Failed;
            turn.failure = Some(error.clone());
            events.push(RuntimeEvent::TurnFailed { session_id: self.session.session_id.clone(), turn_id: turn_id.clone(), error });
            return Ok(());
        }
        // The result is produced only on TaskPromoted's normal complete_tool path.
        // Its immutable source is rechecked by Store in the same sealing transaction.
        self.complete_turn(turn_id, events, effects)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::agent::host::*;
    use nola_host_protocol::delegation::*;

    pub(in crate::agent::runtime) fn runtime(with_catalog: bool) -> AgentRuntime {
        let endpoint = "https://fixture.invalid/v1";
        let parent = DelegationParentSnapshot {
        process_sandbox: None,
            bounded_file_tools: Default::default(),
            project_repository_identity: None,
            task_worktree: None, task_sandbox_requirement: None,
            authorized_workspace_roots: Vec::new(), requested_native_sandbox: None,
            model: DelegatedModel { endpoint: endpoint.into(), model: "fixture".into(), swe_mode: false },
            runtime: DelegatedRuntimeConfiguration { provider: Some("chat_completions".into()), ..Default::default() },
            credential: Some(CredentialReference::inherited(route_identity(endpoint, Some("chat_completions")).unwrap())),
            fallback_credentials: vec![], profiles: nola_domain::delegation::effective_agent_profiles(&Default::default()),
            project_root: "/fixture".into(), read_only_tools: Default::default(), profile_credentials: Default::default(),
            execution_backend: "nola".into(), external_execution_model: None, external_execution_model_provider: None,
            max_iterations: 4,
        };
        let mut identity = crate::agent::model_capability::test_host_identity();
        identity.delegation = Some(ValidatedDelegationParent::new(parent).unwrap());
        let mut catalog = HostCapabilityCatalog::empty("fixture-host");
        if with_catalog {
            catalog.capabilities.insert("spawn_agent".into(), HostCapabilityDescriptor {
                id: "spawn_agent".into(), title: "Delegate".into(), route: HostCapabilityRoute::Tool,
                availability: CapabilityAvailability::Available, exposure: HostCapabilityExposure::default(),
                permission_domain: Some("agent".into()), input_schema: json!({"type":"object",
                    "properties":{"agent_type":{"type":"string"},"description":{"type":"string"},"prompt":{"type":"string"}},
                    "required":["agent_type","description","prompt"],"additionalProperties":false}),
                input_normalization: HostInputNormalization {
                    effect: HostCapabilityEffectDeclaration::fixed(HostCapabilityEffect::Mutate), ..Default::default()
                },
                output_policy: HostOutputPolicy { preview_char_limit: 1000, persist_full_output: false, allow_artifacts: false },
                supports_streaming: false, supports_argument_streaming: false,
                parallelism: Default::default(), cancellation: Default::default(), replay_safety: Default::default(),
            });
        }
        AgentRuntime::try_new(AgentCoreConfig { turn_identity: identity,
            session_id: Some(SessionId::from_string("command-session".into())), host_catalog: Some(catalog),
            ..Default::default() }).unwrap()
    }
    fn command(id: &str, prompt: &str) -> AgentSubmission {
        AgentSubmission::UserDelegation { agent_type: "general".into(), prompt: prompt.into(),
            submission_id: id.into(), submitted_at_epoch_ms: 1000 }
    }
    #[test]
    fn explicit_command_admits_one_tool_without_a_model_projection() {
        let mut runtime = runtime(true);
        let result = runtime.submit(command("command", "核查文件 / verify files"), 1000).unwrap();
        let turn = runtime.session.execution.active_turn.as_ref().unwrap();
        assert_eq!(turn.model_request_count, 0);
        assert!(turn.last_model_tool_projection.is_none());
        assert_eq!(turn.tools.len(), 1);
        assert_eq!(turn.tools[0].name, "spawn_agent");
        assert_eq!(turn.user_delegation_call.as_ref(), Some(&turn.tools[0].call_id));
        assert!(result.host_requests.iter().all(|request| !matches!(request.payload,
            HostRequest::ModelStream { .. } | HostRequest::DelegatedAgent { .. })));
        assert!(runtime.session.conversation.transcript.iter().any(|item|
            item == &crate::agent::protocol::user_delegation_item("general", "核查文件 / verify files", "command").unwrap()));
        // Real policy/resource planning must run; no direct execution permit is fabricated.
        assert!(turn.tools[0].permission_resolution_request.is_some()
            || turn.tools[0].status == ToolCallStatus::AwaitingPermission);
    }
    #[test]
    fn absent_host_capability_fails_without_trying_a_model() {
        let mut runtime = runtime(false);
        runtime.submit(command("unsupported", "check"), 1000).unwrap();
        let turn = runtime.session.execution.active_turn.as_ref().unwrap();
        assert_eq!(turn.status, AgentTurnStatus::Failed);
        assert_eq!(turn.model_request_count, 0);
    }
    #[test]
    fn command_replay_is_content_bound_and_busy_commands_do_not_queue() {
        let mut runtime = runtime(true);
        runtime.submit(command("same", "check"), 1000).unwrap();
        assert!(runtime.submit(command("same", "check"), 1001).unwrap().event_envelopes.is_empty());
        assert!(runtime.submit(command("same", "different"), 1002).is_err());
        assert!(runtime.submit(command("other", "check"), 1003).is_err());
        assert!(!runtime.session.execution.input_queue.has_next_turn_input());
    }
    #[test]
    fn no_frozen_delegation_capability_is_not_a_tool_grant() {
        let mut runtime = runtime(true);
        runtime.session.configuration.turn_identity.delegation = None;
        assert!(runtime.submit(command("unavailable", "check"), 1000).is_err());
        assert!(runtime.session.execution.active_turn.is_none());
    }
}
