// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Reduces host commands into deterministic state, events, and I/O effects.

use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

use super::capability::{
    CapabilityCatalog, CapabilityToolReference, MaterializedSkillInvocation, McpCapabilityCatalog,
    SkillExecutionMode, SkillRunBudget, SkillRunOwner, SkillRunStatus, SkillRuntimeCommand,
    SkillRuntimeCommandEnvelope, SkillRuntimeEvent,
};
use super::command::AgentCommand;
use super::compaction_planner::plan_compaction;
use super::context_window::{CompactionReason, UsageOrigin};
use super::effect::AgentEffect;
use super::host::{
    CapabilityAvailability, HostArtifact, HostCapabilityExposure, HostCapabilityRoute,
    HostEffectId, HostReplaySafety, HostRequest, HostRequestEnvelope, HostResponse,
    HostResponseEnvelope, ModelDeltaChannel,
};
use super::parts::{
    AgentPart, ArtifactReference, CompactionPart, PartUpdate, ReasoningPart, TextPart, ToolPart,
    ToolPartState, ToolPermissionPart,
};
use super::permissions::{PermissionPolicyDecision as RuleDecision, PermissionUserDecision};
use super::protocol::{
    AgentCoreConfig, AgentCoreSnapshot, AgentDurabilityIntent, AgentEvent, AgentInputDelivery,
    AgentItem, AgentItemDeltaKind, AgentReduction, AgentRole, AgentSubmission, AgentTransition,
    CallId, ItemId, ModelRequest, PermissionRequestId, SessionId, TurnId,
};
use super::recovery::{CheckpointReason, CoreCheckpoint, HostEffectAcceptance, PendingHostEffect, PendingHostEffectState, AGENT_CHECKPOINT_SCHEMA_VERSION};
use nola_domain::core_event::{CoreEventEnvelope, EventCursor};
pub(super) use super::runtime_event::{PermissionResolutionKind, RuntimeEvent};
use super::session::{AgentItemOwnership, AgentSessionConfiguration, AgentThreadState};
use super::task::TaskHandle;
use super::tool_lifecycle::ToolCallStatus;
use super::tool_scheduler::ToolExecutionRuntime;
use super::turn::{AgentTurnContext, AgentTurnState, AgentTurnStatus};

/// Internal deterministic reducer. Public clients use `AgentCore`.
#[derive(Debug)]
pub(crate) struct AgentRuntime {
    config: AgentCoreConfig,
    session: AgentThreadState,
    /// Observation time supplied at the submission boundary, never read by the reducer.
    observed_at_ms: u64,
}

mod bootstrap;
mod compaction_driver;
mod effect_dispatch;
mod host_response;
mod model_planner;
mod goal_work;
pub(crate) mod requirements;
mod task_scope;
mod task_scope_coverage;
pub(crate) mod execution_constraints;
mod projection;

#[cfg(test)]
mod heap_ownership_tests;
mod submission;
mod tool_driver;
mod delegation_workspace;
mod user_delegation;
mod code_mode;
mod permission_policy;
mod user_input_wait;
mod turn_lifecycle;

impl AgentRuntime {
    /// Immediate read only. The caller supplies its authenticated Host, never
    /// a model-provided owner. Waiting and cancellation use separate dispatch.
    pub fn code_cell_snapshot(&self, authenticated_host: &str, cell_id: &str, cursor: u64)
        -> Result<super::code_mode::CodeCellView, String>
    {
        let cell = self.session.execution.code_cell(cell_id).ok_or("unknown code cell")?;
        let request = nola_domain::code_mode::CodeCellWaitRequest {
            cell_id: cell_id.to_owned(), after_output_sequence: cursor,
            wait_ms: 0, terminate: false,
        };
        let observation = request.bind(cell.identity(), authenticated_host, self.session.session_id.as_str())?;
        cell.view(&observation.cell, observation.after_output_sequence)
    }

    pub(crate) fn host_execution_state(&self) -> super::HostExecutionState {
        super::HostExecutionState::from_state(&self.session, self.control_snapshot())
    }

    pub(crate) fn checkpoint_view(
        &self,
        reason: CheckpointReason,
    ) -> CoreCheckpoint<std::borrow::Cow<'_, AgentThreadState>> {
        CoreCheckpoint {
            schema_version: AGENT_CHECKPOINT_SCHEMA_VERSION,
            session_id: self.session.session_id.as_str().to_string(),
            revision: self.session.journal.revision,
            last_event_cursor: EventCursor {
                sequence: self.session.journal.next_event_sequence.saturating_sub(1),
            },
            capability_catalog_revision: self.session.host.catalog.revision,
            created_at_ms: 0,
            reason,
            state: self.session.checkpoint_projection_view(),
        }
    }

    pub(crate) fn into_checkpoint(
        self,
        reason: CheckpointReason,
    ) -> CoreCheckpoint<AgentThreadState> {
        CoreCheckpoint {
            schema_version: AGENT_CHECKPOINT_SCHEMA_VERSION,
            session_id: self.session.session_id.as_str().to_string(),
            revision: self.session.journal.revision,
            last_event_cursor: EventCursor {
                sequence: self.session.journal.next_event_sequence.saturating_sub(1),
            },
            capability_catalog_revision: self.session.host.catalog.revision,
            created_at_ms: 0,
            reason,
            state: self.session.into_checkpoint_projection(),
        }
    }

    pub(crate) fn checkpoint(&self, reason: CheckpointReason) -> CoreCheckpoint<AgentThreadState> {
        CoreCheckpoint {
            schema_version: AGENT_CHECKPOINT_SCHEMA_VERSION,
            session_id: self.session.session_id.as_str().to_string(),
            revision: self.session.journal.revision,
            last_event_cursor: EventCursor {
                sequence: self.session.journal.next_event_sequence.saturating_sub(1),
            },
            capability_catalog_revision: self.session.host.catalog.revision,
            created_at_ms: 0,
            reason,
            state: self.session.checkpoint_projection(),
        }
    }

    pub(crate) fn branch_checkpoint_from_checkpoint(
        checkpoint: &CoreCheckpoint<AgentThreadState>,
        new_session_id: &str,
        through_message_id: &str,
    ) -> Result<CoreCheckpoint<AgentThreadState>, String> {
        checkpoint.validate_state_identity()?;
        let through_message_id = through_message_id.trim();
        let through_item_id = checkpoint
            .state
            .conversation
            .transcript
            .iter()
            .rev()
            .find(|item| {
                checkpoint
                    .state
                    .item_ownership
                    .get(item.id().as_str())
                    .is_some_and(|ownership| ownership.message_id == through_message_id)
            })
            .map(|item| item.id().as_str().to_string())
            .ok_or_else(|| format!("unknown Core message identity {through_message_id}"))?;
        let runtime = Self {
            config: AgentCoreConfig {
                session_id: Some(checkpoint.state.session_id.clone()),
                permission_rules: checkpoint.state.execution.permission_rules.clone(),
                ..AgentCoreConfig::default()
            },
            session: checkpoint.state.clone(),
            observed_at_ms: 0,
        };
        runtime.branch_checkpoint(new_session_id, &through_item_id)
    }

    pub(crate) fn branch_checkpoint(
        &self,
        new_session_id: &str,
        through_item_id: &str,
    ) -> Result<CoreCheckpoint<AgentThreadState>, String> {
        let new_session_id = new_session_id.trim();
        let through_item_id = through_item_id.trim();
        if new_session_id.is_empty() || through_item_id.is_empty() {
            return Err("branch session id and boundary item id must be non-empty".to_string());
        }
        if new_session_id == self.session.session_id.as_str() {
            return Err("branch session id must differ from its parent".to_string());
        }
        if self
            .session
            .execution
            .active_turn
            .as_ref()
            .is_some_and(|turn| !turn.status.is_terminal())
        {
            return Err("conversation can only branch while the parent thread is idle".to_string());
        }
        if self.session.host.pending_effects.len() > 0 {
            return Err("conversation cannot branch while Host effects are pending".to_string());
        }
        if self.session.execution.input_queue.len() > 0 {
            return Err("conversation cannot branch while user input is queued".to_string());
        }

        let boundary_index = self
            .session
            .conversation
            .transcript
            .iter()
            .position(|item| item.id().as_str() == through_item_id)
            .ok_or_else(|| format!("unknown branch boundary item {through_item_id}"))?;
        let boundary_ownership = self
            .session
            .item_ownership
            .get(through_item_id)
            .ok_or_else(|| {
                format!("branch boundary item {through_item_id} has no Core ownership")
            })?;
        let later_same_turn = self.session.conversation.transcript[boundary_index + 1..]
            .iter()
            .any(|item| {
                self.session
                    .item_ownership
                    .get(item.id().as_str())
                    .is_some_and(|ownership| ownership.turn_id == boundary_ownership.turn_id)
            });
        if later_same_turn {
            return Err(
                "branch boundary must be the final durable item of a completed Turn".to_string(),
            );
        }

        let retained_items = self.session.conversation.transcript[..=boundary_index].to_vec();
        let retained_item_ids = retained_items
            .iter()
            .map(|item| item.id().as_str().to_string())
            .collect::<HashSet<_>>();
        let mut retained_ownership = self
            .session
            .item_ownership
            .iter()
            .filter(|(item_id, _)| retained_item_ids.contains(*item_id))
            .map(|(item_id, ownership)| (item_id.clone(), ownership.clone()))
            .collect::<std::collections::BTreeMap<_, _>>();
        // Checkpoints created before synthetic compaction Items received
        // explicit ownership may legitimately omit only those Items. Repair
        // them deterministically; every other missing durable identity remains
        // a hard error.
        for item in &retained_items {
            let item_id = item.id().as_str().to_string();
            if !retained_ownership.contains_key(&item_id)
                && matches!(item, AgentItem::Compaction { .. })
            {
                retained_ownership.insert(
                    item_id.clone(),
                    super::session::AgentItemOwnership {
                        source: Default::default(),
                        turn_id: format!("{}:maintenance", self.session.session_id.as_str()),
                        message_id: item_id,
                    },
                );
            }
        }
        if retained_ownership.len() != retained_items.len() {
            return Err("branch source contains durable items without Core ownership".to_string());
        }
        let retained_turn_ids = retained_ownership
            .values()
            .map(|ownership| ownership.turn_id.clone())
            .collect::<HashSet<_>>();
        if self
            .session
            .conversation
            .projection
            .parts
            .values()
            .any(|part| retained_turn_ids.contains(part.turn_id()) && !part.is_terminal())
        {
            return Err("branch source contains a non-terminal projected Part".to_string());
        }

        let mut projection = self.session.conversation.projection.clone();
        projection
            .parts
            .retain(|_, part| retained_turn_ids.contains(part.turn_id()));
        projection
            .order
            .retain(|part_id| projection.parts.contains_key(part_id));
        projection.revision = 0;

        let session_id = SessionId::from_string(new_session_id.to_string());
        let mut branch =
            AgentThreadState::new(session_id.clone(), self.session.configuration.clone());
        let active_compaction_is_retained = self
            .session
            .conversation
            .model_context
            .iter()
            .find_map(|item| match item {
                AgentItem::Compaction { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .is_some_and(|item_id| retained_item_ids.contains(item_id));
        let branch_model_context = if active_compaction_is_retained {
            // Preserve Core's exact active compacted view. This keeps the
            // summary and retained tool-result budget without reintroducing
            // the raw prefix it replaced.
            self.session
                .conversation
                .model_context
                .iter()
                .filter(|item| retained_item_ids.contains(item.id().as_str()))
                .cloned()
                .collect::<Vec<_>>()
        } else {
            // A later compaction may summarize content beyond an older branch
            // boundary. Rebuild from raw durable history and exclude all
            // synthetic summaries so the same content is never represented
            // twice.
            retained_items
                .iter()
                .filter(|item| !matches!(item, AgentItem::Compaction { .. }))
                .filter(|item| retained_ownership.get(item.id().as_str())
                    .is_some_and(|owner| owner.source.is_model_visible()))
                .cloned()
                .collect::<Vec<_>>()
        };
        branch.conversation.transcript = retained_items.clone();
        branch.conversation.model_context = branch_model_context;
        branch.conversation.projection = projection;
        branch.conversation.provider_continuation = branch
            .conversation
            .model_context
            .iter()
            .rev()
            .find_map(|item| match item {
                AgentItem::Reasoning {
                    provider_continuation: Some(continuation),
                    ..
                } => Some(continuation.clone()),
                _ => None,
            });
        branch
            .conversation
            .context_window
            .rebase_estimate(&branch.conversation.model_context);
        branch.item_ownership = retained_ownership;
        branch.lineage = Some(super::session::AgentSessionLineage {
            parent_session_id: self.session.session_id.as_str().to_string(),
            through_item_id: through_item_id.to_string(),
            parent_revision: self.session.journal.revision,
            parent_event_cursor: self.session.journal.next_event_sequence.saturating_sub(1),
        });
        branch.execution.permission_rules = self.config.permission_rules.clone();
        branch.host.catalog = self.session.host.catalog.clone();

        Ok(CoreCheckpoint {
            schema_version: AGENT_CHECKPOINT_SCHEMA_VERSION,
            session_id: session_id.as_str().to_string(),
            revision: 0,
            last_event_cursor: EventCursor::default(),
            capability_catalog_revision: branch.host.catalog.revision,
            created_at_ms: 0,
            reason: CheckpointReason::Manual,
            state: branch,
        })
    }

    pub fn memory_worker_plan(
        &self,
        job_id: &str,
    ) -> Result<super::memory_worker::MemoryWorkerPlan, String> {
        super::memory_worker::plan_memory_worker_from_state(&self.session, job_id)
    }
}

#[cfg(test)]
mod checkpoint_metadata_tests {
    use super::*;
    use crate::agent::host::{HostId, ModelResponseMode};
    use crate::agent::parts::PartOwnership;
    use crate::context_engine::{ContextComponent, ContextComponentKind};

    #[test]
    fn persisted_trace_uses_the_admitted_clock_during_replay() {
        let mut original = recovery_state_fixture(AgentTurnStatus::AwaitingModel, false);
        let mut replay = recovery_state_fixture(AgentTurnStatus::AwaitingModel, false);
        let reduce = |runtime: &mut AgentRuntime| {
            runtime
                .finish_transition(
                    vec![RuntimeEvent::TurnStarted {
                        session_id: runtime.session.session_id.clone(),
                        turn_id: TurnId::from_string("checkpoint-turn".into()),
                        assistant_message_id: "checkpoint-turn:assistant".into(),
                    }],
                    vec![],
                    None,
                )
                .unwrap()
        };
        let original_events = reduce(&mut original);
        let replay_events = reduce(&mut replay);
        assert!(!original_events.trace_events.is_empty());
        assert!(original_events
            .trace_events
            .iter()
            .all(|event| event.emitted_at == "1970-01-01T00:02:03.456Z"));
        assert_eq!(
            serde_json::to_value(original_events.trace_events).unwrap(),
            serde_json::to_value(replay_events.trace_events).unwrap()
        );
        assert_eq!(
            serde_json::to_value(original.checkpoint(CheckpointReason::Manual)).unwrap(),
            serde_json::to_value(replay.checkpoint(CheckpointReason::Manual)).unwrap()
        );
    }

    fn snapshot_checkpoint_reference(
        runtime: &AgentRuntime,
        reason: CheckpointReason,
    ) -> CoreCheckpoint<AgentThreadState> {
        let snapshot = runtime.snapshot();
        CoreCheckpoint {
            schema_version: AGENT_CHECKPOINT_SCHEMA_VERSION,
            session_id: snapshot.session_id.as_str().to_string(),
            revision: snapshot.revision,
            last_event_cursor: snapshot.last_event_cursor,
            capability_catalog_revision: snapshot.host_catalog_revision,
            created_at_ms: 0,
            reason,
            state: runtime.session.checkpoint_projection(),
        }
    }

    fn recovery_state_fixture(status: AgentTurnStatus, has_pending_effect: bool) -> AgentRuntime {
        let mut runtime = AgentRuntime::try_new(AgentCoreConfig {
            turn_identity: crate::agent::model_capability::test_host_identity(),
            session_id: Some(SessionId::from_string("checkpoint-owner".to_string())),
            ..AgentCoreConfig::default()
        })
        .expect("create checkpoint fixture");
        runtime.session.journal.revision = 73;
        runtime.session.journal.next_event_sequence = 42;
        runtime.session.host.catalog.revision = 97;
        runtime.observed_at_ms = 123_456;
        runtime.session.configuration.tools.push(json!({
            "type": "function",
            "function": {"name": "read_file", "description": "frozen tool catalog"}
        }));
        let body = "恢复正文🙂\\path\n".repeat(512);
        let mut turn = AgentTurnState::new(
            TurnId::from_string("checkpoint-turn".to_string()),
            AgentTurnContext {
                input_origin: Default::default(),
                configuration: runtime.session.configuration.clone(),
                platform: Default::default(),
                components: vec![ContextComponent {
                    kind: ContextComponentKind::Skill,
                    id: "frozen-skill".to_string(),
                    content: json!("frozen skill catalog"),
                    source: None,
                }],
                component_metrics: Vec::new(),
                component_payload_digest: String::new(),
                items: vec![AgentItem::user_message(
                    ItemId::from_string("frozen-context".to_string()),
                    &body,
                )],
                host_id: runtime.session.host.catalog.host_id.0.clone(),
                host_catalog_revision: 97,
                host_availability_epoch: runtime.session.host.catalog.availability_epoch,
                capability_projection: Default::default(),
                runtime_snapshot: Default::default(),
            },
        );
        turn.status = status;
        runtime.session.execution.active_turn = Some(Box::new(turn));
        runtime.session.record_item(AgentItem::user_message(
            ItemId::from_string("user-message".to_string()),
            &body,
        ));
        runtime
            .session
            .conversation
            .projection
            .apply(PartUpdate::Created {
                part: AgentPart::Text(TextPart {
                    id: "text-1".to_string(),
                    ownership: PartOwnership::for_turn("checkpoint-turn"),
                    text: body,
                    is_complete: status.is_terminal(),
                    review: None,
                }),
            })
            .expect("create projected Text");
        if has_pending_effect {
            let request = HostRequestEnvelope {
                attempt_started: false,
                attempt_id: None,
                delegated_invocation_id: None,
                request_id: "request-1".to_string(),
                effect_id: HostEffectId("effect-1".to_string()),
                host_id: HostId(runtime.session.host.catalog.host_id.0.clone()),
                lease_id: None,
                session_id: "checkpoint-owner".to_string(),
                turn_id: Some("checkpoint-turn".to_string()),
                catalog_revision: 97,
                availability_epoch: runtime.session.host.catalog.availability_epoch,
                created_at_ms: 1,
                deadline_ms: None,
                batch: None,
                resume_after_sequence: Some(7),
                delivery: None,
                payload: HostRequest::ModelStream {
                    model_id: "model".to_string(),
                    text_part_id: "text-1".to_string(),
                    reasoning_part_id: "reasoning-1".to_string(),
                    response_mode: ModelResponseMode::Conversational,
                    messages: vec![serde_json::to_value(
                        &runtime.session.conversation.model_context[0],
                    )
                    .expect("encode frozen model input")],
                    tools: runtime.session.configuration.tools.clone(),
                    settings: json!({}),
                },
            };
            assert!(runtime
                .session
                .host
                .pending_effects
                .register(PendingHostEffect {
                    control: Default::default(),
                    effect_id: request.effect_id.0.clone(),
                    request_id: request.request_id.clone(),
                    host_id: request.host_id.0.clone(),
                    lease_id: request.lease_id.clone(),
                    session_id: request.session_id.clone(),
                    turn_id: request.turn_id.clone(),
                    catalog_revision: request.catalog_revision,
                    availability_epoch: request.availability_epoch,
                    capability_id: "model.stream".to_string(),
                    call_id: None,
                    task_id: None,
                    replay_safety: HostReplaySafety::Recoverable,
                    state: PendingHostEffectState::Streaming,
                    target_effect_id: None,
                    cancel_effect_id: None,
                    last_response_sequence: Some(7),
                    request: Some(serde_json::to_value(request).expect("encode pending request")),
                }));
        }
        runtime
    }

    #[test]
    fn checkpoint_metadata_matches_snapshot_reference_for_all_recovery_states() {
        for (status, has_pending_effect) in [
            (AgentTurnStatus::AwaitingModel, false),
            (AgentTurnStatus::Compacting, false),
            (AgentTurnStatus::Completed, false),
            (AgentTurnStatus::Cancelled, false),
            (AgentTurnStatus::Failed, false),
            (AgentTurnStatus::Completed, true),
        ] {
            let runtime = recovery_state_fixture(status, has_pending_effect);
            let original = serde_json::to_vec(&runtime.session).expect("encode live session");
            for reason in [
                CheckpointReason::SessionStarted,
                CheckpointReason::ConfigurationChanged,
                CheckpointReason::TurnCompleted,
                CheckpointReason::CompactionCompleted,
                CheckpointReason::PermissionResolved,
                CheckpointReason::PermissionTransportInterrupted,
                CheckpointReason::DelegatedExecutionInterrupted,
                CheckpointReason::CancellationProgress,
                CheckpointReason::TaskUpdated,
                CheckpointReason::GoalUpdated,
                CheckpointReason::MemoryUpdated,
                CheckpointReason::SkillUpdated,
                CheckpointReason::HostDisconnected,
                CheckpointReason::ApplicationSuspended,
                CheckpointReason::SessionClosing,
                CheckpointReason::SessionClosed,
                CheckpointReason::Manual,
            ] {
                let actual = runtime.checkpoint(reason);
                let expected = snapshot_checkpoint_reference(&runtime, reason);
                assert_eq!(
                    serde_json::to_vec(&actual).expect("encode checkpoint"),
                    serde_json::to_vec(&expected).expect("encode reference"),
                    "checkpoint bytes changed for {status:?}, pending={has_pending_effect}, reason={reason:?}"
                );
                assert_eq!(actual.schema_version, AGENT_CHECKPOINT_SCHEMA_VERSION);
                assert_eq!(actual.session_id, "checkpoint-owner");
                assert_eq!(actual.revision, 73);
                assert_eq!(actual.last_event_cursor.sequence, 41);
                assert_eq!(actual.capability_catalog_revision, 97);
                assert_eq!(actual.created_at_ms, 0);
                assert_eq!(actual.reason, reason);
                let turn = actual.state.execution.active_turn.as_ref().unwrap();
                assert_eq!(turn.status, status);
                let retain_catalog = !status.is_terminal() || has_pending_effect;
                assert_eq!(!actual.state.configuration.tools.is_empty(), retain_catalog);
                assert_eq!(!turn.context.components.is_empty(), retain_catalog);
                assert_eq!(
                    actual.state.host.pending_effects.len(),
                    usize::from(has_pending_effect)
                );
                assert_eq!(
                    serde_json::to_vec(&runtime.session).expect("encode live session"),
                    original,
                    "checkpoint must not mutate the live session"
                );
            }
        }
    }

    #[test]
    fn resume_request_decode_preserves_frozen_payload_and_live_cursor() {
        let runtime = recovery_state_fixture(AgentTurnStatus::AwaitingModel, true);
        let original = serde_json::to_vec(&runtime.session).unwrap();
        let request = runtime
            .committed_host_request_resume_snapshot("effect-1")
            .unwrap()
            .unwrap();
        assert_eq!(request.request_id, "request-1");
        assert_eq!(request.resume_after_sequence, Some(7));
        let stored = runtime
            .session
            .host
            .pending_effects
            .get("effect-1")
            .unwrap()
            .request
            .as_ref()
            .unwrap();
        assert_eq!(serde_json::to_value(&request).unwrap(), *stored);
        assert!(runtime
            .committed_host_request_resume_snapshot("missing")
            .unwrap()
            .is_none());
        assert_eq!(serde_json::to_vec(&runtime.session).unwrap(), original);
    }

    #[test]
    fn checkpoint_metadata_preserves_zero_and_saturated_event_cursors() {
        let mut runtime = AgentRuntime::try_new(AgentCoreConfig {
            turn_identity: crate::agent::model_capability::test_host_identity(),
            session_id: Some(SessionId::from_string("idle-checkpoint".to_string())),
            ..AgentCoreConfig::default()
        })
        .expect("create idle runtime");
        for (next_sequence, expected_cursor) in [(0, 0), (1, 0), (42, 41), (u64::MAX, u64::MAX - 1)]
        {
            runtime.session.journal.next_event_sequence = next_sequence;
            runtime.session.journal.revision = u64::MAX;
            runtime.session.host.catalog.revision = u64::MAX;
            let actual = runtime.checkpoint(CheckpointReason::Manual);
            let expected = snapshot_checkpoint_reference(&runtime, CheckpointReason::Manual);
            assert_eq!(
                serde_json::to_vec(&actual).expect("encode checkpoint"),
                serde_json::to_vec(&expected).expect("encode reference")
            );
            assert_eq!(actual.last_event_cursor.sequence, expected_cursor);
            assert_eq!(actual.revision, u64::MAX);
            assert_eq!(actual.capability_catalog_revision, u64::MAX);
            assert_eq!(actual.session_id, "idle-checkpoint");
            assert_eq!(actual.created_at_ms, 0);
            assert!(actual.state.execution.active_turn.is_none());
        }
    }
}

#[cfg(test)]
mod branch_compaction_tests {
    use super::*;
    use crate::agent::protocol::ItemId;
    use crate::agent::session::AgentItemOwnership;

    fn owned(turn: &str, message: &str) -> AgentItemOwnership {
        AgentItemOwnership {
            source: Default::default(),
            turn_id: turn.to_string(),
            message_id: message.to_string(),
        }
    }

    #[test]
    fn rebase_preserves_code_transcript_without_promoting_intermediate_context() {
        let mut runtime = AgentRuntime::try_new(AgentCoreConfig {
            turn_identity: crate::agent::model_capability::test_host_identity(),
            session_id: Some(SessionId::from_string("session".into())),
            ..AgentCoreConfig::default()
        }).unwrap();
        let call = AgentItem::ToolCall {
            id: ItemId::from_string("nested-call".into()),
            call_id: crate::agent::protocol::CallId::from_string("nested".into()),
            name: "read_file".into(), arguments: serde_json::json!({"path":"a"}),
        };
        let result = AgentItem::ToolResult {
            id: ItemId::from_string("nested-result".into()),
            call_id: crate::agent::protocol::CallId::from_string("nested".into()),
            name: "read_file".into(), output: serde_json::json!({"large":"intermediate"}),
            is_error: false, full_output_ref: None, artifacts: vec![],
        };
        let explicit = AgentItem::user_message(ItemId::from_string("user".into()), "continue");
        let mut nested_owner = owned("turn", "assistant");
        nested_owner.source = crate::agent::session::AgentItemSource::CodeCell {
            cell: nola_domain::code_mode::CodeCellIdentity {
                host_id:"host".into(), conversation_id:"session".into(), turn_id:"turn".into(),
                parent_call_id:"parent".into(), cell_id:"cell".into(), generation:"g".into(),
            },
        };
        let ownership = BTreeMap::from([
            ("nested-call".into(), nested_owner.clone()),
            ("nested-result".into(), nested_owner),
            ("user".into(), owned("next", "user")),
        ]);
        let transcript = vec![call, result, explicit.clone()];
        runtime.rebase_conversation(transcript.clone(), ownership).unwrap();
        assert_eq!(runtime.session.conversation.transcript, transcript);
        assert_eq!(runtime.session.conversation.model_context, vec![explicit]);
        assert!(runtime.session.execution.active_turn.is_none());
    }

    #[test]
    fn branch_preserves_safe_active_compaction_and_rebuilds_older_boundaries() {
        let mut runtime = AgentRuntime::try_new(AgentCoreConfig {
            turn_identity: crate::agent::model_capability::test_host_identity(),
            session_id: Some(SessionId::from_string("parent".to_string())),
            ..AgentCoreConfig::default()
        })
        .expect("create runtime");
        let old =
            AgentItem::user_message(ItemId::from_string("old".to_string()), "old raw context");
        let kept = AgentItem::user_message(ItemId::from_string("kept".to_string()), "kept context");
        let compacted = AgentItem::Compaction {
            id: ItemId::from_string("compact".to_string()),
            summary: "summary of old raw context".to_string(),
            replaced_item_count: 1,
            source_item_ids: vec![old.id().clone()],
            source_fingerprint: "fixture".to_string(),
            original_token_count: old.estimated_tokens(),
        };
        let later =
            AgentItem::user_message(ItemId::from_string("later".to_string()), "later context");
        runtime.session.conversation.transcript =
            vec![old.clone(), kept.clone(), compacted.clone(), later.clone()];
        runtime.session.conversation.model_context =
            vec![compacted.clone(), kept.clone(), later.clone()];
        runtime
            .session
            .item_ownership
            .insert("old".to_string(), owned("turn-old", "message-old"));
        runtime
            .session
            .item_ownership
            .insert("kept".to_string(), owned("turn-kept", "message-kept"));
        runtime
            .session
            .item_ownership
            .insert("later".to_string(), owned("turn-later", "message-later"));

        let after_compaction = runtime
            .branch_checkpoint("after-compaction", "later")
            .expect("branch through compacted context");
        assert_eq!(
            after_compaction.state.conversation.model_context,
            vec![compacted.clone(), kept.clone(), later]
        );
        assert!(after_compaction
            .state
            .item_ownership
            .contains_key("compact"));

        let before_compaction = runtime
            .branch_checkpoint("before-compaction", "kept")
            .expect("branch before the active compaction summary");
        assert_eq!(
            before_compaction.state.conversation.model_context,
            vec![old, kept]
        );
        assert!(before_compaction
            .state
            .conversation
            .model_context
            .iter()
            .all(|item| !matches!(item, AgentItem::Compaction { .. })));
    }
}
