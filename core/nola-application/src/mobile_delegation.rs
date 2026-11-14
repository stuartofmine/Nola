// SPDX-License-Identifier: Apache-2.0
//! Embedded Hosts consume the same durable delegation admission as CLI workers.
//! A prepared task is not executable until its parent TaskPromoted is committed.
use crate::repositories::AgentRuntimeCommandRepositories;
use nola_agent_engine::agent::{
    host::{HostRequest, HostRequestEnvelope},
    AgentCoreConfig,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareMobileDelegatedTask {
    pub request: HostRequestEnvelope,
    pub now_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedMobileDelegatedTask {
    pub prepared: crate::delegation_preparation::PreparedDelegatedTask,
    /// Commit this exact response before waking a child. A replay is a current
    /// task update or terminal-receipt acknowledgement, never queued again.
    pub response: nola_agent_engine::agent::host::HostResponse,
}

/// Read-only result access still validates the exact started tool delivery and
/// parent ownership. A timeout ends only the observation, never the child.
pub(crate) fn observe<R: AgentRuntimeCommandRepositories>(
    store: &mut R, input: PrepareMobileDelegatedTask,
) -> Result<serde_json::Value, String> {
    let (_, owner, _) = crate::started_host_effect::validate(store, &input.request, "execute_tool", input.now_ms)?;
    let HostRequest::ExecuteTool { capability_id, call_id, input: arguments, .. } = &input.request.payload else {
        return Err("Task observation requires a started tool effect".into());
    };
    if !matches!(capability_id.as_str(), "read_task" | "wait_task") { return Err("Unsupported Task observation".into()); }
    let task_id = arguments.get("task_id").and_then(|v| v.as_str()).ok_or("Task observation requires task_id")?;
    let task = store.runtime_task(task_id).map_err(|e| e.to_string())?.filter(|task|
        task.kind == "agent" && task.owner_session_id.as_deref() == Some(owner.as_str()))
        .ok_or("Unknown owned Task")?;
    let terminal = store.runtime_task_terminal_receipt(task_id).map_err(|e| e.to_string())?;
    if let Some(receipt) = &terminal {
        if receipt.owner_session_id != owner || receipt.status != task.status {
            return Err("Task result receipt differs from its owner/state".into());
        }
    }
    let wait_ms = if capability_id == "wait_task" {
        arguments.get("timeout_sec").and_then(|v| v.as_u64()).unwrap_or(30).clamp(1, 300) * 1000
    } else { 0 };
    let response = nola_agent_engine::agent::host::HostResponse::ToolCompleted {
        call_id: call_id.clone(), output: json!({"task":{"id":task.id,"status":task.status,
            "title":task.title,"child_session_id":task.child_session_id,
            "result_summary":if terminal.is_some() { task.result_summary } else { None },
            "error":task.error},"completion_confirmed":terminal.is_some(),
            "run_result":terminal.as_ref().and_then(|r| r.journal_result.as_ref()).map(|r| &r.seal.result),
            "terminal_receipt_id":terminal.as_ref().map(|r| &r.receipt_id)}),
        outcome: nola_domain::host_effect::HostToolOutcome::Succeeded,
        activation: None, full_output_ref: None, artifacts: vec![],
    };
    Ok(json!({"terminal":terminal.is_some(),"wait_ms":wait_ms,"response":response}))
}

/// An exact existing Task input, not a new routing decision or an execution grant.
pub(crate) fn guide<R: AgentRuntimeCommandRepositories>(
    store: &mut R, input: PrepareMobileDelegatedTask,
) -> Result<nola_agent_engine::agent::host::HostResponse, String> {
    let (run, owner, _) = crate::started_host_effect::validate(store, &input.request, "execute_tool", input.now_ms)?;
    let HostRequest::ExecuteTool { capability_id, call_id, input: arguments, .. } = &input.request.payload else {
        return Err("Task guidance requires a started tool effect".into());
    };
    if capability_id != "send_agent_message" { return Err("Task guidance requires send_agent_message".into()); }
    let task = arguments.get("task_id").and_then(|v|v.as_str()).ok_or("Task guidance requires task_id")?;
    let content = arguments.get("message").and_then(|v|v.as_str()).map(str::trim).filter(|v|!v.is_empty())
        .ok_or("Task guidance requires message")?;
    let turn = input.request.turn_id.as_deref().ok_or("Task guidance requires a parent Turn")?;
    let message = format!("{run}:{call_id}");
    let previous = crate::task_guidance::receipt(store, task, &message, &owner, &run)?;
    let at = previous.as_ref().map(|r|r.submitted_at_ms).unwrap_or(input.now_ms);
    let record = crate::task_guidance::submit(store, task.into(), message, turn.into(), content.into(), at)?;
    Ok(nola_agent_engine::agent::host::HostResponse::ToolCompleted {
        call_id: call_id.clone(),
        output: json!({"task_id":record.task_id,"message_id":record.message_id,"status":record.status,"reason":record.reason}),
        outcome: if record.status == "rejected" { nola_domain::host_effect::HostToolOutcome::Failed }
            else { nola_domain::host_effect::HostToolOutcome::Deferred },
        activation: None, full_output_ref: None, artifacts: vec![],
    })
}

pub(crate) fn prepare<R: AgentRuntimeCommandRepositories>(
    store: &mut R,
    input: PrepareMobileDelegatedTask,
) -> Result<PreparedMobileDelegatedTask, String> {
    let (run_id, conversation_id, _) =
        crate::started_host_effect::validate(store, &input.request, "execute_tool", input.now_ms)?;
    let HostRequest::ExecuteTool {
        capability_id,
        call_id,
        input: arguments,
        ..
    } = &input.request.payload
    else {
        return Err("Mobile delegation requires a started tool effect".into());
    };
    if capability_id != "spawn_agent" {
        return Err("Mobile delegation requires spawn_agent".into());
    }
    if store.read_room_context_owner(&conversation_id).map_err(|e|e.to_string())?.is_some() {return Err("experts_cannot_spawn_agents".into());}
    let frozen = store
        .load_run_invocation_config(&run_id)
        .map_err(|e| e.to_string())?
        .ok_or("Mobile parent invocation is missing")?;
    // The shared prepare below verifies the persisted digest and the same
    // started delivery again before creating any durable task.
    let config: AgentCoreConfig = serde_json::from_str(&frozen.invocation_config_json)
        .map_err(|e| format!("Invalid mobile parent invocation: {e}"))?;
    let parent = config
        .turn_identity
        .delegation
        .ok_or("Parent did not admit delegation")?
        .snapshot()?;
    let arguments = arguments
        .as_object()
        .ok_or("Delegation arguments must be an object")?;
    let agent = arguments
        .get("agent_type")
        .and_then(|v| v.as_str())
        .unwrap_or("explore");
    let prompt = arguments
        .get("prompt")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("Delegation prompt is missing")?;
    let previous = arguments.get("task_id").and_then(|v| v.as_str())
        .map(|id| store.runtime_task(id).map_err(|e| e.to_string())?
            .ok_or_else(|| "Previous delegation is missing".to_string())).transpose()?;
    // Continuation keeps its original profile, even when the parent's current
    // profile changed. Validate transport support before creating any new Task;
    // the shared preparation still validates its exact immutable ownership.
    let replay = store.runtime_task_for_delegation(&conversation_id, &run_id, &input.request.effect_id.0)
        .map_err(|e|e.to_string())?.is_some();
    if !replay {
        let snapshot = if let Some(previous) = &previous {
            let metadata: serde_json::Value = serde_json::from_str(&previous.metadata_json).map_err(|e|e.to_string())?;
            serde_json::from_value(metadata["delegated_invocation"].clone()).map_err(|e|e.to_string())?
        } else {
            crate::delegation_preparation::capture_delegated_invocation(store, &parent, &conversation_id, agent)?
        };
        crate::mobile_delegation_configuration::validate_profile_model(store, &parent, &snapshot)?;
    }
    let child_id = if let Some(previous) = previous {
        previous.child_session_id.ok_or("Previous delegation has no child conversation")?
    } else {
        format!("agent-{}", uuid::Uuid::new_v4().simple())
    };
    let title = arguments
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(agent);
    let task = crate::CreateRuntimeTask {
        id: format!("task-{}", uuid::Uuid::new_v4().simple()),
        kind: "agent".into(),
        title: title.into(),
        project_root: parent.project_root,
        owner_session_id: Some(conversation_id),
        parent_task_id: None,
        parent_tool_call_id: Some(call_id.clone()),
        child_session_id: Some(child_id),
        agent_type: Some(agent.into()),
        prompt: Some(prompt.into()),
        initial_progress_summary: None,
        initial_activity: None,
        output_path: None,
        initially_unread: false,
        input_watermark: Some(input.now_ms),
        metadata: json!({}),
        created_at_ms: input.now_ms,
        // The shared resolver derives all budgets and permission narrowing
        // from the concrete tool arguments and immutable parent snapshot.
        scheduling: crate::RuntimeTaskScheduling {
            priority: Default::default(),
            queued_at_ms: input.now_ms,
            resource_budget: Default::default(),
            output_bytes: 0,
            resource_usage: Default::default(),
            budget_exhaustion: None,
        },
    };
    let parent_call_id = call_id.clone();
    let prepared = crate::delegation_preparation::prepare(
        store,
        crate::delegation_preparation::PrepareDelegatedTask {
            observed_worktree: None,
            request: input.request,
            input: task,
            now_ms: input.now_ms,
        },
    )?;
    let response = if prepared.source_admitted {
        use nola_agent_engine::agent::host::HostResponse;
        let task = store
            .runtime_task(&prepared.task.id)
            .map_err(|e| e.to_string())?
            .ok_or("Replayed task is missing")?;
        if matches!(task.status.as_str(), "sealed" | "failed" | "cancelled") {
            let receipt = store
                .runtime_task_terminal_receipt(&task.id)
                .map_err(|e| e.to_string())?
                .ok_or("Terminal task has no exact receipt")?;
            crate::task_terminal::enqueue(store, &task.id, &receipt.receipt_id)?;
            HostResponse::ToolCompleted {
                call_id: parent_call_id,
                output: json!({"task_id":task.id,"status":task.status,"replayed":true,"terminal_receipt_delivery":"queued"}),
                outcome: nola_domain::host_effect::HostToolOutcome::Succeeded,
                activation: None,
                full_output_ref: None,
                artifacts: vec![],
            }
        } else {
            let admission = crate::task_ownership::authority(store, &task)?;
            let projection = crate::task_ownership::core_task(&task, &admission)?;
            HostResponse::TaskUpdated {
                task_id: task.id,
                state: projection.state,
                progress: projection.progress,
                current_step: projection.current_step,
                result_ref: projection.result_ref,
                cancel_requested: task.cancel_requested,
                ownership_token: task.ownership_token,
                completion_watermark: task.completion_watermark,
            }
        }
    } else {
        use nola_agent_engine::agent::{
            host::HostResponse,
            task::{TaskKind, TaskLineage, TaskState},
            CallId, SessionId,
        };
        let task = &prepared.task;
        let metadata: serde_json::Value =
            serde_json::from_str(&task.metadata_json).map_err(|e| e.to_string())?;
        HostResponse::TaskPromoted {
            task_id: task.id.clone(),
            input_watermark: task.input_watermark,
            kind: TaskKind::Agent,
            state: TaskState::Queued,
            progress: None,
            current_step: None,
            lineage: TaskLineage {
                parent_task_id: task.parent_task_id.clone(),
                resumed_from_task_id: metadata
                    .get("resumed_from_task_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
                child_session_id: Some(SessionId::from_string(
                    task.child_session_id
                        .clone()
                        .ok_or("Prepared child identity missing")?,
                )),
                parent_call_id: Some(CallId::from_string(
                    task.parent_tool_call_id
                        .clone()
                        .ok_or("Prepared parent call missing")?,
                )),
                permission_scope_id: Some(format!("task:{}", task.id)),
                cancellation_scope_id: Some(format!("task:{}", task.id)),
                ..Default::default()
            },
        }
    };
    Ok(PreparedMobileDelegatedTask { prepared, response })
}

/// Same-Store admission: resolve the original committed source internally.
/// Native callers cannot supply or fabricate an admission certificate.
pub(crate) fn admit_source<R: AgentRuntimeCommandRepositories>(
    store: &mut R,
    parent_conversation_id: &str,
    task_id: &str,
) -> Result<nola_domain::agent_task_admission::RuntimeAgentTaskAuthority, String> {
    let (run_id, cursor) = store
        .source_agent_task_admission_location(parent_conversation_id, task_id)
        .map_err(|e| e.to_string())?
        .ok_or("Parent TaskPromoted has not committed")?;
    let admission = crate::agent_task_admission::export(
        store,
        parent_conversation_id,
        &run_id,
        task_id,
        cursor,
    )?;
    crate::agent_task_admission::import(store, admission)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmitMobileTaskChild {
    pub task_id: String,
    pub parent_conversation_id: String,
    pub worker: nola_host_protocol::HostWorkerIdentity,
    pub now_ms: i64,
    pub lease_until_ms: i64,
    pub candidate_config: AgentCoreConfig,
    /// Fresh Host tool observations; Core narrows them to the frozen child set.
    /// These never replace the frozen schemas, permissions or execution binding.
    pub current_tools: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmittedMobileTaskChild {
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub expert_target: Option<Box<nola_domain::expert_task_target::ExpertTaskTargetV1>>,
    pub task_id: String,
    pub parent_conversation_id: String,
    pub attempt: nola_domain::task_execution::RuntimeTaskAttempt,
    pub cancellation_only: bool,
    pub invocation: crate::ExactRunInvocation,
    pub current: crate::RunRecoveryAuthority,
}

/// Presentation facts for an exact claimed permission effect. A null
/// domain means no platform permission prompt is required; it never grants the
/// requested operation or bypasses the user's Core permission decision.
pub(crate) fn inspect_permission<R: AgentRuntimeCommandRepositories>(
    store: &mut R, mut request: HostRequestEnvelope, now_ms: i64,
) -> Result<serde_json::Value, String> {
    // The native presentation retains its originally delivered envelope while
    // the shared driver renews that same owner. Refresh only the expiry; the
    // exact validator below still requires the original attempt/token/worker.
    if let Some(lease) = request.delivery.as_mut() {
        if let Some(current) = store.active_effect_delivery_attempt(&request.effect_id.0)
            .map_err(|e| e.to_string())? {
            if let Some(expiry) = current.lease_expires_at_ms {
                if lease.lease_expires_at_ms > expiry {
                    return Err("Permission presentation supplied an uncommitted lease expiry".into());
                }
                lease.lease_expires_at_ms = expiry;
            }
        }
    }
    let (run_id, conversation_id, _) = crate::started_host_effect::validate_permission_presentation(
        store, &request, now_ms)?;
    let HostRequest::RequestPermission { capability_id, permission_request_id, .. } = &request.payload else {
        return Err("Expected an exact permission Host request".into());
    };
    let frozen = store.load_run_invocation_config(&run_id).map_err(|e| e.to_string())?
        .ok_or("Permission Run configuration missing")?;
    let config = crate::load_run_invocation_config(store, &run_id, &frozen.invocation_config_digest)?;
    let catalog = config.host_catalog.as_ref().ok_or("Permission catalog missing")?;
    let capability = match catalog.capabilities.get(capability_id) {
        Some(value) => value.clone(),
        None => current_mcp_capability(store, &run_id, &conversation_id, capability_id)?,
    };
    let display_title = if capability_id == "mcp_connect" {
        let selected = match &request.payload { HostRequest::RequestPermission {input,..} => input.get("connection_ref"),_=>None };
        capability.input_schema.pointer("/properties/connection_ref/anyOf").and_then(|v|v.as_array())
            .and_then(|entries|entries.iter().find(|v|v.get("const")==selected))
            .and_then(|v|v["title"].as_str()).unwrap_or(&capability.title).to_string()
    } else {capability.title.clone()};
    Ok(json!({
        "conversation_id": conversation_id, "run_id": run_id,
        "core_session_id": request.session_id, "effect_id": request.effect_id,
        "request_id": permission_request_id, "payload": request.payload,
        "permission_domain": capability.permission_domain, "display_title":display_title,"capability": capability,
        "catalog_revision": request.catalog_revision, "availability_epoch": request.availability_epoch,
    }))
}

/// Read-only discovery. The due time is a scheduling hint, never a lease or
/// permission to execute. Native hosts must still call `admit` for each item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MobileTaskWakePlan {
    pub observed_at_ms: i64,
    pub next_wake_at_ms: Option<i64>,
    pub candidates: Vec<MobileTaskWakeCandidate>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MobileTaskWakeCandidate {
    pub task_id: String,
    pub parent_conversation_id: String,
    pub child_conversation_id: String,
    pub parent_invocation: crate::ExactRunInvocation,
    pub parent_config: AgentCoreConfig,
}

pub(crate) fn wake_plan<R: AgentRuntimeCommandRepositories>(
    store: &R, now_ms: i64,
) -> Result<MobileTaskWakePlan, String> {
    if now_ms < 0 { return Err("Invalid mobile wake clock".into()); }
    let mut plan = MobileTaskWakePlan {
        observed_at_ms: now_ms, next_wake_at_ms: None, candidates: Vec::new(),
    };
    for task in store.list_all_runtime_tasks().map_err(|e| e.to_string())? {
        if task.kind != "agent" || !(matches!(task.status.as_str(), "queued" | "running" | "retry_waiting")
            || task.cancel_requested && matches!(task.status.as_str(), "waiting_permission" | "suspended")) {
            continue;
        }
        let (Some(parent), Some(child)) = (task.owner_session_id.as_ref(), task.child_session_id.as_ref()) else { continue; };
        // Only tasks with a real TaskPromoted in this Store can be discovered.
        // Imported paired/remote task projections do not authorize local work.
        let Some((parent_run, _)) = store.source_agent_task_admission_location(parent, &task.id)
            .map_err(|e| e.to_string())? else { continue; };
        let Some(header) = store.inspect_exact_run(&parent_run).map_err(|e| e.to_string())? else {
            return Err("Mobile task source Run is missing".into());
        };
        if header.source_host_id.is_some() { continue; }
        if header.conversation_id != *parent || !header.assistant_owner_valid {
            return Err("Mobile task parent ownership mismatch".into());
        }
        let due = if task.cancel_requested {
            Some(store.runtime_task_current_attempt(&task.id).map_err(|e| e.to_string())?
                .map_or(now_ms, |a| a.lease_expires_at_ms.max(now_ms)))
        } else { match task.status.as_str() {
            "running" => store.runtime_task_current_attempt(&task.id).map_err(|e| e.to_string())?
                .filter(|a| a.state == "leased").map(|a| a.lease_expires_at_ms),
            "retry_waiting" => task.retry_at_ms,
            _ => Some(now_ms),
        }};
        let Some(due) = due else { continue; };
        if due > now_ms {
            plan.next_wake_at_ms = Some(plan.next_wake_at_ms.map_or(due, |old| old.min(due)));
            continue;
        }
        let assistant = header.assistant_message_id.ok_or("Mobile task parent assistant missing")?;
        let invocation = crate::load_exact_run_invocation(store, parent.clone(), parent_run.clone(), assistant)?
            .ok_or("Mobile task parent invocation missing")?;
        let config = crate::load_run_invocation_config(store, &parent_run, &invocation.invocation_config_digest)?;
        if config.turn_identity.delegation.is_none()
            && store.read_validated_expert_task_target(&task.id, child, false).map_err(|e|e.to_string())?.is_none() { continue; }
        plan.candidates.push(MobileTaskWakeCandidate {
            task_id: task.id, parent_conversation_id: parent.clone(), child_conversation_id: child.clone(),
            parent_invocation: invocation, parent_config: config,
        });
    }
    Ok(plan)
}

/// Native code supplies physical execution facts, never a task ledger, Run ID,
/// user message, or replacement source admission. All are selected here under
/// the Application publication transaction before the mailbox can be opened.
pub(crate) fn admit<R: AgentRuntimeCommandRepositories>(
    store: &mut R,
    input: AdmitMobileTaskChild,
) -> Result<Option<AdmittedMobileTaskChild>, String> {
    crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
        let authority = admit_source(store, &input.parent_conversation_id, &input.task_id)?;
        let admission = authority
            .admission
            .ok_or("Task source admission is not executable")?;
        let binding = store
            .runtime_task_run_binding(&input.task_id)
            .map_err(|e| e.to_string())?;
        if let Some(binding) = &binding {
            if binding.owner_session_id != input.parent_conversation_id
                || binding.child_session_id != admission.child_conversation_id
            {
                return Err("Child Run binding differs from immutable task admission".into());
            }
            if store
                .reconcile_runtime_task_bound_run_terminal(
                    &input.task_id,
                    &binding.run_id,
                    input.now_ms,
                )
                .map_err(|e| e.to_string())?
            {
                return Ok(None);
            }
        }
        let task = store
            .runtime_task(&input.task_id)
            .map_err(|e| e.to_string())?
            .ok_or("Admitted task is missing")?;
        let config = crate::mobile_delegation_configuration::validated_child_config(
            store,
            &input.task_id,
            input.candidate_config,
        )?;
        if task.cancel_requested {
            let Some(binding) = binding else { return Ok(None); };
            let previous = store.runtime_task_current_attempt(&input.task_id)
                .map_err(|e| e.to_string())?.ok_or("Cancelled child attempt is missing")?;
            if previous.lease_expires_at_ms > input.now_ms { return Ok(None); }
            let active = store.active_run(&binding.child_session_id).map_err(|e| e.to_string())?
                .ok_or("Cancelled child has no active bound Run")?;
            if active.run_id != binding.run_id { return Err("Cancelled child Run differs".into()); }
            let assistant = active.assistant_message_id.ok_or("Cancelled child assistant is missing")?;
            crate::request_run_cancellation(store, crate::RequestRunCancellation {
                conversation_id: binding.child_session_id.clone(), expected_run_id: binding.run_id.clone(),
                assistant_message_id: assistant.clone(),
                cancellation_id: format!("task-cancel:{}:{}", task.id, binding.run_id),
                reason: Some("task cancellation requested".into()),
                requested_at_ms: task.cancel_requested_at_ms.ok_or("Task cancellation timestamp is missing")?,
            })?;
            if store.reconcile_runtime_task_bound_run_terminal(&task.id, &binding.run_id, input.now_ms)
                .map_err(|e| e.to_string())? { return Ok(None); }
            let invocation = crate::load_exact_run_invocation(store, binding.child_session_id,
                binding.run_id, assistant)?.ok_or("Cancelled child invocation is missing")?;
            let current = crate::RunRecoveryAuthority {
                host_catalog: config.host_catalog, tools: config.tools,
                permission_rules: config.permission_rules, context_window: config.context_window,
                host_invocation: config.turn_identity.host_invocation.ok_or("Child invocation binding is missing")?,
            };
            // The old attempt is an identity witness, NOT a renewed execution lease.
            // Only the persisted cancellation may be drained; no new model/tool work.
            let expert_target = store.read_validated_expert_task_target(&task.id, &task.child_session_id.clone().ok_or("Task child missing")?, true).map_err(|e|e.to_string())?.map(Box::new);
            return Ok(Some(AdmittedMobileTaskChild { expert_target, task_id: task.id,
                parent_conversation_id: input.parent_conversation_id, attempt: previous,
                cancellation_only: true, invocation, current }));
        }
        if store.read_validated_expert_task_target(&task.id,&admission.child_conversation_id,true).map_err(|e|e.to_string())?.is_some(){
            crate::expert_peer_task::validate_saved_expert_leaf(&config)?;
        }
        let current = crate::RunRecoveryAuthority {
            host_catalog: config.host_catalog.clone(),
            tools: crate::run_recovery_authority::select_fresh_tool_observations(
                &config.tools, input.current_tools)?,
            permission_rules: config.permission_rules.clone(),
            context_window: config.context_window.clone(),
            host_invocation: config
                .turn_identity
                .host_invocation
                .clone()
                .ok_or("Child invocation binding is missing")?,
        };
        crate::run_recovery_authority::validate_facts(&config, &current,
            u64::try_from(input.now_ms).map_err(|_| "Invalid mobile admission clock")?)?;
        // A crashed native process cannot reclaim a still-live lease. Once
        // expired, use the same durable recovery transition as other Hosts,
        // preserving the bound child Run and all unknown-effect evidence.
        if let Some(previous) = store.runtime_task_current_attempt(&input.task_id)
            .map_err(|e| e.to_string())? {
            if previous.state == "leased" {
                if previous.lease_expires_at_ms > input.now_ms { return Ok(None); }
                crate::task_ownership::recover(store, crate::task_ownership::RecoverExpiredAgentTask {
                    owner_conversation_id: input.parent_conversation_id.clone(),
                    lease: (&previous).into(), error: "embedded worker lease expired".into(),
                    retry_at_ms: input.now_ms, now_ms: input.now_ms,
                })?;
            }
        }
        let task_state = store.runtime_task(&input.task_id).map_err(|e| e.to_string())?
            .ok_or("Admitted task is missing")?;
        if !matches!(task_state.status.as_str(), "queued" | "retry_waiting")
            || task_state.retry_at_ms.is_some_and(|due| due > input.now_ms) {
            return Ok(None);
        }
        let worker = input.worker.lease_identity()?;
        let attempt = crate::task_ownership::claim(
            store,
            crate::task_ownership::ClaimAgentTaskFair {
                owner_conversation_id: input.parent_conversation_id.clone(),
                task_id: input.task_id.clone(),
                worker_id: worker.lease_owner,
                worker_generation: worker.worker_generation,
                lease_token: uuid::Uuid::new_v4().to_string(),
                now_ms: input.now_ms,
                lease_until_ms: input.lease_until_ms,
                max_running_global: 4,
                max_running_project: 2,
            },
        )?;
        let Some(attempt) = attempt else {
            return Ok(None);
        };
        let child = admission.child_conversation_id;
        let (run_id, assistant_message_id) = if let Some(binding) = binding {
            if binding.owner_session_id != input.parent_conversation_id
                || binding.child_session_id != child
            {
                return Err("Child Run binding differs from immutable task admission".into());
            }
            let active = store
                .active_run(&child)
                .map_err(|e| e.to_string())?
                .ok_or("Bound child has no active Run; reconcile its committed terminal")?;
            if active.run_id != binding.run_id {
                return Err("Child active Run differs from task binding".into());
            }
            (
                binding.run_id,
                active
                    .assistant_message_id
                    .ok_or("Child assistant owner is missing")?,
            )
        } else {
            let tail = store
                .load_conversation_message_window(&child, None, 1, 1)
                .map_err(|e| e.to_string())?;
            let user_id = format!("{}:user", input.task_id);
            let prompt = task.prompt.ok_or("Delegation prompt is missing")?;
            let item = nola_agent_engine::agent::AgentItem::user_message(
                nola_agent_engine::agent::ItemId::from_string(user_id.clone()),
                prompt.clone(),
            );
            let submission = serde_json::from_value(json!({
                "type":"user_input", "items":[item],
                "submission_id":input.task_id,"submitted_at_epoch_ms":input.now_ms
            }))
            .map_err(|e| format!("Construct child submission: {e}"))?;
            let (accepted, _) = crate::assistant_turn::admit_with_core(
                store,
                crate::BeginAssistantTurn {
                    conversation_id: child.clone(),
                    expected_configuration_revision: tail
                        .as_ref()
                        .map(|v| v.conversation.configuration_revision),
                    expected_parent_message_id: tail
                        .as_ref()
                        .and_then(|v| v.conversation.messages.last().map(|m| m.id.clone())),
                    conversation_title: task.title,
                    user_message_id: user_id.clone(),
                    user_part_id: format!("{user_id}:text"),
                    user_text: prompt,
                    user_content_json: None,
                    assistant_message_id: format!("{}:assistant", input.task_id),
                    context_checkpoint_id: None,
                    core_config: config,
                    submission,
                    created_at_ms: input.now_ms,
                },
                Some((&attempt).into()),
            )?;
            (accepted.begin.run_id, accepted.begin.assistant_message_id)
        };
        let invocation = crate::load_exact_run_invocation(
            store,
            child.clone(),
            run_id.clone(),
            assistant_message_id.clone(),
        )?
        .ok_or("Admitted child invocation is missing")?;
        crate::validate_run_recovery_authority(
            store,
            child,
            run_id,
            assistant_message_id,
            invocation.invocation_config_digest.clone(),
            current.clone(),
        )?;
        let expert_target = store.read_validated_expert_task_target(&input.task_id, &invocation.conversation_id, true).map_err(|e|e.to_string())?.map(Box::new);
        Ok(Some(AdmittedMobileTaskChild {
            expert_target,
            task_id: input.task_id,
            parent_conversation_id: input.parent_conversation_id,
            attempt,
            cancellation_only: false,
            invocation,
            current,
        }))
    })
}

// Dynamic MCP activation lives in the committed reducer checkpoint, not the
// initial invocation catalog. Never trust a descriptor supplied by the client.
fn current_mcp_capability<R: AgentRuntimeCommandRepositories>(store:&R,run:&str,conversation:&str,id:&str)
 ->Result<nola_agent_engine::agent::host::HostCapabilityDescriptor,String> {
    let frozen=store.load_run_invocation_config(run).map_err(|e|e.to_string())?.ok_or("MCP Run invocation missing")?;
    let recovery=crate::load_exact_run_checkpoint(store,conversation,run)?;
    let core=crate::restore_run_recovery(store,run,&frozen.invocation_config_digest,&recovery)?;
    let checkpoint=core.checkpoint_view(nola_agent_engine::agent::recovery::CheckpointReason::Manual);
    let descriptor=checkpoint.state.host.catalog.capabilities.get(id).ok_or("MCP capability is not admitted")?;
    if descriptor.input_normalization.origin.kind!=nola_agent_engine::agent::host::HostCapabilityOriginKind::Mcp {
        return Err("Execution capability is not an MCP capability".into());
    }
    Ok(descriptor.clone())
}
pub(crate) fn inspect_mcp<R: AgentRuntimeCommandRepositories>(store:&mut R,request:HostRequestEnvelope,now_ms:i64)->Result<serde_json::Value,String> {
    let (run,conversation,_)=crate::started_host_effect::validate(store,&request,"execute_tool",now_ms)?;
    let HostRequest::ExecuteTool{capability_id,..}=&request.payload else {return Err("Expected admitted MCP tool request".into());};
    let capability=current_mcp_capability(store,&run,&conversation,capability_id)?;
    Ok(json!({"capability":capability,"conversation_id":conversation,"run_id":run,"session_id":request.session_id,"host_id":request.host_id,"effect_id":request.effect_id}))
}
