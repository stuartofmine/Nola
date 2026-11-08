// SPDX-License-Identifier: Apache-2.0
//! Read-only, source-owned Today facts. This projection never grants or resumes work.
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use nola_domain::{ConversationExecutionOwnership, ConversationSourceOwner};
use nola_store_port::automation_facts::{AutomationFactsRepository, QueuedAutomationFact};
use nola_store_port::user_input_wait::{StoredUserInputWait, UserInputWaitRepository};
use crate::repositories::ConversationCommandRepositories;
use nola_store_port::room_presentation::{RoomPresentationRepository,RoomPresentationKey};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TodayFactsQuery {
    pub conversation_offset: usize,
    pub conversation_limit: usize,
    pub queued_limit: usize,
}
impl TodayFactsQuery {
    fn validate(&self) -> Result<(), String> {
        if !(1..=200).contains(&self.conversation_limit) || !(1..=50).contains(&self.queued_limit)
            || self.conversation_offset > i64::MAX as usize {
            return Err("invalid Today facts page bounds".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodayRunOwner { pub run_id: String, pub assistant_message_id: String }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodayOwnedTaskWait {
    pub task_id: String,
    pub parent_conversation_id: String,
    pub parent_run_id: String,
    pub child_conversation_id: String,
    pub child_run_id: String,
    pub admission_attempt_id: String,
    pub stored: StoredUserInputWait,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodayOwnedTaskPermission {
    pub task_id: String,
    pub parent_conversation_id: String,
    pub parent_run_id: String,
    pub child_conversation_id: String,
    pub child_run_id: String,
    pub admission_attempt_id: String,
    pub permission_projection: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodayExpertContextFact {
    pub room_id: String,
    pub room_revision: i64,
    pub association_revision: Option<i64>,
    pub canonical_conversation_id: Option<String>,
    pub participant_id: String,
    pub agent_id: nola_domain::persistent_agent::AgentId,
    pub agent_revision: i64,
    pub display_name: String,
    pub context_conversation_id: String,
    pub context_revision: i64,
    pub fact: TodayConversationFact,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodayConversationFact {
    pub conversation_id: String,
    pub title: String,
    pub revision: i64,
    pub updated_at_ms: i64,
    pub needs_response: bool,
    pub active_run: Option<TodayRunOwner>,
    /// Unmodified exact current-Run permission projection; null is no evidence.
    pub permission_projection: Option<Value>,
    pub open_waits: Vec<StoredUserInputWait>,
    pub owned_task_waits: Vec<TodayOwnedTaskWait>,
    pub owned_task_permissions: Vec<TodayOwnedTaskPermission>,
    pub expert_contexts: Vec<TodayExpertContextFact>,
}
impl TodayConversationFact {
    fn has_waiting_response(&self) -> bool {
        !self.open_waits.is_empty() || !self.owned_task_waits.is_empty() || !self.owned_task_permissions.is_empty() || self.expert_contexts.iter().any(|context|context.fact.needs_response)
            || self.permission_projection.as_ref().and_then(|p| p["permissions"]["pending"].as_array())
                .is_some_and(|pending| !pending.is_empty())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodayFactsSnapshot {
    pub conversations: Vec<TodayConversationFact>,
    pub recent_conversations: Vec<TodayConversationFact>,
    pub unrepresented_expert_contexts: Vec<TodayExpertContextFact>,
    pub needs_attention: bool,
    pub has_more_conversations: bool,
    pub next_conversation_offset: Option<usize>,
    pub queued_instances: Vec<QueuedAutomationFact>,
    pub has_more_queued_instances: bool,
}

struct RoomContextDestination {
    room_id: String,
    room_revision: i64,
    association_revision: Option<i64>,
    canonical_conversation_id: Option<String>,
    participant: nola_domain::agent_room::RoomParticipant,
}
fn room_context_destination<R>(store:&R, context:&str)->Result<Option<RoomContextDestination>,String>
where R: ConversationCommandRepositories + RoomPresentationRepository {
    let Some(owner)=store.read_room_context_owner(context).map_err(|e|e.to_string())? else {return Ok(None)};
    let record=store.read_room_presentation_setting(RoomPresentationKey::Room(&owner.room_id)).map_err(|e|e.to_string())?
        .ok_or("Today expert context Room is missing")?;
    let room=crate::agent_room::room_from_record(&owner.room_id,&record)?;
    // Read ownership includes inactive members' retained history. It does not
    // use recipient() to grant new execution to an inactive participant.
    let participant=room.room.participants().iter().find(|p|p.id==owner.participant_id)
        .ok_or("Today expert context participant is missing")?.clone();
    if participant.context_conversation_id!=context || participant.agent_id!=owner.agent_id {
        return Err("Today expert context owner differs from Room membership".into());
    }
    let mut canonical_conversation_id=None;
    let mut association_revision=None;
    if let Some(reverse)=store.read_room_presentation_setting(RoomPresentationKey::RoomAssociation(&owner.room_id)).map_err(|e|e.to_string())? {
        let parent=crate::agent_room::association_conversation_id(&reverse)?;
        let forward=store.read_room_presentation_setting(RoomPresentationKey::ConversationAssociation(&parent)).map_err(|e|e.to_string())?
            .ok_or("Today expert Room forward association is missing")?;
        let association=crate::agent_room::validated_association(&parent,&forward,&reverse,room)?;
        association_revision=Some(association.revision);
        if store.conversation_summary(&parent).map_err(|e|e.to_string())?.is_some()
            && store.conversation_source_owner(&parent).map_err(|e|e.to_string())?==ConversationSourceOwner::Local
            && store.conversation_execution_ownership(&parent).map_err(|e|e.to_string())?==ConversationExecutionOwnership::User
            && store.read_room_context_owner(&parent).map_err(|e|e.to_string())?.is_none() {
            canonical_conversation_id=Some(parent);
        }
    }
    Ok(Some(RoomContextDestination{room_id:owner.room_id,room_revision:record.revision,
        association_revision,canonical_conversation_id,participant}))
}

/// Every page is an independent committed read. Refresh from offset zero after
/// changes; clients must not combine old page witnesses into new authority.
pub fn load_today_facts<R>(store: &mut R, input: TodayFactsQuery) -> Result<TodayFactsSnapshot, String>
where R: ConversationCommandRepositories + UserInputWaitRepository + AutomationFactsRepository + RoomPresentationRepository {
    input.validate()?;
    store.with_consistent_read(|reader| read(reader, &input))
}
fn read<R>(store: &R, input: &TodayFactsQuery) -> Result<TodayFactsSnapshot, String>
where R: ConversationCommandRepositories + UserInputWaitRepository + AutomationFactsRepository + RoomPresentationRepository {
    let mut recoverable = BTreeMap::new();
    let mut after: Option<String> = None;
    loop {
        let page = store.list_recoverable_runs(after.as_deref(), 256).map_err(|e| e.to_string())?;
        let more = page.len() == 256;
        after = page.last().map(|run| run.run_id.clone());
        for run in page { recoverable.insert(run.run_id.clone(), run); }
        if !more { break; }
    }
    let mut child_permissions: BTreeMap<String, Vec<TodayOwnedTaskPermission>> = BTreeMap::new();
    for owner in recoverable.values() {
        if store.conversation_source_owner(&owner.conversation_id).map_err(|e| e.to_string())? != ConversationSourceOwner::Local { continue; }
        let ownership = store.conversation_execution_ownership(&owner.conversation_id).map_err(|e| e.to_string())?;
        let ConversationExecutionOwnership::TaskChild { parent_conversation_id, parent_run_id, task_id, child_conversation_id } = ownership
            else { continue; };
        let task = store.runtime_task(&task_id).map_err(|e| e.to_string())?.ok_or("Today permission task is missing")?;
        if task.cancel_requested || matches!(task.status.as_str(), "sealed"|"completed"|"failed"|"cancelled") { continue; }
        let binding = store.runtime_task_run_binding(&task_id).map_err(|e| e.to_string())?
            .ok_or("Today permission task binding is missing")?;
        if binding.task_id != task_id || binding.owner_session_id != parent_conversation_id
            || binding.child_session_id != child_conversation_id || child_conversation_id != owner.conversation_id
            || task.owner_session_id.as_deref() != Some(parent_conversation_id.as_str())
            || task.child_session_id.as_deref() != Some(child_conversation_id.as_str()) {
            return Err("Today child permission task owner differs".into());
        }
        // An older still-recoverable Run is not today's bound task attempt.
        if binding.run_id != owner.run_id { continue; }
        let Some(active) = store.active_run(&child_conversation_id).map_err(|e| e.to_string())? else { continue; };
        if active.run_id != owner.run_id { continue; }
        if active.assistant_message_id.as_deref() != Some(owner.assistant_message_id.as_str()) {
            return Err("Today child permission assistant owner differs".into());
        }
        let projection = crate::application_command::read_current_permission_projection(store, child_conversation_id.clone(), owner.run_id.clone(),
            crate::PermissionProjectionReadScope::CurrentRun)?;
        if projection["permissions"]["pending"].as_array().is_some_and(|pending| !pending.is_empty()) {
            child_permissions.entry(parent_conversation_id.clone()).or_default().push(TodayOwnedTaskPermission {
                task_id, parent_conversation_id, parent_run_id, child_conversation_id,
                child_run_id: binding.run_id, admission_attempt_id: binding.admission_attempt_id, permission_projection: projection });
        }
    }
    let mut conversations = Vec::new();
    let mut expert_contexts: BTreeMap<String,Vec<TodayExpertContextFact>> = BTreeMap::new();
    let mut unrepresented_expert_contexts = Vec::new();
    // Do not apply recent-history limits before discovering owned child waits:
    // a sealed parent with an old timestamp can still own a waiting child.
    for summary in store.list_conversations().map_err(|e| e.to_string())? {
        if store.conversation_source_owner(&summary.id).map_err(|e| e.to_string())? != ConversationSourceOwner::Local { continue; }
        let active = store.active_run(&summary.id).map_err(|e|e.to_string())?;
        if let Some(active) = &active {
            let inspection=store.inspect_exact_run(&active.run_id).map_err(|e|e.to_string())?.ok_or("Today active Run is missing")?;
            if inspection.conversation_id!=summary.id || inspection.kind!=active.kind || inspection.source_host_id.is_some() {
                return Err("Today active Run source or kind differs".into());
            }
            if active.kind=="maintenance" {
                if !inspection.checkpoint_present || inspection.completed_at_ms.is_some() || inspection.terminal.is_some()
                    || !matches!(inspection.status.as_str(),"queued"|"running"|"waiting_input")
                    || recoverable.values().any(|run|run.conversation_id==summary.id) {
                    return Err("Today maintenance Run state or ownership is inconsistent".into());
                }
            } else if !recoverable.contains_key(&active.run_id) {
                return Err("Today active conversation Run has no validated recoverable owner".into());
            }
        }
        let execution_owner=store.conversation_execution_ownership(&summary.id).map_err(|e|e.to_string())?;
        if let ConversationExecutionOwnership::TaskChild{parent_conversation_id,..}=&execution_owner {
            if active.is_some() && store.conversation_source_owner(parent_conversation_id).map_err(|e|e.to_string())?!=ConversationSourceOwner::Local {
                return Err("Today active task has no local presentation parent".into());
            }
        }
        if execution_owner!=ConversationExecutionOwnership::User { continue; }
        let room_owner = room_context_destination(store, &summary.id)?;
        let mut fact = TodayConversationFact { conversation_id: summary.id.clone(), title: summary.title,
            revision: summary.revision, updated_at_ms: summary.updated_at_ms, needs_response: false, active_run: None,
            permission_projection: None, open_waits: Vec::new(), owned_task_waits: Vec::new(),
            owned_task_permissions: child_permissions.remove(&summary.id).unwrap_or_default(), expert_contexts: Vec::new() };
        if let Some(active) = active {
            if active.kind != "maintenance" {
                let owner = recoverable.get(&active.run_id).ok_or("Today active conversation Run has no validated recoverable owner")?;
                if owner.conversation_id != summary.id || active.assistant_message_id.as_deref() != Some(owner.assistant_message_id.as_str()) {
                    return Err("Today active Run owner differs".into());
                }
                fact.active_run = Some(TodayRunOwner { run_id: owner.run_id.clone(), assistant_message_id: owner.assistant_message_id.clone() });
                let projection = crate::application_command::read_current_permission_projection(store, summary.id.clone(), owner.run_id.clone(),
                    crate::PermissionProjectionReadScope::CurrentRun)?;
                if !projection.is_null() { fact.permission_projection = Some(projection); }
                fact.open_waits = store.open_user_input_waits(&summary.id, Some(&owner.run_id)).map_err(|e| e.to_string())?;
                for wait in &fact.open_waits {
                    wait.wait.validate()?;
                    if wait.conversation_id != summary.id || wait.run_id != owner.run_id || !wait.wait.is_open() {
                        return Err("Today question owner differs".into());
                    }
                }
            }
        }
        for child in store.owned_task_user_input_waits(&summary.id, None).map_err(|e| e.to_string())? {
            if store.conversation_source_owner(&child.stored.conversation_id).map_err(|e| e.to_string())? != ConversationSourceOwner::Local { continue; }
            let binding = store.runtime_task_run_binding(&child.task_id).map_err(|e| e.to_string())?
                .ok_or("Today child question has no task Run binding")?;
            let ownership = store.conversation_execution_ownership(&child.stored.conversation_id).map_err(|e| e.to_string())?;
            let ConversationExecutionOwnership::TaskChild { parent_conversation_id, parent_run_id, task_id, child_conversation_id } = ownership
                else { return Err("Today child question has no task owner".into()); };
            child.stored.wait.validate()?;
            if parent_conversation_id != summary.id || task_id != child.task_id || binding.task_id != task_id
                || binding.owner_session_id != summary.id || binding.child_session_id != child_conversation_id
                || child_conversation_id != child.stored.conversation_id || binding.run_id != child.stored.run_id
                || !child.stored.wait.is_open() {
                return Err("Today child question binding differs".into());
            }
            fact.owned_task_waits.push(TodayOwnedTaskWait { task_id, parent_conversation_id, parent_run_id,
                child_conversation_id, child_run_id: binding.run_id, admission_attempt_id: binding.admission_attempt_id, stored: child.stored });
        }
        fact.needs_response = fact.has_waiting_response();
        if let Some(owner) = room_owner {
            let context = TodayExpertContextFact { room_id:owner.room_id,room_revision:owner.room_revision,
                association_revision:owner.association_revision,canonical_conversation_id:owner.canonical_conversation_id.clone(),
                participant_id:owner.participant.id,agent_id:owner.participant.agent_id,agent_revision:owner.participant.agent_revision,
                display_name:owner.participant.display_name,context_conversation_id:summary.id,context_revision:summary.revision,fact };
            if let Some(parent) = owner.canonical_conversation_id { expert_contexts.entry(parent).or_default().push(context); }
            else if context.fact.needs_response || context.fact.active_run.is_some() { unrepresented_expert_contexts.push(context); }
        } else { conversations.push(fact); }
    }
    if !child_permissions.is_empty() {
        return Err("Today child permission has no local presentation owner".into());
    }
    for fact in &mut conversations {
        fact.expert_contexts = expert_contexts.remove(&fact.conversation_id).unwrap_or_default();
        fact.needs_response = fact.has_waiting_response();
    }
    // A referenced outer owner not represented in this local ordinary snapshot
    // must remain visible as attention, not disappear with a private context.
    for (_, contexts) in expert_contexts {
        unrepresented_expert_contexts.extend(contexts.into_iter().filter(|context|context.fact.needs_response || context.fact.active_run.is_some()));
    }
    let mut recent_conversations = conversations.clone();
    recent_conversations.sort_by(|a,b| b.updated_at_ms.cmp(&a.updated_at_ms)
        .then_with(|| a.conversation_id.cmp(&b.conversation_id)));
    recent_conversations.truncate(5);
    conversations.sort_by(|a,b| b.needs_response.cmp(&a.needs_response)
        .then_with(|| b.updated_at_ms.cmp(&a.updated_at_ms)).then_with(|| a.conversation_id.cmp(&b.conversation_id)));
    let total = conversations.len();
    let conversations: Vec<_> = conversations.into_iter().skip(input.conversation_offset).take(input.conversation_limit).collect();
    let next = input.conversation_offset.saturating_add(conversations.len());
    let more = next < total;
    let mut queued = store.queued_automation_facts(input.queued_limit + 1).map_err(|e| e.to_string())?;
    let queued_more = queued.len() > input.queued_limit;
    queued.truncate(input.queued_limit);
    Ok(TodayFactsSnapshot { conversations, recent_conversations, needs_attention: !unrepresented_expert_contexts.is_empty(),
        unrepresented_expert_contexts, has_more_conversations: more, next_conversation_offset: more.then_some(next),
        queued_instances: queued, has_more_queued_instances: queued_more })
}

#[cfg(test)]
#[path = "today_facts_tests.rs"]
mod tests;
