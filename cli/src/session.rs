use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex, OnceLock,
};

use chrono::Local;
use nola_application::{
    ApplicationCommand, BeginAssistantTurn, BeginMaintenanceTurn, ConversationMessage,
    ConversationPart, CreateConversation, ReduceAndAcceptHostResponse,
    ReduceAndCommitActiveSubmission,
};
use nola_core::agent::recovery::CoreCheckpoint;
use nola_core::agent::task::TaskProjection;
use nola_core::agent::MemoryRecord;
use nola_core::agent::{AgentCoreConfig, AgentThreadState};
use nola_core::agent_trace::AgentTraceEvent;
use nola_protocol::{ConversationTranscriptImportSource, ImportConversationTranscript};
use nola_protocol::{
    TranscriptMessageInput as MessageInput, TranscriptPartInput as MessagePartInput,
};
use nola_runtime_driver::{
    monotonic_epoch_millis, CommittedRuntimeAcceptance as ProtocolAcceptance, CommittedRuntimeApplicationPort,
    RuntimeEffectDeliveryClaim, RuntimeEffectDeliveryRenewal,
};
type CommittedRuntimeAcceptance = ProtocolAcceptance<nola_core::agent::host::AgentRuntimeProtocol>;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::application_port::CliApplicationPort;
use crate::i18n::{tr, Message};

static SESSION_ID_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn canonical_timestamp(milliseconds: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(milliseconds)
        .map(|value| value.to_rfc3339())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    pub messages: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionTraceRun {
    pub trace_id: String,
    pub message_id: Option<String>,
    pub events: Vec<AgentTraceEvent>,
}

#[derive(Debug, Clone)]
pub struct SessionResumeInfo {
    pub id: String,
    pub updated_at: String,
    pub last_user_message_at: String,
    #[allow(dead_code)]
    pub message_count: usize,
    pub first_user_message: String,
}

/// A bounded, display-only projection used when an interactive CLI resumes a
/// conversation. The canonical transcript remains authoritative and is loaded
/// by the Agent Host when the user submits the next Turn; terminal restoration
/// must not deserialize or print an arbitrarily large transcript first.
#[derive(Debug, Clone)]
pub struct InteractiveSessionSnapshot {
    pub saved: SavedSession,
    pub total_message_count: usize,
    pub total_user_turns: usize,
}

const INTERACTIVE_REPLAY_MESSAGE_LIMIT: usize = 96;
const INTERACTIVE_REPLAY_CHARACTER_LIMIT: usize = 160_000;
const INTERACTIVE_REPLAY_PART_LIMIT: usize = 160_000;
const INTERACTIVE_REPLAY_JSON_LIMIT: usize = 64_000;

#[derive(Debug)]
pub struct SessionStore {
    pub id: String,
    application_port: CliApplicationPort,
    active_canonical_turn: Option<ActiveCanonicalTurn>,
    initial_configuration_revision: Option<i64>,
    task_execution: Option<crate::runtime_task_port::RuntimeTaskExecutionAuthority>,
    assistant_journal_notifications: AssistantJournalNotificationQueue,
    task_result_dispatches: std::collections::HashMap<String, TaskResultDispatch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskResultDispatch {
    run_id: String,
    request_id: String,
    session_id: String,
    turn_id: Option<String>,
    call_id: String,
    task_id: String,
}

/// Coalesced committed journal targets for the transport presenter. Content
/// remains in the durable journal; this queue never holds cumulative messages.
pub(crate) type AssistantJournalNotificationQueue =
    Arc<Mutex<AssistantJournalNotificationDelivery>>;

#[derive(Debug, Default)]
pub(crate) struct AssistantJournalNotificationDelivery {
    pub(crate) journal_publications: Vec<nola_domain::assistant_journal::AssistantJournalCursor>,
    pub(crate) observer: Option<Box<dyn crate::runtime_output::JournalPublicationObserver>>,
    pub(crate) observer_error: Option<String>,
    journal_seeded_run: Option<String>,
    pub(crate) permission_update: Option<PermissionControlUpdate>,
    permission_owner: Option<(String, String, Option<String>, u64)>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionControlProjection {
    pub active_policy: Option<nola_domain::ActivePermissionPolicy>,
    pub revision: u64,
    pub pending: Vec<nola_core::agent::permissions::PendingPermissionRequest>,
}

#[cfg(test)]
mod permission_control_projection_contract_tests {
    use super::PermissionControlProjection;
    use serde_json::json;

    #[test]
    fn current_projection_preserves_active_policy_and_stays_strict() {
        let value = json!({"revision":3,"pending":[],"active_policy":{
            "run_id":"current-run","configuration_revision":4,"profile":"cautious"}});
        let projection: PermissionControlProjection = serde_json::from_value(value.clone()).unwrap();
        let policy = projection.active_policy.unwrap();
        assert_eq!(policy.run_id,"current-run");
        assert_eq!(policy.configuration_revision,4);
        assert_eq!(policy.profile,nola_domain::PermissionProfile::Cautious);
        let mut unknown = value; unknown["obsolete_policy"] = json!(true);
        assert!(serde_json::from_value::<PermissionControlProjection>(unknown).is_err());
    }
}
#[derive(Debug)]
pub(crate) struct PermissionControlUpdate {
    pub session_id: String,
    pub turn_id: Option<String>,
    pub permissions: PermissionControlProjection,
}

#[derive(Debug, Clone)]
struct ActiveCanonicalTurn {
    run_id: String,
    kind: String,
    next_event_sequence: i64,
    invocation_config_digest: Option<String>,
}

impl SessionStore {
    /// The trusted Host calls this only after building a new target policy.
    /// Never accepts model bindings, policy proof, or private handles from UI.
    pub(crate) fn attest_edited_resend_admission(&mut self,operation:&str,begin:&mut BeginAssistantTurn)
        -> Result<(),String> {
        let prepared:Option<nola_domain::edited_resend::PreparedEditedResend>=self.application_port.execute(
            nola_application::ApplicationCommand::LoadPreparedEditedResend {conversation_id:self.id.clone()})?;
        let prepared=prepared.ok_or("edited resend target has no durable preparation")?;
        if prepared.input.operation_id!=operation || prepared.input.target_conversation_id!=self.id
            || prepared.admitted_run_id.is_some() || begin.conversation_id!=self.id
            || begin.user_message_id!=prepared.user_message_id || begin.assistant_message_id!=prepared.assistant_message_id
            || begin.user_text!=prepared.input.edited_text || begin.expected_parent_message_id.is_some()
            || begin.context_checkpoint_id.is_some() {
            return Err("edited resend admission differs from the exact prepared target".into());
        }
        let witness=self.application_port.attest_edited_resend_host_policy(&self.id,&begin.core_config,&begin.submission)
            .map_err(|error| {
                // Report only schema field names, never route values, credential
                // hashes, prompts, or the private model descriptor.
                if !error.contains("edited_resend_model_differs_from_prepared_intent") {return error;}
                let actual=serde_json::to_value(&begin.core_config).ok()
                    .and_then(|value| nola_domain::edited_resend::FrozenResendModel::from_invocation(&value))
                    .and_then(|value| serde_json::to_value(value).ok());
                let expected=serde_json::to_value(&prepared.source.model).ok();
                let mut fields=Vec::new();
                if let (Some(serde_json::Value::Object(expected)),Some(serde_json::Value::Object(actual)))=(expected,actual) {
                    for (key,value) in expected {
                        if actual.get(&key)==Some(&value) {continue;}
                        if let Some(nested)=value.as_object() {
                            fields.extend(nested.iter().filter(|(field,value)|actual.get(&key).and_then(|value|value.get(*field))!=Some(*value))
                                .map(|(field,_)|format!("{key}.{field}")));
                        } else {fields.push(key);}
                    }
                }
                format!("{error}; differing_model_fields={}",fields.join(","))
            })?;
        begin.core_config.turn_identity.fresh_resend_policy=Some(witness);
        Ok(())
    }

    pub(crate) fn apply_initial_creation_boundary(&self,
        input: &mut crate::host_invocation_request::HostSessionRequest) -> Result<(), String> {
        if input.session_id.as_deref().is_some_and(|id| id != self.id) {
            return Err("submission creation witness belongs to another conversation".into());
        }
        if input.expected_configuration_revision.is_none() && input.expected_parent_message_id.is_none() {
            // This immutable receipt is from the exact Create transaction.
            // It is intentionally not refreshed after messages/config edits.
            input.expected_configuration_revision = self.initial_configuration_revision;
        }
        Ok(())
    }

    pub(crate) fn read_submission_boundary(&mut self) -> Result<(i64, Option<String>), String> {
        let tail: Option<nola_application::ConversationTailSnapshot> = self.application_port.execute(
            nola_application::ApplicationCommand::LoadConversationTailSnapshotV1 {
                conversation_id: self.id.clone(), message_limit: 1, part_text_limit: 1, part_json_limit: 1,
            },
        )?;
        let tail = tail.ok_or("submission conversation no longer exists")?;
        if tail.conversation.id != self.id { return Err("submission snapshot owner mismatch".into()); }
        Ok((tail.conversation.configuration_revision, tail.conversation.messages.last().map(|message| message.id.clone())))
    }

    pub(crate) fn conversation_knowledge_context(&mut self) -> Result<Vec<nola_core::context_engine::ContextComponent>, String> {
        crate::conversation_knowledge_context::load(&mut self.application_port, &self.id)
    }

    pub(crate) fn load_host_execution_state(&mut self) -> Result<nola_application::HostExecutionState, String> {
        let before = self.application_port.active_run(&self.id)?;
        if let Some(active) = &before {
            let retained = self.active_canonical_turn.as_ref()
                .ok_or("Execution state cannot adopt an unowned canonical Run")?;
            if retained.run_id != active.run_id || retained.invocation_config_digest != active.invocation_config_digest {
                return Err("Execution Run ownership changed; reopen the exact canonical Run".into());
            }
        }
        let state: nola_application::HostExecutionState = self.application_port.execute(nola_application::ApplicationCommand::LoadHostExecutionState {
            conversation_id: self.id.clone(),
        })?;
        let after = self.application_port.active_run(&self.id)?;
        if before.as_ref().map(|run| (&run.run_id, &run.invocation_config_digest))
            != after.as_ref().map(|run| (&run.run_id, &run.invocation_config_digest)) {
            return Err("Canonical Run changed while reading execution state".into());
        }
        if before.is_none() && ((!state.control.turn_status.is_terminal() && state.control.active_turn_id.is_some())
            || !state.pending_effects.is_empty()) {
            return Err("Execution state has pending work without a canonical Run".into());
        }
        Ok(state)
    }

    pub(crate) fn memory_maintenance_state(&mut self) -> Result<(
        nola_core::agent::memory_runtime::MemoryRuntimeProjection,
        nola_core::memory::MemoryProjection,
    ), String> {
        // Explicit background maintenance, not a per-delivery execution view.
        let checkpoint = self.load_recovery_core_checkpoint()?;
        Ok((checkpoint.state.memory_runtime, checkpoint.state.memory_projection))
    }

    pub(crate) fn claimable_memory_jobs(&mut self, now_epoch_ms: i64)
        -> Result<Vec<nola_core::agent::MemoryJobHandle>, String>
    {
        self.application_port.execute(nola_application::ApplicationCommand::QueryClaimableMemoryJobs {
            input: nola_application::QueryClaimableMemoryJobs { conversation_id: self.id.clone(), now_epoch_ms },
        })
    }

    pub(crate) fn build_memory_worker_plan(&mut self, job_id: &str)
        -> Result<nola_core::agent::MemoryWorkerPlan, String>
    {
        self.application_port.execute(nola_application::ApplicationCommand::BuildMemoryWorkerPlan {
            input: nola_application::BuildMemoryWorkerPlan { conversation_id: self.id.clone(), job_id: job_id.into() },
        })
    }
    fn release_active_canonical_turn(&mut self) {
        self.active_canonical_turn = None;
        self.task_result_dispatches.clear();
    }
    fn retain_task_result_dispatches(
        &mut self,
        requests: &[nola_core::agent::host::HostRequestEnvelope],
    ) -> Result<(), String> {
        let run_id = self
            .active_canonical_turn
            .as_ref()
            .ok_or("Task dispatch has no active source Run")?
            .run_id
            .clone();
        for request in requests {
            let nola_core::agent::host::HostRequest::ExecuteTool {
                capability_id,
                call_id,
                input,
                ..
            } = &request.payload
            else {
                continue;
            };
            if capability_id != "wait_task" {
                continue;
            }
            let Some(task_id) = input.get("task_id").and_then(Value::as_str) else {
                continue;
            };
            let dispatch = TaskResultDispatch {
                run_id: run_id.clone(),
                request_id: request.request_id.clone(),
                session_id: request.session_id.clone(),
                turn_id: request.turn_id.clone(),
                call_id: call_id.clone(),
                task_id: task_id.into(),
            };
            if let Some(previous) = self.task_result_dispatches.get(&request.effect_id.0) {
                if previous != &dispatch {
                    return Err("wait_task dispatch identity changed within its source Run".into());
                }
            } else {
                if self.task_result_dispatches.len() >= 4096 {
                    return Err("bounded wait_task dispatch witness capacity exhausted".into());
                }
                self.task_result_dispatches
                    .insert(request.effect_id.0.clone(), dispatch);
            }
        }
        Ok(())
    }

    fn task_result_delivery(
        &self,
        response: &nola_core::agent::host::HostResponseEnvelope,
    ) -> Result<Option<nola_application::AcknowledgeRuntimeTaskResult>, String> {
        let nola_core::agent::host::HostResponse::ToolCompleted {
            call_id,
            output,
            outcome,
            ..
        } = &response.payload
        else {
            return Ok(None);
        };
        let Some(dispatch) = self.task_result_dispatches.get(&response.effect_id.0) else {
            return Ok(None);
        };
        if self
            .active_canonical_turn
            .as_ref()
            .map(|run| run.run_id.as_str())
            != Some(dispatch.run_id.as_str())
            || dispatch.request_id != response.request_id
            || dispatch.session_id != response.session_id
            || dispatch.turn_id != response.turn_id
            || dispatch.call_id != *call_id
        {
            return Err("Task result delivery does not match its actual wait_task dispatch".into());
        }
        let result: nola_host_protocol::tool_result::ToolResult =
            serde_json::from_value(output.clone())
                .map_err(|error| format!("decode committed wait_task result failed: {error}"))?;
        let Some(data) = result.data else {
            return Ok(None);
        };
        let crate::runtime_task_port::RuntimeTaskResultData::RuntimeTaskResultDelivery {
            cursor,
            ..
        } = serde_json::from_value(data)
            .map_err(|error| format!("decode wait_task delivery cursor failed: {error}"))?;
        if cursor.owner_conversation_id != self.id
            || cursor.task_id != dispatch.task_id
            || *outcome != nola_core::agent::host::HostToolOutcome::Succeeded
        {
            return Err(
                "wait_task delivery cursor belongs to another owner or failed invocation".into(),
            );
        }
        Ok(Some(cursor))
    }
    fn capture_control_permissions(
        &mut self,
        run_id: &str,
        control: &nola_core::agent::protocol::AgentControlSnapshot,
    ) -> Result<(), String> {
        if control.turn_status == nola_core::agent::AgentTurnStatus::AwaitingInput {
            // The same committed transition retired the Task attempt. Stop
            // local renewal without cancelling independent Core effects.
            if let Some(authority) = &self.task_execution { authority.revoke(); }
        }
        // Maintenance has its own admitted Run, but no conversation assistant
        // root or permission occurrence. Never query the previous conversation
        // Run to manufacture a permission owner for this control response.
        if self
            .active_canonical_turn
            .as_ref()
            .is_some_and(|active| active.run_id == run_id && active.kind == "maintenance")
        {
            return Ok(());
        }
        let owner = (
            run_id.to_owned(),
            control.session_id.as_str().to_owned(),
            control
                .active_turn_id
                .as_ref()
                .map(|turn| turn.as_str().to_owned()),
            control.permission_projection_revision,
        );
        if self
            .assistant_journal_notifications
            .lock()
            .map_err(|_| "control queue poisoned")?
            .permission_owner
            .as_ref()
            == Some(&owner)
        {
            return Ok(());
        }
        let witness = serde_json::json!({"conversation_id":self.id,"run_id":run_id,
            "session_id":owner.1,"active_turn_id":owner.2,"event_cursor":control.last_event_cursor.sequence,
            "permission_projection_revision":owner.3});
        let command = serde_json::json!({"type":"load_conversation_permission_projection_v1",
            "conversation_id":self.id,"run_id":run_id,"scope":{"type":"control",
                "session_id":owner.1,"active_turn_id":owner.2,"event_cursor":control.last_event_cursor.sequence,
                "permission_projection_revision":owner.3}});
        let response: serde_json::Value = self.application_port.execute(
            serde_json::from_value::<nola_application::ApplicationCommand>(command)
                .map_err(|error| error.to_string())?,
        )?;
        for (key, value) in witness.as_object().unwrap() {
            if key == "event_cursor" {
                if response
                    .get(key)
                    .and_then(serde_json::Value::as_u64)
                    .is_none_or(|cursor| cursor < control.last_event_cursor.sequence)
                {
                    return Err("permission projection cursor regressed".into());
                }
                continue;
            }
            if response.get(key) != Some(value) {
                return Err(format!("permission projection changed its exact {key}"));
            }
        }
        let permissions: PermissionControlProjection =
            serde_json::from_value(response["permissions"].clone())
                .map_err(|error| error.to_string())?;
        if permissions.revision != owner.3 {
            return Err("permission projection revision changed".into());
        }
        let mut delivery = self
            .assistant_journal_notifications
            .lock()
            .map_err(|_| "control queue poisoned")?;
        delivery.permission_update = Some(PermissionControlUpdate {
            session_id: owner.1.clone(),
            turn_id: owner.2.clone(),
            permissions,
        });
        delivery.permission_owner = Some(owner);
        Ok(())
    }
    pub(crate) fn read_source_run_journal(
        &mut self,
        run_id: &str,
        assistant_message_id: &str,
    ) -> Result<crate::source_journal::SourceRunJournal, String> {
        let mut journal = crate::source_journal::SourceRunJournal::new(
            self.id.clone(),
            run_id.to_owned(),
            assistant_message_id.to_owned(),
        );
        journal.catch_up(&mut self.application_port)?;
        Ok(journal)
    }

    pub(crate) fn goal_claim_for_seal(
        &mut self,seal:&nola_domain::assistant_run_terminal::AssistantRunTerminalSeal,
    )->Result<Option<crate::runtime_output::GoalCompleteSignal>,String>{
        if seal.identity.conversation_id!=self.id {return Err("Goal claim conversation mismatch".into());}
        let journal=self.read_source_run_journal(&seal.identity.run_id,&seal.identity.assistant_message_id)?;
        if journal.seal.as_ref()!=Some(seal) {return Err("Goal claim terminal seal changed".into());}
        crate::runtime_output::goal_claim_from_journal(&journal)
    }

    pub(crate) fn load_recovery_core_checkpoint(
        &mut self,
    ) -> Result<CoreCheckpoint<AgentThreadState>, String> {
        let active = self.application_port.active_run(&self.id)?;
        if let Some(active) = active {
            let retained = self
                .active_canonical_turn
                .as_ref()
                .ok_or("Recovery cannot adopt a Run not owned by this Session Store")?;
            if retained.run_id != active.run_id
                || retained.invocation_config_digest != active.invocation_config_digest
            {
                return Err(
                    "Recovery Run ownership changed; reopen the exact canonical Run".into(),
                );
            }
            return crate::core_agent_runtime::load_exact_recovery_checkpoint(
                &mut self.application_port,
                &self.id,
                &active,
            )
            .map(|(checkpoint, _)| checkpoint);
        }
        let stored = self.application_port.load_checkpoint(&self.id)?
            .ok_or("canonical Core checkpoint is missing")?;
        let checkpoint: CoreCheckpoint<AgentThreadState> = serde_json::from_str(&stored.checkpoint_json)
            .map_err(|error| format!("decode canonical Core checkpoint failed: {error}"))?;
        if checkpoint
            .state
            .execution
            .active_turn
            .as_ref()
            .is_some_and(|turn| !turn.status.is_terminal())
            || checkpoint
                .state
                .host
                .pending_effects
                .iter()
                .next()
                .is_some()
        {
            return Err(
                "Recovery checkpoint has pending execution without an active canonical Run".into(),
            );
        }
        if self.application_port.active_run(&self.id)?.is_some() {
            return Err("Canonical Run changed while reading idle recovery state".into());
        }
        self.release_active_canonical_turn();
        Ok(checkpoint)
    }

    pub(crate) fn set_task_execution_authority(
        &mut self,
        authority: Option<crate::runtime_task_port::RuntimeTaskExecutionAuthority>,
    ) {
        self.task_execution = authority;
    }

    pub(crate) fn apply_task_guidance(
        &mut self,
        task_id: &str,
        message_id: &str,
        expected_run_id: &str,
    ) -> Result<nola_application::TaskGuidanceApplyResult, String> {
        let owns_active_run = self.active_core_run_id() == Some(expected_run_id);
        // Keep checkpoint-bearing acknowledgements off the nested lease-lock
        // frames. The JSON contract is unchanged by this allocation boundary.
        let mut apply = |lease| {
            self.application_port.apply_task_guidance(
                task_id,
                message_id,
                expected_run_id,
                lease,
                nola_runtime_driver::monotonic_epoch_millis(),
            )
        };
        let result = if !owns_active_run {
            apply(None)?
        } else if let Some(authority) = &self.task_execution {
            match authority.with_active(|lease| {
                lease.with_attempt(|attempt| {
                    apply(Some(nola_application::RuntimeTaskLeaseWitness::from(
                        &*attempt,
                    )))
                })
            })? {
                Some(result) => result,
                None => apply(None)?,
            }
        } else {
            apply(None)?
        };
        if result.guidance.task_id != task_id
            || result.guidance.message_id != message_id
            || result.guidance.child_session_id != self.id
            || result.guidance.run_id != expected_run_id
        {
            return Err("task guidance acknowledgement identity changed".into());
        }
        if let Some(accepted) = &result.accepted {
            let active = self
                .active_canonical_turn
                .as_mut()
                .filter(|active| active.run_id == expected_run_id)
                .ok_or("guidance committed to a Run not owned by this Host")?;
            active.next_event_sequence = accepted.commit.next_event_sequence;
            self.capture_assistant_publications(&accepted.commit.assistant_publications);
            self.capture_control_permissions(expected_run_id, &accepted.control)?;
        } else if let Some(recovered) = &result.recovered {
            self.capture_assistant_publications(&recovered.assistant_publications);
            if self.active_core_run_id() == Some(expected_run_id) {
                self.release_active_canonical_turn();
            }
        } else if result.guidance.status != "rejected" {
            return Err("guidance has no committed delivery or rejection result".into());
        }
        Ok(*result)
    }

    #[cfg(test)]
    pub(crate) fn with_service_receipts(mut self) -> Self {
        self.application_port = self.application_port.with_service_receipts();
        self
    }
    pub(crate) fn assistant_journal_notifications_queue(
        &self,
    ) -> AssistantJournalNotificationQueue {
        Arc::clone(&self.assistant_journal_notifications)
    }

    fn capture_assistant_publications(
        &mut self,
        cursors: &[nola_domain::assistant_journal::AssistantJournalCursor],
    ) {
        if cursors.is_empty() {
            return;
        }
        if Arc::strong_count(&self.assistant_journal_notifications) == 1 {
            // With no JSON subscriber, durable heads are the recovery source.
            // Do not retain an ever-growing list of past Run owners in a plain
            // terminal session. A later subscriber must seed again, including
            // when it rejoins the same Run after an unobserved commit.
            let mut delivery = self
                .assistant_journal_notifications
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            delivery.journal_publications.clear();
            delivery.journal_seeded_run = None;
            return;
        }
        let mut delivery = self
            .assistant_journal_notifications
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for cursor in cursors {
            if let Some(observer) = delivery.observer.as_mut() {
                if let Err(error)=observer.observe(&mut self.application_port,cursor) {
                    delivery.observer_error=Some(error);
                }
            }
        }
        for cursor in cursors {
            // A notification is a target, not the content itself. Coalesce only
            // within an exact owner; consumers read every intervening record.
            if let Some(previous) = delivery
                .journal_publications
                .iter_mut()
                .find(|previous| previous.identity == cursor.identity)
            {
                if cursor.sequence > previous.sequence {
                    *previous = cursor.clone();
                }
            } else {
                delivery.journal_publications.push(cursor.clone());
            }
        }
    }

    pub(crate) fn prime_assistant_journal_notifications(&mut self) {
        if let Some(run_id) = self.active_core_run_id().map(str::to_owned) {
            let seed = self
                .assistant_journal_notifications
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .journal_seeded_run
                .as_deref()
                != Some(run_id.as_str());
            if seed {
                // A reopened Host has no process-local publication queue. Seed
                // from committed small heads, never from a checkpoint, text
                // body, trace sequence, or the latest arbitrary active Run.
                let result = (|| -> Result<(), String> {
                    let mut after_message_id = None;
                    loop {
                        let heads: Vec<nola_domain::assistant_journal::AssistantJournalCursor> = self.application_port.execute(
                            nola_application::ApplicationCommand::LocalRunAssistantJournalHeads {
                                conversation_id: self.id.clone(), run_id: run_id.clone(),
                                after_message_id: after_message_id.clone(), limit: 64,
                            })?;
                        self.capture_assistant_publications(&heads);
                        if heads.len() < 64 {
                            return Ok(());
                        }
                        after_message_id = heads
                            .last()
                            .map(|head| head.identity.assistant_message_id.clone());
                    }
                })();
                match result {
                    Ok(()) => {
                        self.assistant_journal_notifications
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .journal_seeded_run = Some(run_id.clone())
                    }
                    Err(error) => crate::llm::log_local_debug_payload(
                        "assistant_journal_recovery_read_failed",
                        &serde_json::json!({"conversation_id":self.id,"run_id":run_id,"error":error}),
                    ),
                }
            }
        }
    }

    pub(crate) fn recover_expired_core_deliveries(
        &mut self, binding: &nola_host_protocol::HostInvocationBinding,
    ) -> Result<usize, String> {
        let active = self.active_canonical_turn.as_ref()
            .ok_or("expired delivery recovery has no active canonical Run")?;
        let now = chrono::Local::now();
        self.application_port.execute(ApplicationCommand::RecoverExpiredRunDeliveries {
            conversation_id: self.id.clone(), expected_run_id: active.run_id.clone(),
            host_invocation: binding.clone(), interrupted_at: now.to_rfc3339(),
            interrupted_at_ms: now.timestamp_millis(),
        })
    }

    /// Recovery policy and request rebinding belong to the shared Application
    /// delivery ledger, including the decision to leave a live attempt alone.
    pub(crate) fn prepare_core_effect_recovery(&mut self) -> Result<nola_application::PreparedRunEffectRecovery, String> {
        let Some(active) = self.active_canonical_turn.as_ref() else { return Ok(Default::default()); };
        self.application_port.execute(ApplicationCommand::PrepareRunEffectRecovery {
            input: nola_application::PrepareRunEffectRecovery {
                conversation_id: self.id.clone(),
                run_id: active.run_id.clone(),
                invocation_config_digest: active.invocation_config_digest.clone()
                    .ok_or("active canonical Core turn has no invocation config digest")?,
                now_ms: monotonic_epoch_millis(),
            },
        })
    }

    pub(crate) fn reduce_active_core_submission(
        &mut self,
        transition_id: String,
        submission: nola_core::agent::protocol::AgentSubmission,
    ) -> Result<nola_application::AcceptedActiveSubmission, String> {
        let active = self
            .active_canonical_turn
            .clone()
            .ok_or_else(|| "canonical Core turn is not active".to_string())?;
        let accepted = self.application_port.reduce_and_commit_active_submission(
            ReduceAndCommitActiveSubmission {
                conversation_id: self.id.clone(),
                run_id: active.run_id.clone(),
                transition_id,
                invocation_config_digest: active.invocation_config_digest.ok_or_else(|| {
                    "active canonical Core turn has no invocation config digest".to_string()
                })?,
                submission,
                committed_at_ms: monotonic_epoch_millis(),
            },
        )?;
        self.capture_assistant_publications(&accepted.commit.assistant_publications);
        if let Some(active_turn) = self.active_canonical_turn.as_mut() {
            active_turn.next_event_sequence = accepted.commit.next_event_sequence;
        }
        self.capture_control_permissions(&active.run_id, &accepted.control)?;
        Ok(accepted)
    }

    pub fn open(
        requested_id: Option<&str>,
        continue_last: bool,
        project_root: &str,
    ) -> Result<Self, String> {
        let mut application_port = CliApplicationPort::persistent_default()?;
        let id = if continue_last {
            latest_canonical_session_id(&mut application_port, project_root)?
                .unwrap_or_else(new_session_id)
        } else {
            requested_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(new_session_id)
        };
        validate_session_id(&id)?;
        let now = Local::now().to_rfc3339();
        let restoring_existing_conversation = application_port
            .load_conversation(&id)
            .map_err(|error| format!("load canonical conversation failed: {error}"))?
            .is_some();
        let mut initial_configuration_revision = None;
        if !restoring_existing_conversation {
            let timestamp_ms = timestamp_ms_from_session_date(&now);
            let creation: nola_application::ConversationCreation = application_port
                .execute(nola_application::ApplicationCommand::CreateConversation { input: CreateConversation {
                    id: id.clone(),
                    title: default_session_title().to_string(),
                    settings_json: Some(
                        serde_json::to_string(&serde_json::json!({
                            "project_root": project_root
                        }))
                        .map_err(|error| format!("encode conversation settings failed: {error}"))?,
                    ),
                    created_at_ms: timestamp_ms,
                    updated_at_ms: timestamp_ms,
                } })
                .map_err(|error| format!("create canonical conversation failed: {error}"))?;
            if creation.created { initial_configuration_revision = creation.initial_configuration_revision; }
        }
        application_port
            .load_conversation(&id)
            .map_err(|error| format!("load canonical conversation failed: {error}"))?
            .ok_or_else(|| "canonical conversation disappeared after creation".to_string())?;
        // Only reopening an existing source can recover a previously committed
        // Task birth. A new conversation must not depend on Task-owner recovery
        // when there cannot yet be an original source commit to recover.
        if restoring_existing_conversation {
            application_port.recover_prepared_agent_tasks(
                &id,
                &mut CliApplicationPort::persistent_default()?,
            ).map_err(|error| format!("recover committed Task admission failed: {error}"))?;
        }
        let active_canonical_turn = application_port
            .active_run(&id)
            .map_err(|error| format!("load active canonical run failed: {error}"))?
            .map(|run| ActiveCanonicalTurn {
                run_id: run.run_id,
                kind: run.kind,
                next_event_sequence: run.next_event_sequence,
                invocation_config_digest: run.invocation_config_digest,
            });

        let store = Self {
            id,
            application_port,
            active_canonical_turn,
            initial_configuration_revision,
            task_execution: None,
            assistant_journal_notifications: Arc::default(),
            task_result_dispatches: Default::default(),
        };
        Ok(store)
    }

    /// Opens a transaction-complete session without retaining its transcript
    /// after process exit. `--no-save` changes retention, not execution
    /// semantics: every Turn still uses the canonical Store/outbox protocol.
    pub fn open_ephemeral(requested_id: Option<&str>, project_root: &str) -> Result<Self, String> {
        let mut application_port = CliApplicationPort::ephemeral()?;
        let id = requested_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(new_session_id);
        validate_session_id(&id)?;
        let now = Local::now();
        let now_ms = now.timestamp_millis();
        let creation: nola_application::ConversationCreation = application_port
            .execute(nola_application::ApplicationCommand::CreateConversation { input: CreateConversation {
                id: id.clone(),
                title: default_session_title().to_string(),
                settings_json: Some(
                    serde_json::to_string(&serde_json::json!({
                        "project_root": project_root,
                        "retention": "process"
                    }))
                    .map_err(|error| format!("encode ephemeral settings failed: {error}"))?,
                ),
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
            } })
            .map_err(|error| format!("create ephemeral canonical conversation failed: {error}"))?;
        Ok(Self {
            id: id.clone(),
            application_port,
            active_canonical_turn: None,
            initial_configuration_revision: if creation.created { creation.initial_configuration_revision } else { None },
            task_execution: None,
            assistant_journal_notifications: Arc::default(),
            task_result_dispatches: Default::default(),
        })
    }

    pub fn load_messages(&mut self) -> Result<Vec<Value>, String> {
        let conversation = self
            .application_port
            .load_conversation(&self.id)?
            .ok_or_else(|| format!("canonical conversation {} disappeared", self.id))?;
        conversation
            .messages
            .iter()
            .map(canonical_message_value)
            .collect()
    }

    pub fn recent_global_memory_records(
        &mut self,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, String> {
        let records: Vec<nola_application::MemoryRecord> = serde_json::from_value(
            self.application_port.execute(nola_application::ApplicationCommand::Memory {
                command: nola_application::MemoryCommand::Recall {
                    input: nola_application::MemoryRecallQuery {
                        binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
                        namespace: nola_core::agent::MemoryNamespace::Personal,
                        scope: nola_core::agent::MemoryScope::Global,
                        query: None, kind: None,
                        now_iso: chrono::Utc::now().to_rfc3339(), limit,
                    },
                },
            })?,
        ).map_err(|error| format!("decode canonical memory recall failed: {error}"))?;
        Ok(records.into_iter().map(|memory| memory.record).collect())
    }

    pub fn recent_global_memory_records_with_usage(
        &mut self,
        limit: usize,
    ) -> Result<(Vec<MemoryRecord>, std::collections::BTreeMap<String, u64>), String> {
        let records = self.recent_global_memory_records(limit)?;
        let ids = records.iter().map(|record| record.id.clone()).collect();
        let stats: Vec<nola_application::MemoryUsageStats> = serde_json::from_value(
            self.application_port
                .execute(nola_application::ApplicationCommand::Memory {
                    command: nola_application::MemoryCommand::UsageStats { ids },
                })?,
        )
        .map_err(|error| format!("decode canonical memory usage stats failed: {error}"))?;
        Ok((
            records,
            stats
                .into_iter()
                .map(|stats| (stats.memory_id, stats.usage_count))
                .collect(),
        ))
    }

    pub fn load_agent_checkpoint(&mut self) -> Result<Option<Value>, String> {
        self.application_port
            .load_checkpoint(&self.id)
            .map_err(|error| format!("load canonical Agent checkpoint failed: {error}"))?
            .map(|stored| {
                serde_json::from_str(&stored.checkpoint_json)
                    .map_err(|error| format!("decode canonical Agent checkpoint failed: {error}"))
            })
            .transpose()
    }

    pub(crate) fn import_task_terminal_projection(
        &mut self,
        task_id: &str,
        receipt_id: &str,
        guidance_ids: &[String],
    ) -> Result<nola_protocol::TaskTerminalReconciliationApplied, String> {
        let source = CliApplicationPort::persistent_default()?
            .export_agent_task_terminal_projection(task_id, receipt_id, guidance_ids)?;
        let ack = self
            .application_port
            .import_exported_agent_task_terminal_projection(&source)?;
        if ack.conversation_id != self.id {
            return Err("Task terminal acknowledgement belongs to another Session Store".into());
        }
        self.capture_task_projection_publications(source.parent_run_id())?;
        Ok(ack)
    }

    pub(crate) fn import_task_guidance_projection(
        &mut self,
        task_id: &str,
        message_id: &str,
        settlement: bool,
    ) -> Result<nola_protocol::TaskGuidanceProjectionApplied, String> {
        let source = CliApplicationPort::persistent_default()?
            .export_agent_task_guidance_projection(task_id, message_id, settlement)?;
        let ack = self
            .application_port
            .import_exported_agent_task_guidance_projection(&source)?;
        if ack.conversation_id != self.id {
            return Err("Task guidance acknowledgement belongs to another Session Store".into());
        }
        self.capture_task_projection_publications(source.parent_run_id())?;
        Ok(ack)
    }

    fn capture_task_projection_publications(&mut self, run_id: &str) -> Result<(), String> {
        // The acknowledgement proves control application, not presentation
        // delivery. Seed exact source-owned committed heads, including after
        // lost ACK or idempotent import, without reading body or latest Run.
        let mut after_message_id = None;
        loop {
            let heads: Vec<nola_domain::assistant_journal::AssistantJournalCursor> = self
                .application_port
                .execute(ApplicationCommand::LocalRunAssistantJournalHeads {
                    conversation_id: self.id.clone(),
                    run_id: run_id.into(),
                    after_message_id: after_message_id.clone(),
                    limit: 64,
                })?;
            self.capture_assistant_publications(&heads);
            if heads.len() < 64 {
                return Ok(());
            }
            after_message_id = heads
                .last()
                .map(|head| head.identity.assistant_message_id.clone());
        }
    }

    pub fn load_agent_checkpoint_for_session(session_id: &str) -> Result<Option<Value>, String> {
        validate_session_id(session_id)?;
        let mut canonical = application_port_default()?;
        if let Some(stored) = canonical
            .load_checkpoint(&session_id)
            .map_err(|error| format!("load canonical Agent checkpoint failed: {error}"))?
        {
            let checkpoint: Value = serde_json::from_str(&stored.checkpoint_json)
                .map_err(|error| format!("decode canonical Agent checkpoint failed: {error}"))?;
            return Ok(Some(checkpoint));
        }
        Ok(None)
    }

    pub fn save_messages(&mut self, messages: Vec<Value>) -> Result<(), String> {
        self.save_messages_ref(&messages)
    }

    pub fn begin_core_turn(
        &mut self,
        input: BeginAssistantTurn,
    ) -> Result<nola_application::AcceptedAssistantTurn, nola_service_client::ApplicationServiceCommandError> {
        let result = self.application_port.begin_assistant_turn(input)?;
        self.adopt_accepted_core_turn(&result);
        Ok(result)
    }

    pub(crate) fn begin_task_core_turn(
        &mut self,
        lease: nola_application::RuntimeTaskLeaseWitness,
        input: BeginAssistantTurn,
    ) -> Result<nola_application::AcceptedAssistantTurn, nola_service_client::ApplicationServiceCommandError> {
        let result = self
            .application_port
            .begin_task_assistant_turn(lease, input)?;
        self.adopt_accepted_core_turn(&result);
        Ok(result)
    }

    pub(crate) fn begin_queued_core_turn(
        &mut self,
        request_id: &str,
        input: nola_application::BeginQueuedUserTurn,
    ) -> Result<nola_application::AcceptedAssistantTurn, nola_service_client::ApplicationServiceCommandError> {
        let result = self.application_port.begin_queued_user_turn(request_id, input)?;
        self.adopt_accepted_core_turn(&result);
        Ok(result)
    }

    pub(crate) fn begin_automation_core_turn(
        &mut self,
        lease: nola_domain::automation_execution::AutomationExecutionLease,
        input: BeginAssistantTurn,
    ) -> Result<nola_application::AcceptedAssistantTurn, nola_service_client::ApplicationServiceCommandError> {
        let result = self.application_port.begin_automation_assistant_turn(lease, input)?;
        self.adopt_accepted_core_turn(&result);
        Ok(result)
    }

    fn adopt_accepted_core_turn(&mut self, result: &nola_application::AcceptedAssistantTurn) {
        self.task_result_dispatches.clear();
        self.capture_assistant_publications(&result.assistant_publications);
        self.active_canonical_turn = Some(ActiveCanonicalTurn {
            run_id: result.begin.run_id.clone(),
            kind: "conversation".into(),
            next_event_sequence: 1,
            invocation_config_digest: Some(result.invocation_config_digest.clone()),
        });
    }

    /// Promote a committed queued submission only through Application. The
    /// previous process-local owner is not authority to select its successor.
    pub(crate) fn promote_queued_core_turn(
        &mut self,
    ) -> Result<Option<nola_application::AcceptedAssistantTurn>, String> {
        if self.task_execution.is_some() {
            // A Task lease is bound to one admitted Run. A queued user Run may
            // not borrow that execution authority merely by sharing a session.
            return Ok(None);
        }
        let accepted: Option<nola_application::AcceptedAssistantTurn> = self.application_port
            .execute(ApplicationCommand::PromoteQueuedAssistantTurn {
                input: nola_application::PromoteQueuedAssistantTurn {
                    conversation_id: self.id.clone(),
                    created_at_ms: monotonic_epoch_millis(),
                },
            })?;
        if let Some(accepted) = &accepted {
            self.adopt_accepted_core_turn(accepted);
        }
        Ok(accepted)
    }

    pub fn begin_maintenance_core_turn(
        &mut self,
        input: BeginMaintenanceTurn,
    ) -> Result<nola_application::AcceptedMaintenanceTurn, String> {
        let result = self.application_port.begin_maintenance_turn(input)?;
        self.task_result_dispatches.clear();
        self.active_canonical_turn = Some(ActiveCanonicalTurn {
            run_id: result.run_id.clone(),
            kind: "maintenance".into(),
            next_event_sequence: 1,
            invocation_config_digest: Some(result.invocation_config_digest.clone()),
        });
        Ok(result)
    }

    pub(crate) fn active_core_run_id(&self) -> Option<&str> {
        self.active_canonical_turn
            .as_ref()
            .map(|turn| turn.run_id.as_str())
    }

    pub fn has_active_core_turn(&self) -> bool {
        self.active_canonical_turn.is_some()
    }

    pub fn pending_core_effect_requests(
        &mut self,
    ) -> Result<
        (
            Vec<String>,
            Vec<nola_core::agent::host::HostRequestEnvelope>,
        ),
        String,
    > {
        let active = self
            .active_canonical_turn
            .as_ref()
            .ok_or_else(|| "canonical Core turn is not active".to_string())?;
        let committed = self
            .application_port
            .load_committed_host_requests(&active.run_id)?;
        self.retain_task_result_dispatches(&committed.requests)?;
        Ok((committed.effect_ids, committed.requests))
    }

    pub fn save_messages_ref(&mut self, messages: &[Value]) -> Result<(), String> {
        if self.active_canonical_turn.is_some() {
            return Err(
                "canonical transcript is owned by the active Core Run; provider snapshots cannot write it"
                    .to_string(),
            );
        }
        let persisted = self.load_messages()?;
        let persisted_len = persisted.len();
        let can_append = persisted_len <= messages.len()
            && persisted
                .iter()
                .zip(messages.iter())
                .all(|(left, right)| left == right);
        if can_append {
            return self.append_messages_ref(&messages[persisted_len..]);
        }
        Err("canonical transcript conflict: persisted history is immutable; reload the conversation before appending".to_string())
    }

    pub fn append_messages_ref(&mut self, messages: &[Value]) -> Result<(), String> {
        if messages.is_empty() {
            return Ok(());
        }
        let start_position = self.load_messages()?.len();
        let appended = messages
            .iter()
            .map(sanitize_session_message)
            .collect::<Vec<_>>();
        let timestamp_ms = Local::now().timestamp_millis();
        let canonical_messages = appended
            .iter()
            .enumerate()
            .map(|(offset, message)| {
                canonical_message_input(
                    &self.id,
                    start_position + offset,
                    message,
                    timestamp_ms + offset as i64,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.application_port
            .import_conversation_transcript(ImportConversationTranscript {
                import_id: format!("external-archive:{}:{start_position}", self.id),
                source: ConversationTranscriptImportSource::ExternalArchive,
                conversation_id: self.id.clone(),
                expected_message_count: start_position,
                messages: canonical_messages,
                imported_at_ms: timestamp_ms,
            })
            .map_err(|error| format!("import canonical transcript failed: {error}"))?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "task_result_delivery_tests.rs"]
mod task_result_delivery_tests;

#[cfg(test)]
#[path = "session_foreground_cancellation_tests.rs"]
mod foreground_cancellation_tests;

#[cfg(test)]
#[path = "session_code_history_tests.rs"]
mod code_history_tests;

impl CommittedRuntimeApplicationPort for SessionStore {
    type Protocol = nola_core::agent::host::AgentRuntimeProtocol;
    fn dispatch_application_request(
        &mut self,
        request: &nola_core::agent::host::HostRequestEnvelope,
        publish: &mut dyn FnMut(CommittedRuntimeAcceptance),
    ) -> Result<bool, String> {
        use nola_core::agent::host::{HostRequest, HostResponse, HostResponseEnvelope, HostToolOutcome};
        if !matches!(request.payload, HostRequest::ReadCodeCellHistory { .. } | HostRequest::CancelTask { .. }) {
            return Ok(false);
        }
        let mut sequence = request.resume_after_sequence.map_or(0, |value| value.saturating_add(1));
        let mut receipt = |payload| {
            let response = HostResponseEnvelope {
                delegated_invocation_id: request.delegated_invocation_id.clone(),
                attempt_id: request.attempt_id.clone(), request_id: request.request_id.clone(),
                effect_id: request.effect_id.clone(), host_id: request.host_id.clone(),
                lease_id: request.lease_id.clone(), session_id: request.session_id.clone(),
                turn_id: request.turn_id.clone(), catalog_revision: request.catalog_revision,
                availability_epoch: request.availability_epoch, delivery: request.delivery.clone(),
                sequence, payload,
            };
            sequence = sequence.saturating_add(1);
            response
        };
        if matches!(request.payload, HostRequest::CancelTask { .. })
            && request.is_expired_at(nola_runtime_driver::monotonic_epoch_millis() as u64) {
            publish(self.accept_raw_host_response(receipt(HostResponse::ExecutionRejected {
                code:"deadline_exceeded".into(),message:"Task cancellation request expired before execution started".into(),retryable:false,
            }))?);
            return Ok(true);
        }
        if request.delivery.is_some() && !request.attempt_started {
            publish(self.accept_raw_host_response(receipt(HostResponse::ExecutionStarted))?);
        }
        if let HostRequest::CancelTask { task_id } = &request.payload {
            let outcome = self.application_port.execute_task_cancellation_control(request);
            let response = match outcome {
                Ok(cancel_requested) => HostResponse::Completed {
                    output: serde_json::json!({"task_id":task_id,"cancel_requested":cancel_requested}),
                    usage:None,full_output_ref:None,artifacts:Vec::new(),
                },
                Err(message) => HostResponse::Failed { error:nola_core::agent::host::HostError {
                    code:"task_cancellation_failed".into(),message,retryable:true,http_status:None,
                } },
            };
            publish(self.accept_raw_host_response(receipt(response))?);
            return Ok(true);
        }
        let HostRequest::ReadCodeCellHistory { call_id, .. } = &request.payload else { unreachable!() };
        let observed: Result<Option<nola_domain::code_mode::CodeCellView>, String> =
            self.application_port.execute(ApplicationCommand::ReadCodeCellHistoryForEffect {
                request: request.clone(),
            });
        let (output, outcome) = match observed {
            Ok(Some(view)) => (serde_json::to_value(view).map_err(|error| error.to_string())?, HostToolOutcome::Succeeded),
            Ok(None) => (serde_json::json!({"error":"code cell history not found"}), HostToolOutcome::Failed),
            Err(error) => (serde_json::json!({"error":error}), HostToolOutcome::Failed),
        };
        publish(self.accept_raw_host_response(receipt(HostResponse::ToolCompleted {
            call_id: call_id.clone(), output, outcome, activation: None,
            full_output_ref: None, artifacts: Vec::new(),
        }))?);
        Ok(true)
    }

    fn load_committed_requests(
        &mut self,
    ) -> Result<Vec<nola_core::agent::host::HostRequestEnvelope>, String> {
        // A terminal Host response can close the Run while the Runtime Driver
        // is still completing its current dispatch batch. Reloading the
        // committed outbox after that response is a normal drain operation,
        // not an attempt to resume an inactive turn.
        if self.active_canonical_turn.is_none() {
            return Ok(Vec::new());
        }
        let requests = self
            .pending_core_effect_requests()
            .map(|(_, requests)| requests)?;
        if requests.is_empty()
            && self
                .application_port
                .active_run(&self.id)
                .map_err(|error| format!("refresh canonical Run failed: {error}"))?
                .is_none()
        {
            self.release_active_canonical_turn();
        }
        Ok(requests)
    }

    fn claim_application_control_requests(
        &mut self, claim: RuntimeEffectDeliveryClaim,
        observed: &[nola_core::agent::host::HostRequestEnvelope],
    ) -> Result<Vec<nola_core::agent::host::HostRequestEnvelope>, String> {
        // Read-only wake hint avoids empty durable command receipts. Core
        // selects and validates the current outbox again inside its claim UoW.
        if !observed.iter().any(|request| matches!(request.payload,
            nola_core::agent::host::HostRequest::CancelTask { .. })) { return Ok(Vec::new()); }
        let Some(active) = self.active_canonical_turn.as_ref() else { return Ok(Vec::new()); };
        let committed: nola_application::CommittedHostRequests = self.application_port.execute(
            ApplicationCommand::ClaimApplicationControlRequests { input:nola_application::ClaimCommittedHostRequests {
                run_id:active.run_id.clone(),worker_generation:claim.worker_generation,
                lease_owner:claim.lease_owner,claimed_at_ms:claim.claimed_at_ms,
                lease_expires_at_ms:claim.lease_expires_at_ms,limit:claim.limit,
            } },
        )?;
        Ok(committed.requests)
    }

    fn claim_committed_requests(
        &mut self,
        claim: RuntimeEffectDeliveryClaim,
    ) -> Result<Vec<nola_core::agent::host::HostRequestEnvelope>, String> {
        let active = match self.active_canonical_turn.as_ref() {
            Some(active) => active,
            None => return Ok(Vec::new()),
        };
        let committed = self.application_port.claim_committed_host_requests(
            nola_application::ClaimCommittedHostRequests {
                run_id: active.run_id.clone(),
                worker_generation: claim.worker_generation,
                lease_owner: claim.lease_owner,
                claimed_at_ms: claim.claimed_at_ms,
                lease_expires_at_ms: claim.lease_expires_at_ms,
                limit: claim.limit,
            },
        )?;
        self.retain_task_result_dispatches(&committed.requests)?;
        if committed.requests.is_empty()
            && self
                .application_port
                .active_run(&self.id)
                .map_err(|error| format!("refresh canonical Run failed: {error}"))?
                .is_none()
        {
            self.release_active_canonical_turn();
        }
        Ok(committed.requests)
    }

    fn renew_effect_delivery(
        &mut self,
        renewal: RuntimeEffectDeliveryRenewal,
    ) -> Result<nola_core::agent::host::EffectDeliveryLease, String> {
        let active = self
            .active_canonical_turn
            .as_ref()
            .ok_or_else(|| "canonical Core turn is not active during lease renewal".to_string())?;
        self.application_port
            .heartbeat_effect_delivery(nola_application::HeartbeatEffectDelivery {
                run_id: active.run_id.clone(),
                attempt_id: renewal.delivery.attempt_id,
                worker_generation: renewal.delivery.worker_generation,
                lease_owner: renewal.delivery.lease_owner,
                lease_token: renewal.delivery.lease_token,
                current_lease_expires_at_ms: renewal.delivery.lease_expires_at_ms,
                heartbeat_at_ms: renewal.renewed_at_ms,
                requested_lease_expires_at_ms: renewal.lease_expires_at_ms,
            })
    }


    fn accept_raw_host_response(
        &mut self,
        response: nola_core::agent::host::HostResponseEnvelope,
    ) -> Result<CommittedRuntimeAcceptance, String> {
        let active = self
            .active_canonical_turn
            .clone()
            .ok_or_else(|| "canonical Core turn is not active".to_string())?;
        let invocation_config_digest = active.invocation_config_digest.ok_or_else(|| {
            "active canonical Core turn has no invocation config digest".to_string()
        })?;
        let task_result_delivery = self.task_result_delivery(&response)?;
        let promoted_task = match &response.payload {
            nola_core::agent::host::HostResponse::TaskPromoted { task_id, kind, .. }
                if *kind == nola_core::agent::TaskKind::Agent => Some(task_id.clone()),
            _ => None,
        };
        let (response_effect_id, response_request_id, response_sequence) = (
            response.effect_id.0.clone(),
            response.request_id.clone(),
            response.sequence,
        );
        let accepted =
            self.application_port
                .reduce_and_accept_host_response(ReduceAndAcceptHostResponse {
                    conversation_id: self.id.clone(),
                    run_id: active.run_id.clone(),
                    invocation_config_digest,
                    response,
                    received_at_ms: monotonic_epoch_millis(),
                })?;
        if let Some(active_turn) = self.active_canonical_turn.as_mut() {
            active_turn.next_event_sequence = accepted.commit.next_event_sequence;
        }
        self.capture_assistant_publications(&accepted.commit.assistant_publications);
        // Source acceptance is final even if target admission fails. Replay
        // the same immutable proof on an exact response retry, including
        // applied=false; cold open uses this same path for Prepared targets.
        if let Some(task_id) = promoted_task {
            if !self.application_port.recover_agent_task_admission(
                &self.id, &task_id, &mut CliApplicationPort::persistent_default()?,
            )? {
                return Err("committed Task promotion has no original admission evidence".into());
            }
            // Promotion and admission are committed. Wake the existing owner's
            // Supervisor now; a foreground Goal need not return to its composer
            // or call wait_task before its independent child can claim a slot.
            // An explicit owner never falls back to another project's policy.
            if accepted.control.as_ref().is_none_or(|control| !matches!(control.turn_status,
                nola_core::agent::AgentTurnStatus::Cancelling | nola_core::agent::AgentTurnStatus::Cancelled)) {
                crate::task_runtime::reconcile_bound_supervised_tasks(
                    Some(&self.id), std::path::Path::new(""),
                );
            }
        }
        // Acceptance, including an exact replay, proves the source tool result
        // committed before any target mailbox mutation. A failed ACK retains
        // both that source result and the unconsumed Task for later delivery;
        // it must never redispatch the already settled tool.
        if let Some(cursor) = task_result_delivery {
            let delivery_id = format!(
                "{}:{}:{}:{}",
                active.run_id, response_effect_id, response_request_id, response_sequence
            );
            acknowledge_wait_task_result(&cursor,&delivery_id).map_err(|error|format!(
                "source wait_task result committed; target delivery remains unacknowledged; do not redispatch: {error}"))?;
        }
        // Do not release the process-local turn while accepting one response.
        if let Some(control) = &accepted.control {
            self.capture_control_permissions(&active.run_id, control)?;
        }
        // A response can atomically commit another effect even when its step
        // snapshot is terminal. `load_committed_requests` performs the durable
        // drain check after the whole response transaction is visible.
        Ok(CommittedRuntimeAcceptance {
            events: accepted.events,
            traces: accepted.traces,
            control: accepted.control,
        })
    }

    fn accept_boundary_input(
        &mut self,
        input: nola_core::agent::host::AgentBoundaryInput,
    ) -> Result<CommittedRuntimeAcceptance, String> {
        use nola_core::agent::host::AgentBoundaryInput;
        match input {
            AgentBoundaryInput::PermissionPolicyAvailable { policy } => {
                let run_id = policy.run_id.clone();
                let accepted: Option<nola_application::AcceptedActiveSubmission> = self.application_port.execute(
                    nola_application::ApplicationCommand::ApplyActivePermissionPolicy {
                        conversation_id: self.id.clone(), policy,
                        now_ms: monotonic_epoch_millis(),
                    })?;
                return match accepted {
                    Some(accepted) => {
                        self.capture_control_permissions(&run_id, &accepted.control)?;
                        if let Some(retained) = self.active_canonical_turn.as_mut() {
                            retained.next_event_sequence = accepted.commit.next_event_sequence;
                        }
                        Ok(CommittedRuntimeAcceptance { events: accepted.events, traces: accepted.traces,
                            control: Some(accepted.control) })
                    }
                    None => Ok(CommittedRuntimeAcceptance { events: Vec::new(), traces: Vec::new(), control: None }),
                };
            }
            AgentBoundaryInput::Submission { submission } => {
                return self.accept_boundary_submission(submission)
            }
            AgentBoundaryInput::TaskTerminalAvailable {
                task_id,
                receipt_id,
            } => {
                let guidance_ids = self.load_host_execution_state()?.pending_guidance_ids
                    .remove(&task_id).unwrap_or_default();
                self.import_task_terminal_projection(&task_id, &receipt_id, &guidance_ids)?;
            }
            AgentBoundaryInput::TaskGuidanceAvailable {
                task_id,
                message_id,
                settlement,
            } => {
                self.import_task_guidance_projection(&task_id, &message_id, settlement)?;
            }
        }
        let control = self.load_host_execution_state()?.control;
        if let Some(active) = self.application_port.active_run(&self.id)? {
            if let Some(retained) = self.active_canonical_turn.as_mut() {
                if retained.run_id != active.run_id {
                    return Err("Task import changed active Run owner".into());
                }
                retained.next_event_sequence = active.next_event_sequence;
            }
            self.capture_control_permissions(&active.run_id, &control)?;
        }
        Ok(CommittedRuntimeAcceptance {
            events: Vec::new(),
            traces: Vec::new(),
            control: Some(control),
        })
    }

}

impl SessionStore {
    /// Foreground Interrupt has one boundary owner. Use Application's exact
    /// cancellation command instead of reducing a second raw Interrupt after
    /// an earlier poll (or a lost ACK) already committed Cancelling.
    fn accept_foreground_cancellation(
        &mut self,
        cancellation_id: String,
        reason: Option<String>,
    ) -> Result<CommittedRuntimeAcceptance, String> {
        let retained = self.active_canonical_turn.clone()
            .ok_or("foreground cancellation has no retained canonical Run")?;
        let active = self.application_port.active_run(&self.id)?;
        let assistant_message_id = match active.as_ref() {
            Some(active) if active.run_id == retained.run_id
                && active.invocation_config_digest == retained.invocation_config_digest => {
                active.assistant_message_id.clone()
                    .ok_or("foreground cancellation Run has no assistant owner")?
            }
            Some(_) => return Err("foreground cancellation cannot target a successor Run".into()),
            None => self.load_assistant_run_terminal_seal(&retained.run_id)?
                .ok_or("foreground cancellation has neither an active owner nor a terminal seal")?
                .identity.assistant_message_id,
        };
        let receipt: nola_application::RunCancellationReceipt = self.application_port.execute(
            nola_application::ApplicationCommand::RequestRunCancellation {
                input: nola_application::RequestRunCancellation {
                    conversation_id: self.id.clone(),
                    expected_run_id: retained.run_id.clone(),
                    assistant_message_id: assistant_message_id.clone(),
                    cancellation_id: cancellation_id.clone(),
                    reason,
                    requested_at_ms: monotonic_epoch_millis(),
                },
            },
        )?;
        if receipt.conversation_id != self.id || receipt.run_id != retained.run_id
            || receipt.assistant_message_id != assistant_message_id
            || receipt.cancellation_id != cancellation_id {
            return Err("foreground cancellation receipt changed its exact owner".into());
        }
        // AlreadyCancelling acknowledges this stop request, not that its
        // echoed ID was the original committed Cancellation fact. Only the
        // following exact control/Host drain can establish completion.
        let state = self.load_host_execution_state()?;
        if state.control.active_turn_id.as_ref().map(|turn| turn.as_str())
            != Some(retained.run_id.as_str())
            || !(state.control.turn_status == nola_core::agent::AgentTurnStatus::Cancelling
                || state.control.turn_status.is_terminal()) {
            return Err("foreground cancellation has no matching committed control".into());
        }
        if state.control.turn_status.is_terminal() {
            let seal = self.load_assistant_run_terminal_seal(&retained.run_id)?
                .ok_or("foreground cancellation terminal control has no exact seal")?;
            if receipt.terminal_result.as_ref() != Some(&seal.result)
                || receipt.result_digest.as_ref() != Some(&seal.result_digest) {
                return Err("foreground cancellation terminal receipt differs from its seal".into());
            }
        } else if receipt.disposition == nola_application::RunCancellationDisposition::AlreadyTerminal {
            return Err("foreground cancellation terminal receipt has nonterminal control".into());
        }
        let mut after_message_id = None;
        loop {
            let heads: Vec<nola_domain::assistant_journal::AssistantJournalCursor> = self.application_port.execute(
                nola_application::ApplicationCommand::LocalRunAssistantJournalHeads {
                    conversation_id: self.id.clone(), run_id: retained.run_id.clone(),
                    after_message_id: after_message_id.clone(), limit: 64,
                },
            )?;
            self.capture_assistant_publications(&heads);
            if heads.len() < 64 { break; }
            after_message_id = heads.last().map(|head| head.identity.assistant_message_id.clone());
        }
        self.capture_control_permissions(&retained.run_id, &state.control)?;
        Ok(CommittedRuntimeAcceptance {
            events: Vec::new(), traces: Vec::new(), control: Some(state.control),
        })
    }

    fn accept_boundary_submission(
        &mut self,
        submission: nola_core::agent::protocol::AgentSubmission,
    ) -> Result<CommittedRuntimeAcceptance, String> {
        if let nola_core::agent::AgentSubmission::Interrupt { goal_control: None, cancellation_id, reason } = submission {
            return self.accept_foreground_cancellation(cancellation_id, reason);
        }
        if let nola_core::agent::AgentSubmission::DeliverTaskGuidance {
            task_id,
            message_id,
            expected_run_id,
            ..
        } = &submission
        {
            let result = self.apply_task_guidance(task_id, message_id, expected_run_id)?;
            let acceptance = match (result.accepted, result.recovered) {
                (Some(accepted), _) => CommittedRuntimeAcceptance {
                    events: accepted.events,
                    traces: accepted.traces,
                    control: Some(accepted.control),
                },
                (None, Some(recovered)) => CommittedRuntimeAcceptance {
                    events: recovered.events,
                    traces: recovered.traces,
                    control: Some(recovered.execution.control),
                },
                (None, None) => CommittedRuntimeAcceptance {
                    events: Vec::new(),
                    traces: Vec::new(),
                    control: None,
                },
            };
            if let Some(control) = &acceptance.control {
                self.capture_control_permissions(expected_run_id, control)?;
            }
            return Ok(acceptance);
        }
        let accepted = self.reduce_active_core_submission(
            format!("boundary:{}", uuid::Uuid::new_v4()),
            submission,
        )?;
        Ok(CommittedRuntimeAcceptance {
            events: accepted.events,
            traces: accepted.traces,
            control: Some(accepted.control),
        })
    }
}

impl SessionStore {
    pub(crate) fn load_assistant_run_terminal_seal(
        &mut self,
        run_id: &str,
    ) -> Result<Option<nola_domain::assistant_run_terminal::AssistantRunTerminalSeal>, String> {
        self.application_port.execute(
            nola_application::ApplicationCommand::LocalAssistantRunTerminalSeal {
                conversation_id: self.id.clone(),
                run_id: run_id.to_owned(),
            },
        )
    }

    pub(crate) fn start_goal_execution(&mut self, input: nola_application::goal_execution::StartGoalExecution)
        -> Result<nola_application::AcceptedIdleSubmission, nola_service_client::ApplicationServiceCommandError> {
        let accepted: nola_application::AcceptedIdleSubmission = self.application_port.execute_typed(
            ApplicationCommand::StartGoalExecution { input })?;
        self.capture_assistant_publications(&accepted.assistant_publications);
        Ok(accepted)
    }

    pub(crate) fn reduce_idle_core_submission(
        &mut self,
        core_config: AgentCoreConfig,
        submission: nola_core::agent::protocol::AgentSubmission,
    ) -> Result<nola_application::AcceptedIdleSubmission, nola_service_client::ApplicationServiceCommandError> {
        let accepted = self.application_port.reduce_and_commit_idle_submission(
            nola_application::ReduceAndCommitIdleSubmission {
                conversation_id: self.id.clone(),
                core_config,
                submission,
                committed_at_ms: monotonic_epoch_millis(),
            },
        )?;
        // Idle task/skill changes can publish for historical assistant owners.
        // There is deliberately no active Run from which to infer that owner.
        self.capture_assistant_publications(&accepted.assistant_publications);
        Ok(accepted)
    }

    pub(crate) fn revert_conversation(
        &mut self,
        core_config: AgentCoreConfig,
        operation_id: String,
        through_message_id: String,
        keep_message: bool,
    ) -> Result<nola_application::AcceptedConversationRevert, String> {
        self.application_port
            .revert_conversation(nola_application::RevertConversation {
                conversation_id: self.id.clone(),
                core_config,
                operation_id,
                through_message_id,
                keep_message,
                committed_at_ms: monotonic_epoch_millis(),
            })
    }
}

fn canonical_message_input(
    conversation_id: &str,
    zero_based_index: usize,
    message: &Value,
    timestamp_ms: i64,
) -> Result<MessageInput, String> {
    let ordinal = zero_based_index as i64 + 1;
    let provider_role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("assistant");
    let role = match provider_role {
        "user" | "assistant" | "system" => provider_role,
        _ => "assistant",
    }
    .to_string();
    let content_text = message
        .get("content")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| message.to_string());
    let content_json = serde_json::to_string(message)
        .map_err(|error| format!("encode canonical provider message failed: {error}"))?;
    let stable_prefix = format!("cli-{conversation_id}-{ordinal}");
    Ok(MessageInput {
        id: format!("{stable_prefix}-message"),
        run_id: None,
        role,
        status: "completed".to_string(),
        ordinal,
        created_at_ms: timestamp_ms,
        completed_at_ms: Some(timestamp_ms),
        parts: vec![MessagePartInput {
            id: format!("{stable_prefix}-part"),
            kind: "provider_message".to_string(),
            ordinal: 1,
            state: "completed".to_string(),
            content_text,
            content_json: Some(content_json),
            content_digest: None,
            created_at_ms: timestamp_ms,
            updated_at_ms: timestamp_ms,
        }],
    })
}

fn canonical_message_value(message: &ConversationMessage) -> Result<Value, String> {
    let part = message
        .parts
        .first()
        .ok_or_else(|| format!("canonical message {} has no parts", message.id))?;
    // Provider messages contain the complete provider envelope. Core Parts
    // carry their own typed JSON and are projected below instead.
    if part.kind == "provider_message" {
        if part.content_json_deferred {
            // The preview is not a provider envelope. In particular, a user
            // envelope may actually contain only tool results.
            return Ok(serde_json::json!({
                "role": message.role,
                "content": "",
                "message_id": message.id,
                "turn_id": message.run_id,
                "projection_truncated": true,
                "metadata_deferred": true,
            }));
        }
        if let Some(content_json) = part.content_json.as_deref() {
            return serde_json::from_str(content_json)
                .map_err(|error| format!("decode canonical provider message failed: {error}"));
        }
        if let Ok(envelope) = serde_json::from_str::<Value>(&part.content_text) {
            if envelope.get("role").and_then(Value::as_str).is_some() {
                return Ok(envelope);
            }
        }
        return Ok(serde_json::json!({
            "role": message.role,
            "content": part.content_text,
            "message_id": message.id,
            "turn_id": message.run_id,
            "projection_truncated": true,
        }));
    }
    let display_text_parts: Vec<Value> = message
        .parts
        .iter()
        .filter(|part| part.kind == "text")
        .map(|part| {
            let review = part
                .content_json
                .as_deref()
                .and_then(|value| serde_json::from_str::<Value>(value).ok())
                .and_then(|value| value.get("review").cloned());
            serde_json::json!({"id": part.id, "text": part.content_text, "review": review,
                "metadata_deferred": part.content_json_deferred})
        })
        .collect();
    let text = display_text_parts
        .iter()
        .filter(|part| {
            part["metadata_deferred"] != true
                && (part["review"].is_null() || part["review"] == "approved")
        })
        .filter_map(|part| part["text"].as_str())
        .collect::<String>();
    let has_reviewed_parts = display_text_parts
        .iter()
        .any(|part| !part["review"].is_null() || part["metadata_deferred"] == true);
    let text = if display_text_parts.is_empty() && part.kind == "ui_surface" {
        String::new()
    } else if display_text_parts.is_empty() {
        part.content_text.clone()
    } else {
        text
    };
    let mut value = serde_json::json!({
        "role": message.role,
        "display_status": message.status,
        "content": text,
        "message_id": message.id,
        "turn_id": message.run_id,
    });
    if has_reviewed_parts {
        value["display_text_parts"] = Value::Array(display_text_parts);
    }
    let mut surface_summaries = Vec::new();
    for part in &message.parts {
        if part.kind != "ui_surface" || part.content_json_deferred { continue; }
        let Some(raw) = &part.content_json else { continue; };
        let typed: nola_domain::MessagePartView = serde_json::from_str(raw)
            .map_err(|error| format!("decode UI Surface Part {} failed: {error}", part.id))?;
        if let nola_domain::MessagePartView::UiSurface { surface, summary, .. } = typed {
            surface.validate().map_err(|error| error.to_string())?;
            if surface.identity.owner.message_id != message.id {
                return Err("UI Surface summary has a different message owner".into());
            }
            if !summary.trim().is_empty() { surface_summaries.push(summary); }
        }
    }
    if !surface_summaries.is_empty() {
        value["display_ui_surface_summaries"] = serde_json::to_value(surface_summaries)
            .map_err(|error| error.to_string())?;
    }
    Ok(value)
}

fn timestamp_ms_from_session_date(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|date| date.timestamp_millis())
        .unwrap_or_else(|_| Local::now().timestamp_millis())
}

fn first_user_text(messages: &[Value]) -> Option<String> {
    messages.iter().find_map(|message| {
        (message.get("role").and_then(Value::as_str) == Some("user"))
            .then(|| message.get("content").and_then(Value::as_str))
            .flatten()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(|text| text.chars().take(80).collect())
    })
}

fn latest_canonical_session_id(
    store: &mut CliApplicationPort,
    project_root: &str,
) -> Result<Option<String>, String> {
    Ok(store
        .list_conversation_page(
            (!project_root.is_empty()).then_some(project_root),
            true,
            1,
            0,
        )?
        .into_iter()
        .next()
        .map(|summary| summary.id))
}

fn application_port_default() -> Result<CliApplicationPort, String> {
    CliApplicationPort::persistent_default()
}

fn ensure_canonical_conversation(
    store: &mut CliApplicationPort,
    session_id: &str,
) -> Result<(), String> {
    if store
        .load_conversation(session_id)
        .map_err(|error| format!("load canonical conversation failed: {error}"))?
        .is_some()
    {
        return Ok(());
    }
    let now = Local::now().timestamp_millis();
    store
        .create_conversation(CreateConversation {
            id: session_id.to_string(),
            title: default_session_title().to_string(),
            settings_json: None,
            created_at_ms: now,
            updated_at_ms: now,
        })
        .map_err(|error| format!("create canonical checkpoint conversation failed: {error}"))?;
    Ok(())
}

fn default_session_title() -> &'static str {
    "New Conversation"
}

pub fn list_sessions() -> Result<Vec<SessionInfo>, String> {
    application_port_default()?
        .list_conversations()
        .map_err(|error| format!("list canonical conversations failed: {error}"))?
        .into_iter()
        .map(|item| {
            Ok(SessionInfo {
                id: item.id,
                title: item.title,
                created_at: canonical_timestamp(item.created_at_ms),
                updated_at: canonical_timestamp(item.updated_at_ms),
                message_count: item.message_count.max(0) as usize,
            })
        })
        .collect()
}

pub fn session_message_count(id: &str) -> Result<usize, String> {
    validate_session_id(id)?;
    application_port_default()?
        .conversation_summary(id)
        .map_err(|error| format!("read canonical conversation summary failed: {error}"))?
        .map(|summary| summary.message_count.max(0) as usize)
        .ok_or_else(|| format!("{} {id}", tr(Message::FailedReadSessionPrefix)))
}

#[allow(dead_code)]
pub fn list_sessions_with_first_user_message() -> Result<Vec<SessionResumeInfo>, String> {
    list_sessions_with_first_user_message_with_limit(usize::MAX)
}

#[allow(dead_code)]
pub fn list_sessions_with_first_user_message_with_limit(
    limit: usize,
) -> Result<Vec<SessionResumeInfo>, String> {
    canonical_resume_sessions(None, limit)
}

#[allow(dead_code)]
pub fn list_sessions_with_first_user_message_for_project(
    project_root: &str,
) -> Result<Vec<SessionResumeInfo>, String> {
    list_sessions_with_first_user_message_for_project_with_limit(project_root, usize::MAX)
}

pub fn list_sessions_with_first_user_message_for_project_with_limit(
    project_root: &str,
    limit: usize,
) -> Result<Vec<SessionResumeInfo>, String> {
    canonical_resume_sessions(Some(project_root), limit)
}

#[allow(dead_code)]
pub fn session_has_messages(id: &str) -> Result<bool, String> {
    validate_session_id(id)?;
    Ok(application_port_default()?
        .load_conversation(id)
        .map_err(|error| format!("load canonical conversation failed: {error}"))?
        .is_some_and(|conversation| !conversation.messages.is_empty()))
}

fn canonical_resume_sessions(
    project_root: Option<&str>,
    limit: usize,
) -> Result<Vec<SessionResumeInfo>, String> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut application = application_port_default()?;
    let mut summaries = Vec::new();
    while summaries.len() < limit {
        let count = (limit - summaries.len()).min(1024);
        let page = application
            .list_conversation_page(project_root, true, count, summaries.len())
            .map_err(|error| format!("list canonical resume sessions failed: {error}"))?;
        let complete = page.len() < count;
        summaries.extend(page);
        if complete {
            break;
        }
    }
    let mut sessions = Vec::with_capacity(limit.min(summaries.len()));
    for summary in summaries {
        if summary.message_count <= 0 {
            continue;
        }
        if let Some(expected_project_root) = project_root {
            if summary.project_root.as_deref() != Some(expected_project_root) {
                continue;
            }
        }
        let first_user_message = summary
            .first_user_message
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        sessions.push(SessionResumeInfo {
            id: summary.id,
            updated_at: canonical_timestamp(summary.updated_at_ms),
            message_count: summary.message_count.max(0) as usize,
            first_user_message,
            last_user_message_at: canonical_timestamp(
                summary
                    .last_user_message_at_ms
                    .unwrap_or(summary.updated_at_ms),
            ),
        });
        if sessions.len() == limit {
            break;
        }
    }
    Ok(sessions)
}

fn extract_plain_text_from_content(content: &Value) -> Option<String> {
    match content {
        Value::String(text) => {
            if text.is_empty() {
                None
            } else {
                Some(text.clone())
            }
        }
        Value::Array(items) => {
            for item in items {
                if let Some(text) = extract_plain_text_from_content(item) {
                    if !text.trim().is_empty() {
                        return Some(text);
                    }
                }
            }
            None
        }
        Value::Object(item) => item
            .get("text")
            .or_else(|| item.get("content"))
            .or_else(|| item.get("value"))
            .or_else(|| item.get("input"))
            .and_then(extract_plain_text_from_content)
            .or_else(|| {
                item.get("parts")
                    .and_then(Value::as_array)
                    .and_then(|parts| parts.iter().find_map(extract_plain_text_from_content))
            }),
        _ => {
            let text = content.to_string();
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
    }
}

pub fn latest_session_id_for_project(project_root: &str) -> Result<Option<String>, String> {
    let mut store = application_port_default()?;
    latest_canonical_session_id(&mut store, project_root)
}

/// Loads the complete recovery read model in one Application command and one
/// SQLite snapshot. Callers must not reconstruct it from separate Store reads.
pub fn load_resume_snapshot_for_project(
    project_root: &str,
    requested_at_ms: i64,
) -> Result<Option<nola_application::ResumeSnapshot>, String> {
    let mut store = application_port_default()?;
    store
        .load_resume_snapshot(project_root, None, requested_at_ms)
        .map_err(|error| format!("load canonical resume snapshot failed: {error}"))
}

pub fn acknowledge_resume_snapshot(
    snapshot: &nola_application::ResumeSnapshot,
) -> Result<(), String> {
    let mut store = application_port_default()?;
    store
        .acknowledge_resume_snapshot(nola_application::AcknowledgeResumeSnapshot {
            conversation_id: snapshot.projection.conversation_id.clone(),
            checkpoint_revision: snapshot.projection.store_revision,
            mailbox_cursor: snapshot.mailbox_cursor.clone(),
        })
        .map_err(|error| format!("acknowledge canonical resume snapshot failed: {error}"))
}

/// Atomically consumes one terminal task result after `wait_task` has placed
/// that exact durable result in the parent model's tool response.
///
/// A wait result is one exact delivery, not the complete resume mailbox.
/// Store fences the immutable terminal receipt and row without consuming a
/// concurrently completed peer or claiming that the UI displayed either one.
pub(crate) fn acknowledge_wait_task_result(
    cursor: &nola_application::AcknowledgeRuntimeTaskResult,
    source_delivery_id: &str,
) -> Result<bool, String> {
    application_port_default()?
        .acknowledge_task_result_delivery(cursor, source_delivery_id)
        .map_err(|error| format!("acknowledge wait_task result failed: {error}"))
}

#[allow(dead_code)]
pub fn latest_session_id() -> Result<Option<String>, String> {
    let mut store = application_port_default()?;
    latest_canonical_session_id(&mut store, "")
}

pub fn load_session(id: &str) -> Result<SavedSession, String> {
    validate_session_id(id)?;
    let canonical = application_port_default()?
        .load_conversation(id)
        .map_err(|error| format!("load canonical conversation failed: {error}"))?
        .ok_or_else(|| format!("{} {id}", tr(Message::FailedReadSessionPrefix)))?;
    let mut session = SavedSession {
        id: id.to_string(),
        created_at: canonical_timestamp(canonical.created_at_ms),
        updated_at: canonical_timestamp(canonical.updated_at_ms),
        messages: canonical
            .messages
            .iter()
            .map(canonical_message_value)
            .collect::<Result<Vec<_>, _>>()?,
    };
    sanitize_saved_session(&mut session);
    Ok(session)
}

/// Loads only the recent, bounded transcript needed to restore the terminal.
///
/// This deliberately uses a read-only SQLite projection in the CLI layer. It
/// does not mutate or bypass the canonical Application service, and it avoids
/// adding a terminal-specific query to Core. Provider envelopes that fit the
/// display budget are preserved (including Tool and Skill calls); oversized
/// text is represented by a tail excerpt with an explicit truncation marker.
pub fn load_interactive_session_snapshot(id: &str) -> Result<InteractiveSessionSnapshot, String> {
    validate_session_id(id)?;
    let mut application = application_port_default()?;
    let snapshot: Option<nola_application::ConversationTailSnapshot> =
        serde_json::from_value(application.execute(
            nola_application::ApplicationCommand::LoadConversationTailSnapshotV1 {
                conversation_id: id.to_string(),
                message_limit: INTERACTIVE_REPLAY_MESSAGE_LIMIT,
                part_text_limit: INTERACTIVE_REPLAY_PART_LIMIT,
                part_json_limit: INTERACTIVE_REPLAY_JSON_LIMIT,
            },
        )?)
        .map_err(|error| format!("decode interactive session projection failed: {error}"))?;
    let snapshot =
        snapshot.ok_or_else(|| format!("{} {id}", tr(Message::FailedReadSessionPrefix)))?;
    let messages = snapshot
        .conversation
        .messages
        .iter()
        .map(canonical_message_value)
        .collect::<Result<Vec<_>, _>>()?;
    let messages = bound_interactive_replay_messages(messages, INTERACTIVE_REPLAY_CHARACTER_LIMIT);
    let mut saved = SavedSession {
        id: snapshot.conversation.id,
        created_at: canonical_timestamp(snapshot.conversation.created_at_ms),
        updated_at: canonical_timestamp(snapshot.conversation.updated_at_ms),
        messages,
    };
    sanitize_saved_session(&mut saved);
    Ok(InteractiveSessionSnapshot {
        saved,
        total_message_count: snapshot.total_message_count,
        total_user_turns: snapshot.total_user_turns,
    })
}

fn bound_interactive_replay_messages(messages: Vec<Value>, budget: usize) -> Vec<Value> {
    let mut remaining = budget;
    let mut kept = Vec::new();
    for mut message in messages.into_iter().rev() {
        if remaining == 0 {
            break;
        }
        if let Some(summaries) = message.get_mut("display_ui_surface_summaries") {
            let size = interactive_value_character_count(summaries);
            if size > remaining {
                truncate_interactive_value_tail(summaries, remaining);
                message["nola_history_truncated"] = Value::Bool(true);
            }
            remaining = remaining.saturating_sub(size);
        }
        if let Some(parts) = message
            .get_mut("display_text_parts")
            .and_then(Value::as_array_mut)
        {
            let before = remaining;
            for part in parts.iter_mut().rev() {
                if let Some(text) = part.get_mut("text") {
                    let size = interactive_value_character_count(text);
                    if remaining == 0 {
                        *text = Value::String(String::new());
                    } else if size > remaining {
                        truncate_interactive_value_tail(text, remaining);
                    }
                    remaining = remaining.saturating_sub(size);
                }
            }
            if before > 0 && remaining == 0 {
                message["nola_history_truncated"] = Value::Bool(true);
            }
            kept.push(message);
            continue;
        }
        let size = message
            .get("content")
            .map(interactive_value_character_count)
            .unwrap_or(0);
        if size > remaining {
            if let Some(content) = message.get_mut("content") {
                if remaining == 0 { *content = Value::String(String::new()); }
                else { truncate_interactive_value_tail(content, remaining); }
            }
            if let Some(object) = message.as_object_mut() {
                object.insert("nola_history_truncated".into(), Value::Bool(true));
            }
            remaining = 0;
        } else {
            remaining = remaining.saturating_sub(size);
        }
        kept.push(message);
    }
    kept.reverse();
    kept
}

fn interactive_value_character_count(value: &Value) -> usize {
    match value {
        Value::String(text) => text.chars().count(),
        Value::Array(items) => items.iter().map(interactive_value_character_count).sum(),
        Value::Object(object) => object.values().map(interactive_value_character_count).sum(),
        _ => 0,
    }
}

fn truncate_interactive_value_tail(value: &mut Value, budget: usize) {
    match value {
        Value::String(text) => {
            if text.chars().count() > budget {
                let tail = text
                    .chars()
                    .rev()
                    .take(budget.saturating_sub(1))
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>();
                *text = format!("…{tail}");
            }
        }
        Value::Array(items) => {
            let mut remaining = budget;
            for item in items.iter_mut().rev() {
                let size = interactive_value_character_count(item);
                truncate_interactive_value_tail(item, remaining.min(size));
                remaining = remaining.saturating_sub(size);
                if remaining == 0 {
                    break;
                }
            }
        }
        Value::Object(object) => {
            for key in ["text", "content", "value"] {
                if let Some(content) = object.get_mut(key) {
                    truncate_interactive_value_tail(content, budget);
                    break;
                }
            }
        }
        _ => {}
    }
}

pub fn load_session_messages_with_ids(id: &str) -> Result<Vec<Value>, String> {
    validate_session_id(id)?;
    let conversation = application_port_default()?
        .load_conversation(id)
        .map_err(|error| format!("load canonical conversation failed: {error}"))?
        .ok_or_else(|| format!("{} {id}", tr(Message::FailedReadSessionPrefix)))?;
    conversation
        .messages
        .iter()
        .map(|message| {
            let mut value = canonical_message_value(message)?;
            let object = value
                .as_object_mut()
                .ok_or_else(|| format!("canonical message {} is not an object", message.id))?;
            object.insert("id".to_string(), Value::String(message.id.clone()));
            object.insert("message_id".to_string(), Value::String(message.id.clone()));
            object.insert(
                "created_at".to_string(),
                Value::String(canonical_timestamp(message.created_at_ms)),
            );
            object.insert(
                "parts".to_string(),
                Value::Array(
                    message
                        .parts
                        .iter()
                        .map(canonical_part_value)
                        .collect::<Result<Vec<_>, _>>()?,
                ),
            );
            Ok(value)
        })
        .collect()
}

pub fn search_session_text(
    id: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<nola_protocol::ConversationRecallHit>, String> {
    validate_session_id(id)?;
    application_port_default()?.execute(
        nola_protocol::ConversationRecallCommand::SearchConversationText {
            conversation_id: id.to_string(),
            query: query.to_string(),
            limit,
        },
    )
}

pub fn query_canonical_history(input: nola_domain::canonical_history::CanonicalHistoryQuery)
    -> Result<nola_domain::canonical_history::CanonicalHistoryResponse, String> {
    input.validate()?;
    application_port_default()?.execute(nola_protocol::conversation_recall::CanonicalHistoryCommand::QueryCanonicalHistory { input })
}

pub fn read_session_text_range(
    id: &str,
    message_id: &str,
    part_id: &str,
    offset_bytes: u64,
    limit_bytes: usize,
) -> Result<nola_protocol::ConversationTextRange, String> {
    validate_session_id(id)?;
    application_port_default()?.execute(
        nola_protocol::ConversationRecallCommand::ReadConversationTextRange {
            conversation_id: id.to_string(),
            message_id: message_id.to_string(),
            part_id: part_id.to_string(),
            offset_bytes,
            limit_bytes,
        },
    )
}

/// Serializes the durable product Part rather than reconstructing a provider
/// message. Core Parts contain their complete typed envelope in `content_json`.
fn canonical_part_value(part: &ConversationPart) -> Result<Value, String> {
    if let Some(content_json) = part.content_json.as_deref() {
        let decoded: Value = serde_json::from_str(content_json)
            .map_err(|error| format!("decode canonical part {} failed: {error}", part.id))?;
        if decoded.get("kind").and_then(Value::as_str) == Some(part.kind.as_str()) {
            return Ok(decoded);
        }
        return Ok(serde_json::json!({
            "id": part.id,
            "kind": part.kind,
            "state": part.state,
            "ordinal": part.ordinal,
            "content": decoded,
        }));
    }
    Ok(serde_json::json!({
        "id": part.id,
        "kind": part.kind,
        "state": part.state,
        "ordinal": part.ordinal,
        "text": part.content_text,
    }))
}

pub fn load_session_trace_runs(id: &str) -> Result<Vec<SessionTraceRun>, String> {
    validate_session_id(id)?;
    let mut application = application_port_default()?;
    let runs: Vec<nola_application::TraceRun> = serde_json::from_value(application.execute(
        nola_application::ApplicationCommand::TraceRunsForConversation {
            conversation_id: id.to_string(),
        },
    )?)
    .map_err(|error| format!("decode canonical trace runs failed: {error}"))?;
    runs.into_iter()
        .map(|run| {
            let records: Vec<nola_application::TraceRecord> = serde_json::from_value(
                application.execute(nola_application::ApplicationCommand::TraceEventsAfter {
                    trace_id: run.trace_id.clone(),
                    sequence: 0,
                })?,
            )
            .map_err(|error| format!("decode canonical trace events failed: {error}"))?;
            Ok(SessionTraceRun {
                trace_id: run.trace_id,
                message_id: run.message_id,
                events: records.into_iter().map(|record| record.event).collect(),
            })
        })
        .collect()
}

/// Loads Core's durable background-task projection for one session.
/// Tool traces are deliberately not consulted: only Host-promoted Tasks
/// belong in this projection.
pub fn load_session_projection(
    id: &str,
) -> Result<Option<nola_application::SessionProjection>, String> {
    validate_session_id(id)?;
    application_port_default()?
        .load_session_projection(id)
        .map_err(|error| format!("load canonical session projection failed: {error}"))
}

pub fn load_session_task_projection(id: &str) -> Result<Option<TaskProjection>, String> {
    Ok(load_session_projection(id)?.map(|projection| projection.task_projection))
}

/// Loads Core's durable memory extraction/consolidation runtime. The
/// projection includes ownership leases, retry deadlines, frozen input
/// watermarks and usage accounting; Host traces are not used to reconstruct it.
pub fn load_session_memory_runtime_projection(
    id: &str,
) -> Result<Option<nola_core::agent::MemoryRuntimeProjection>, String> {
    Ok(load_session_projection(id)?.map(|projection| projection.memory_runtime_projection))
}
/// Loads the authoritative long-horizon Goal projection from the Core checkpoint.
pub fn load_session_goal_projection(
    id: &str,
) -> Result<Option<nola_core::agent::GoalProjection>, String> {
    validate_session_id(id)?;
    application_port_default()?.load_session_goal_projection(id)
        .map_err(|error| format!("load canonical Goal projection failed: {error}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueGoalSession {
    pub session_id: String,
    pub objective: String,
    pub constraints: Vec<String>,
}

/// Returns runnable unclaimed Goals and Goals whose durable wake/lease expired. Discovery is
/// read-only; the worker must still acquire the Core Goal lease before doing
/// any work, which closes races between scheduler processes.
pub fn due_goal_sessions(now_epoch_ms: i64) -> Result<Vec<DueGoalSession>, String> {
    let mut due = Vec::new();
    for saved in list_sessions()? {
        let Some(projection) = load_session_goal_projection(&saved.id)? else {
            continue;
        };
        let Some(goal) = projection.active else {
            continue;
        };
        if !goal_runtime_is_due(
            goal.status,
            goal.next_wake_at_epoch_ms,
            goal.lease.as_ref().map(|lease| lease.lease_until_epoch_ms),
            goal.updated_at_epoch_ms,
            now_epoch_ms,
        ) {
            continue;
        }
        due.push(DueGoalSession {
            session_id: saved.id,
            objective: goal.objective,
            constraints: goal.constraints,
        });
    }
    due.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    Ok(due)
}

fn goal_runtime_is_due(
    status: nola_core::agent::GoalStatus,
    wake_at_epoch_ms: Option<i64>,
    lease_until_epoch_ms: Option<i64>,
    updated_at_epoch_ms: i64,
    now_epoch_ms: i64,
) -> bool {
    match status {
        nola_core::agent::GoalStatus::Paused => {
            wake_at_epoch_ms.is_some_and(|wake| wake <= now_epoch_ms)
        }
        nola_core::agent::GoalStatus::Running => {
            lease_until_epoch_ms.is_some_and(|lease_until| lease_until <= now_epoch_ms)
                || (lease_until_epoch_ms.is_none()
                    && updated_at_epoch_ms <= now_epoch_ms)
        }
        _ => false,
    }
}

/// Loads the authoritative pending-permission projection from the same Core
/// checkpoint as conversation Parts, Tasks, and Skill runtime state.
pub fn load_session_permission_projection(
    id: &str,
) -> Result<Option<nola_core::agent::permissions::PermissionProjection>, String> {
    Ok(load_session_projection(id)?.map(|projection| projection.permission_projection))
}

/// Canonical Web/Host projection of Core's durable Skill runtime.
///
/// This deliberately carries the Core checkpoint cursor so clients can apply
/// the snapshot before trace history and reject replayed Skill events that are
/// already represented by the checkpoint.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionSkillProjection {
    pub session_id: String,
    pub cursor: u64,
    pub revision: u64,
    pub catalog_revision: u64,
    pub order: Vec<String>,
    pub runs: std::collections::BTreeMap<String, nola_core::agent::capability::SkillRun>,
}

impl SessionSkillProjection {
    fn from_runtime(
        session_id: &str,
        cursor: u64,
        runtime: nola_core::agent::capability::SkillRuntimeState,
    ) -> Result<Self, String> {
        for (run_key, run) in &runtime.runs {
            if run_key != &run.run_id {
                return Err(format!(
                    "Agent skill checkpoint run key `{run_key}` does not match run_id `{}`",
                    run.run_id
                ));
            }
            if run.owner.session_id != session_id {
                return Err(format!(
                    "Agent skill checkpoint run `{run_key}` belongs to a different session"
                ));
            }
        }
        let order = runtime.runs.keys().cloned().collect::<Vec<_>>();
        Ok(Self {
            session_id: session_id.to_string(),
            cursor,
            revision: runtime.revision,
            catalog_revision: runtime.catalog.revision,
            order,
            runs: runtime.runs,
        })
    }
}

/// Restores the typed Skill runtime projection from the same canonical Core
/// checkpoint used for conversation Parts and background Tasks.
pub fn load_session_skill_projection(id: &str) -> Result<Option<SessionSkillProjection>, String> {
    Ok(load_session_projection(id)?.map(|projection| {
        let runtime = projection.skill_runtime_projection;
        SessionSkillProjection {
            session_id: runtime.session_id,
            cursor: runtime.cursor,
            revision: runtime.revision,
            catalog_revision: runtime.catalog_revision,
            order: runtime.order,
            runs: runtime.runs,
        }
    }))
}

#[cfg(test)]
mod skill_projection_contract_tests {
    use super::SessionSkillProjection;
    use nola_core::agent::capability::SkillRuntimeState;

    #[test]
    fn typed_skill_projection_uses_canonical_web_keys_and_checkpoint_cursor() {
        let projection = SessionSkillProjection::from_runtime(
            "session-typed-skill",
            73,
            SkillRuntimeState::default(),
        )
        .expect("empty runtime is a valid projection");
        let value = serde_json::to_value(&projection).expect("serialize Skill projection");

        assert_eq!(value["sessionId"], "session-typed-skill");
        assert_eq!(value["cursor"], 73);
        assert_eq!(value["revision"], 0);
        assert_eq!(value["catalogRevision"], 0);
        assert_eq!(value["order"], serde_json::json!([]));
        assert_eq!(value["runs"], serde_json::json!({}));
        assert!(value.get("session_id").is_none());
    }
}

pub fn delete_session(id: &str) -> Result<(), String> {
    validate_session_id(id)?;
    let mut canonical = application_port_default()?;
    canonical
        .delete_conversation(id)
        .map_err(|error| format!("delete canonical conversation failed: {error}"))?;
    Ok(())
}

fn validate_session_id(id: &str) -> Result<(), String> {
    let ok = id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if ok {
        Ok(())
    } else {
        Err(tr(Message::InvalidSessionId).to_string())
    }
}

fn new_session_id() -> String {
    let now = Local::now();
    let sequence = SESSION_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}{:09}-{}-{sequence:x}",
        now.format("%Y%m%d%H%M%S"),
        now.timestamp_subsec_nanos(),
        std::process::id(),
    )
}

#[allow(dead_code)]
fn now_string() -> String {
    Local::now().to_rfc3339()
}

fn sanitize_saved_session(session: &mut SavedSession) {
    session.messages = session
        .messages
        .iter()
        .map(sanitize_session_message)
        .collect();
}

fn sanitize_session_message(message: &Value) -> Value {
    let Some(object) = message.as_object() else {
        return sanitize_session_value(message, None);
    };
    let role = object
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut out = Map::new();
    for (key, value) in object {
        let cautious = match (role, key.as_str()) {
            ("tool", "content") => {
                let tool_name = object
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                sanitize_tool_message_content(tool_name, value)
            }
            ("assistant", "tool_calls") => sanitize_tool_calls(value),
            (_, "content") | (_, "reasoning_content") => sanitize_session_text_value(value),
            _ => sanitize_session_value(value, Some(key)),
        };
        out.insert(key.clone(), cautious);
    }
    Value::Object(out)
}

fn sanitize_tool_message_content(tool_name: &str, value: &Value) -> Value {
    if force_sensitive_tool_result(tool_name) {
        return Value::String("[sensitive tool result hidden; status=unknown]".to_string());
    }
    sanitize_session_text_value(value)
}

fn sanitize_tool_calls(value: &Value) -> Value {
    let Some(calls) = value.as_array() else {
        return sanitize_session_value(value, Some("tool_calls"));
    };
    Value::Array(calls.iter().map(sanitize_tool_call).collect())
}

fn sanitize_tool_call(value: &Value) -> Value {
    let Some(call) = value.as_object() else {
        return sanitize_session_value(value, None);
    };
    let mut out = Map::new();
    for (key, child) in call {
        let cautious = if key == "function" {
            sanitize_tool_call_function(child)
        } else {
            sanitize_session_value(child, Some(key))
        };
        out.insert(key.clone(), cautious);
    }
    Value::Object(out)
}

fn sanitize_tool_call_function(value: &Value) -> Value {
    let Some(function) = value.as_object() else {
        return sanitize_session_value(value, Some("function"));
    };
    let tool_name = function
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut out = Map::new();
    for (key, child) in function {
        let cautious = if key == "arguments" {
            sanitize_tool_call_arguments(tool_name, child)
        } else {
            sanitize_session_value(child, Some(key))
        };
        out.insert(key.clone(), cautious);
    }
    Value::Object(out)
}

fn sanitize_tool_call_arguments(tool_name: &str, value: &Value) -> Value {
    let Some(arguments) = value.as_str() else {
        return sanitize_session_value(value, Some("arguments"));
    };
    match serde_json::from_str::<Value>(arguments) {
        Ok(Value::Object(args)) => {
            let cautious = sanitize_tool_args(tool_name, &args);
            Value::String(
                serde_json::to_string(&cautious).unwrap_or_else(|_| redact_sensitive_text(arguments)),
            )
        }
        Ok(other) => Value::String(redact_sensitive_text(&other.to_string())),
        Err(_) => Value::String(redact_sensitive_text(arguments)),
    }
}

pub(crate) fn sanitize_tool_args(tool_name: &str, args: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in args {
        let cautious = if should_redact_tool_arg(tool_name, key) {
            redacted_json_value(value)
        } else {
            sanitize_session_value(value, Some(key))
        };
        out.insert(key.clone(), cautious);
    }
    out
}

fn sanitize_session_text_value(value: &Value) -> Value {
    if let Some(text) = value.as_str() {
        Value::String(redact_sensitive_text(text))
    } else {
        sanitize_session_value(value, None)
    }
}

fn sanitize_session_value(value: &Value, key: Option<&str>) -> Value {
    if key.map(sensitive_key).unwrap_or(false) {
        return redacted_json_value(value);
    }
    match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| sanitize_session_value(item, None))
                .collect(),
        ),
        Value::Object(object) => {
            let mut out = Map::new();
            for (child_key, child_value) in object {
                out.insert(
                    child_key.clone(),
                    sanitize_session_value(child_value, Some(child_key)),
                );
            }
            Value::Object(out)
        }
        Value::String(text) => Value::String(redact_sensitive_text(text)),
        _ => value.clone(),
    }
}

fn should_redact_tool_arg(tool_name: &str, key: &str) -> bool {
    if sensitive_key(key) {
        return true;
    }
    matches!(
        (tool_name, key),
        ("write_file", "content")
            | ("append_file", "content")
            | ("edit_file", "old_text")
            | ("edit_file", "new_text")
            | ("multi_edit_file", "edits")
            | ("apply_patch", "patch")
            | ("chrome_native_type_text", "text")
            | ("computer_type_at", "text")
            | ("computer_type_element", "text")
            | ("computer_set_element_value", "value")
            | ("computer_type_text", "text")
            | ("computer_set_clipboard", "text")
            | ("computer_assert_clipboard", "text")
            | ("computer_wait_for_clipboard", "text")
    )
}

fn force_sensitive_tool_result(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "computer_get_clipboard"
            | "computer_copy_selection"
            | "computer_assert_clipboard"
            | "computer_wait_for_clipboard"
    )
}

fn sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.contains("password")
        || lower.contains("passwd")
        || lower == "pwd"
        || lower.contains("token")
        || lower.contains("secret")
        || lower.contains("cookie")
        || lower.contains("session")
        || lower.contains("auth")
        || lower.contains("credential")
        || lower.contains("private_key")
        || lower.contains("apikey")
        || lower.contains("api_key")
        || lower == "api-key"
        || lower == "jwt"
}

fn redacted_json_value(value: &Value) -> Value {
    if let Some(text) = value.as_str() {
        if text == "[redacted]" || (text.starts_with("[redacted: ") && text.ends_with(" chars]")) {
            return Value::String(text.to_string());
        }
        Value::String(format!("[redacted: {} chars]", text.chars().count()))
    } else {
        Value::String("[redacted]".to_string())
    }
}

pub(crate) fn redact_sensitive_text(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut redacted = text.to_string();
    for (regex, replacement) in session_redaction_patterns() {
        redacted = regex.replace_all(&redacted, *replacement).into_owned();
    }
    redacted
}

fn session_redaction_patterns() -> &'static [(Regex, &'static str)] {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            (
                Regex::new(r#"(?i)(["']?\b(?:api[_-]?key|apikey|access[_-]?token|refresh[_-]?token|id[_-]?token|token|password|passwd|pwd|secret|credential|session[_-]?id|session|cookie|auth|authorization)\b["']?\s*(?::|=|\bis\b)\s*)(?:"[^"]*"|'[^']*'|(?:Bearer\s+)?[A-Za-z0-9._~+/=-]{8,})"#)
                    .expect("valid redaction regex"),
                "${1}[redacted]",
            ),
            (
                Regex::new(r#"(?i)\b(token|secret|password|passwd)\s+([A-Za-z0-9._~+/=-]{8,})\b"#)
                    .expect("valid whitespace redaction regex"),
                "${1} [redacted]",
            ),
            (
                Regex::new(r#"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{8,}"#)
                    .expect("valid redaction regex"),
                "Bearer [redacted]",
            ),
            (
                Regex::new(r#"\b(?:sk-[A-Za-z0-9_-]{8,}|ghp_[A-Za-z0-9_]{8,}|github_pat_[A-Za-z0-9_]{8,})\b"#)
                    .expect("valid redaction regex"),
                "[redacted]",
            ),
        ]
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn creation_receipt_is_immutable_and_never_refreshes_an_external_witness() {
        let _environment = crate::store::test_env_lock();
        let mut store = super::SessionStore::open_ephemeral(Some("creation-owner"), "/fixture").unwrap();
        let initial = store.initial_configuration_revision.unwrap();
        let mut input = crate::host_invocation_request::HostInvocationRequest::for_test().unwrap().session;
        input.session_id = Some("creation-owner".into());
        input.expected_configuration_revision = None;
        input.expected_parent_message_id = None;
        store.apply_initial_creation_boundary(&mut input).unwrap();
        assert_eq!(input.expected_configuration_revision, Some(initial));
        let _: serde_json::Value = store.application_port.execute(
            nola_application::ApplicationCommand::PatchConversationConfiguration {
                input: nola_application::conversation_configuration::PatchConversationConfigurationInput {
                    conversation_id: "creation-owner".into(), expected_revision: initial,
                    patch_json: r#"{"permissionProfile":"cautious"}"#.parse().unwrap(),
                    updated_at_ms: chrono::Utc::now().timestamp_millis(),
                },
            },
        ).unwrap();
        let latest = store.application_port.load_conversation("creation-owner").unwrap().unwrap();
        assert!(latest.configuration_revision > initial);
        input.expected_configuration_revision = None;
        store.apply_initial_creation_boundary(&mut input).unwrap();
        assert_eq!(input.expected_configuration_revision, Some(initial));
        assert_ne!(input.expected_configuration_revision, Some(latest.configuration_revision));
        input.expected_configuration_revision = Some(77);
        input.expected_parent_message_id = Some("caller-parent".into());
        store.apply_initial_creation_boundary(&mut input).unwrap();
        assert_eq!(input.expected_configuration_revision, Some(77));
        assert_eq!(input.expected_parent_message_id.as_deref(), Some("caller-parent"));
        let duplicate: nola_application::ConversationCreation = store.application_port.execute(
            nola_application::ApplicationCommand::CreateConversation { input: nola_application::CreateConversation {
                id: "creation-owner".into(), title: "ignored".into(), settings_json: None, created_at_ms: 1, updated_at_ms: 1,
            } },
        ).unwrap();
        assert!(!duplicate.created);
        assert_eq!(duplicate.initial_configuration_revision, None);
        input.session_id = Some("foreign-owner".into());
        assert!(store.apply_initial_creation_boundary(&mut input).is_err());
    }

    #[test]
    fn reopening_an_existing_owner_cannot_supply_a_missing_caller_version() {
        let _environment = crate::store::test_env_lock();
        struct Restore(Option<std::ffi::OsString>);
        impl Drop for Restore {
            fn drop(&mut self) {
                match &self.0 { Some(value) => std::env::set_var("NOLA_DB_PATH", value), None => std::env::remove_var("NOLA_DB_PATH") }
            }
        }
        let _restore = Restore(std::env::var_os("NOLA_DB_PATH"));
        let root = tempfile::tempdir().unwrap();
        std::env::set_var("NOLA_DB_PATH", root.path().join("canonical.sqlite"));
        let created = super::SessionStore::open(Some("existing-owner"), false, "/fixture").unwrap();
        assert!(created.initial_configuration_revision.is_some());
        drop(created);
        let reopened = super::SessionStore::open(Some("existing-owner"), false, "/fixture").unwrap();
        assert!(reopened.initial_configuration_revision.is_none());
        let mut input = crate::host_invocation_request::HostInvocationRequest::for_test().unwrap().session;
        input.session_id = Some("existing-owner".into());
        input.expected_configuration_revision = None;
        input.expected_parent_message_id = None;
        reopened.apply_initial_creation_boundary(&mut input).unwrap();
        assert_eq!(input.expected_configuration_revision, None);
    }
    use super::{
        default_session_title, goal_runtime_is_due, redact_sensitive_text, validate_session_id,
        SessionStore,
    };
    use serde_json::json;
    use std::fs;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::store::test_env_lock()
    }

    fn test_root(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "nola-cli-session-{label}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn session_id_validation_blocks_paths() {
        assert!(validate_session_id("abc-123_ok").is_ok());
        assert!(validate_session_id("../secret").is_err());
        assert!(validate_session_id("a/b").is_err());
    }

    #[test]
    fn invalidated_display_drafts_obey_the_history_replay_budget() {
        let messages = vec![json!({"role":"assistant", "content":"",
        "display_text_parts":[
            {"text":"old rejected draft", "review":"rejected"},
            {"text":"latest candidate", "review":"pending"}
        ]})];
        let bounded = super::bound_interactive_replay_messages(messages, 5);
        assert_eq!(bounded[0]["display_text_parts"][0]["text"], "");
        assert_eq!(bounded[0]["display_text_parts"][1]["text"], "…date");
        assert_eq!(bounded[0]["display_text_parts"][1]["review"], "pending");
        assert_eq!(bounded[0]["content"], "");
    }

    #[test]
    fn deferred_metadata_is_not_an_approved_answer_or_provider_envelope() {
        let mut message = super::ConversationMessage {
            id: "assistant".into(),
            run_id: Some("run".into()),
            role: "assistant".into(),
            status: "completed".into(),
            ordinal: 1,
            created_at_ms: 1,
            completed_at_ms: Some(2),
            parts: vec![super::ConversationPart {
                id: "p".into(),
                kind: "text".into(),
                ordinal: 0,
                state: "completed".into(),
                content_text: "UNREVIEWED".into(),
                content_json: None,
                content_json_deferred: true,
                content_digest: None,
                created_at_ms: 1,
                updated_at_ms: 2,
            }],
        };
        let projected = super::canonical_message_value(&message).unwrap();
        assert_eq!(projected["content"], "");
        assert_eq!(
            projected["display_text_parts"][0]["metadata_deferred"],
            true
        );
        assert_eq!(projected["display_text_parts"][0]["text"], "UNREVIEWED");
        message.role = "user".into();
        message.parts[0].kind = "provider_message".into();
        message.parts[0].content_text =
            json!({"role":"assistant", "content":"FAKE ANSWER"}).to_string();
        let projected = super::canonical_message_value(&message).unwrap();
        assert_eq!(projected["role"], "user");
        assert_eq!(projected["content"], "");
        assert_eq!(projected["metadata_deferred"], true);
        assert_eq!(projected["projection_truncated"], true);
        // Actual absence is distinct from omitted metadata.
        message.parts[0].content_json_deferred = false;
        assert_eq!(
            super::canonical_message_value(&message).unwrap()["content"],
            "FAKE ANSWER"
        );
    }

    #[test]
    fn ui_action_history_displays_summary_without_exposing_model_envelope() {
        let mut message = super::ConversationMessage {
            id: "action-user".into(), run_id: Some("followup-run".into()), role: "user".into(),
            status: "completed".into(), ordinal: 1, created_at_ms: 1, completed_at_ms: Some(2),
            parts: vec![super::ConversationPart {
                id: "action-part".into(), kind: "ui_action".into(), ordinal: 0, state: "completed".into(),
                content_text: "Submitted form · submit".into(),
                content_json: Some(json!({"kind":"ui_action", "part_id":"action-part", "revision":1,
                    "surface":{"identity":{"owner":{
                        "authority_id":"host", "conversation_id":"conversation", "participant_id":"assistant",
                        "context_id":"conversation", "originating_run_id":"original-run", "message_id":"original-assistant", "part_id":"tool"
                    },"surface_id":"form","generation":1},"revision":2},
                    "summary":"Submitted form · submit",
                    "envelope":{"action":{"context":{"name":"PRIVATE_FORM_VALUE"}}}
                }).to_string()),
                content_json_deferred: false, content_digest: None, created_at_ms: 1, updated_at_ms: 2,
            }],
        };
        for deferred in [false, true] {
            message.parts[0].content_json_deferred = deferred;
            let displayed = super::canonical_message_value(&message).unwrap();
            assert_eq!(displayed["content"], "Submitted form · submit");
            assert_eq!(displayed["role"], "user");
            assert!(!displayed.to_string().contains("PRIVATE_FORM_VALUE"));
            assert!(message.parts[0].content_json.as_ref().unwrap().contains("PRIVATE_FORM_VALUE"));
        }
    }

    #[test]
    fn surface_history_summary_is_display_only_and_requires_exact_owner() {
        let envelope = json!({
            "kind":"ui_surface", "part_id":"ui-part", "revision":2,
            "surface":{"identity":{"owner":{
                "authority_id":"host", "conversation_id":"conversation", "participant_id":"assistant",
                "context_id":"conversation", "originating_run_id":"run", "message_id":"assistant", "part_id":"tool"
            },"surface_id":"form","generation":1},"revision":3},
            "summary":"Choose a destination"
        });
        let mut message = super::ConversationMessage {
            id: "assistant".into(), run_id: Some("run".into()), role: "assistant".into(),
            status: "completed".into(), ordinal: 1, created_at_ms: 1, completed_at_ms: Some(2),
            parts: vec![super::ConversationPart {
                id: "ui-part".into(), kind: "ui_surface".into(), ordinal: 0, state: "completed".into(),
                content_text: "Choose a destination".into(), content_json: Some(envelope.to_string()),
                content_json_deferred: false, content_digest: None, created_at_ms: 1, updated_at_ms: 2,
            }],
        };
        let value = super::canonical_message_value(&message).unwrap();
        assert_eq!(value["content"], "");
        assert_eq!(value["display_ui_surface_summaries"], json!(["Choose a destination"]));
        message.parts[0].content_json_deferred = true;
        assert!(super::canonical_message_value(&message).unwrap().get("display_ui_surface_summaries").is_none());
        message.parts[0].content_json_deferred = false;
        message.id = "other-message".into();
        assert!(super::canonical_message_value(&message).is_err());
        let bounded = super::bound_interactive_replay_messages(vec![json!({
            "content":"old text", "display_ui_surface_summaries":["destination"]
        })], 5);
        assert_eq!(bounded[0]["content"], "");
        assert_eq!(bounded[0]["display_ui_surface_summaries"], json!(["…tion"]));
        assert_eq!(bounded[0]["nola_history_truncated"], true);
    }

    #[test]
    fn reviewed_history_keeps_display_drafts_out_of_model_content() {
        let parts = ["rejected", "interrupted", "pending", "approved"]
            .iter()
            .enumerate()
            .map(|(index, state)| super::ConversationPart {
                id: format!("p{index}"),
                kind: "text".into(),
                ordinal: index as i64,
                state: "completed".into(),
                content_text: format!("{state} body"),
                content_json: Some(json!({"review": state}).to_string()),
                content_json_deferred: false,
                content_digest: None,
                created_at_ms: 1,
                updated_at_ms: 2,
            })
            .collect();
        let mut message = super::ConversationMessage {
            id: "assistant".into(),
            run_id: Some("run".into()),
            role: "assistant".into(),
            status: "completed".into(),
            ordinal: 1,
            created_at_ms: 1,
            completed_at_ms: Some(2),
            parts,
        };
        let value = super::canonical_message_value(&message).unwrap();
        assert_eq!(value["content"], "approved body");
        assert_eq!(value["display_text_parts"].as_array().unwrap().len(), 4);
        message.parts.pop();
        let rejected_only = super::canonical_message_value(&message).unwrap();
        assert_eq!(rejected_only["content"], "");
        assert_eq!(
            rejected_only["display_text_parts"][0]["text"],
            "rejected body"
        );
    }

    #[test]
    fn default_session_title_does_not_expose_canonical_identity() {
        let id = "01DAF2BB-1906-4993-9B5A-9AC74E05B5D8";
        assert_ne!(default_session_title(), id);
        assert_eq!(default_session_title(), "New Conversation");
    }

    #[test]
    fn goal_discovery_includes_unclaimed_work_without_stealing_live_owners() {
        use nola_core::agent::GoalStatus;

        assert!(goal_runtime_is_due(
            GoalStatus::Paused,
            Some(1_000),
            None,
            100,
            1_000,
        ));
        assert!(!goal_runtime_is_due(
            GoalStatus::Paused,
            Some(1_001),
            None,
            100,
            1_000,
        ));
        assert!(goal_runtime_is_due(
            GoalStatus::Running,
            None,
            Some(999),
            100,
            1_000,
        ));
        assert!(!goal_runtime_is_due(
            GoalStatus::Running,
            None,
            Some(1_001),
            100,
            1_000,
        ));
        assert!(goal_runtime_is_due(
            GoalStatus::Running,
            None,
            None,
            1_000,
            1_000,
        ));
        assert!(!goal_runtime_is_due(GoalStatus::Running, None, None, 1_001, 1_000));
        assert!(!goal_runtime_is_due(GoalStatus::Paused, None, None, 100, 1_000));
        for status in [GoalStatus::Completed, GoalStatus::Cancelled, GoalStatus::Failed] {
            assert!(!goal_runtime_is_due(status, Some(999), Some(999), 100, 1_000));
        }
        assert!(!goal_runtime_is_due(
            GoalStatus::NeedsUser,
            Some(1_000),
            Some(999),
            100,
            1_000,
        ));
    }

    #[test]
    fn session_redaction_preserves_code_placeholders() {
        let source = concat!(
            "SELECT * FROM sessions WHERE session_id = $1 AND token = ?2 ",
            "AND auth = :auth; api_key='secret-value-123';"
        );
        let redacted = redact_sensitive_text(source);

        assert!(redacted.contains("session_id = $1"));
        assert!(redacted.contains("token = ?2"));
        assert!(redacted.contains("auth = :auth"));
        assert!(!redacted.contains("secret-value-123"));
    }

    #[test]
    fn saves_and_continues_latest_session() {
        let _guard = env_lock();
        let root =
            std::env::temp_dir().join(format!("nola-cli-session-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));

        let mut store = SessionStore::open(Some("unit"), false, "").unwrap();
        store
            .save_messages(vec![json!({"role": "user", "content": "hello"})])
            .unwrap();

        let mut continued = SessionStore::open(None, true, "").unwrap();
        assert_eq!(continued.id, "unit");
        assert_eq!(continued.load_messages().unwrap()[0]["content"], "hello");

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn resume_loads_saved_messages() -> Result<(), String> {
        let _guard = env_lock();
        let root = test_root("resume-load");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));

        let mut store = SessionStore::open(Some("resume-1"), false, "")?;
        store.save_messages(vec![
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "hi there"}),
            json!({"role": "user", "content": "what is 1+1?"}),
            json!({"role": "assistant", "content": "2"}),
        ])?;

        // Resume via continue_last
        let mut resumed = SessionStore::open(None, true, "")?;
        assert_eq!(resumed.id, "resume-1");
        assert_eq!(
            resumed.load_messages().unwrap().len(),
            4,
            "should load all 4 messages"
        );
        assert_eq!(resumed.load_messages().unwrap()[0]["role"], "user");
        assert_eq!(resumed.load_messages().unwrap()[0]["content"], "hello");
        assert_eq!(resumed.load_messages().unwrap()[3]["role"], "assistant");
        assert_eq!(resumed.load_messages().unwrap()[3]["content"], "2");

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn resume_preserves_tool_call_and_result() -> Result<(), String> {
        let _guard = env_lock();
        let root = test_root("resume-tool");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));

        let mut store = SessionStore::open(Some("resume-2"), false, "")?;
        store.save_messages(vec![
            json!({"role": "user", "content": "read file"}),
            json!({
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": "read_file",
                        "arguments": "{\"path\":\"/tmp/test.txt\"}"
                    }
                }]
            }),
            json!({
                "role": "tool",
                "tool_call_id": "call_1",
                "name": "read_file",
                "content": "file contents here"
            }),
            json!({"role": "assistant", "content": "I read the file"}),
        ])?;

        let mut resumed = SessionStore::open(None, true, "")?;
        assert_eq!(
            resumed.load_messages().unwrap().len(),
            4,
            "should preserve all messages including tool calls"
        );
        assert_eq!(resumed.load_messages().unwrap()[1]["role"], "assistant");
        assert!(
            resumed.load_messages().unwrap()[1]["tool_calls"].is_array(),
            "tool_calls should be array"
        );
        assert_eq!(resumed.load_messages().unwrap()[2]["role"], "tool");
        assert_eq!(
            resumed.load_messages().unwrap()[2]["tool_call_id"],
            "call_1"
        );
        // content should be preserved (not sensitive for read_file)
        assert_eq!(
            resumed.load_messages().unwrap()[2]["content"],
            "file contents here"
        );

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn resume_uses_latest_session_id() -> Result<(), String> {
        let _guard = env_lock();
        let root = test_root("resume-latest");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));

        // Save session A
        let mut store_a = SessionStore::open(Some("session-a"), false, "")?;
        store_a.save_messages(vec![json!({"role": "user", "content": "msg A"})])?;

        // Save session B (later)
        let mut store_b = SessionStore::open(Some("session-b"), false, "")?;
        store_b.save_messages(vec![json!({"role": "user", "content": "msg B"})])?;

        // Resume should get the latest (session-b)
        let mut resumed = SessionStore::open(None, true, "")?;
        assert_eq!(
            resumed.id, "session-b",
            "should return the last saved session"
        );
        assert_eq!(resumed.load_messages().unwrap()[0]["content"], "msg B");

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn resume_ignores_explicit_session_id_when_continue_last() -> Result<(), String> {
        let _guard = env_lock();
        let root = test_root("resume-continue-ignore-id");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));

        let mut ignored = SessionStore::open(Some("ignored"), false, "")?;
        ignored.save_messages(vec![json!({"role": "user", "content": "first"})])?;

        let mut continued = SessionStore::open(Some("continued"), false, "")?;
        continued.save_messages(vec![json!({"role": "user", "content": "second"})])?;

        let mut resumed = SessionStore::open(Some("explicit"), true, "")?;
        assert_eq!(
            resumed.id, "continued",
            "continue_last should prefer the latest session even when explicit id is set"
        );
        assert_eq!(resumed.load_messages().unwrap()[0]["content"], "second");

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn resume_ignores_stale_latest_meta() -> Result<(), String> {
        let _guard = env_lock();
        let root = test_root("resume-stale-latest");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));

        // Create an older non-empty session.
        let mut keep_me = SessionStore::open(Some("keep-me"), false, "")?;
        keep_me.save_messages(vec![json!({"role": "user", "content": "keep this"})])?;

        // Create a newer empty session and force it into latest_session_id metadata.
        let mut empty_latest = SessionStore::open(Some("empty-latest"), false, "")?;
        empty_latest.save_messages(vec![])?;

        let mut resumed = SessionStore::open(None, true, "")?;
        assert_eq!(
            resumed.id, "keep-me",
            "stale latest pointer should fall back to non-empty session"
        );
        assert_eq!(resumed.load_messages().unwrap()[0]["content"], "keep this");

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn resume_loads_empty_when_no_sessions() -> Result<(), String> {
        let _guard = env_lock();
        let root = test_root("resume-empty");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));

        let mut resumed = SessionStore::open(None, true, "")?;
        assert!(
            resumed.load_messages().unwrap().is_empty(),
            "no sessions should give empty messages"
        );

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn resume_prefers_project_latest_session() -> Result<(), String> {
        let _guard = env_lock();
        let root = test_root("resume-project-latest");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));

        let mut project_a_old = SessionStore::open(Some("project-a-old"), false, "/project/a")?;
        project_a_old.save_messages(vec![json!({"role": "user", "content": "old a"})])?;

        let mut project_a_new = SessionStore::open(Some("project-a-new"), false, "/project/a")?;
        project_a_new.save_messages(vec![json!({"role": "user", "content": "new a"})])?;

        let mut project_b = SessionStore::open(Some("project-b"), false, "/project/b")?;
        project_b.save_messages(vec![json!({"role": "user", "content": "project b"})])?;

        let resumed = SessionStore::open(None, true, "/project/a")?;
        assert_eq!(resumed.id, "project-a-new");

        let mut resumed_missing_project = SessionStore::open(None, true, "/missing")?;
        assert_ne!(resumed_missing_project.id, "project-b");
        assert!(resumed_missing_project.load_messages().unwrap().is_empty());

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn resume_handles_multiple_saves_same_session() -> Result<(), String> {
        let _guard = env_lock();
        let root = test_root("resume-multi");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));

        let mut store = SessionStore::open(Some("resume-3"), false, "")?;
        store.save_messages(vec![
            json!({"role": "user", "content": "msg1"}),
            json!({"role": "assistant", "content": "reply1"}),
        ])?;

        // Overwrite with more messages
        store.save_messages(vec![
            json!({"role": "user", "content": "msg1"}),
            json!({"role": "assistant", "content": "reply1"}),
            json!({"role": "user", "content": "msg2"}),
            json!({"role": "assistant", "content": "reply2"}),
        ])?;

        let mut resumed = SessionStore::open(None, true, "")?;
        assert_eq!(
            resumed.load_messages().unwrap().len(),
            4,
            "should load the latest 4 messages"
        );
        assert_eq!(resumed.load_messages().unwrap()[0]["content"], "msg1");
        assert_eq!(resumed.load_messages().unwrap()[3]["content"], "reply2");

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn resume_with_system_prompt() -> Result<(), String> {
        let _guard = env_lock();
        let root = test_root("resume-system");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));

        let mut store = SessionStore::open(Some("resume-4"), false, "")?;
        store.save_messages(vec![
            json!({"role": "system", "content": "You are a helpful assistant"}),
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "hi"}),
        ])?;

        let mut resumed = SessionStore::open(None, true, "")?;
        assert_eq!(
            resumed.load_messages().unwrap().len(),
            3,
            "should preserve system prompt"
        );
        assert_eq!(resumed.load_messages().unwrap()[0]["role"], "system");
        assert_eq!(
            resumed.load_messages().unwrap()[0]["content"],
            "You are a helpful assistant"
        );

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn sanitize_preserves_non_sensitive_content() {
        use super::sanitize_saved_session;
        use super::SavedSession;

        let mut session = SavedSession {
            id: "test".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            messages: vec![
                json!({"role": "user", "content": "read the file at /tmp/test.txt"}),
                json!({"role": "assistant", "content": "The file contains: hello world"}),
            ],
        };

        sanitize_saved_session(&mut session);

        assert_eq!(
            session.messages[0]["content"],
            "read the file at /tmp/test.txt"
        );
        assert_eq!(
            session.messages[1]["content"],
            "The file contains: hello world"
        );
    }

    #[test]
    fn sanitize_clears_sensitive_fields() {
        use super::sanitize_saved_session;
        use super::SavedSession;

        let mut session = SavedSession {
            id: "test".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            messages: vec![
                json!({"role": "user", "content": "my api key is sk-12345abcde"}),
                json!({
                    "role": "tool",
                    "tool_call_id": "call_1",
                    "name": "bash",
                    "content": "result with token abc123secret"
                }),
            ],
        };

        sanitize_saved_session(&mut session);

        let user_content = session.messages[0]["content"].as_str().unwrap();
        assert!(
            !user_content.contains("sk-12345abcde"),
            "api key should be redacted, got: {}",
            user_content
        );

        let tool_content = session.messages[1]["content"].as_str().unwrap();
        assert!(
            !tool_content.contains("abc123secret"),
            "token should be redacted, got: {}",
            tool_content
        );
    }
}
