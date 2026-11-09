// SPDX-License-Identifier: Apache-2.0
//! Cooperative adapter for the shared Goal driver. Native code executes only
//! already-admitted exact Runs; no Goal transition is owned by a client loop.
use crate::repositories::AgentRuntimeCommandRepositories;
use nola_agent_engine::agent::{
    protocol::*, recovery::CoreCheckpoint, AgentThreadState, GoalProjection, GoalStatus,
};
use crate::goal_coordinator::{GoalCompleteSignal, GoalRoundResult, GoalRuntimePort};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    time::Duration,
};

type Round = GoalRoundResult<()>;
fn checkpoint<R: AgentRuntimeCommandRepositories>(
    store: &R,
    owner: &str,
) -> Result<CoreCheckpoint<AgentThreadState>, String> {
    let stored = crate::load_current_checkpoint(store, owner)?
        .ok_or("A local Goal requires an existing configured conversation")?;
    let checkpoint: CoreCheckpoint<AgentThreadState> =
        serde_json::from_str(&stored.checkpoint_json).map_err(|_| "invalid Goal checkpoint")?;
    crate::assistant_owner::validate_checkpoint_identity(
        owner,
        &stored.conversation_id,
        &checkpoint.session_id,
        checkpoint.state.session_id.as_str(),
    )
    .map_err(|e| e.to_string())?;
    checkpoint.validate_state_identity()?;
    Ok(checkpoint)
}
fn configuration(state: &AgentThreadState) -> AgentCoreConfig {
    AgentCoreConfig {
        session_id: Some(state.session_id.clone()),
        model: state.configuration.model.clone(),
        max_model_iterations: state.configuration.max_model_iterations,
        max_strategy_recovery_attempts: state.configuration.max_strategy_recovery_attempts,
        initial_items: vec![],
        tools: state.configuration.tools.clone(),
        context_window: state.configuration.context_window.clone(),
        memory_policy: state.configuration.memory_policy.clone(),
        turn_identity: state.configuration.turn_identity.clone(),
        host_catalog: Some(state.host.catalog.clone()),
        permission_rules: state.execution.permission_rules.clone(),
        automation_execution_spec: None,
    }
}
/// The descriptor points to the frozen, Core-owned invocation in this Goal's
/// binding, not another credential store. Native recovery already resolves that
/// invocation through its existing secure model resolver.
fn descriptor(
    config: &AgentCoreConfig,
    owner: &str,
    goal: &str,
) -> Result<nola_host_protocol::HostRunRecoveryReference, String> {
    let binding = config
        .turn_identity
        .host_invocation
        .as_ref()
        .ok_or("Goal requires frozen model configuration")?;
    binding.validate().map_err(str::to_string)?;
    Ok(nola_host_protocol::HostRunRecoveryReference {
        adapter: "embedded-goal-invocation".into(),
        handle: format!("{owner}:{goal}"),
        sha256: format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(binding).map_err(|_| "invalid Goal invocation")?)
        ),
    })
}

pub(super) fn sealed_round<R: AgentRuntimeCommandRepositories>(
    store: &mut R,
    owner: &str,
    run: &str,
) -> Result<Option<Round>, String> {
    use nola_domain::assistant_journal_operations::{
        JournalContentField, JournalNumber, JournalPartMetadata,
    };
    use nola_domain::assistant_run_terminal::AssistantRunTerminalManifestEntry;
    let Some(seal) = store
        .local_assistant_run_terminal_seal(owner, run)
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    seal.validate()?;
    if seal.identity.conversation_id != owner || seal.identity.run_id != run {
        return Err("Goal seal owner mismatch".into());
    }
    // Metadata commands after sealing may replace the current checkpoint.
    // Read only the immutable terminal frontier, as every other source reader does.
    let mut after = JournalNumber(0);
    let mut entries = Vec::new();
    loop {
        let page = store
            .read_local_assistant_run_terminal_manifest(&seal, after, 64, 262144)
            .map_err(|e| e.to_string())?;
        if page.after_entry != after || page.next_entry.0 <= after.0 || page.seal != seal {
            return Err("Goal terminal manifest did not advance".into());
        }
        entries.extend(page.entries);
        after = page.next_entry;
        if page.complete {
            break;
        }
    }
    seal.validate_manifest(&entries)?;
    let mut last = None;
    for entry in entries {
        let AssistantRunTerminalManifestEntry::Assistant {
            through, ordinal, ..
        } = entry
        else {
            continue;
        };
        if through.identity.conversation_id != owner || through.identity.run_id != run {
            return Err("Goal terminal part owner mismatch".into());
        }
        let snapshot = crate::assistant_journal_read::open_snapshot(store, through)?;
        let read = (|| -> Result<(), String> {
            let mut after = None;
            loop {
                let page = crate::assistant_journal_read::read_parts(
                    store,
                    snapshot.clone(),
                    after.as_deref(),
                    64,
                    262144,
                )?;
                for part in page.parts {
                    let JournalPartMetadata::Tool(tool) = &part.metadata else {
                        continue;
                    };
                    if tool.capability_id != "goal_complete" {
                        continue;
                    }
                    let order = (
                        ordinal,
                        part.identity.assistant_message_id.clone(),
                        part.order_key.section,
                        part.order_key.position,
                        part.part_id.clone(),
                    );
                    if last
                        .as_ref()
                        .is_some_and(|(previous, _)| previous >= &order)
                    {
                        continue;
                    }
                    let output = if tool.state == nola_domain::ActivityState::Completed {
                        let content = part
                            .bodies
                            .iter()
                            .find(|body| body.field == JournalContentField::ToolOutput)
                            .ok_or("Goal declaration has no structured output")?;
                        let mut offset = JournalNumber(0);
                        let mut text = String::new();
                        loop {
                            let range = crate::assistant_journal_read::read_range(
                                store,
                                snapshot.clone(),
                                &part.part_id,
                                content,
                                offset,
                                32768,
                            )?;
                            text.push_str(&range.text);
                            if range.end_of_content {
                                break;
                            }
                            if range.next_offset_bytes.0 <= offset.0 {
                                return Err("Goal output range did not advance".into());
                            }
                            offset = range.next_offset_bytes;
                        }
                        let packet: Value = serde_json::from_str(&text)
                            .map_err(|_| "invalid Goal journal output")?;
                        if packet["protocol_version"] != json!(1) {
                            return Err("invalid Goal journal output version".into());
                        }
                        Some(GoalCompleteSignal::from_tool_output(&packet["value"])?)
                    } else {
                        None
                    };
                    last = Some((order, output));
                }
                let Some(next) = page.next_part_id else { break };
                if after.as_ref().is_some_and(|previous| previous >= &next) {
                    return Err("Goal part directory did not advance".into());
                }
                after = Some(next);
            }
            Ok(())
        })();
        let release = crate::assistant_journal_read::release_snapshot(store, snapshot);
        read?;
        release?;
    }
    use nola_domain::run_result::RunResultValue;
    let (status, reason) = match seal.result.value {
        RunResultValue::Answered { .. } | RunResultValue::ActionCompleted { .. } => {
            (nola_domain::AgentTurnStatus::Completed, "completed")
        }
        RunResultValue::Blocked { .. } => (nola_domain::AgentTurnStatus::Completed, "blocked"),
        RunResultValue::Incomplete { .. } => {
            (nola_domain::AgentTurnStatus::Completed, "incomplete")
        }
        RunResultValue::Cancelled { .. } => (nola_domain::AgentTurnStatus::Cancelled, "cancelled"),
        RunResultValue::Failed { .. } => (nola_domain::AgentTurnStatus::Failed, "failed"),
        RunResultValue::DelegationAccepted { .. } => {
            return Err("Delegation admission cannot complete a Goal round".into())
        }
    };
    let pending = checkpoint(store, owner)?
        .state
        .execution
        .input_queue
        .pending_successor()
        .is_some();
    Ok(Some(Round {
        turn_status: status,
        output: String::new(),
        goal_signal: last.and_then(|(_, signal)| signal),
        terminal_reason: Some(reason.into()),
        pending_user_input: pending,
        handoff: (),
    }))
}

struct Port<'a, R> {
    store: RefCell<&'a mut R>,
    owner: String,
    host: String,
    config: AgentCoreConfig,
    now: i64,
    handoff: Cell<bool>,
    run: RefCell<Option<String>>,
}
impl<R: AgentRuntimeCommandRepositories> Port<'_, R> {
    fn binding(&self) -> Result<crate::goal_execution::GoalExecutionBinding, String> {
        let goal = self
            .load_projection()?
            .and_then(|p| p.active)
            .ok_or("Goal is unavailable")?;
        let binding = crate::goal_execution::prepare_goal_execution(
            &mut **self.store.borrow_mut(),
            &self.owner,
            &goal.id,
        )?;
        if binding.identity.source_host_id != self.host {
            return Err("Goal belongs to a different execution Host".into());
        }
        if binding
            .template
            .core_config
            .turn_identity
            .host_recovery
            .as_ref()
            .is_some_and(|reference| reference.adapter == "embedded-goal-invocation")
            && binding
                .template
                .core_config
                .turn_identity
                .host_recovery
                .as_ref()
                != Some(&descriptor(
                    &binding.template.core_config,
                    &self.owner,
                    &goal.id,
                )?)
        {
            return Err("Goal recovery descriptor changed".into());
        }
        Ok(binding)
    }
    fn round(&self, run: &str) -> Result<Round, String> {
        if let Some(result) = sealed_round(&mut **self.store.borrow_mut(), &self.owner, run)? {
            return Ok(result);
        }
        let checkpoint = checkpoint(&**self.store.borrow(), &self.owner)?;
        let turn = checkpoint
            .state
            .execution
            .active_turn
            .as_ref()
            .filter(|t| t.turn_id.as_str() == run)
            .ok_or("Goal Run has no exact active Turn")?;
        *self.run.borrow_mut() = Some(run.into());
        self.handoff.set(true);
        Ok(Round {
            turn_status: turn.status,
            output: String::new(),
            goal_signal: None,
            terminal_reason: None,
            pending_user_input: false,
            handoff: (),
        })
    }
}
impl<R: AgentRuntimeCommandRepositories> GoalRuntimePort for Port<'_, R> {
    type Handoff = ();
    fn session_id(&self) -> &str {
        &self.owner
    }
    fn load_projection(&self) -> Result<Option<GoalProjection>, String> {
        Ok(Some(
            checkpoint(&**self.store.borrow(), &self.owner)?
                .state
                .goal_projection,
        ))
    }
    fn submit(
        &self,
        envelope: nola_agent_engine::agent::GoalRuntimeCommandEnvelope,
    ) -> Result<(), String> {
        let mut store = self.store.borrow_mut();
        let active = store.active_run(&self.owner).map_err(|e| e.to_string())?;
        // The shared active-submission transaction owns Interrupt selection;
        // queued controls and StepGoal cannot diverge here.
        let submission = AgentSubmission::GoalRuntime { envelope };
        if let Some(active) = active {
            crate::reduce_and_commit_active_submission(
                &mut **store,
                crate::ReduceAndCommitActiveSubmission {
                    conversation_id: self.owner.clone(),
                    run_id: active.run_id,
                    transition_id: format!("goal:{}", uuid::Uuid::new_v4()),
                    invocation_config_digest: active
                        .invocation_config_digest
                        .ok_or("Goal active Run has no invocation digest")?,
                    submission,
                    committed_at_ms: self.now,
                },
            )?;
        } else {
            crate::reduce_and_commit_idle_submission(
                &mut **store,
                crate::ReduceAndCommitIdleSubmission {
                    conversation_id: self.owner.clone(),
                    core_config: self.config.clone(),
                    submission,
                    committed_at_ms: self.now,
                },
            )?;
        }
        Ok(())
    }
    fn active_run_id(&self) -> Result<Option<String>, String> {
        Ok(self
            .store
            .borrow()
            .active_run(&self.owner)
            .map_err(|e| e.to_string())?
            .map(|run| run.run_id))
    }
    fn prepare_exact_run(&mut self, run: &str) -> Result<AgentInputOrigin, String> {
        self.binding()?;
        let checkpoint = checkpoint(&**self.store.borrow(), &self.owner)?;
        let turn = checkpoint
            .state
            .execution
            .active_turn
            .as_ref()
            .filter(|t| t.turn_id.as_str() == run)
            .ok_or("Goal recovery Run mismatch")?;
        Ok(turn.context.input_origin)
    }
    fn execute_round(
        &mut self,
        run: &str,
        context: &str,
        prompt: &str,
        visible: bool,
    ) -> Result<Round, String> {
        let bound = self.binding()?;
        let snapshot = checkpoint(&**self.store.borrow(), &self.owner)?;
        let goal = snapshot
            .state
            .goal_projection
            .active
            .as_ref()
            .ok_or("Goal is unavailable")?;
        let core = bound.template.core_config;
        let platform = snapshot
            .state
            .execution
            .active_turn
            .as_ref()
            .map(|t| t.context.platform.clone())
            .unwrap_or_default();
        let context_items = vec![AgentItem::user_message(
            ItemId::from_string(format!("{run}:internal-context")),
            context,
        )];
        let items = if visible {
            vec![AgentItem::user_message(
                ItemId::from_string(format!("{run}:user")),
                prompt,
            )]
        } else {
            vec![InternalTurnTrigger {
                trigger_id: format!("{run}:internal-trigger"),
                core_session_id: snapshot.session_id,
                conversation_id: Some(self.owner.clone()),
                origin: AgentInputOrigin::GoalContinuation,
                context_digest: internal_turn_context_digest(&context_items, &platform, &[]),
                goal: Some(GoalTriggerBinding {
                    goal_id: goal.id.clone(),
                    goal_revision: snapshot.state.goal_projection.revision,
                }),
            }
            .item()?]
        };
        let submission = if visible {
            json!({"type":"goal_user_input","items":items,"turn_context_items":context_items,
            "turn_context_platform":platform,"turn_context_components":[],"submission_id":run,"submitted_at_epoch_ms":self.now,"delivery":"current_turn"})
        } else {
            json!({"type":"automated_input","origin":"goal_continuation","items":items,"turn_context_items":context_items,
            "turn_context_platform":platform,"turn_context_components":[],"submission_id":run,"submitted_at_epoch_ms":self.now})
        };
        let conversation = self
            .store
            .borrow()
            .load_conversation_message_window(&self.owner, None, 1, 1024)
            .map_err(|e| e.to_string())?
            .map(|window| window.conversation);
        let input=serde_json::from_value::<crate::BeginAssistantTurn>(json!({
            "conversation_id":self.owner,"conversation_title":conversation.as_ref().map(|c|c.title.clone()).unwrap_or_else(||prompt.chars().take(80).collect()),"expected_configuration_revision":conversation.as_ref().map(|c|c.configuration_revision),
            "expected_parent_message_id":conversation.as_ref().and_then(|c|c.messages.last().map(|m|m.id.clone())),"user_message_id":format!("{run}:user"),
            "user_part_id":format!("{run}:user:text"),"user_text":if visible{prompt}else{""},"user_content_json":null,
            "assistant_message_id":format!("{run}:assistant"),"context_checkpoint_id":null,"core_config":core,"submission":submission,"created_at_ms":self.now}))
            .map_err(|_|"invalid embedded Goal admission")?;
        crate::begin_assistant_turn(&mut **self.store.borrow_mut(), input)?;
        self.round(run)
    }
    fn resume_round(&mut self, run: &str) -> Result<Round, String> {
        self.round(run)
    }
    fn sealed_round_result(&self, run: &str) -> Result<Option<Round>, String> {
        sealed_round(&mut **self.store.borrow_mut(), &self.owner, run)
    }
    fn round_handoff_pending(&self) -> bool {
        self.handoff.get()
    }
    fn now_epoch_ms(&self) -> i64 {
        self.now
    }
    fn next_nonce(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn new_lease_owner(&self) -> String {
        format!("embedded-goal:{}:{}", std::process::id(), self.next_nonce())
    }
    // A whole scheduling step commits atomically, including lease release.
    // This Host never guesses another Goal owner's death from an identifier.
    fn owner_is_dead(&self, _: &str) -> bool {
        false
    }
    fn is_scheduled(&self) -> bool {
        true
    }
    fn explicit_resume(&self) -> bool {
        false
    }
    fn runtime_limit(&self) -> Duration {
        Duration::from_secs(3600)
    }
    fn stop_requested(&self) -> bool {
        false
    }
    fn acknowledge_slice(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn discard_slice_snapshot(&mut self) {}
}

pub(super) fn advance<R: AgentRuntimeCommandRepositories>(
    store: &mut R,
    owner: String,
    host: String,
    control: Option<nola_agent_engine::agent::GoalRuntimeCommandEnvelope>,
    start_configuration: Option<Box<AgentCoreConfig>>,
    selected_model: Option<super::GoalSelectedModel>,
    user_prompt: String,
    host_available: bool,
) -> Result<Value, String> {
    // Native execution starts only after this command returns its committed
    // receipt. No scheduler lease or half-planned round escapes a crash here.
    crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
        advance_in_transaction(
            store,
            owner,
            host,
            control,
            start_configuration,
            selected_model,
            user_prompt,
            host_available,
        )
    })
}

fn advance_in_transaction<R: AgentRuntimeCommandRepositories>(
    store: &mut R,
    owner: String,
    host: String,
    control: Option<nola_agent_engine::agent::GoalRuntimeCommandEnvelope>,
    start_configuration: Option<Box<AgentCoreConfig>>,
    selected_model: Option<super::GoalSelectedModel>,
    user_prompt: String,
    host_available: bool,
) -> Result<Value, String> {
    let now = crate::goal_coordinator::monotonic_epoch_millis();
    if store
        .is_runtime_task_child_session(&owner)
        .map_err(|error| error.to_string())?
    {
        return Err("Goal cannot claim a delegated child conversation".into());
    }
    if start_configuration.is_none() && crate::load_current_checkpoint(store, &owner)?.is_none() {
        if control.is_some() {
            return Err("Goal requires an existing configured conversation".into());
        }
        return Ok(
            json!({"instruction":{"type":"terminal"},"goal":null,"interrupted_run_id":null,"error":null}),
        );
    }
    let mut config = match start_configuration {
        Some(config) => *config,
        None => configuration(&checkpoint(store, &owner)?.state),
    };
    if config.host_catalog.as_ref().map(|c| c.host_id.0.as_str()) != Some(host.as_str()) {
        return Err("Goal execution belongs to another Host".into());
    }
    let mut interrupted_run = None;
    if let Some(envelope) = control {
        use nola_agent_engine::agent::GoalRuntimeCommand;
        let stopping = match &envelope.command {
            GoalRuntimeCommand::GuardedControl { control, .. } => matches!(
                control.as_ref(),
                GoalRuntimeCommand::Pause { .. } | GoalRuntimeCommand::Cancel { .. }
            ),
            GoalRuntimeCommand::Pause { .. } | GoalRuntimeCommand::Cancel { .. } => true,
            _ => false,
        };
        if stopping {
            let snapshot = checkpoint(store, &owner)?;
            if let Some(active) = store
                .active_run(&owner)
                .map_err(|error| error.to_string())?
            {
                if snapshot
                    .state
                    .goal_projection
                    .execution_owner(&active.run_id)
                    .is_some()
                {
                    interrupted_run = Some(active.run_id);
                }
            }
        }
        let operation_id = envelope.operation_id.clone();
        if let nola_agent_engine::agent::GoalRuntimeCommand::Start { id, .. } = &envelope.command {
            if let Some(selected) = selected_model.as_ref() {
                let frozen = config
                    .turn_identity
                    .host_invocation
                    .as_ref()
                    .ok_or("Goal requires a frozen model")?;
                if selected.primary != frozen.primary
                    || selected.generation != frozen.generation
                    || selected.output_admission != frozen.output_admission
                {
                    return Err("Model settings changed since the last Run. Send a message with the selected settings before starting a Goal.".into());
                }
            }
            if config.turn_identity.host_recovery.is_none() {
                config.turn_identity.host_recovery = Some(descriptor(&config, &owner, id)?);
            }
            let profile = selected_model
                .as_ref()
                .and_then(|model| model.profile.clone())
                .or(config
                    .turn_identity
                    .delegation
                    .as_ref()
                    .map(|value| value.snapshot())
                    .transpose()?
                    .and_then(|value| value.runtime.selected_profile));
            let selection = profile
                .map(
                    |profile| nola_domain::goal_execution::GoalModelSelection::Profile {
                        profile_id: profile.profile_id,
                        revision: profile.admitted_revision,
                    },
                )
                .unwrap_or(nola_domain::goal_execution::GoalModelSelection::DirectRoute);
            crate::goal_execution::start_goal_execution(
                store,
                crate::goal_execution::StartGoalExecution {
                    identity: nola_domain::goal_execution::GoalExecutionBindingIdentity {
                        conversation_id: owner.clone(),
                        goal_id: id.clone(),
                        operation_id: envelope.operation_id.clone(),
                        source_host_id: host.clone(),
                    },
                    template: crate::goal_execution::GoalExecutionTemplate {
                        model_selection: selection,
                        core_config: config.clone(),
                    },
                    envelope,
                },
            )?;
        } else {
            let port = Port {
                store: RefCell::new(&mut *store),
                owner: owner.clone(),
                host: host.clone(),
                config: config.clone(),
                now,
                handoff: Cell::new(false),
                run: RefCell::new(None),
            };
            port.submit(envelope)?;
        }
        let projection = checkpoint(store, &owner)?.state.goal_projection;
        if let Some(error) = projection
            .applied_operations
            .get(&operation_id)
            .and_then(|receipt| receipt.rejection.as_ref())
        {
            return directive(
                store,
                &owner,
                None,
                None,
                Some(error.to_string()),
                host_available,
            );
        }
    }
    if !host_available {
        let goal = checkpoint(store, &owner)?.state.goal_projection.active;
        if goal
            .as_ref()
            .is_some_and(|goal| goal.status == GoalStatus::Running)
        {
            let port = Port {
                store: RefCell::new(&mut *store),
                owner: owner.clone(),
                host: host.clone(),
                config: config.clone(),
                now,
                handoff: Cell::new(false),
                run: RefCell::new(None),
            };
            interrupted_run = port.active_run_id()?.filter(|run| {
                goal.as_ref().is_some_and(|goal| {
                    goal.contracts
                        .iter()
                        .any(|c| c.execution_run_id.as_ref() == Some(run))
                })
            });
            port.submit(nola_agent_engine::agent::GoalRuntimeCommandEnvelope {
                operation_id: format!("goal-host-unavailable:{}", uuid::Uuid::new_v4()),
                command: nola_agent_engine::agent::GoalRuntimeCommand::Pause {
                    status: GoalStatus::Paused,
                    wake_at_epoch_ms: None,
                    now_epoch_ms: now,
                },
            })?;
        }
        return directive(store, &owner, None, interrupted_run, None, false);
    }
    let before = checkpoint(store, &owner)?.state.goal_projection.active;
    if let Some(goal) = before.as_ref().filter(|goal| {
        goal.status == GoalStatus::Paused
            && goal.next_wake_at_epoch_ms.is_some_and(|wake| wake <= now)
    }) {
        let binding = crate::goal_execution::prepare_goal_execution(store, &owner, &goal.id)?;
        let port = Port {
            store: RefCell::new(&mut *store),
            owner: owner.clone(),
            host: host.clone(),
            config: binding.template.core_config,
            now,
            handoff: Cell::new(false),
            run: RefCell::new(None),
        };
        port.submit(nola_agent_engine::agent::GoalRuntimeCommandEnvelope {
            operation_id: format!("embedded-due:{}:{now}", goal.id),
            command: nola_agent_engine::agent::GoalRuntimeCommand::ResumeDue { now_epoch_ms: now },
        })?;
    }
    let goal = checkpoint(store, &owner)?.state.goal_projection.active;
    let Some(goal) = goal.as_ref().filter(|g| g.status == GoalStatus::Running) else {
        return directive(store, &owner, None, interrupted_run, None, host_available);
    };
    let binding = crate::goal_execution::prepare_goal_execution(store, &owner, &goal.id)?;
    let mut port = Port {
        store: RefCell::new(store),
        owner: owner.clone(),
        host,
        config: binding.template.core_config,
        now,
        handoff: Cell::new(false),
        run: RefCell::new(None),
    };
    // A scheduled wake cannot implicitly resume paused/needs-user/incomplete work.
    crate::goal_coordinator::run_goal_cycle(
        &mut port,
        &user_prompt,
        &goal.objective,
        &goal.constraints.join("\n"),
    )?;
    let run = port.run.borrow().clone();
    drop(port);
    directive(store, &owner, run, interrupted_run, None, host_available)
}

fn directive<R: AgentRuntimeCommandRepositories>(
    store: &R,
    owner: &str,
    run: Option<String>,
    interrupted_run: Option<String>,
    error: Option<String>,
    available: bool,
) -> Result<Value, String> {
    let checkpoint = checkpoint(store, owner)?;
    let goal = checkpoint.state.goal_projection.active.clone();
    let run = run.or_else(|| {
        checkpoint
            .state
            .execution
            .active_turn
            .as_ref()
            .filter(|turn| {
                matches!(
                    turn.status,
                    nola_agent_engine::agent::AgentTurnStatus::Cancelling
                        | nola_agent_engine::agent::AgentTurnStatus::AwaitingPermission
                        | nola_agent_engine::agent::AgentTurnStatus::AwaitingInput
                ) && checkpoint
                    .state
                    .goal_projection
                    .execution_owner(turn.turn_id.as_str())
                    .is_some()
            })
            .map(|turn| turn.turn_id.as_str().to_string())
    });
    let instruction = if !available {
        json!({"type":"wait","reason":"host_unavailable"})
    } else if let Some(run) = run {
        let status = checkpoint
            .state
            .execution
            .active_turn
            .as_ref()
            .map(|turn| turn.status);
        if matches!(
            status,
            Some(
                nola_agent_engine::agent::AgentTurnStatus::AwaitingPermission
                    | nola_agent_engine::agent::AgentTurnStatus::AwaitingInput
            )
        ) {
            json!({"type":"needs_user","run_id":run})
        } else {
            json!({"type":"execute_run","run_id":run})
        }
    } else {
        match goal.as_ref() {
            None => json!({"type":"terminal"}),
            Some(goal) if goal.status.is_terminal() => {
                json!({"type":"terminal","outcome":if goal.status==GoalStatus::Completed{"completed"}else{"cancelled"}})
            }
            Some(goal) if goal.next_wake_at_epoch_ms.is_some() => {
                json!({"type":"wait","wake_at_epoch_ms":goal.next_wake_at_epoch_ms})
            }
            Some(_) => json!({"type":"needs_user"}),
        }
    };
    Ok(
        json!({"instruction":instruction,"interrupted_run_id":interrupted_run,"goal":goal,"error":error}),
    )
}
