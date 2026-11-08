// SPDX-License-Identifier: Apache-2.0
//! Agent execution, recovery, permission, memory, and durable work dispatch.

use serde_json::{json, Value};
use nola_store_port::automation::AutomationRepository;

use super::{encode_application_result, ApplicationCommand};
use crate::{
    repositories::AgentRuntimeCommandRepositories, ActiveRun, AgentCommandLease,
    ConversationCheckpoint, TraceRecord, TraceRun,
};

pub(super) fn execute<R: AgentRuntimeCommandRepositories>(
    store: &mut R,
    command: ApplicationCommand,
) -> Result<Value, String> {
    crate::task_terminal::validate_command(store, &command)?;
    match command {
        ApplicationCommand::StepGoal { conversation_id, source_host_id, control, start_configuration, selected_model, user_prompt, host_available } =>
            super::goal_step::advance(store, conversation_id, source_host_id, control, start_configuration, selected_model, user_prompt, host_available.unwrap_or(true)),
        ApplicationCommand::PrepareDelegatedTask { input } => encode_application_result(crate::delegation_preparation::prepare(store,input)?),
        ApplicationCommand::CaptureMobileDelegationConfiguration { config, settings } => encode_application_result(crate::mobile_delegation_configuration::capture_parent(config,settings)?),
        ApplicationCommand::LoadMobileTaskChildConfiguration { task_id } => encode_application_result(crate::mobile_delegation_configuration::child_config(store,&task_id)?),
        ApplicationCommand::PrepareMobileTaskGuidance { input } => encode_application_result(crate::mobile_delegation::guide(store,input)?),
        ApplicationCommand::ObserveMobileTask { input } => crate::mobile_delegation::observe(store, input),
        ApplicationCommand::ApplyPendingTaskNotice { .. } => Err("Task terminal delivery requires the ApplicationRuntime boundary".into()),
        ApplicationCommand::PrepareMobileDelegatedTask { input } => encode_application_result(crate::mobile_delegation::prepare(store,input)?),
        ApplicationCommand::InspectMobilePermissionRequest { request, now_ms } => crate::mobile_delegation::inspect_permission(store, request, now_ms),
        ApplicationCommand::InspectMobileMcpRequest { request, now_ms } => crate::mobile_delegation::inspect_mcp(store, request, now_ms),
        ApplicationCommand::LoadMobileTaskWakePlan { now_ms } => encode_application_result(crate::mobile_delegation::wake_plan(store, now_ms)?),
        ApplicationCommand::AdmitMobileTaskChild { input } => encode_application_result(crate::mobile_delegation::admit(store,input)?),
        ApplicationCommand::PrepareGoalControlIntent { input } => {
            // This command is receipt-backed. Retries return the original
            // prepared envelope without recomputing its Host timestamp.
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "Host clock precedes epoch")?.as_millis();
            let now = i64::try_from(now).map_err(|_| "Host clock exceeds Goal time range")?;
            encode_application_result(crate::goal_control_intent::prepare(input, now)?)
        },
        ApplicationCommand::StartGoalExecution { input } =>
            encode_application_result(crate::goal_execution::start_goal_execution(store, input)?),
        ApplicationCommand::PrepareGoalExecution { conversation_id, goal_id } =>
            encode_application_result(crate::goal_execution::prepare_goal_execution(store, &conversation_id, &goal_id)?),
        ApplicationCommand::LoadGoalExecutionBinding { conversation_id, goal_id } =>
            encode_application_result(crate::goal_execution::load_goal_execution_binding(store, &conversation_id, &goal_id)?),
        ApplicationCommand::LoadHostExecutionState { conversation_id } =>
            encode_application_result(crate::host_execution_state::load(store, &conversation_id)?),
        ApplicationCommand::LoadConversationTaskProjectionV1 { conversation_id } =>
            crate::host_execution_state::load_task_projection(store, &conversation_id),
        ApplicationCommand::ExportAgentTaskAdmission { conversation_id, run_id, task_id, event_cursor } => {
            encode_application_result(crate::agent_task_admission::export(
                store, &conversation_id, &run_id, &task_id, event_cursor,
            )?)
        }
        ApplicationCommand::ImportHostAgentTaskAdmission { admission } => {
            encode_application_result(crate::agent_task_admission::import(store, admission)?)
        }
        ApplicationCommand::ExportAgentTaskTerminalProjection { task_id, expected_receipt_id, guidance_message_ids } => {
            encode_application_result(crate::task_terminal::export_projection(store, &task_id, &expected_receipt_id, &guidance_message_ids)?)
        }
        ApplicationCommand::ExportAgentTaskGuidanceProjection { task_id, message_id, settlement } => {
            encode_application_result(crate::task_guidance::export_guidance(store, &task_id, &message_id, settlement)?)
        }
        ApplicationCommand::ImportHostAgentTaskGuidanceProjection { projection } => {
            use crate::task_guidance::PreparedTaskGuidanceImport;
            let revision = match crate::task_guidance::prepare_guidance_import(store, &projection)? {
                PreparedTaskGuidanceImport::Active(input) => crate::reduce_and_commit_active_submission(store, input)?.control.revision,
                PreparedTaskGuidanceImport::Idle(input) => crate::reduce_and_commit_idle_submission(store, input)?.execution.control.revision,
            };
            encode_application_result(nola_protocol::TaskGuidanceProjectionApplied {
                conversation_id: projection.admission.parent_conversation_id,
                task_id: projection.admission.task_id,
                message_id: projection.guidance.message_id,
                settlement: projection.settlement,
                core_revision: revision,
            })
        }
        ApplicationCommand::ImportHostAgentTaskTerminalProjection { projection } => {
            use crate::task_terminal::PreparedTaskTerminalImport;
            let revision = match crate::task_terminal::prepare_import(store, &projection)? {
                PreparedTaskTerminalImport::Active(input) => crate::reduce_and_commit_active_submission(store, input)?.control.revision,
                PreparedTaskTerminalImport::Idle(input) => crate::reduce_and_commit_idle_submission(store, input)?.execution.control.revision,
                PreparedTaskTerminalImport::AlreadyApplied(ack) => return encode_application_result(ack),
            };
            encode_application_result(crate::TaskTerminalReconciliationApplied {
                conversation_id: projection.admission.parent_conversation_id,
                task_id: projection.admission.task_id,
                receipt_id: projection.receipt_id,
                core_revision: revision,
            })
        }
        ApplicationCommand::RecoverExpiredAgentTask { input } => {
            encode_application_result(crate::task_ownership::recover(store,input)?)
        }
        ApplicationCommand::ClaimAgentTaskFair { input } => {
            encode_application_result(crate::task_ownership::claim(store,input)?)
        }
        ApplicationCommand::BeginAssistantTurn { input } => {
            let (accepted, _) = crate::assistant_turn::admit_with_core(store, input, None)?;
            encode_application_result(accepted)
        }
        ApplicationCommand::PromoteQueuedAssistantTurn { input } => {
            encode_application_result(crate::promote_queued_assistant_turn(store, input)?)
        }
        ApplicationCommand::BeginQueuedUserTurn { input } => {
            encode_application_result(crate::begin_queued_user_turn(store, input)?)
        }
        ApplicationCommand::ListOpenUserInputWaits { conversation_id, run_id } => {
            encode_application_result(store.open_user_input_waits(&conversation_id, run_id.as_deref())
                .map_err(|error| error.to_string())?)
        }
        ApplicationCommand::ListOwnedTaskUserInputWaits { owner_conversation_id, task_id } => {
            encode_application_result(store.owned_task_user_input_waits(&owner_conversation_id, task_id.as_deref())
                .map_err(|error| error.to_string())?)
        }
        ApplicationCommand::LoadUserInputWait { conversation_id, wait_id } => {
            encode_application_result(store.load_user_input_wait(&conversation_id, &wait_id)
                .map_err(|error| error.to_string())?)
        }
        ApplicationCommand::RespondToConversationWait { input } => {
            encode_application_result(crate::user_input_wait::respond(store, input)?)
        }
        ApplicationCommand::BeginTaskAssistantTurn { lease, input } => {
            let (accepted, _) = crate::assistant_turn::admit_with_core(store, input, Some(lease))?;
            encode_application_result(accepted)
        }
        ApplicationCommand::BeginAutomationAssistantTurn { lease, input } => {
            let (accepted, _) = crate::automation_execution::admit_with_core(store, lease, input)?;
            encode_application_result(accepted)
        }
        ApplicationCommand::ObserveAutomationExecution { occurrence_id } =>
            encode_application_result(store.automation_repository().observe_automation_execution(&occurrence_id).map_err(|e|e.to_string())?),
        ApplicationCommand::ListPendingAutomationExecutions { after_occurrence_id, limit } =>
            encode_application_result(store.automation_repository().pending_automation_executions(after_occurrence_id.as_deref(), limit).map_err(|e|e.to_string())?),
        ApplicationCommand::ReconcileAutomationExecution { occurrence_id, reconciled_at_ms } =>
            crate::assistant_publication_transaction::with_publication_transaction(store, |store|
                encode_application_result(store.automation_repository().reconcile_automation_execution(&occurrence_id, reconciled_at_ms).map_err(|e|e.to_string())?)),
        ApplicationCommand::ObserveAutomationWorker { lease, observation, observed_at_ms } =>
            encode_application_result(store.automation_repository().observe_automation_worker(&lease, &observation, observed_at_ms).map_err(|e|e.to_string())?),
        ApplicationCommand::AckAutomationNotification { occurrence_id, notification_id, delivered_at_ms } =>
            encode_application_result(store.automation_repository().acknowledge_automation_notification(&occurrence_id, &notification_id, delivered_at_ms).map_err(|e|e.to_string())?),
        ApplicationCommand::BeginMaintenanceTurn { input } => {
            encode_application_result(crate::begin_maintenance_turn(store, input)?)
        }
        ApplicationCommand::QueryClaimableMemoryJobs { input } => {
            encode_application_result(crate::query_claimable_memory_jobs(store, input)?)
        }
        ApplicationCommand::BuildMemoryWorkerPlan { input } => {
            encode_application_result(crate::build_memory_worker_plan(store, input)?)
        }
        ApplicationCommand::LoadCommittedHostRequests { run_id } => {
            encode_application_result(crate::load_committed_host_requests(store, &run_id)?)
        }
        ApplicationCommand::InspectRunProcessOwners { conversation_id, expected_run_id } => encode_application_result(store.with_consistent_read(|reader| crate::run_process_observation::inspect(reader, conversation_id, expected_run_id))?),
        ApplicationCommand::ValidateRunProcessObservations { plan, observations } => encode_application_result(store.with_consistent_read(|reader| crate::run_process_observation::validate(reader, plan, observations))?),
        ApplicationCommand::LoadRunDeliveryOwnership { run_id } => encode_application_result(
            crate::effect_delivery::load_run_delivery_ownership(store, &run_id)?,
        ),
        ApplicationCommand::ClaimCommittedHostRequests { input } => {
            encode_application_result(crate::claim_committed_host_requests(store, input)?)
        }
        ApplicationCommand::ClaimApplicationControlRequests { input } =>
            encode_application_result(crate::task_cancellation_control::claim(store, input)?),
        ApplicationCommand::ImportHostTaskCancellation { authorization, now_ms } =>
            encode_application_result(crate::task_cancellation_control::import(store, authorization, now_ms)?),
        ApplicationCommand::ExecuteTaskCancellationForEffect { request, now_ms } =>
            encode_application_result(crate::task_cancellation_control::execute(store, &request, now_ms)?),
        ApplicationCommand::PrepareRunEffectRecovery { input } => {
            encode_application_result(crate::effect_delivery::prepare_run_effect_recovery(store, input)?)
        }
        ApplicationCommand::HeartbeatEffectDelivery { input } => {
            encode_application_result(crate::heartbeat_effect_delivery(store, input)?)
        }
        ApplicationCommand::InspectOwnedTaskRecovery { .. } | ApplicationCommand::ResolveOwnedTaskRecovery { .. } =>
            Err("Owned task recovery requires the ApplicationRuntime boundary".into()),
        ApplicationCommand::ResolveUnknownEffectRecovery { input } => {
            encode_application_result(crate::unknown_effect_recovery::resolve(store, input)?)
        }
        ApplicationCommand::LoadRuntimeEffectWorkSchedule { input } => {
            encode_application_result(crate::load_runtime_effect_work_schedule(store, input)?)
        }
        ApplicationCommand::ActiveRun { conversation_id } => {
            let active = store
                .active_run(&conversation_id)
                .map_err(|error| error.to_string())?
                .map(ActiveRun::from);
            encode_application_result(active)
        }
        ApplicationCommand::ReduceAndAcceptHostResponse { input } => {
            encode_application_result(crate::reduce_and_accept_host_response(store, input)?)
        }
        ApplicationCommand::ReduceAndCommitActiveSubmission { input } => {
            if matches!(input.submission, crate::AgentSubmission::RespondToUserInputWait { .. }) {
                return Err("wait responses require the dedicated conversation wait command".into());
            }
            if matches!(
                input.submission,
                nola_agent_engine::agent::AgentSubmission::ApplyTaskGuidanceInput { .. }
            ) {
                return Err(
                    "guidance input requires the dedicated ApplicationRuntime transaction".into(),
                );
            }
            encode_application_result(crate::reduce_and_commit_active_submission(store, input)?)
        }
        ApplicationCommand::RequestRunCancellation { input } => {
            encode_application_result(crate::request_run_cancellation(store, input)?)
        }
        ApplicationCommand::InspectCancellationProcessOwners { identity, control } =>
            encode_application_result(crate::cancellation_process_observation::inspect_cancellation_process_owners(store, identity, control)?),
        ApplicationCommand::ValidateCancellationProcessObservations { plan, current_process, observations } =>
            encode_application_result(crate::cancellation_process_observation::validate_cancellation_process_observations(store, plan, current_process, observations)?),
        ApplicationCommand::LoadRunCancellationDeliveryFacts { input } => {
            encode_application_result(
                crate::cancellation_delivery::load_run_cancellation_delivery_facts(store, &input)?,
            )
        }
        ApplicationCommand::ClaimCancellationHostRequests { input } => encode_application_result(
            crate::cancellation_delivery::claim_cancellation_host_requests(store, input)?,
        ),
        ApplicationCommand::ReduceAndCommitIdleSubmission { input } => {
            encode_application_result(crate::reduce_and_commit_idle_submission(store, input)?)
        }
        ApplicationCommand::UpsertPermissionGrant { input } => {
            encode_application_result(crate::record_permission_grant(store, input)?)
        }
        ApplicationCommand::EffectivePermissionGrants {
            conversation_id,
            host_id,
        } => encode_application_result(crate::load_permission_rule_set(
            store,
            conversation_id.as_deref(),
            host_id.as_deref(),
        )?),
        ApplicationCommand::ListPermissionGrants {
            conversation_id,
            host_id,
        } => encode_application_result(crate::list_effective_permission_grants(
            store,
            conversation_id.as_deref(),
            host_id.as_deref(),
        )?),
        ApplicationCommand::RevokePermissionGrant { id, revoked_at_ms } => Ok(json!(
            crate::revoke_permission_grant(store, &id, revoked_at_ms)?
        )),
        ApplicationCommand::Memory { command } => {
            crate::memory::execute_memory_command(store, command)
        }
        ApplicationCommand::ConversationPlan { command } => {
            crate::conversation_plan::execute_conversation_plan_command(store, command)
        }
        ApplicationCommand::RuntimeTask { command } => {
            crate::task_terminal::validate_terminal_mutation(store, &command)?;
            let terminal_task = match &command {
                crate::RuntimeTaskCommand::ReconcileBoundRunTerminal { task_id, .. } => {
                    Some(task_id.clone())
                }
                crate::RuntimeTaskCommand::FinishDetailedWithEvent { lease, .. } => {
                    Some(lease.task_id.clone())
                }
                _ => None,
            };
            if let crate::RuntimeTaskCommand::RecoverPermissionContinuationFair { input } = &command
            {
                let active = store
                    .active_run(&input.expected_child_session_id)
                    .map_err(|error| format!("read permission continuation Run failed: {error}"))?;
                if active.as_ref().map(|run| run.run_id.as_str())
                    != Some(input.expected_run_id.as_str())
                {
                    return Err("permission recovery no longer owns the exact child Run".into());
                }
            }
            let result = crate::runtime_task::execute_runtime_task_command(store, command)?;
            if let Some(task_id) = terminal_task {
                let task = store.runtime_task(&task_id).map_err(|error| error.to_string())?;
                if task.is_some_and(|task| task.kind == "agent") {
                    if let Some(receipt) = store
                        .runtime_task_terminal_receipt(&task_id)
                        .map_err(|error| error.to_string())?
                    {
                        // Agent receipt and parent delivery commit together.
                        // Shell receipts never authorize an Agent-parent update.
                        crate::task_terminal::enqueue(store, &task_id, &receipt.receipt_id)?;
                    }
                }
            }
            Ok(result)
        }
        ApplicationCommand::SopRun { command } => {
            crate::sop_runtime::execute_sop_run_command(store, command)
        }
        ApplicationCommand::PermissionRequest { command } => {
            crate::permission::execute_permission_request_command(store, command)
        }
        ApplicationCommand::EnqueueAgentCommand { input } => {
            crate::task_terminal::validate_generic_enqueue(store, &input)?;
            let submission = serde_json::from_str::<nola_agent_engine::agent::AgentSubmission>(&input.payload_json)
                .map_err(|error| format!("decode admitted Agent submission: {error}"))?;
            let mut store_input: nola_store_port::agent_queue::EnqueueAgentCommandInput = input.into();
            store_input.payload_json = serde_json::to_string(
                &nola_agent_engine::agent::host::AgentBoundaryInput::Submission { submission }
            ).map_err(|error| error.to_string())?;
            Ok(json!(
                nola_store_port::agent_queue::AgentQueueRepository::enqueue_agent_command(
                    store,
                    &store_input
                )
                .map_err(|error| error.to_string())?
            ))
        }
        ApplicationCommand::EnqueueTaskTerminalReconciliation {
            task_id,
            expected_receipt_id,
        } => encode_application_result(crate::task_terminal::enqueue(
            store,
            &task_id,
            &expected_receipt_id,
        )?),
        ApplicationCommand::LoadTaskGuidanceReceipt { task_id, message_id, parent_conversation_id, parent_run_id } =>
            encode_application_result(crate::task_guidance::receipt(store, &task_id, &message_id,
                &parent_conversation_id, &parent_run_id)?),
        ApplicationCommand::SubmitTaskGuidance {
            task_id,
            message_id,
            parent_turn_id,
            content,
            submitted_at_ms,
        } => encode_application_result(crate::task_guidance::submit(
            store,
            task_id,
            message_id,
            parent_turn_id,
            content,
            submitted_at_ms,
        )?),
        ApplicationCommand::ApplyTaskGuidance { .. } => {
            Err("ApplyTaskGuidance requires the ApplicationRuntime hot Run boundary".into())
        }
        ApplicationCommand::ApplyActivePermissionPolicy { .. } => {
            Err("active policy application requires the ApplicationRuntime boundary".into())
        }
        ApplicationCommand::PeekAgentCommand { session_id } => {
            let command = nola_store_port::agent_queue::AgentQueueRepository::peek_agent_command(store, &session_id)
                .map_err(|error| error.to_string())?
                .map(crate::AgentCommandHead::try_from).transpose()?;
            encode_application_result(command)
        }
        ApplicationCommand::ClaimNextAgentCommand {
            session_id,
            lease_owner,
            now_ms,
            lease_until_ms,
        } => {
            let command =
                nola_store_port::agent_queue::AgentQueueRepository::claim_next_agent_command(
                    store,
                    &session_id,
                    &lease_owner,
                    now_ms,
                    lease_until_ms,
                )
                .map_err(|error| error.to_string())?
                .map(AgentCommandLease::try_from)
                .transpose()?;
            encode_application_result(command)
        }
        ApplicationCommand::LeasedAgentCommands { session_id } => {
            let commands =
                nola_store_port::agent_queue::AgentQueueRepository::leased_agent_commands(
                    store,
                    &session_id,
                )
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(AgentCommandLease::try_from)
                .collect::<Result<Vec<_>, _>>()?;
            encode_application_result(commands)
        }
        ApplicationCommand::ReleaseAgentCommandLease {
            session_id,
            command_id,
            lease_owner,
        } => Ok(json!(
            nola_store_port::agent_queue::AgentQueueRepository::release_agent_command_lease(
                store,
                &session_id,
                &command_id,
                &lease_owner
            )
            .map_err(|error| error.to_string())?
        )),
        ApplicationCommand::AcknowledgeAgentCommand {
            session_id,
            command_id,
            lease_owner,
        } => Ok(json!(
            nola_store_port::agent_queue::AgentQueueRepository::acknowledge_agent_command(
                store,
                &session_id,
                &command_id,
                &lease_owner
            )
            .map_err(|error| error.to_string())?
        )),
        ApplicationCommand::RecordAgentTrace { event, envelope } => {
            Ok(json!(crate::execution_trace_journal::record(store, &event, envelope.as_ref())?))
        }
        ApplicationCommand::LoadExactRunTraceReplay { conversation_id,run_id,assistant_message_id,after_sequence,through_sequence,limit } => {
            let page = nola_store_port::trace_journal::TraceJournalRepository::exact_run_trace_replay(
                store,&conversation_id,&run_id,&assistant_message_id,after_sequence,through_sequence,limit,
            ).map_err(|error|error.to_string())?;
            let mut events = Vec::with_capacity(page.records.len());
            for record in page.records {
                let sequence = record.sequence;
                let event_id = record.event_id.clone();
                let event = TraceRecord::try_from(record)?.event;
                if event.conversation_id != conversation_id || event.run_id != run_id || event.turn_id != run_id
                    || event.message_id.as_deref() != Some(assistant_message_id.as_str())
                    || Some(&event.trace_id) != page.trace_id.as_ref() || event.sequence != sequence as u64
                    || event.event_id != event_id {
                    return Err("exact trace replay payload identity mismatch".into());
                }
                events.push(event);
            }
            Ok(json!({"trace_id":page.trace_id,"through_sequence":page.through_sequence,
                "next_sequence":page.next_sequence,"events":events}))
        }
        ApplicationCommand::TraceEventsAfter { trace_id, sequence } => {
            let records =
                nola_store_port::trace_journal::TraceJournalRepository::trace_journal_events_after(
                    store, &trace_id, sequence,
                )
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(TraceRecord::try_from)
                .collect::<Result<Vec<_>, _>>()?;
            encode_application_result(records)
        }
        ApplicationCommand::TraceRunsForConversation { conversation_id } => {
            let runs = nola_store_port::trace_journal::TraceJournalRepository::trace_journal_runs_for_conversation(store, &conversation_id)
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(TraceRun::from)
                .collect::<Vec<_>>();
            encode_application_result(runs)
        }
        ApplicationCommand::LatestTraceId { conversation_id } => encode_application_result(
            nola_store_port::trace_journal::TraceJournalRepository::latest_trace_journal_id(
                store,
                &conversation_id,
            )
            .map_err(|error| error.to_string())?,
        ),
        ApplicationCommand::RecoverObservedOrphanedRun {
            conversation_id,
            expected_run_id,
            host_invocation,
            observed_session_lease,
            observed_deliveries,
            interrupted_at,
            interrupted_at_ms,
        } => Ok(json!(
            crate::run_recovery::recover_observed(
                store,
                &conversation_id,
                &expected_run_id,
                &host_invocation,
                observed_session_lease.as_ref(),
                &observed_deliveries,
                &interrupted_at,
                interrupted_at_ms
            )?
            .changed
        )),
        ApplicationCommand::RecoverExpiredRunDeliveries {
            conversation_id, expected_run_id, host_invocation, interrupted_at, interrupted_at_ms,
        } => Ok(json!(crate::run_recovery::recover_expired(store, &conversation_id, &expected_run_id,
            &host_invocation, &interrupted_at, interrupted_at_ms)?.changed)),
        ApplicationCommand::RecoverOrphanedRuns {
            conversation_id,
            reason,
            interrupted_at,
            interrupted_at_ms,
        } => Ok(json!(
            crate::run_recovery::recover(
                store,
                conversation_id.as_deref(),
                &reason,
                &interrupted_at,
                interrupted_at_ms
            )?
            .changed
        )),
        ApplicationCommand::WorkStepTransaction { command } => {
            crate::work_step_transaction::execute_work_step_transaction_command(store, command)
        }
        ApplicationCommand::LoadCheckpoint { conversation_id } => {
            let checkpoint = store.with_consistent_read(|reader|
                crate::load_current_checkpoint(reader, &conversation_id))?
                .map(ConversationCheckpoint::from);
            encode_application_result(checkpoint)
        }
        ApplicationCommand::LoadSessionProjection { conversation_id } => {
            encode_application_result(store.with_consistent_read(|reader|
                crate::load_session_projection(reader, &conversation_id))?)
        }
        ApplicationCommand::LoadGoalControlObservation { conversation_id } => {
            encode_application_result(store.with_consistent_read(|reader|
                crate::load_session_projection(reader, &conversation_id))?
                .and_then(|projection| projection.goal_projection.control_observation()))
        }
        ApplicationCommand::LoadGoalExecutionOwner { conversation_id, goal_id, run_id } => {
            #[derive(serde::Serialize)]
            struct Witness {
                #[serde(flatten)] owner: nola_agent_engine::agent::goal::GoalExecutionOwner,
                assistant_message_id: String,
            }
            let witness = store.with_consistent_read(|reader| {
                let owner = crate::load_session_projection(reader, &conversation_id)?
                    .and_then(|projection| projection.goal_projection.execution_owner(&run_id))
                    .filter(|owner| owner.goal_id == goal_id);
                owner.map(|owner| {
                    let head = reader.local_run_message_admission_head(&conversation_id, &run_id)
                        .map_err(|error| error.to_string())?;
                    Ok::<_, String>(Witness { owner, assistant_message_id: head.identity.assistant_message_id })
                }).transpose()
            })?;
            encode_application_result(witness)
        }
        ApplicationCommand::LoadSessionGoalProjection { conversation_id } => {
            encode_application_result(store.with_consistent_read(|reader|
                crate::load_session_projection(reader, &conversation_id))?
                .map(|projection| projection.goal_projection))
        }
        ApplicationCommand::FindResumableGoalSession { project_root } => {
            encode_application_result(store.with_consistent_read(|store| {
                crate::resume_snapshot::find_resumable_projection(store, &project_root)
            })?)
        }
        ApplicationCommand::LoadResumeSnapshot {
            project_root,
            host_id,
            requested_at_ms,
        } => encode_application_result(crate::load_resume_snapshot(
            store,
            &project_root,
            host_id.as_deref(),
            requested_at_ms,
        )?),
        ApplicationCommand::AcknowledgeResumeSnapshot { input } => {
            crate::acknowledge_resume_snapshot(store, input)?;
            Ok(Value::Null)
        }
        _ => Err("command is outside the agent runtime family".into()),
    }
}
