// SPDX-License-Identifier: Apache-2.0
//! Changed-input boundary for one canonical Application commit.
//!
//! These borrowed inputs are not journal records or ownership authority. The
//! production coordinator must bind them to the exact persisted owners after
//! the canonical writer applies, and publish the complete set in its same UoW.
//! No trace, cumulative conversation projection, or historical owner scan is
//! needed to discover this source commit's changes.

use std::collections::{BTreeMap, BTreeSet};

use nola_domain::{ModelAttemptMutation, ModelAttemptRecord, PartMutation};
use nola_agent_engine::agent::capability::{
    SkillRun, SkillRuntimeEvent, SkillRuntimeState,
};
use nola_agent_engine::agent::parts::{AgentPart, PartUpdate};
use nola_agent_engine::agent::protocol::AgentEvent;
use nola_domain::core_event::CoreEventEnvelope;
use nola_agent_engine::agent::task::TaskHandle;

#[derive(Clone, Copy, Debug)]
pub(crate) struct AssistantJournalChangeScope<'a> {
    pub(crate) conversation_id: &'a str,
    /// Core's session identity is not the canonical conversation identity.
    /// Supply it from the actual final typed checkpoint, never a default.
    pub(crate) core_session_id: &'a str,
    /// Idle source commits can change an older Task/Skill without an active Run.
    /// Its target Run must then come from the exact persisted Turn binding.
    pub(crate) source_run_id: Option<&'a str>,
    pub(crate) maintenance: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AttemptDisplayChange {
    Upsert,
    /// Includes an explicit failure clear; absence of a failure event is not a
    /// removal. The downstream planner may no-op if no card was ever declared.
    Remove,
}

#[derive(Debug)]
pub(crate) enum AssistantJournalChange<'a> {
    /// Empty slot ownership from admission, not a visible Text declaration.
    /// Validate its persisted owner without reserving presentation order.
    AdmissionText {
        part_id: &'a str,
    },
    Part {
        turn_id: &'a str,
        update: &'a PartUpdate,
        part: &'a AgentPart,
    },
    Task {
        projection_revision: u64,
        task: &'a TaskHandle,
    },
    Skill {
        projection_revision: u64,
        /// Only the exact ID named by this event is read from the final map.
        run: &'a SkillRun,
    },
    SyntheticClose {
        mutation: &'a PartMutation,
    },
    ModelAttempt {
        record: &'a ModelAttemptRecord,
        display: AttemptDisplayChange,
    },
}

#[derive(Debug)]
pub(crate) struct ChangedAttemptPredecessor<'a> {
    pub(crate) predecessor_id: &'a str,
    /// Gives the keyed predecessor/successor reader the immutable execution
    /// scope it must validate. Never resolve this dependency by latest Run.
    pub(crate) successor: &'a ModelAttemptRecord,
}

#[derive(Debug, Default)]
pub(crate) struct AssistantJournalChanges<'a> {
    /// Original Core event order, then canonical-only closes, then the final
    /// Attempt overlay in mutation first-touch order. Original mutations remain
    /// unchanged for the canonical writer's ordered revision-CAS application.
    pub(crate) changes: Vec<AssistantJournalChange<'a>>,
    /// A newly inserted successor also changes its predecessor's fallback
    /// display even when the predecessor has no mutation in this source commit.
    pub(crate) attempt_predecessors: Vec<ChangedAttemptPredecessor<'a>>,
}

pub(crate) fn collect_assistant_journal_changes<'a>(
    scope: AssistantJournalChangeScope<'_>,
    events: &'a [CoreEventEnvelope<AgentEvent>],
    final_skills: &'a SkillRuntimeState,
    merged_attempt_mutations: &'a [ModelAttemptMutation],
    canonical_part_mutations: &'a [PartMutation],
) -> Result<AssistantJournalChanges<'a>, String> {
    if scope.conversation_id.trim().is_empty()
        || scope.core_session_id.trim().is_empty()
        || scope.source_run_id.is_some_and(|run| run.trim().is_empty())
    {
        return Err("assistant changed-input source identity is empty".into());
    }
    let mut collected = AssistantJournalChanges::default();
    let mut final_parts = BTreeSet::new();
    for envelope in events {
        if envelope.session_id != scope.core_session_id {
            return Err("assistant changed-input event belongs to another session".into());
        }
        // Deliberately exhaustive: adding a Core event requires an explicit
        // assistant/non-assistant source classification here.
        match &envelope.event {
            AgentEvent::RuntimeDecisionFactObserved { turn_id, .. }
            | AgentEvent::RequirementScopeChanged { turn_id, .. }
            | AgentEvent::RequirementAssessed { turn_id, .. }
            | AgentEvent::RunResultDecided { turn_id, .. } => {
                validate_turn(envelope, turn_id.as_str())?;
                // These are canonical control/evidence facts, not assistant
                // Parts. initial_run_events / committed_event_payload_for_persistence
                // persist them with their Core source frontier. Store binds
                // requirements/results at that commit; final body and closes
                // still arrive via Part mutations below. Do not manufacture a
                // success card or seal from a decision event here.
            }
            AgentEvent::PartUpdated {
                turn_id,
                projection_revision: _,
                update,
                part,
            } => {
                if scope.maintenance || !is_assistant_part(part) {
                    continue;
                }
                validate_turn(envelope, turn_id.as_str())?;
                if part.turn_id() != turn_id.as_str()
                    || part.message_id().trim().is_empty()
                    || update.part_id() != part.id()
                    || part.id().trim().is_empty()
                {
                    return Err("assistant changed-input Part identity differs".into());
                }
                if establishes_final_metadata(update, part) {
                    final_parts.insert(part.id());
                }
                collected.changes.push(AssistantJournalChange::Part {
                    turn_id: turn_id.as_str(),
                    update,
                    part,
                });
            }
            AgentEvent::TaskUpdated {
                task_projection_revision,
                task,
            } => {
                if scope.maintenance {
                    continue;
                }
                validate_turn(envelope, task.owner_turn_id.as_str())?;
                if task.owner_session_id.as_str() != scope.core_session_id
                    || task.id.trim().is_empty()
                {
                    return Err("assistant changed-input Task identity differs".into());
                }
                collected.changes.push(AssistantJournalChange::Task {
                    projection_revision: *task_projection_revision,
                    task,
                });
            }
            AgentEvent::SkillRuntimeUpdated {
                turn_id,
                skill_runtime_revision,
                update,
            } => {
                let Some(run_id) = changed_skill_id(update) else {
                    // Catalog changes are session capability state, not a
                    // Skill activity card. No run map iteration is permitted.
                    continue;
                };
                if scope.maintenance {
                    continue;
                }
                let run = final_skills.runs.get(run_id).ok_or_else(|| {
                    "assistant changed-input Skill has no exact final record".to_string()
                })?;
                let declared_turn = turn_id.as_ref().ok_or_else(|| {
                    "assistant changed-input Skill has no declared owner Turn".to_string()
                })?;
                validate_turn(envelope, declared_turn.as_str())?;
                if run_id.trim().is_empty()
                    || run.run_id != run_id
                    || run.owner.session_id != scope.core_session_id
                    || run.owner.parent_turn_id != declared_turn.as_str()
                    || matches!(update, SkillRuntimeEvent::Started { run: started }
                        if started.owner != run.owner)
                {
                    return Err("assistant changed-input Skill identity differs".into());
                }
                collected.changes.push(AssistantJournalChange::Skill {
                    projection_revision: *skill_runtime_revision,
                    run,
                });
            }
            AgentEvent::SessionStarted { .. }
            | AgentEvent::TurnStarted { .. }
            | AgentEvent::TurnContextCaptured { .. }
            | AgentEvent::CapabilitiesActivated { .. }
            | AgentEvent::InputQueued { .. }
            | AgentEvent::InputApplied { .. }
            | AgentEvent::AssistantSegmentStarted { .. }
            | AgentEvent::AssistantEffectBound { .. }
            | AgentEvent::AssistantActivityBound { .. }
            | AgentEvent::InputAccepted { .. }
            | AgentEvent::InputWithdrawn { .. }
            | AgentEvent::UserInputWaitChanged { .. }
            | AgentEvent::PermissionSettled { .. }
            | AgentEvent::HostProgress { .. }
            | AgentEvent::LifecycleHookUpdated { .. }
            | AgentEvent::GoalUpdated { .. }
            | AgentEvent::MemoryUpdated { .. }
            | AgentEvent::EngineeringLoopUpdated { .. }
            | AgentEvent::ExecutionEvidenceObserved { .. }
            | AgentEvent::HostCatalogUpdated { .. }
            | AgentEvent::HostLeaseRenewed { .. }
            | AgentEvent::SessionConfigurationUpdated { .. }
            | AgentEvent::HostDisconnected { .. }
            | AgentEvent::ContextUpdated { .. }
            | AgentEvent::CompactionRequested { .. }
            | AgentEvent::CompactionCompleted { .. }
            | AgentEvent::CompactionFailed { .. }
            | AgentEvent::TurnCompleted { .. }
            | AgentEvent::TurnCancelled { .. }
            | AgentEvent::TurnFailed { .. } => {
                // Terminal status changes are not a new Part. Their concrete
                // canonical-only Close mutations are collected below.
            }
        }
    }
    if scope.maintenance {
        return Ok(collected);
    }
    for mutation in canonical_part_mutations {
        match mutation {
            PartMutation::Close { part_id, .. } => {
                if part_id.trim().is_empty() {
                    return Err("assistant synthetic Close has no Part identity".into());
                }
                if final_parts.insert(part_id.as_str()) {
                    collected
                        .changes
                        .push(AssistantJournalChange::SyntheticClose { mutation });
                }
            }
            PartMutation::Stream { .. }
            | PartMutation::Metadata { .. }
            | PartMutation::Upsert { .. }
            | PartMutation::AppendText { .. } => {}
        }
    }
    collect_attempt_overlay(scope, merged_attempt_mutations, &mut collected)?;
    Ok(collected)
}

fn validate_turn(envelope: &CoreEventEnvelope<AgentEvent>, turn_id: &str) -> Result<(), String> {
    if turn_id.trim().is_empty() || envelope.turn_id.as_deref() != Some(turn_id) {
        return Err("assistant changed-input envelope Turn identity differs".into());
    }
    Ok(())
}

fn is_assistant_part(part: &AgentPart) -> bool {
    match part {
        AgentPart::Text(_)
        | AgentPart::Reasoning(_)
        | AgentPart::Tool(_)
        | AgentPart::Source(_)
        | AgentPart::Citation(_)
        | AgentPart::Artifact(_)
        | AgentPart::Step(_) => true,
        AgentPart::Compaction(_) => false,
    }
}

fn changed_skill_id(event: &SkillRuntimeEvent) -> Option<&str> {
    match event {
        SkillRuntimeEvent::Started { run } => Some(&run.run_id),
        SkillRuntimeEvent::CapabilityRefreshRequested { run_id, .. }
        | SkillRuntimeEvent::Resumed { run_id, .. }
        | SkillRuntimeEvent::UsageUpdated { run_id, .. }
        | SkillRuntimeEvent::ProgressUpdated { run_id, .. }
        | SkillRuntimeEvent::ArtifactRegistered { run_id, .. }
        | SkillRuntimeEvent::VerificationRecorded { run_id, .. }
        | SkillRuntimeEvent::ResumePointUpdated { run_id, .. }
        | SkillRuntimeEvent::BudgetExceeded { run_id, .. }
        | SkillRuntimeEvent::Completed { run_id, .. }
        | SkillRuntimeEvent::Failed { run_id, .. }
        | SkillRuntimeEvent::Cancelled { run_id, .. } => Some(run_id),
        SkillRuntimeEvent::CatalogReplaced { .. } => None,
    }
}

fn establishes_final_metadata(update: &PartUpdate, part: &AgentPart) -> bool {
    let final_update = match update {
        PartUpdate::Created { .. } | PartUpdate::Completed { .. } => true,
        PartUpdate::TextReplaced { is_complete, .. }
        | PartUpdate::ReasoningReplaced { is_complete, .. } => *is_complete,
        PartUpdate::ToolState { state, .. } | PartUpdate::ToolInputReplaced { state, .. } => {
            state.is_terminal()
        }
        PartUpdate::TextReviewChanged { .. }
        | PartUpdate::TextDelta { .. }
        | PartUpdate::TextCommittedDelta { .. }
        | PartUpdate::ReasoningDelta { .. }
        | PartUpdate::ToolInputDelta { .. }
        | PartUpdate::ToolInputResolved { .. }
        | PartUpdate::ToolPermission { .. }
        | PartUpdate::ToolProgress { .. } => false,
    };
    final_update
        && match part {
            AgentPart::Text(text) => text.is_complete,
            AgentPart::Reasoning(reasoning) => reasoning.is_complete,
            AgentPart::Tool(tool) => tool.state.is_terminal(),
            AgentPart::Source(_)
            | AgentPart::Citation(_)
            | AgentPart::Artifact(_)
            | AgentPart::Step(_) => true,
            AgentPart::Compaction(_) => false,
        }
}

fn collect_attempt_overlay<'a>(
    scope: AssistantJournalChangeScope<'_>,
    mutations: &'a [ModelAttemptMutation],
    collected: &mut AssistantJournalChanges<'a>,
) -> Result<(), String> {
    let mut final_records = Vec::<&ModelAttemptRecord>::new();
    let mut index = BTreeMap::new();
    for mutation in mutations {
        let record = &mutation.record;
        if record.id.trim().is_empty()
            || record.run_id.trim().is_empty()
            || record.effect_id.trim().is_empty()
            || record.invocation_id.trim().is_empty()
            || record.text_part_id.trim().is_empty()
            || record.reasoning_part_id.trim().is_empty()
            || scope.source_run_id != Some(record.run_id.as_str())
            || record
                .predecessor_id
                .as_ref()
                .is_some_and(|id| id.trim().is_empty() || id == &record.id)
            || match mutation.expected_revision {
                None => record.revision != 1,
                Some(expected) => expected <= 0 || expected.checked_add(1) != Some(record.revision),
            }
        {
            return Err("assistant changed-input Attempt identity/revision differs".into());
        }
        if let Some(position) = index.get(record.id.as_str()).copied() {
            let previous: &ModelAttemptRecord = final_records[position];
            if mutation.expected_revision != Some(previous.revision)
                || previous.run_id != record.run_id
                || previous.effect_id != record.effect_id
                || previous.invocation_id != record.invocation_id
                || previous.text_part_id != record.text_part_id
                || previous.reasoning_part_id != record.reasoning_part_id
                || previous.predecessor_id != record.predecessor_id
                || previous.created_at_ms != record.created_at_ms
            {
                return Err("assistant changed-input Attempt overlay conflicts".into());
            }
            final_records[position] = record;
        } else {
            index.insert(record.id.as_str(), final_records.len());
            final_records.push(record);
        }
    }
    let mut predecessors = BTreeSet::new();
    for record in &final_records {
        let record = *record;
        if let Some(predecessor_id) = record.predecessor_id.as_deref() {
            if let Some(position) = index.get(predecessor_id).copied() {
                // The predecessor may have changed earlier in this same CAS
                // list. Read only that keyed final overlay entry.
                let predecessor = final_records[position];
                if predecessor.run_id != record.run_id
                    || predecessor.effect_id != record.effect_id
                    || predecessor.invocation_id != record.invocation_id
                {
                    return Err("assistant changed-input Attempt predecessor scope differs".into());
                }
            }
            if predecessors.insert(predecessor_id) {
                collected
                    .attempt_predecessors
                    .push(ChangedAttemptPredecessor {
                        predecessor_id,
                        successor: record,
                    });
            } else {
                return Err(
                    "assistant changed-input Attempt predecessor has multiple successors".into(),
                );
            }
        }
        collected
            .changes
            .push(AssistantJournalChange::ModelAttempt {
                record,
                display: if record.failure.is_some() {
                    AttemptDisplayChange::Upsert
                } else {
                    AttemptDisplayChange::Remove
                },
            });
    }
    Ok(())
}

#[cfg(test)]
#[path = "assistant_journal_changes_tests.rs"]
mod tests;
