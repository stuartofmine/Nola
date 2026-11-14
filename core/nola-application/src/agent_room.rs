// SPDX-License-Identifier: Apache-2.0
//! Shared chat rooms own participant mappings; each participant runs in a fresh
//! canonical conversation. Presentation never becomes shared model history.
use nola_domain::agent_room::{AgentRoom, RoomParticipant};
use nola_domain::persistent_agent::AgentId;
use nola_store_port::conversation_profile::ConversationWriteRepositoryProvider;
use nola_store_port::product_setting::{
    ProductSettingRepository, ProductSettingRepositoryProvider,
};
use nola_store_port::protocol_journal::{
    CommandProtocolRepositoryProvider, CommandTransactionRepository,
};
use serde::{Deserialize, Serialize};

mod submission;
pub use submission::*;
mod execution_intent;
pub use execution_intent::*;
mod children;
pub use children::*;

pub use nola_domain::agent_room::ROOM_KEY_PREFIX;
use nola_domain::agent_room::{ContextOwner, CONTEXT_OWNER_FIELD};

/// A normal conversation can own one hidden multi-agent Room. The association
/// is stored separately from Room so existing Room records and their direct
/// links remain valid without rewriting them.
pub const CONVERSATION_ROOM_KEY_PREFIX: &str = "agent_chat_conversation_room/";
pub const ROOM_CONVERSATION_KEY_PREFIX: &str = "agent_chat_room_conversation/";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationRoomAssociation {
    pub conversation_id: String,
    pub room_id: String,
    pub revision: i64,
    pub room: RoomSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentRoomCommand {
    ChildConversations { conversation_id: String, limit: usize, offset: usize },
    PeerExchanges { conversation_id: String, limit: usize, offset: usize },
    ChildHistory { conversation_id: String, child_conversation_id: String, before_ordinal: Option<i64>, limit: usize, content_byte_limit: usize },
    EnsureForConversation { conversation_id: String, at_ms: i64 },
    AttachConversation { conversation_id: String, room_id: String, at_ms: i64 },
    ForConversation { conversation_id: String },
    PrepareExecution { room_id: String, submission_id: String, expected_revision: i64, intent: RoomExecutionIntent, at_ms: i64 },
    ExecutionIntent { room_id: String, submission_id: String },
    ReconcileExecution { room_id: String, submission_id: String, at_ms: i64 },
    Submit { input: SubmitRoomMessage },
    CancelPending { room_id: String, submission_id: String, expected_revision: i64, at_ms: i64 },
    ObserveExecution { room_id: String, submission_id: String, run_id: String, at_ms: i64 },
    ClaimExecution { room_id: String, submission_id: String, expected_revision: i64, at_ms: i64 },
    FailSubmission { room_id: String, submission_id: String, expected_revision: i64, code: String, at_ms: i64 },
    SelectRecipient { room_id: String, submission_id: String, expected_revision: i64, participant_id: String, at_ms: i64 },
    Submissions { room_id: String, limit: usize, offset: usize },
    Submission { room_id: String, submission_id: String },
    Directory,
    RoleTemplates,
    List { limit: usize, offset: usize },
    ConfigureAgent {
        input: crate::persistent_agent::ConfigureAgent,
    },
    Create {
        title: String,
        at_ms: i64,
    },
    Read {
        room_id: String,
    },
    Join {
        input: JoinAgent,
    },
    Rejoin {
        input: JoinAgent,
    },
    Deactivate {
        room_id: String,
        participant_id: String,
        expected_revision: i64,
        at_ms: i64,
    },
}

impl AgentRoomCommand {
    pub fn is_read_only_query(&self) -> bool {
        matches!(self,Self::Directory|Self::ChildConversations{..}|Self::PeerExchanges{..}|Self::ChildHistory{..}
            |Self::RoleTemplates|Self::Read{..}|Self::List{..}|Self::Submission{..}|Self::Submissions{..}
            |Self::ExecutionIntent{..}|Self::ForConversation{..})
    }
}

fn conversation_room_key(conversation_id: &str) -> Result<String, String> {
    // Conversation IDs are opaque across clients (Web uses conv-*; CLI also
    // accepts named sessions). Existence is checked by the owning repository.
    if conversation_id.trim().is_empty() || conversation_id.chars().any(char::is_control) {
        return Err("invalid conversation ID".into());
    }
    Ok(format!("{CONVERSATION_ROOM_KEY_PREFIX}{conversation_id}"))
}

fn room_conversation_key(room_id: &str) -> Result<String, String> {
    uuid::Uuid::parse_str(room_id).map_err(|_| "invalid room ID")?;
    Ok(format!("{ROOM_CONVERSATION_KEY_PREFIX}{room_id}"))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ConversationRoomLinkRecord {
    conversation_id: String,
    room_id: String,
}

fn decode_link(value: &str) -> Result<ConversationRoomLinkRecord, String> {
    serde_json::from_str(value).map_err(|_| "invalid conversation Room association".into())
}

/// Reads the durable association and verifies both indexes before exposing it.
/// An incomplete or conflicting index fails closed; reads never repair data.
pub fn for_conversation<S>(store: &mut S, conversation_id: &str) -> Result<Option<ConversationRoomAssociation>, String>
where
    S: ProductSettingRepositoryProvider + nola_store_port::conversation_query::ConversationQueryRepository + ?Sized,
{
    if store.conversation_summary(conversation_id).map_err(|e| e.to_string())?.is_none() {
        return Err("conversation_not_found".into());
    }
    let Some(value) = store.product_setting_repository()
        .product_setting(&conversation_room_key(conversation_id)?).map_err(|e| e.to_string())?
    else { return Ok(None); };
    let link = decode_link(&value.value_json)?;
    if link.conversation_id != conversation_id { return Err("conversation Room association identity mismatch".into()); }
    let reverse = store.product_setting_repository()
        .product_setting(&room_conversation_key(&link.room_id)?).map_err(|e| e.to_string())?
        .ok_or("conversation Room reverse association missing")?;
    let room = load(&store.product_setting_repository(), &link.room_id)?.ok_or("associated room not found")?;
    Ok(Some(validated_association(conversation_id, &value, &reverse, room)?))
}

/// Shared by normal Room reads and a read-only aggregate on an immutable Store snapshot.
pub(crate) fn validated_association(conversation_id: &str,
    forward: &nola_store_port::product_setting::StoredProductSetting,
    reverse: &nola_store_port::product_setting::StoredProductSetting, room: RoomSnapshot,
) -> Result<ConversationRoomAssociation, String> {
    let link = decode_link(&forward.value_json)?;
    if link.conversation_id != conversation_id || forward.key != conversation_room_key(conversation_id)?
        || reverse.key != room_conversation_key(&link.room_id)? || decode_link(&reverse.value_json)? != link
        || room.room.id() != link.room_id || room.revision < 1 || forward.revision < 1 || reverse.revision < 1 {
        return Err("conversation Room association witness mismatch".into());
    }
    Ok(ConversationRoomAssociation { conversation_id: link.conversation_id, room_id: link.room_id,
        revision: forward.revision, room })
}
pub(crate) fn association_conversation_id(value: &nola_store_port::product_setting::StoredProductSetting) -> Result<String, String> {
    Ok(decode_link(&value.value_json)?.conversation_id)
}

/// Links an existing Room to a canonical conversation without changing either
/// record. A Room and a conversation each have at most one association.
pub fn attach_conversation<S>(store: &mut S, conversation_id: String, room_id: String, at_ms: i64)
    -> Result<ConversationRoomAssociation, String>
where
    S: ProductSettingRepositoryProvider + nola_store_port::conversation_query::ConversationQueryRepository + ?Sized
        + CommandProtocolRepositoryProvider,
{
    store.command_protocol_repository().begin_command_mutations().map_err(|e| e.to_string())?;
    let result = attach_conversation_inner(store, conversation_id, room_id, at_ms);
    match result {
        Ok(value) => {
            store.command_protocol_repository().commit_command_mutations().map_err(|e| e.to_string())?;
            Ok(value)
        }
        Err(error) => {
            store.command_protocol_repository().rollback_command_mutations()
                .map_err(|rollback| format!("{error}; association rollback failed: {rollback}"))?;
            Err(error)
        }
    }
}

fn attach_conversation_inner<S>(store: &mut S, conversation_id: String, room_id: String, at_ms: i64)
    -> Result<ConversationRoomAssociation, String>
where
    S: ProductSettingRepositoryProvider + nola_store_port::conversation_query::ConversationQueryRepository + ?Sized,
{
    if at_ms < 0 { return Err("invalid timestamp".into()); }
    if store.conversation_summary(&conversation_id).map_err(|e| e.to_string())?.is_none() {
        return Err("conversation_not_found".into());
    }
    let room = load(&store.product_setting_repository(), &room_id)?.ok_or("room_not_found")?;
    let forward_key = conversation_room_key(&conversation_id)?;
    let reverse_key = room_conversation_key(&room_id)?;
    let mut settings = store.product_setting_repository();
    let forward = settings.product_setting(&forward_key).map_err(|e| e.to_string())?;
    let reverse = settings.product_setting(&reverse_key).map_err(|e| e.to_string())?;
    let record = ConversationRoomLinkRecord { conversation_id: conversation_id.clone(), room_id: room_id.clone() };
    if let Some(existing) = &forward {
        if decode_link(&existing.value_json)? != record { return Err("conversation_already_has_a_room".into()); }
    }
    if let Some(existing) = &reverse {
        if decode_link(&existing.value_json)? != record { return Err("room_already_belongs_to_a_conversation".into()); }
    }
    if forward.is_none() { settings.put_product_setting(&forward_key, &serde_json::to_string(&record).map_err(|e| e.to_string())?, at_ms).map_err(|e| e.to_string())?; }
    if reverse.is_none() { settings.put_product_setting(&reverse_key, &serde_json::to_string(&record).map_err(|e| e.to_string())?, at_ms).map_err(|e| e.to_string())?; }
    let revision = settings.product_setting(&forward_key).map_err(|e| e.to_string())?.ok_or("association write not visible")?.revision;
    Ok(ConversationRoomAssociation { conversation_id, room_id, revision, room })
}

/// Creates an empty, associated backing Room only when the user explicitly
/// opens the expert picker. It does not add a participant or dispatch work.
pub fn ensure_for_conversation<S>(store: &mut S, conversation_id: String, at_ms: i64)
    -> Result<ConversationRoomAssociation, String>
where
    S: ProductSettingRepositoryProvider + nola_store_port::conversation_query::ConversationQueryRepository + ?Sized
        + CommandProtocolRepositoryProvider,
{
    store.command_protocol_repository().begin_command_mutations().map_err(|e| e.to_string())?;
    let result = ensure_for_conversation_inner(store, conversation_id, at_ms);
    match result {
        Ok(value) => {
            store.command_protocol_repository().commit_command_mutations().map_err(|e| e.to_string())?;
            Ok(value)
        }
        Err(error) => {
            store.command_protocol_repository().rollback_command_mutations()
                .map_err(|rollback| format!("{error}; association rollback failed: {rollback}"))?;
            Err(error)
        }
    }
}

fn ensure_for_conversation_inner<S>(store: &mut S, conversation_id: String, at_ms: i64)
    -> Result<ConversationRoomAssociation, String>
where
    S: ProductSettingRepositoryProvider + nola_store_port::conversation_query::ConversationQueryRepository + ?Sized,
{
    if let Some(value) = for_conversation(store, &conversation_id)? { return Ok(value); }
    if at_ms < 0 { return Err("invalid timestamp".into()); }
    let summary = store.conversation_summary(&conversation_id).map_err(|e| e.to_string())?
        .ok_or("conversation_not_found")?;
    // An unsent conversation legitimately has no generated title yet.
    // Name the new room without changing the conversation or existing data.
    let title = if summary.title.trim().is_empty() { "Conversation".into() } else { summary.title };
    let room = AgentRoom::new(uuid::Uuid::new_v4().to_string(), title, vec![])?;
    let room = write(&mut store.product_setting_repository(), room, at_ms)?;
    attach_conversation_inner(store, conversation_id, room.room.id().into(), at_ms)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomSnapshot {
    pub revision: i64,
    pub room: AgentRoom,
}

pub fn list<R: ProductSettingRepository + ?Sized>(store: &R, limit: usize, offset: usize)
    -> Result<Vec<RoomSnapshot>, String> {
    store.list_product_settings(ROOM_KEY_PREFIX, limit, offset).map_err(|e| e.to_string())?
        .into_iter().map(|value| {
            let room: AgentRoom = serde_json::from_str(&value.value_json).map_err(|e| e.to_string())?;
            if key(room.id())? != value.key || value.revision < 1 {
                return Err("room storage identity mismatch".into());
            }
            Ok(RoomSnapshot { revision: value.revision, room })
        }).collect()
}

fn key(room_id: &str) -> Result<String, String> {
    // Room IDs are generated UUIDs, not user-selected setting paths.
    uuid::Uuid::parse_str(room_id).map_err(|_| "invalid room ID")?;
    Ok(format!("{ROOM_KEY_PREFIX}{room_id}"))
}

pub fn load<R: ProductSettingRepository + ?Sized>(
    store: &R,
    room_id: &str,
) -> Result<Option<RoomSnapshot>, String> {
    let Some(value) = store
        .product_setting(&key(room_id)?)
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    Ok(Some(room_from_record(room_id, &value)?))
}

pub(crate) fn room_from_record(room_id: &str, value: &nola_store_port::product_setting::StoredProductSetting) -> Result<RoomSnapshot,String> {
    let room: AgentRoom = serde_json::from_str(&value.value_json).map_err(|e|e.to_string())?;
    if value.key != key(room_id)? || room.id()!=room_id || value.revision<1 { return Err("room storage identity mismatch".into()); }
    Ok(RoomSnapshot { revision:value.revision,room })
}

fn write<R: ProductSettingRepository + ?Sized>(
    store: &mut R,
    room: AgentRoom,
    at: i64,
) -> Result<RoomSnapshot, String> {
    if !store.has_write_unit_of_work() {
        return Err("room writes require an admitted transaction".into());
    }
    let value = store
        .put_product_setting(
            &key(room.id())?,
            &serde_json::to_string(&room).map_err(|e| e.to_string())?,
            at,
        )
        .map_err(|e| e.to_string())?;
    Ok(RoomSnapshot {
        revision: value.revision,
        room,
    })
}

pub fn create<R: ProductSettingRepository + ?Sized>(
    store: &mut R,
    title: String,
    at: i64,
) -> Result<RoomSnapshot, String> {
    let room = AgentRoom::new(uuid::Uuid::new_v4().to_string(), title, vec![])?;
    if load(store, room.id())?.is_some() {
        return Err("room ID collision".into());
    }
    write(store, room, at)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinAgent {
    pub room_id: String,
    pub expected_room_revision: i64,
    pub agent_id: AgentId,
    pub expected_agent_revision: i64,
    pub at_ms: i64,
}

/// Savepoint protects the two writes even when the caller continues its outer
/// receipt transaction after a rejected command. IDs cannot bind old contexts.
pub fn join<S>(store: &mut S, command: JoinAgent) -> Result<RoomSnapshot, String>
where
    S: ProductSettingRepositoryProvider
        + ConversationWriteRepositoryProvider + ?Sized
        + CommandProtocolRepositoryProvider,
{
    store
        .command_protocol_repository()
        .begin_command_mutations()
        .map_err(|e| e.to_string())?;
    let result = join_inner(store, command);
    match result {
        Ok(result) => {
            store
                .command_protocol_repository()
                .commit_command_mutations()
                .map_err(|e| e.to_string())?;
            Ok(result)
        }
        Err(error) => {
            store
                .command_protocol_repository()
                .rollback_command_mutations()
                .map_err(|rollback| format!("{error}; room rollback failed: {rollback}"))?;
            Err(error)
        }
    }
}

fn join_inner<S>(store: &mut S, command: JoinAgent) -> Result<RoomSnapshot, String>
where
    S: ProductSettingRepositoryProvider + ConversationWriteRepositoryProvider + ?Sized,
{
    let snapshot =
        load(&store.product_setting_repository(), &command.room_id)?.ok_or("room_not_found")?;
    join_with_snapshot(store, command, snapshot)
}

fn join_with_snapshot<S>(store: &mut S, command: JoinAgent, snapshot: RoomSnapshot) -> Result<RoomSnapshot, String>
where
    S: ProductSettingRepositoryProvider + ConversationWriteRepositoryProvider + ?Sized,
{
    if snapshot.revision != command.expected_room_revision {
        return Err("room_revision_conflict".into());
    }
    let origin = crate::persistent_agent::capture_defaults(
        &store.product_setting_repository(),
        &command.agent_id,
        command.expected_agent_revision,
    )?;
    let agent = crate::persistent_agent::load(&store.product_setting_repository())?
        .agents
        .into_iter()
        .find(|agent| agent.id() == &command.agent_id)
        .ok_or("agent_not_found")?;
    let participant = RoomParticipant {
        id: uuid::Uuid::new_v4().to_string(),
        agent_id: command.agent_id,
        agent_revision: origin.agent_revision,
        display_name: agent.name().into(),
        description: agent.description().into(),
        // Native clients expose canonical conversation identities as Foundation
        // UUID.uuidString. Generate that representation once; storage is exact-keyed.
        context_conversation_id: uuid::Uuid::new_v4().to_string().to_uppercase(),
        active: true,
    };
    let next_room = snapshot.room.add(participant.clone())?;
    let mut configuration = origin.initial_configuration;
    configuration[CONTEXT_OWNER_FIELD] = serde_json::to_value(ContextOwner {
        room_id: command.room_id,
        participant_id: participant.id,
        agent_id: participant.agent_id,
    })
    .map_err(|e| e.to_string())?;
    let created = crate::conversation::create_conversation(
        &mut store.conversation_write_repository(),
        crate::conversation::CreateConversation {
            id: participant.context_conversation_id,
            title: participant.display_name,
            settings_json: Some(configuration.to_string()),
            created_at_ms: command.at_ms,
            updated_at_ms: command.at_ms,
        },
    )?;
    if !created.created {
        return Err("context ID collision".into());
    }
    write(
        &mut store.product_setting_repository(),
        next_room,
        command.at_ms,
    )
}

/// Explicitly replace a frozen participant snapshot. Old context/history stays
/// addressable; the entire roster-replace/create sequence either commits or rolls
/// back. This is never performed as a side effect of directory refresh.
pub fn rejoin<S>(store: &mut S, command: JoinAgent) -> Result<RoomSnapshot, String>
where
    S: ProductSettingRepositoryProvider + ConversationWriteRepositoryProvider
        + CommandProtocolRepositoryProvider
        + nola_store_port::conversation_history::ConversationProjectionRepository
        + nola_store_port::remote_session::RemoteSessionRepository
        + nola_store_port::run_recovery_inspection::RunRecoveryInspectionRepository
        + nola_store_port::conversation_query::ConversationQueryRepository
        + nola_store_port::model::ModelRepositoryProvider,
{
    store.command_protocol_repository().begin_command_mutations().map_err(|e| e.to_string())?;
    let result = (|| {
        let snapshot = load(&store.product_setting_repository(), &command.room_id)?.ok_or("room_not_found")?;
        if snapshot.revision != command.expected_room_revision { return Err("room_revision_conflict".into()); }
        let current = snapshot.room.participants().iter()
            .find(|member| member.active && member.agent_id == command.agent_id)
            .ok_or("active_agent_participant_not_found")?;
        if current.agent_revision == command.expected_agent_revision {
            return Err("expert_already_uses_current_configuration".into());
        }
        if store.active_run(&current.context_conversation_id).map_err(|e| e.to_string())?.is_some() {
            return Err("expert_configuration_in_use_by_active_run".into());
        }
        let mut offset = 0;
        loop {
            let pending = all_submissions(&store.product_setting_repository(), &command.room_id, 200, offset)?;
            for submission in &pending {
                let current = if submission.state == RoomSubmissionState::Executing {
                    reconcile_execution(store, &command.room_id, &submission.input.submission_id, command.at_ms)?
                } else { submission.clone() };
                if matches!(current.state, RoomSubmissionState::Accepted | RoomSubmissionState::Selected | RoomSubmissionState::Executing) {
                    return Err("expert_configuration_in_use_by_pending_submission".into());
                }
            }
            if pending.len() < 200 { break; }
            offset += pending.len();
        }
        // The published Room format permits one roster entry per Agent ID.
        // Historical manifests and context ownership retain the old participant
        // identity; only the current roster is replaced, never its conversation.
        let next = AgentRoom::new(snapshot.room.id().into(), snapshot.room.title().into(),
            snapshot.room.participants().iter().filter(|member| member.id != current.id).cloned().collect())?;
        join_with_snapshot(store, command, RoomSnapshot { room: next, revision: snapshot.revision })
    })();
    match result {
        Ok(snapshot) => {
            store.command_protocol_repository().commit_command_mutations().map_err(|e| e.to_string())?;
            Ok(snapshot)
        }
        Err(error) => {
            store.command_protocol_repository().rollback_command_mutations()
                .map_err(|rollback| format!("{error}; rejoin rollback failed: {rollback}"))?;
            Err(error)
        }
    }
}

pub fn deactivate<R: ProductSettingRepository + ?Sized>(
    store: &mut R,
    room_id: &str,
    participant_id: &str,
    expected_revision: i64,
    at: i64,
) -> Result<RoomSnapshot, String> {
    if !store.has_write_unit_of_work() {
        return Err("room writes require an admitted transaction".into());
    }
    let snapshot = load(store, room_id)?.ok_or("room_not_found")?;
    if snapshot.revision != expected_revision {
        return Err("room_revision_conflict".into());
    }
    write(store, snapshot.room.deactivate(participant_id)?, at)
}


/// Installs a fresh expert resend Room after its empty contexts were created
/// from validated frozen defaults. Never reads today's persistent-agent directory.
pub(crate) fn install_resend_room<S>(store: &mut S, conversation_id: String,
    room: AgentRoom, at_ms: i64) -> Result<ConversationRoomAssociation, String>
where S: ProductSettingRepositoryProvider + nola_store_port::conversation_query::ConversationQueryRepository + ?Sized,
{
    if load(&store.product_setting_repository(),room.id())?.is_some() {
        return Err("expert_resend_room_already_exists".into());
    }
    write(&mut store.product_setting_repository(), room.clone(), at_ms)?;
    attach_conversation_inner(store,conversation_id,room.id().into(),at_ms)
}

/// Reconcile a known successful physical routing receipt. Unknown Attempts
/// remain unknown; a repeated request never causes model dispatch here.
pub fn reconcile_routing<R>(store:&mut R,room:&str,submission_id:&str,source:Option<&str>,at_ms:i64)
    ->Result<Option<RoomSubmission>,String>
where R: ProductSettingRepositoryProvider + CommandProtocolRepositoryProvider
    + nola_store_port::auxiliary_invocation::AuxiliaryInvocationRepository {
    use nola_domain::{AuxiliaryInvocationOwner,AuxiliaryInvocationScenario,ModelAttemptState};
    crate::assistant_publication_transaction::with_publication_transaction(store,|store| {
        let receipt=submission(&store.product_setting_repository(),room,submission_id)?.ok_or("room_submission_not_found")?;
        if receipt.state!=RoomSubmissionState::Accepted {return Ok(Some(receipt));}
        let operation=submission_id.to_ascii_lowercase();
        let owner=AuxiliaryInvocationOwner::Room {room_id:room.into()};
        let (record,attempt)=match source {
            Some(host)=>{let Some(value)=store.auxiliary_source_observation(&owner,host,&operation).map_err(|e|e.to_string())? else{return Ok(None)};
                (value.receipt.operation,value.receipt.attempt)},
            None=>{let Some(record)=store.auxiliary_operation(&operation).map_err(|e|e.to_string())? else{return Ok(None)};
                let attempt=store.auxiliary_attempt_tail(&operation).map_err(|e|e.to_string())?;(record,attempt)},
        };
        if record.scenario!=AuxiliaryInvocationScenario::RoomRouting || source.is_none() && record.owner!=owner {
            return Err("room_routing_owner_mismatch".into());
        }
        if record.cancellation_requested_at_ms.is_some() {return Ok(None)}
        let Some(attempt)=attempt.filter(|attempt|attempt.state==ModelAttemptState::Succeeded) else{return Ok(None)};
        let output=attempt.output.ok_or("Successful routing receipt has no recoverable output")?;
        if !output.as_object().is_some_and(|fields|fields.len()==1) {return Err("Invalid Room routing output".into());}
        let member=output["participant_id"].as_str().ok_or("Invalid Room routing recipient")?;
        select_recipient(&mut store.product_setting_repository(),room,submission_id,receipt.revision,member,at_ms).map(Some)
    })
}
