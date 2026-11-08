// SPDX-License-Identifier: Apache-2.0
//! Production, transaction-local changed-item publication. Canonical writes,
//! order reservations and this complete publication set share the caller's UoW.
//! No historical projection scan, latest-owner guess or checkpoint copy.

use std::collections::{BTreeMap, BTreeSet};

use nola_domain::assistant_journal::AssistantJournalIdentity;
use nola_domain::assistant_journal_operations::{JournalOrderKey, JournalSection};
use nola_domain::{ModelAttemptRecord, PartMutation};
use nola_store_port::assistant_journal::{
    AssistantJournalRepository, AssistantPublicationCommit, AssistantSourceCommitStamp,
};
use nola_store_port::assistant_ownership::{AssistantJournalPlanningRepository, AssistantOwner};
use nola_store_port::run_effect::RunEffectRepository;

use crate::assistant_journal_changes::{
    AssistantJournalChange, AssistantJournalChanges, AttemptDisplayChange,
};
use crate::assistant_journal_planner::AssistantJournalPlanner;
use crate::assistant_journal_publication::{
    publish_assistant_publication_set, StoreJournalPlanningReader,
};
use crate::assistant_synthetic_semantics::{model_attempt_part, skill_part, task_part};
use crate::canonical_part_projection::CanonicalPartProjection;

pub(crate) fn publication_cursors(
    publication: Option<AssistantPublicationCommit>,
) -> Vec<nola_domain::assistant_journal::AssistantJournalCursor> {
    match publication {
        Some(
            AssistantPublicationCommit::Applied(cursors)
            | AssistantPublicationCommit::Replay(cursors),
        ) => cursors,
        None => Vec::new(),
    }
}

enum Input<'a, 'event> {
    Changed(&'a AssistantJournalChange<'event>),
    Attempt(&'a ModelAttemptRecord, &'a AttemptDisplayChange),
}

struct Resolved<'a, 'event> {
    input: Input<'a, 'event>,
    order: JournalOrderKey,
    successor: Option<ModelAttemptRecord>,
}

/// Call after all canonical mutations have applied, before committing the UoW.
/// A failure is a command failure, never an optional display-update failure.
pub(crate) fn publish_changed_assistant_journal<S>(
    store: &mut S,
    stamp: AssistantSourceCommitStamp,
    conversation_id: &str,
    core_session_id: &str,
    source_run_id: Option<&str>,
    changes: &AssistantJournalChanges<'_>,
) -> Result<Option<AssistantPublicationCommit>, String>
where
    S: nola_store_port::remote_session::CanonicalRunTerminalRepository
        + nola_store_port::runtime_task::TaskAuthorityReadRepository
        + AssistantJournalRepository
        + AssistantJournalPlanningRepository
        + RunEffectRepository
        + ?Sized,
{
    if changes.changes.is_empty() && changes.attempt_predecessors.is_empty() {
        return Ok(None);
    }
    if conversation_id.trim().is_empty() || core_session_id.trim().is_empty() {
        return Err("assistant publication requires canonical and Core session identities".into());
    }
    let epoch = store.assistant_source_epoch().map_err(|e| e.to_string())?;
    // A successor insertion changes its predecessor's fallback card even when
    // that predecessor has no mutation. Read only those exact dependencies.
    let explicit_attempts: BTreeSet<_> = changes
        .changes
        .iter()
        .filter_map(|change| match change {
            AssistantJournalChange::ModelAttempt { record, .. } => Some(record.id.as_str()),
            _ => None,
        })
        .collect();
    let mut predecessors = BTreeMap::new();
    for dependency in &changes.attempt_predecessors {
        let predecessor = store
            .model_attempt(dependency.predecessor_id)
            .map_err(|e| e.to_string())?
            .ok_or("assistant Attempt predecessor is missing")?;
        validate_lineage(&predecessor, dependency.successor)?;
        if !explicit_attempts.contains(dependency.predecessor_id) {
            predecessors.insert(predecessor.id.clone(), predecessor);
        }
    }
    // Group by immutable owner, preserving changed-event order within an owner.
    // Reservations precede readers so planners hold only immutable Store borrows.
    let mut groups: BTreeMap<(String, String), (AssistantJournalIdentity, Vec<Resolved<'_, '_>>)> =
        BTreeMap::new();
    let inputs = changes
        .changes
        .iter()
        .map(|change| match change {
            AssistantJournalChange::ModelAttempt { record, display } => {
                Input::Attempt(record, display)
            }
            _ => Input::Changed(change),
        })
        .chain(predecessors.values().map(|record| {
            Input::Attempt(
                record,
                if record.failure.is_some() {
                    &AttemptDisplayChange::Upsert
                } else {
                    &AttemptDisplayChange::Remove
                },
            )
        }));
    for input in inputs {
        let (owner, part_id, section, successor) = match &input {
            Input::Changed(AssistantJournalChange::AdmissionText { part_id }) => {
                let run = source_run_id.ok_or("assistant admission has no exact source Run")?;
                let bound = store
                    .assistant_owner_for_part(conversation_id, run, part_id)
                    .map_err(|e| e.to_string())?
                    .ok_or("assistant admission has no persisted Part owner")?;
                (
                    bound.owner,
                    (*part_id).to_owned(),
                    JournalSection::Transcript,
                    None,
                )
            }
            Input::Changed(AssistantJournalChange::Part { turn_id, part, .. }) => {
                let turn_owner = turn_owner(store, conversation_id, turn_id)?;
                let bound = store
                    .assistant_owner_for_part(conversation_id, &turn_owner.run_id, part.id())
                    .map_err(|e| e.to_string())?
                    .ok_or("changed assistant Part has no persisted owner")?;
                if bound.owner.run_id != turn_owner.run_id || bound.owner.conversation_id != conversation_id || bound.turn_id.as_deref() != Some(*turn_id) {
                    return Err("changed assistant Part and Turn owners differ".into());
                }
                (
                    bound.owner,
                    part.id().to_owned(),
                    JournalSection::Transcript,
                    None,
                )
            }
            Input::Changed(AssistantJournalChange::Task { task, .. }) => {
                let owner = activity_owner(store, conversation_id, task.owner_turn_id.as_str(), "task", &task.id)?;
                // Reconcile the durable Task even after its parent stopped, but
                // keep that parent's sealed chat journal an immutable snapshot.
                if source_run_id != Some(owner.run_id.as_str())
                    && store.read_canonical_run_terminal(&owner.conversation_id, &owner.run_id)
                    .map_err(|error| error.to_string())?.is_some() { continue; }
                (owner, format!("task:{}", task.id), JournalSection::Tasks, None)
            },
            Input::Changed(AssistantJournalChange::Skill { run, .. }) => (
                activity_owner(store, conversation_id, &run.owner.parent_turn_id, "skill", &run.run_id)?,
                format!("skill:{}", run.run_id),
                JournalSection::Skills,
                None,
            ),
            Input::Changed(AssistantJournalChange::SyntheticClose { mutation }) => {
                let PartMutation::Close { part_id, .. } = mutation else {
                    return Err("synthetic journal close is not a Close mutation".into());
                };
                let run = source_run_id.ok_or("synthetic journal close has no exact source Run")?;
                let bound = store
                    .assistant_owner_for_part(conversation_id, run, part_id)
                    .map_err(|e| e.to_string())?
                    .ok_or("synthetic close has no persisted assistant owner")?;
                (
                    bound.owner,
                    part_id.clone(),
                    JournalSection::Transcript,
                    None,
                )
            }
            Input::Attempt(record, display) => {
                let Some(owner) = attempt_owner(store, conversation_id, core_session_id, record)?
                else {
                    // Prepared attempts may have no materialized Part yet and
                    // no failure card. Never invent an owner for a visible card.
                    if **display == AttemptDisplayChange::Remove {
                        continue;
                    }
                    return Err("visible Attempt has no exact persisted assistant owner".into());
                };
                let successor = store
                    .model_attempt_successor(
                        &record.run_id,
                        &record.effect_id,
                        &record.invocation_id,
                        &record.id,
                    )
                    .map_err(|e| e.to_string())?;
                if let Some(next) = &successor {
                    validate_lineage(record, next)?;
                }
                (
                    owner,
                    format!("model-attempt:{}", record.id),
                    JournalSection::ModelAttempts,
                    successor,
                )
            }
            Input::Changed(AssistantJournalChange::ModelAttempt { .. }) => unreachable!(),
        };
        if owner.conversation_id != conversation_id {
            return Err("assistant publication owner belongs to another conversation".into());
        }
        let identity = AssistantJournalIdentity {
            source_store_epoch: epoch.clone(),
            conversation_id: owner.conversation_id,
            run_id: owner.run_id,
            assistant_message_id: owner.assistant_message_id,
        };
        identity.validate().map_err(|e| e.to_string())?;
        // Admission owns an empty canonical slot, not a visible transcript
        // item. Reserving its display position here puts a later answer before
        // reasoning which actually arrived first. The first Core Created event
        // materializes that slot in the same chronological stream as all parts.
        if matches!(input, Input::Changed(AssistantJournalChange::AdmissionText { .. })) {
            continue;
        }
        let order = store
            .reserve_assistant_journal_order(&stamp, &identity, section, &part_id)
            .map_err(|e| e.to_string())?;
        groups
            .entry((
                identity.run_id.clone(),
                identity.assistant_message_id.clone(),
            ))
            .or_insert_with(|| (identity, Vec::new()))
            .1
            .push(Resolved {
                input,
                order,
                successor,
            });
    }
    let reader = StoreJournalPlanningReader(&*store);
    let mut plans = Vec::with_capacity(groups.len());
    for (_, (identity, inputs)) in groups {
        let expected = store
            .assistant_journal_head(&identity)
            .map_err(|e| e.to_string())?;
        let mut planner =
            AssistantJournalPlanner::new(&reader, &identity).map_err(|e| e.to_string())?;
        for Resolved {
            input,
            order,
            successor,
        } in inputs
        {
            let result = match input {
                Input::Changed(AssistantJournalChange::AdmissionText { part_id }) => {
                    planner.begin_text(part_id, order)
                }
                Input::Changed(AssistantJournalChange::Part { update, part, .. }) => {
                    let view = CanonicalPartProjection::new(
                        part,
                        &identity.run_id,
                        Some(&identity.assistant_message_id),
                    )?;
                    planner.part_update_view(update, &view, order)
                }
                Input::Changed(AssistantJournalChange::Task {
                    task,
                    projection_revision,
                    ..
                }) => {
                    let mut part = task_part(task, *projection_revision);
                    part.supports_guidance = crate::task_guidance::available(store, &task.id,
                        task.owner_session_id.as_str(), task.owner_turn_id.as_str())?;
                    // A later parent guidance event may still carry its older
                    // queued Task checkpoint. Keep the bound nonterminal state
                    // tied to durable execution; terminal publication remains
                    // exclusively the exact terminal reconciliation's event.
                    if !task.state.is_terminal() && store.read_runtime_task_run_binding(&task.id)
                        .map_err(|e|e.to_string())?.is_some() {
                        if let Some(stored)=store.read_runtime_task(&task.id).map_err(|e|e.to_string())? {
                            if !matches!(stored.status.as_str(),"sealed"|"failed"|"cancelled") {
                                let admission=crate::task_ownership::authority(store,&stored)?;
                                part.state=task_part(&crate::task_ownership::core_task(&stored,&admission)?,0).state;
                            }
                        }
                    }
                    planner.task_update(&part, order)
                },
                Input::Changed(AssistantJournalChange::Skill {
                    run,
                    projection_revision,
                    ..
                }) => planner.skill_update(&skill_part(run, *projection_revision), order),
                Input::Changed(AssistantJournalChange::SyntheticClose { mutation }) => {
                    let PartMutation::Close { part_id, state, .. } = mutation else {
                        unreachable!()
                    };
                    planner.close_admitted_part(part_id, state, order)
                }
                Input::Attempt(record, display) => match display {
                    AttemptDisplayChange::Remove => planner.remove_model_attempt(&record.id),
                    AttemptDisplayChange::Upsert => {
                        let part = model_attempt_part(record, successor.as_ref())?
                            .ok_or("Attempt upsert has no public failure")?;
                        planner.model_attempt_update(&part, order)
                    }
                },
                Input::Changed(AssistantJournalChange::ModelAttempt { .. }) => unreachable!(),
            };
            result.map_err(|e| format!("plan committed assistant change: {e}"))?;
        }
        plans.push((expected, planner.finish().map_err(|e| e.to_string())?));
    }
    publish_assistant_publication_set(store, stamp, plans)
}

/// The child binding is the source fact; no parent checkpoint or model work is
/// required merely to enable a Task control already present in its journal.
pub(crate) fn publish_task_run_binding<S: crate::repositories::AssistantPublicationRepositories + ?Sized>(
    store: &mut S, task_id: &str, child_run_id: &str,
) -> Result<Vec<nola_domain::assistant_journal::AssistantJournalCursor>, String> {
    let task=store.read_runtime_task(task_id).map_err(|e|e.to_string())?.ok_or("bound Task missing")?;
    let admission=crate::task_ownership::authority(store,&task)?;
    let source_store_epoch=store.assistant_source_epoch().map_err(|e|e.to_string())?;
    // A trusted receiver can own child execution without owning the source
    // conversation's journal. Its binding is durable here; only the source
    // Store may publish changes to the parent's immutable activity owner.
    if admission.source_store_epoch != source_store_epoch {
        return Ok(Vec::new());
    }
    let owner=activity_owner(store,&admission.parent_conversation_id,&admission.parent_turn_id,"task",&task.id)?;
    if owner.run_id!=admission.parent_run_id { return Err("bound Task parent Run differs".into()) }
    let identity=AssistantJournalIdentity {
        source_store_epoch,
        conversation_id:owner.conversation_id,run_id:owner.run_id,assistant_message_id:owner.assistant_message_id,
    };
    let available=crate::task_guidance::available(store,task_id,&admission.parent_core_session_id,&admission.parent_turn_id)?;
    let expected=store.assistant_journal_head(&identity).map_err(|e|e.to_string())?;
    let reader=StoreJournalPlanningReader(&*store);
    let mut planner=AssistantJournalPlanner::new(&reader,&identity).map_err(|e|e.to_string())?;
    let core_task=crate::task_ownership::core_task(&task,&admission)?;
    let state=task_part(&core_task,0).state;
    planner.task_binding_update(task_id,state,available).map_err(|e|e.to_string())?;
    let plan=planner.finish().map_err(|e|e.to_string())?;
    let result=publish_assistant_publication_set(store,AssistantSourceCommitStamp::TaskRunBinding {
        task_id:task_id.into(),child_run_id:child_run_id.into(),
    },vec![(expected,plan)])?;
    Ok(publication_cursors(result))
}

fn activity_owner<S: AssistantJournalPlanningRepository + ?Sized>(store: &S, conversation: &str, turn: &str, kind: &str, id: &str) -> Result<AssistantOwner,String> {
    let root=turn_owner(store,conversation,turn)?;
    store.assistant_owner_for_activity(conversation,&root.run_id,kind,id).map_err(|e|e.to_string())?
        .ok_or_else(||"Activity lacks exact Core birth ownership".into())
}

fn turn_owner<S: AssistantJournalPlanningRepository + ?Sized>(
    store: &S,
    conversation: &str,
    turn: &str,
) -> Result<AssistantOwner, String> {
    store
        .assistant_owner_for_core_turn(conversation, turn)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("changed assistant Turn {turn} has no persisted owner"))
}

fn validate_lineage(parent: &ModelAttemptRecord, child: &ModelAttemptRecord) -> Result<(), String> {
    if parent.id == child.id
        || child.predecessor_id.as_deref() != Some(parent.id.as_str())
        || parent.run_id != child.run_id
        || parent.effect_id != child.effect_id
        || parent.invocation_id != child.invocation_id
    {
        return Err("assistant Attempt dependency belongs to another execution".into());
    }
    Ok(())
}

fn attempt_owner<S: AssistantJournalPlanningRepository + RunEffectRepository + ?Sized>(
    store: &S,
    conversation: &str,
    core_session: &str,
    attempt: &ModelAttemptRecord,
) -> Result<Option<AssistantOwner>, String> {
    let mut current = std::borrow::Cow::Borrowed(attempt);
    let mut visited = BTreeSet::new();
    loop {
        if !visited.insert(current.id.clone()) {
            return Err("assistant Attempt ownership lineage is cyclic".into());
        }
        let parent = current
            .predecessor_id
            .as_deref()
            .map(|id| {
                store
                    .model_attempt(id)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "assistant Attempt ownership predecessor is missing".to_string())
            })
            .transpose()?;
        if let Some(parent) = &parent {
            validate_lineage(parent, &current)?;
        }
        let text = store
            .assistant_owner_for_part(conversation, &current.run_id, &current.text_part_id)
            .map_err(|e| e.to_string())?;
        let reasoning = store
            .assistant_owner_for_part(conversation, &current.run_id, &current.reasoning_part_id)
            .map_err(|e| e.to_string())?;
        if let (Some(text), Some(reasoning)) = (&text, &reasoning) {
            if text.owner != reasoning.owner {
                return Err("assistant Attempt text and reasoning owners differ".into());
            }
        }
        if let Some(bound) = text.or(reasoning) {
            return Ok(Some(bound.owner));
        }
        let Some(parent) = parent else {
            return attempt_effect_owner(store, conversation, core_session, attempt);
        };
        current = std::borrow::Cow::Owned(parent);
    }
}

/// Read only immutable execution identity from the exact outbox payload. Model
/// history/settings are deliberately ignored, not deserialized into another
/// cumulative in-memory request. Compaction attempts have no transcript Parts.
fn attempt_effect_owner<S: AssistantJournalPlanningRepository + RunEffectRepository + ?Sized>(
    store: &S,
    conversation: &str,
    core_session: &str,
    attempt: &ModelAttemptRecord,
) -> Result<Option<AssistantOwner>, String> {
    let Some(effect) = store
        .outbox_effect_identity(&attempt.effect_id)
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let envelope = &effect.binding;
    // prepare_effects starts invocation_id at the effect ID, and fallback
    // attempts preserve it. Outbox attempt_id may already name its successor.
    if !matches!(effect.effect_kind.as_str(), "model_stream" | "compact")
        || effect.run_id != attempt.run_id
        || effect.effect_id != attempt.effect_id
        || envelope.effect_id.0 != effect.effect_id
        || envelope.request_id != effect.request_id
        || envelope.session_id != core_session
        || attempt.invocation_id != effect.effect_id
    {
        return Err("Attempt outbox execution identity differs".into());
    }
    let attached_id = envelope
        .attempt_id
        .as_deref()
        .ok_or("Attempt outbox has no ledger binding")?;
    let attached = store
        .model_attempt(attached_id)
        .map_err(|e| e.to_string())?
        .ok_or("Attempt outbox ledger binding is missing")?;
    if attached.run_id != attempt.run_id
        || attached.effect_id != attempt.effect_id
        || attached.invocation_id != attempt.invocation_id
    {
        return Err("Attempt outbox ledger binding belongs to another execution".into());
    }
    let turn = envelope
        .turn_id
        .as_deref()
        .ok_or("Attempt outbox has no Core Turn")?;
    let owner = store.assistant_owner_for_effect(conversation, &attempt.run_id, &attempt.effect_id)
        .map_err(|e| e.to_string())?;
    if owner.is_none() { return Err("Attempt lacks its immutable effect presentation owner".into()); }
    let _ = turn; // Turn was checked on the exact outbox above; never select a latest message.
    Ok(owner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant_journal_changes::{
        collect_assistant_journal_changes, AssistantJournalChangeScope,
    };
    use nola_domain::assistant_journal_operations::{JournalNumber, JournalPartMetadata};
    use nola_agent_engine::agent::capability::SkillRuntimeState;
    use nola_agent_engine::agent::parts::{AgentPart, PartOwnership, PartUpdate, TextPart};
    use nola_agent_engine::agent::protocol::{AgentEvent, TurnId};
    use nola_domain::core_event::{CoreEventEnvelope, EventCursor};
    use nola_store::{BeginTurnInput, NolaStore};

    fn begin(store: &mut NolaStore) {
        store.begin_turn(&begin_input()).unwrap();
    }

    fn begin_input() -> BeginTurnInput {
        let core = nola_agent_engine::agent::AgentCore::try_new(nola_agent_engine::agent::AgentCoreConfig {
            session_id: Some(nola_agent_engine::agent::SessionId::from_string("actual-core-session".into())),
            turn_identity: crate::model_fixture::identity(), ..Default::default()
        }).unwrap();
        let mut checkpoint = core.checkpoint(nola_agent_engine::agent::recovery::CheckpointReason::Manual);
        checkpoint.revision = 1;
        checkpoint.last_event_cursor.sequence = 2;
        BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            initial_events: vec![nola_store_port::run_creation::InitialCoreRunEvent {
                core_session_id:"actual-core-session".into(), core_turn_id:Some("run".into()),
                core_cursor:1, core_revision:1, event_kind:"assistant_activity_bound".into(),
                payload_json:serde_json::json!({"type":"assistant_activity_bound","turn_id":"run",
                    "assistant_message_id":"assistant","activity_id":"task","activity_kind":"task"}).to_string(),
            }],
            conversation_id: "conversation".into(),
            conversation_title: "producer".into(),
            user_message_id: "user".into(),
            user_part_id: "user-text".into(),
            user_text: "continue".into(),
            user_content_json: None,
            additional_user_parts: vec![],
            assistant_message_id: "assistant".into(),
            assistant_part_id: "text".into(),
            initial_assistant_part_mutations: vec![],
            assistant_turn_bindings: vec![
                nola_domain::AssistantTurnBinding::Activity {turn_id:"run".into(),
                    assistant_message_id:"assistant".into(),activity_id:"task".into(),activity_kind:"task".into()},
                nola_domain::AssistantTurnBinding::Admission {
                    turn_id: "run".into(),
                    assistant_message_id: "assistant".into(),
                },
                nola_domain::AssistantTurnBinding::Part {
                    turn_id: "run".into(),
                    assistant_message_id: "assistant".into(),
                    part_id: "text".into(),
                },
            ],
            run_id: "run".into(),
            context_checkpoint_id: None,
            checkpoint_revision: 1,
            checkpoint_event_cursor: 2,
            checkpoint_json: serde_json::to_string(&checkpoint).unwrap(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: "fixture".into(),
            outbox_effects: vec![],
            model_attempt_mutations: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            memory_usage_entries: vec![],
            created_at_ms: 1,
        }
    }

    fn events() -> Vec<CoreEventEnvelope<AgentEvent>> {
        let part = AgentPart::Text(TextPart {
            id: "text".into(),
            ownership: PartOwnership::for_turn("run"),
            text: String::new(),
            review: None,
            is_complete: false,
        });
        vec![CoreEventEnvelope {
            session_id: "actual-core-session".into(), turn_id: Some("run".into()),
            cursor: EventCursor { sequence: 1 }, revision: 1,
            event: AgentEvent::PartUpdated {
                turn_id: TurnId::from_string("run".into()), projection_revision: 1,
                update: PartUpdate::Created { part: part.clone() }, part,
            },
        }, CoreEventEnvelope {
            session_id: "actual-core-session".into(), turn_id: Some("run".into()),
            cursor: EventCursor { sequence: 2 }, revision: 2,
            event: AgentEvent::TaskUpdated {
                task_projection_revision: 1,
                task: serde_json::from_value(serde_json::json!({
                    "id":"task", "owner_session_id":"actual-core-session", "owner_turn_id":"run",
                    "kind":"tool", "state":"running", "current_step":"working",
                    "cancel_requested":false
                })).unwrap(),
            },
        }]
    }

    fn scope() -> AssistantJournalChangeScope<'static> {
        AssistantJournalChangeScope {
            conversation_id: "conversation",
            core_session_id: "actual-core-session",
            source_run_id: Some("run"),
            maintenance: false,
        }
    }

    #[test]
    fn producer_publishes_exact_parts_and_tasks_once_with_canonical_not_core_identity() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = NolaStore::open(directory.path().join("journal.db")).unwrap();
        store.begin_write_unit_of_work().unwrap();
        begin(&mut store);
        let events = events();
        let skills = SkillRuntimeState::default();
        let changes =
            collect_assistant_journal_changes(scope(), &events, &skills, &[], &[]).unwrap();
        let result = publish_changed_assistant_journal(
            &mut store,
            AssistantSourceCommitStamp::Admission {
                run_id: "run".into(),
            },
            "conversation",
            "actual-core-session",
            Some("run"),
            &changes,
        )
        .unwrap()
        .unwrap();
        let AssistantPublicationCommit::Applied(cursors) = result else {
            panic!("first publication");
        };
        assert_eq!(cursors.len(), 1);
        let identity = &cursors[0].identity;
        assert_eq!(identity.conversation_id, "conversation");
        assert_eq!(identity.assistant_message_id, "assistant");
        let text = store
            .assistant_journal_part(identity, "text")
            .unwrap()
            .unwrap();
        let task = store
            .assistant_journal_part(identity, "task:task")
            .unwrap()
            .unwrap();
        assert_eq!(text.order_key.section, JournalSection::Transcript);
        assert_eq!(task.order_key.section, JournalSection::Tasks);
        assert_eq!(task.order_key.position, JournalNumber(1));
        assert!(matches!(task.metadata, JournalPartMetadata::TaskControl(_)));
        store.commit_write_unit_of_work().unwrap();
    }

    #[test]
    fn producer_missing_exact_turn_rolls_back_canonical_admission_and_reservations() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = NolaStore::open(directory.path().join("journal.db")).unwrap();
        store.begin_write_unit_of_work().unwrap();
        begin(&mut store);
        let mut events = events();
        events[1].turn_id = Some("absent-turn".into());
        if let AgentEvent::TaskUpdated { task, .. } = &mut events[1].event {
            task.owner_turn_id = TurnId::from_string("absent-turn".into());
        }
        let skills = SkillRuntimeState::default();
        let changes =
            collect_assistant_journal_changes(scope(), &events, &skills, &[], &[]).unwrap();
        assert!(publish_changed_assistant_journal(
            &mut store,
            AssistantSourceCommitStamp::Admission {
                run_id: "run".into()
            },
            "conversation",
            "actual-core-session",
            Some("run"),
            &changes
        )
        .unwrap_err()
        .contains("no persisted owner"));
        store.rollback_write_unit_of_work().unwrap();
        assert!(store.active_run("conversation").unwrap().is_none());
        // A fresh admission can reuse its original IDs only because both its
        // canonical records and earlier order reservations were rolled back.
        store.begin_write_unit_of_work().unwrap();
        begin(&mut store);
        store.commit_write_unit_of_work().unwrap();
    }

    #[test]
    fn producer_admission_does_not_reserve_visible_text_order() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = NolaStore::open(directory.path().join("journal.db")).unwrap();
        store.begin_write_unit_of_work().unwrap();
        begin(&mut store);
        let changes = AssistantJournalChanges {
            changes: vec![AssistantJournalChange::AdmissionText { part_id: "text" }],
            attempt_predecessors: vec![],
        };
        let publication = publish_changed_assistant_journal(
            &mut store,
            AssistantSourceCommitStamp::Admission {
                run_id: "run".into(),
            },
            "conversation",
            "actual-core-session",
            Some("run"),
            &changes,
        )
        .unwrap();
        assert!(publication.is_none(), "an empty admission is not a visible part");
        let identity = AssistantJournalIdentity {
            source_store_epoch: store.assistant_source_epoch().unwrap(),
            conversation_id: "conversation".into(), run_id: "run".into(),
            assistant_message_id: "assistant".into(),
        };
        assert_eq!(store.assistant_journal_head(&identity).unwrap().sequence, 0);
        assert!(store.assistant_journal_part(&identity, "text").unwrap().is_none());
        // No invisible admission reservation should steal position one.
        let first = store.reserve_assistant_journal_order(
            &AssistantSourceCommitStamp::Admission { run_id: "run".into() },
            &identity, JournalSection::Transcript, "text",
        ).unwrap();
        assert_eq!(first.position.0, 1);
        store.commit_write_unit_of_work().unwrap();
    }

    #[test]
    fn compaction_attempt_without_parts_uses_exact_outbox_turn_and_actual_core_session() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = NolaStore::open(directory.path().join("journal.db")).unwrap();
        store.begin_write_unit_of_work().unwrap();
        let mut input = begin_input();
        input.initial_events.push(nola_store_port::run_creation::InitialCoreRunEvent {
            core_session_id:"actual-core-session".into(),core_turn_id:Some("run".into()),
            core_cursor:2,core_revision:1,event_kind:"assistant_effect_bound".into(),
            payload_json:serde_json::json!({"type":"assistant_effect_bound","turn_id":"run",
                "assistant_message_id":"assistant","effect_id":"compact-effect"}).to_string(),
        });
        input.assistant_turn_bindings.push(nola_domain::AssistantTurnBinding::Effect {
            turn_id:"run".into(),assistant_message_id:"assistant".into(),effect_id:"compact-effect".into(),
        });
        let attempt = nola_kernel::prepare_model_attempt(
            "run",
            "compact-effect",
            "compact-effect",
            "compact-attempt",
            "unmaterialized-text",
            "unmaterialized-reasoning",
            1,
        )
        .unwrap();
        input
            .model_attempt_mutations
            .push(nola_domain::ModelAttemptMutation {
                expected_revision: None,
                record: attempt.clone(),
            });
        input
            .outbox_effects
            .push(nola_store_port::run_effect::OutboxEffectInput {
            model_attempt_id: Some(attempt.id.clone()),
            delegated_invocation_id: None,
            effect_id: "compact-effect".into(),
            request_id: "compact-request".into(),
            effect_kind: "compact".into(),
            write: nola_store_port::run_effect::OutboxEffectWrite::Declare { payload_json: serde_json::json!({
                "attempt_id":"compact-attempt", "request_id":"compact-request",
                "effect_id":"compact-effect", "host_id":"host", "session_id":"actual-core-session",
                "turn_id":"run", "catalog_revision":1, "availability_epoch":1,
                "created_at_ms":1, "deadline_ms":null,
                "payload":{"type":"compact", "strategy":"summary", "instructions":"",
                    "messages":[], "target_tokens":500, "context_window_tokens":128000}
            })
            .to_string() },
        });
        store.begin_turn(&input).unwrap();
        let owner = attempt_owner(&store, "conversation", "actual-core-session", &attempt)
            .unwrap()
            .unwrap();
        assert_eq!(owner.assistant_message_id, "assistant");
        assert!(
            attempt_owner(&store, "conversation", "conversation", &attempt)
                .unwrap_err()
                .contains("execution identity differs")
        );
        assert!(attempt_owner(
            &store,
            "foreign-conversation",
            "actual-core-session",
            &attempt
        )
        .is_err());
        store.rollback_write_unit_of_work().unwrap();
    }
}
