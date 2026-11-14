// SPDX-License-Identifier: Apache-2.0
//! Host-bound expert cooperation. Private capture records never synthesize parent user messages.
use crate::expert_template_capture::{
    ExpertTemplateCaptureFact, ExpertTemplateCapturePreview, HostExpertTemplateAuthority,
    HostExpertTemplateCaptureContext,
};
use nola_domain::expert_message::{
    ExpertMessageEntry, ExpertParticipantReference, ExpertPromptDefinition,
};
use serde::{Deserialize, Serialize};

pub const PRIVATE_PREFIX: &str = "expert_peer_private/v1/";
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PeerExchangeInput {
    pub source_context_id: String,
    pub source_run_id: String,
    pub call_id: String,
    pub target_participant_id: String,
    pub kind: String,
    pub text: String,
    pub in_reply_to: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PeerClarificationInput {
    pub source_context_id: String,
    pub source_run_id: String,
    pub call_id: String,
    pub exchange_id: String,
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PeerClarification {
    pub message_id: String,
    pub source: ExpertParticipantReference,
    pub target: ExpertParticipantReference,
    pub text: String,
    pub created_at_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PeerView {
    pub version: u32,
    pub exchange_id: String,
    pub room_id: String,
    pub parent_conversation_id: String,
    pub source: nola_domain::expert_task_target::ExpertTaskSourceV1,
    pub target: ExpertParticipantReference,
    pub source_run_id: String,
    pub source_call_id: String,
    pub source_display_name: String,
    pub target_display_name: String,
    pub kind: String,
    pub text: String,
    pub in_reply_to: Option<String>,
    pub root_exchange_id: String,
    pub hop: u32,
    pub created_at_ms: i64,
    pub clarifications: Vec<PeerClarification>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerExchanges {
    pub version: u32,
    pub conversation_id: String,
    pub exchanges: Vec<PeerView>,
    pub has_more: bool,
    pub next_offset: Option<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PeerMaterial {
    pub view: PeerView,
    pub entry: ExpertMessageEntry,
    pub prompt_definitions: Vec<ExpertPromptDefinition>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PeerRecord {
    input: PeerExchangeInput,
    material: PeerMaterial,
    preview: ExpertTemplateCapturePreview,
    capture: ExpertTemplateCaptureFact,
    visited: Vec<String>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    task_configuration: Option<serde_json::Value>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    task_exchange_context: Option<PeerView>,
}

pub type ExpertPeerView = PeerView;
pub type ExpertPeerViews = PeerExchanges;

impl PeerExchangeInput {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_identity(&self.source_context_id)
            || !valid_identity(&self.call_id)
            || !valid_identity(&self.target_participant_id)
            || uuid::Uuid::parse_str(&self.source_run_id).is_err()
            || self.kind != "request"
            || !valid_text(&self.text)
            || self
                .in_reply_to
                .as_ref()
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_err())
        {
            return Err("expert_peer_invalid_request".into());
        }
        Ok(())
    }
}
impl PeerClarificationInput {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_identity(&self.source_context_id)
            || !valid_identity(&self.call_id)
            || uuid::Uuid::parse_str(&self.source_run_id).is_err()
            || uuid::Uuid::parse_str(&self.exchange_id).is_err()
            || !valid_text(&self.text)
        {
            return Err("expert_peer_invalid_clarification".into());
        }
        Ok(())
    }
}
fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.trim() == value
        && !value.chars().any(char::is_control)
}
fn valid_text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 262_144 && !value.contains('\0')
}

use crate::expert_template_capture as capture_policy;
use crate::repositories::ApplicationCommandRepositories;
use nola_domain::{agent_room::RoomParticipant, ConversationSourceOwner};
use nola_store_port::{
    conversation_history::ConversationProjectionRepository,
    conversation_query::ConversationQueryRepository,
    product_setting::{ProductSettingRepository, ProductSettingRepositoryProvider},
    protocol_journal::{CommandProtocolRepositoryProvider, CommandTransactionRepository},
    room_presentation::{RoomPresentationKey, RoomPresentationRepository},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallIndex {
    operation: String,
    input: serde_json::Value,
    exchange_id: String,
    message_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Chain {
    version: u32,
    root_exchange_id: String,
    parent_conversation_id: String,
    messages: u32,
}
fn record_key(id: &str) -> Result<String, String> {
    uuid::Uuid::parse_str(id).map_err(|_| "expert_peer_invalid_exchange_id")?;
    Ok(format!("{PRIVATE_PREFIX}exchange/{id}"))
}
fn call_key(context: &str, run: &str, call: &str) -> Result<String, String> {
    Ok(format!(
        "{PRIVATE_PREFIX}call/{}",
        serde_json::to_string(&(context, run, call)).map_err(|e| e.to_string())?
    ))
}
fn read_private<R: RoomPresentationRepository, T: serde::de::DeserializeOwned>(
    store: &R,
    key: &str,
) -> Result<Option<T>, String> {
    store
        .read_expert_peer_setting(key)
        .map_err(|e| e.to_string())?
        .map(|row| {
            serde_json::from_str(&row.value_json)
                .map_err(|_| "expert_peer_private_record_invalid".into())
        })
        .transpose()
}
fn write_private<S: ProductSettingRepositoryProvider, T: Serialize>(
    store: &mut S,
    key: &str,
    value: &T,
    at: i64,
) -> Result<(), String> {
    let json = serde_json::to_string(value).map_err(|e| e.to_string())?;
    if json.len() > nola_domain::expert_message::EXPERT_TEMPLATE_CAPTURE_MAX_BYTES {
        return Err("expert_peer_capture_exceeds_budget".into());
    }
    store
        .product_setting_repository()
        .put_product_setting(key, &json, at)
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn transaction<S: CommandProtocolRepositoryProvider, T>(
    store: &mut S,
    f: impl FnOnce(&mut S) -> Result<T, String>,
) -> Result<T, String> {
    store
        .command_protocol_repository()
        .begin_command_mutations()
        .map_err(|e| e.to_string())?;
    match f(store) {
        Ok(value) => {
            store
                .command_protocol_repository()
                .commit_command_mutations()
                .map_err(|e| e.to_string())?;
            Ok(value)
        }
        Err(error) => {
            store
                .command_protocol_repository()
                .rollback_command_mutations()
                .map_err(|e| format!("{error}; peer rollback failed: {e}"))?;
            Err(error)
        }
    }
}
fn reference(member: &RoomParticipant) -> ExpertParticipantReference {
    ExpertParticipantReference {
        participant_id: member.id.clone(),
        agent_id: member.agent_id.clone(),
        agent_revision: member.agent_revision,
        context_conversation_id: member.context_conversation_id.clone(),
    }
}
fn authorize(record: &PeerRecord, authority: &HostExpertTemplateAuthority) -> Result<(), String> {
    authority.validate()?;
    if record.capture.actor != authority.actor
        || record.capture.request_host_id != authority.request_host_id
    {
        return Err("expert_peer_capture_authority_mismatch".into());
    }
    let material = &record.material;
    if material.view.version != 1
        || material.view.exchange_id != material.entry.child_submission_id
        || material.view.target != material.entry.participant
        || record.capture.conversation_id != material.view.parent_conversation_id
        || record.capture.submission_id != material.view.exchange_id
        || record.capture.intent_sha256 != capture_policy::payload_sha256(&record.input)?
    {
        return Err("expert_peer_private_binding_changed".into());
    }
    record.input.validate()?;
    let view = &material.view;
    if view.text != record.input.text
        || view.kind != "request"
        || view.in_reply_to != record.input.in_reply_to
        || view.source_run_id != record.input.source_run_id
        || view.source_call_id != record.input.call_id
        || view.source.context_conversation_id() != record.input.source_context_id
        || view.source.is_main_agent() && view.source.context_conversation_id()!=view.parent_conversation_id
        || view.target.participant_id != record.input.target_participant_id
        || view.source.legacy_expert().is_some_and(|source|source.participant_id==view.target.participant_id)
        || view.created_at_ms != record.capture.captured_at_ms
        || !(1..=3).contains(&view.hop)
        || record.capture.message_id != format!("{}:peer", view.exchange_id)
        || uuid::Uuid::parse_str(&view.room_id).is_err()
        || uuid::Uuid::parse_str(&view.root_exchange_id).is_err()
        || record.visited.len() != view.hop as usize + usize::from(!view.source.is_main_agent())
        || record.visited.last() != Some(&view.target.participant_id)
        || record
            .visited
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != record.visited.len()
        || view.source.legacy_expert().is_some_and(|source|!record.visited.contains(&source.participant_id))
        || material.entry.child_user_message_id != format!("{}:user", view.exchange_id)
        || material.entry.child_assistant_message_id != format!("{}:assistant", view.exchange_id)
        || view.clarifications.iter().any(|c| {
            c.source != view.target
                || view.source.legacy_expert()!=Some(&c.target)
                || !valid_text(&c.text)
                || !valid_identity(&c.message_id)
                || c.created_at_ms < view.created_at_ms
        })
        || view
            .clarifications
            .iter()
            .map(|c| &c.message_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != view.clarifications.len()
    {
        return Err("expert_peer_corrupt_frozen_record".into());
    }
    let expected =
        crate::expert_message::capture::validate_peer_entry(&record.capture, &material.entry)?;
    if expected != material.prompt_definitions {
        return Err("expert_peer_frozen_instructions_changed".into());
    }
    Ok(())
}
pub(crate) fn load_record<R: RoomPresentationRepository>(
    store: &R,
    id: &str,
    authority: &HostExpertTemplateAuthority,
) -> Result<PeerRecord, String> {
    let record: PeerRecord =
        read_private(store, &record_key(id)?)?.ok_or("expert_peer_not_found")?;
    if record.material.view.exchange_id != id {
        return Err("expert_peer_record_identity_changed".into());
    }
    authorize(&record, authority)?;
    Ok(record)
}
fn prior_call<R: RoomPresentationRepository, T: Serialize>(
    store: &R,
    context: &str,
    run: &str,
    call: &str,
    operation: &str,
    input: &T,
    authority: &HostExpertTemplateAuthority,
) -> Result<Option<(PeerRecord, Option<String>)>, String> {
    let Some(index): Option<CallIndex> = read_private(store, &call_key(context, run, call)?)?
    else {
        return Ok(None);
    };
    if index.operation != operation
        || index.input != serde_json::to_value(input).map_err(|e| e.to_string())?
    {
        return Err("expert_peer_call_conflict".into());
    }
    let record = load_record(store, &index.exchange_id, authority)?;
    Ok(Some((record, index.message_id)))
}

struct SourceScope {
    parent: String,
    room: crate::agent_room::RoomSnapshot,
    source: nola_domain::expert_task_target::ExpertTaskSourceV1,
    target: RoomParticipant,
}
pub(crate) struct SourceBinding {
    parent: String,
    room: crate::agent_room::RoomSnapshot,
    source: nola_domain::expert_task_target::ExpertTaskSourceV1,
}
fn legacy_source_binding<S: ApplicationCommandRepositories>(
    store: &S,
    source_context_id: &str,
    source_run_id: &str,
    authority: &HostExpertTemplateAuthority,
) -> Result<SourceBinding, String> {
    authority.validate()?;
    if !valid_identity(source_context_id) || uuid::Uuid::parse_str(source_run_id).is_err() {
        return Err("expert_peer_invalid_source".into());
    }
    let owner = store
        .read_room_context_owner(source_context_id)
        .map_err(|e| e.to_string())?
        .ok_or("expert_peer_source_not_expert")?;
    if store
        .conversation_source_owner(source_context_id)
        .map_err(|e| e.to_string())?
        != ConversationSourceOwner::Local
    {
        return Err("expert_peer_source_requires_local_canonical_owner".into());
    }
    let run = store
        .active_run(source_context_id)
        .map_err(|e| e.to_string())?
        .ok_or("expert_peer_source_run_inactive")?;
    if run.run_id != source_run_id || run.kind != "conversation" {
        return Err("expert_peer_source_run_mismatch".into());
    }
    let reverse = store
        .read_room_presentation_setting(RoomPresentationKey::RoomAssociation(&owner.room_id))
        .map_err(|e| e.to_string())?
        .ok_or("expert_peer_parent_association_missing")?;
    let parent = crate::agent_room::association_conversation_id(&reverse)?;
    let room_id = public_room(store, &parent)?.ok_or("expert_peer_parent_association_missing")?;
    let room_row = store
        .read_room_presentation_setting(RoomPresentationKey::Room(&room_id))
        .map_err(|e| e.to_string())?
        .ok_or("expert_peer_room_missing")?;
    let room = crate::agent_room::room_from_record(&room_id, &room_row)?;
    if room_id != owner.room_id
        || store
            .conversation_source_owner(&parent)
            .map_err(|e| e.to_string())?
            != ConversationSourceOwner::Local
        || store
            .read_room_context_owner(&parent)
            .map_err(|e| e.to_string())?
            .is_some()
        || store
            .conversation_execution_ownership(&parent)
            .map_err(|e| e.to_string())?
            != nola_domain::ConversationExecutionOwnership::User
    {
        return Err("expert_peer_parent_source_mismatch".into());
    }
    let source = room
        .room
        .participants()
        .iter()
        .find(|m| {
            m.active
                && m.id == owner.participant_id
                && m.agent_id == owner.agent_id
                && m.context_conversation_id == source_context_id
        })
        .cloned()
        .ok_or("expert_peer_source_snapshot_retired")?;
    // Source authority comes from its exact frozen parent capture or peer record, never its text.
    if let Some(exchange) =
        read_private::<_, String>(store, &format!("{PRIVATE_PREFIX}run/{}", source_run_id))?
    {
        let prior = load_record(store, &exchange, authority)?;
        if prior.material.view.target != reference(&source) {
            return Err("expert_peer_source_capture_changed".into());
        }
    } else {
        let manifest = store
            .expert_message_for_child(source_context_id, source_run_id)
            .map_err(|e| e.to_string())?
            .ok_or("expert_peer_source_capture_missing")?;
        if !manifest.entries.iter().any(|entry| {
            entry.child_submission_id == source_run_id && entry.participant == reference(&source)
        }) {
            return Err("expert_peer_source_capture_changed".into());
        }
        crate::expert_message::capture::authorized(store, &manifest, authority)?;
    }
    Ok(SourceBinding {
        parent,
        room,
        source: nola_domain::expert_task_target::ExpertTaskSourceV1::LegacyExpert(reference(&source)),
    })
}

pub(crate) fn source_binding<S: ApplicationCommandRepositories>(store:&S,context:&str,run_id:&str,authority:&HostExpertTemplateAuthority)->Result<SourceBinding,String>{
    use nola_domain::expert_task_target::ExpertTaskSourceV1;
    authority.validate()?;
    if !valid_identity(context) || uuid::Uuid::parse_str(run_id).is_err(){return Err("invalid_main_agent_source".into());}
    if store.read_room_context_owner(context).map_err(|e|e.to_string())?.is_some()
        || store.conversation_execution_ownership(context).map_err(|e|e.to_string())?!=nola_domain::ConversationExecutionOwnership::User {return Err("only_main_agent_may_delegate_experts".into());}
    if store.conversation_source_owner(context).map_err(|e|e.to_string())?!=ConversationSourceOwner::Local{return Err("main_agent_requires_canonical_host".into());}
    let run=store.active_run(context).map_err(|e|e.to_string())?.ok_or("main_agent_run_inactive")?;
    if run.run_id!=run_id || run.kind!="conversation" {return Err("main_agent_run_mismatch".into());}
    let room_id=public_room(store,context)?.ok_or("main_conversation_has_no_experts")?;
    let row=store.read_room_presentation_setting(RoomPresentationKey::Room(&room_id)).map_err(|e|e.to_string())?.ok_or("main_expert_room_missing")?;
    let room=crate::agent_room::room_from_record(&room_id,&row)?;
    Ok(SourceBinding{parent:context.into(),room,source:ExpertTaskSourceV1::main_agent(context.into())})
}

fn source_scope<S: ApplicationCommandRepositories>(
    store: &S,
    input: &PeerExchangeInput,
    authority: &HostExpertTemplateAuthority,
) -> Result<SourceScope, String> {
    input.validate()?;
    let binding = source_binding(
        store,
        &input.source_context_id,
        &input.source_run_id,
        authority,
    )?;
    let target = binding
        .room
        .room
        .recipient(&input.target_participant_id)?
        .clone();
    if binding.source.context_conversation_id() == target.context_conversation_id
    {
        return Err("expert_peer_self_target_rejected".into());
    }
    Ok(SourceScope {
        parent: binding.parent,
        room: binding.room,
        source: binding.source,
        target,
    })
}

fn capture_input<S: ConversationProjectionRepository>(
    store: &S,
    scope: &SourceScope,
    input: &PeerExchangeInput,
) -> Result<nola_domain::expert_message::AdmitExpertConversationMessage, String> {
    let parent = store
        .load_conversation(&scope.parent)
        .map_err(|e| e.to_string())?
        .ok_or("expert_peer_parent_missing")?;
    // Read-only resolver input. It is never admitted or persisted as a user message/manifest.
    Ok(
        nola_domain::expert_message::AdmitExpertConversationMessage {
            conversation_id: scope.parent.clone(),
            expected_revision: parent.revision,
            room_id: scope.room.room.id().into(),
            expected_room_revision: scope.room.revision,
            submission_id: input.source_run_id.clone(),
            user_message_id: format!("{}:peer-preview", input.source_run_id),
            text: input.text.clone(),
            selected_participants: vec![reference(&scope.target)],
            created_at_ms: 0,
        },
    )
}
pub fn preview<S: ApplicationCommandRepositories>(
    store: &mut S,
    input: &PeerExchangeInput,
    authority: &HostExpertTemplateAuthority,
) -> Result<ExpertTemplateCapturePreview, String> {
    input.validate()?;
    authority.validate()?;
    store.with_consistent_read(|reader| {
        if let Some((record, _)) = prior_call(
            reader,
            &input.source_context_id,
            &input.source_run_id,
            &input.call_id,
            "request",
            input,
            authority,
        )? {
            return Ok(record.preview);
        }
        let scope = source_scope(reader, input, authority)?;
        let capture = capture_input(reader, &scope, input)?;
        let resolved = crate::expert_message::preview::resolve_peer(reader, &capture)?;
        Ok(ExpertTemplateCapturePreview {
            participants: resolved
                .participants
                .into_iter()
                .map(|p| p.preview)
                .collect(),
        })
    })
}

fn target_available<S: ApplicationCommandRepositories>(
    store: &mut S,
    context: &str,
    at: i64,
) -> Result<(), String> {
    crate::expert_message::require_context_available(store, context, at)
}
pub(crate) fn require_peer_context_available<S>(
    store: &mut S,
    context: &str,
    at: i64,
) -> Result<(), String>
where
    S: ProductSettingRepositoryProvider
        + RoomPresentationRepository
        + nola_store_port::run_recovery_inspection::RunRecoveryInspectionRepository,
{
    if let Some(id) = read_private::<_, String>(store, &format!("{PRIVATE_PREFIX}slot/{context}"))?
    {
        let record: PeerRecord = read_private(store, &record_key(&id)?)?
            .ok_or("expert_peer_target_slot_record_missing")?;
        let view = &record.material.view;
        let submission =
            crate::agent_room::submission(&store.product_setting_repository(), &view.room_id, &id)?
                .ok_or("expert_peer_target_receipt_missing")?;
        if matches!(
            submission.state,
            crate::agent_room::RoomSubmissionState::Accepted
                | crate::agent_room::RoomSubmissionState::Selected
                | crate::agent_room::RoomSubmissionState::Executing
        ) {
            if submission.state == crate::agent_room::RoomSubmissionState::Executing {
                if let Some(run) = store.inspect_exact_run(&id).map_err(|e| e.to_string())? {
                    if run.conversation_id == context
                        && run.terminal.is_some()
                        && run.status == "sealed"
                    {
                        crate::agent_room::observe_execution(store, &view.room_id, &id, &id, at)?;
                        return Ok(());
                    }
                }
            }
            if matches!(submission.state,crate::agent_room::RoomSubmissionState::Accepted|crate::agent_room::RoomSubmissionState::Selected)
                && store.expert_task_is_terminal_without_run(context,&id,at).map_err(|e|e.to_string())? {
                crate::agent_room::cancel_pending(&mut store.product_setting_repository(),&view.room_id,&id,submission.revision,at)?;
                return Ok(());
            }
            return Err("expert_peer_target_busy".into());
        }
    }
    Ok(())
}
fn chain_scope<S: ApplicationCommandRepositories>(
    store: &mut S,
    scope: &SourceScope,
    input: &PeerExchangeInput,
    id: &str,
    authority: &HostExpertTemplateAuthority,
) -> Result<(String, u32, Vec<String>), String> {
    if let Some(reply) = &input.in_reply_to {
        let prior = load_record(store, reply, authority)?;
        let view = &prior.material.view;
        if view.parent_conversation_id != scope.parent
            || view.source != scope.source
            || view.target != reference(&scope.target)
        {
            return Err("expert_peer_reply_scope_mismatch".into());
        }
        let receipt = crate::agent_room::submission(
            &store.product_setting_repository(),
            &view.room_id,
            reply,
        )?
        .ok_or("expert_peer_reply_receipt_missing")?;
        if receipt.terminal.is_none() {
            return Err("expert_peer_reply_requires_target_terminal".into());
        }
        return Ok((view.root_exchange_id.clone(), view.hop, prior.visited));
    }
    {
        let root: Option<String> = read_private(
            store,
            &format!("{PRIVATE_PREFIX}source-root/{}", input.source_run_id),
        )?;
        if let Some(root) = root {
            let first = load_record(store, &root, authority)?;
            if first.material.view.parent_conversation_id != scope.parent
                || first.material.view.source != scope.source
                || first.material.view.source_run_id != input.source_run_id
            {
                return Err("expert_peer_root_scope_changed".into());
            }
            Ok((
                root,
                1,
                vec![scope.target.id.clone()],
            ))
        } else {
            Ok((
                id.into(),
                1,
                vec![scope.target.id.clone()],
            ))
        }
    }
}
fn count_message<S: ProductSettingRepositoryProvider + RoomPresentationRepository>(
    store: &mut S,
    root: &str,
    parent: &str,
    at: i64,
    allow_new: bool,
) -> Result<(), String> {
    let key = format!("{PRIVATE_PREFIX}chain/{root}");
    let mut chain: Chain = match read_private(store, &key)? {
        Some(chain) => chain,
        None if allow_new => Chain {
            version: 1,
            root_exchange_id: root.into(),
            parent_conversation_id: parent.into(),
            messages: 0,
        },
        None => return Err("expert_peer_chain_record_missing".into()),
    };
    if chain.version != 1
        || chain.root_exchange_id != root
        || chain.parent_conversation_id != parent
        || chain.messages >= 16
    {
        return Err("expert_peer_chain_message_limit".into());
    }
    chain.messages += 1;
    write_private(store, &key, &chain, at)
}
pub fn admit<S: ApplicationCommandRepositories>(
    store: &mut S,
    input: PeerExchangeInput,
    host: &HostExpertTemplateCaptureContext,
    at: i64,
) -> Result<PeerMaterial, String> {
    input.validate()?;
    host.validate()?;
    if at < 0 {
        return Err("expert_peer_invalid_time".into());
    }
    transaction(store, |store| {
        if let Some((record, _)) = prior_call(
            store,
            &input.source_context_id,
            &input.source_run_id,
            &input.call_id,
            "request",
            &input,
            &host.authority(),
        )? {
            return Ok(record.material);
        }
        let scope = source_scope(store, &input, &host.authority())?;
        let projection_input = capture_input(store, &scope, &input)?;
        host.validate_input(&projection_input)?;
        let mut resolved = crate::expert_message::preview::resolve_peer(store, &projection_input)?;
        target_available(store, &scope.target.context_conversation_id, at)?;
        let participant = resolved
            .participants
            .pop()
            .ok_or("expert_peer_target_capture_missing")?;
        let preview = participant.preview;
        let selection = host
            .selections
            .first()
            .ok_or("expert_peer_host_selection_missing")?;
        if selection.model_profile.profile_id != preview.model_declaration.profile_id
            || selection.model_profile.admitted_revision
                != preview.model_declaration.profile_revision
        {
            return Err("expert_peer_capture_model_changed".into());
        }
        let catalog = capture_policy::load_custom_catalog(&store.product_setting_repository())?;
        let mut configuration = preview.defaults.conversation_configuration();
        let selected = capture_policy::resolve_selection(
            host,
            &scope.target.context_conversation_id,
            preview.context_configuration_revision,
            &configuration,
            catalog.as_ref(),
            at,
        )?;
        if let Some(template) = selected.template_id {
            configuration["selectedPromptTemplateID"] = serde_json::json!(template);
        } else {
            configuration
                .as_object_mut()
                .ok_or("expert_peer_invalid_configuration")?
                .remove("selectedPromptTemplateID");
        }
        let id = uuid::Uuid::new_v4().to_string();
        let (root, hop, visited) = chain_scope(store, &scope, &input, &id, &host.authority())?;
        count_message(store, &root, &scope.parent, at, root == id)?;
        let entry = ExpertMessageEntry {
            participant: reference(&scope.target),
            display_name: participant.display_name,
            description: participant.description,
            defaults: nola_domain::persistent_agent::AgentDefaults::try_from(configuration)?,
            model_declaration: preview.model_declaration.clone(),
            context_configuration_revision: preview.context_configuration_revision,
            child_submission_id: id.clone(),
            child_user_message_id: format!("{id}:user"),
            child_assistant_message_id: format!("{id}:assistant"),
            expected_prompt_definition: selected.definition,
            model_witness: None,
            instruction_witness: None,
        };
        let capture = ExpertTemplateCaptureFact {
            version: 1,
            conversation_id: scope.parent.clone(),
            submission_id: id.clone(),
            message_id: format!("{id}:peer"),
            intent_sha256: capture_policy::payload_sha256(&input)?,
            actor: host.actor.clone(),
            request_host_id: host.request_host_id.clone(),
            captured_at_ms: at,
            entries: vec![capture_policy::ExpertTemplateCaptureEntry {
                participant_id: scope.target.id.clone(),
                context_conversation_id: scope.target.context_conversation_id.clone(),
                context_configuration_revision: preview.context_configuration_revision,
                child_submission_id: id.clone(),
                definition_sha256: entry
                    .expected_prompt_definition
                    .as_ref()
                    .map(|d| d.content_sha256.clone()),
                host_user_instructions: selection.host_user_instructions.clone(),
                origin: selected.origin,
            }],
        };
        let definitions = crate::expert_message::capture::validate_peer_entry(&capture, &entry)?;
        let view = PeerView {
            version: 1,
            exchange_id: id.clone(),
            room_id: scope.room.room.id().into(),
            parent_conversation_id: scope.parent.clone(),
            source: scope.source.clone(),
            target: reference(&scope.target),
            source_run_id: input.source_run_id.clone(),
            source_call_id: input.call_id.clone(),
            source_display_name: "Nola".into(),
            target_display_name: scope.target.display_name.clone(),
            kind: input.kind.clone(),
            text: input.text.clone(),
            in_reply_to: input.in_reply_to.clone(),
            root_exchange_id: root,
            hop,
            created_at_ms: at,
            clarifications: Vec::new(),
        };
        let record = PeerRecord {
            input: input.clone(),
            material: PeerMaterial {
                view,
                entry,
                prompt_definitions: definitions,
            },
            preview: ExpertTemplateCapturePreview {
                participants: vec![preview],
            },
            capture,
            visited,
            task_configuration: None,
            task_exchange_context: None,
        };
        authorize(&record, &host.authority())?;
        crate::agent_room::submit(
            &mut store.product_setting_repository(),
            crate::agent_room::SubmitRoomMessage {
                room_id: record.material.view.room_id.clone(),
                submission_id: id.clone(),
                expected_room_revision: scope.room.revision,
                text: input.text.clone(),
                participant_id: Some(scope.target.id.clone()),
                at_ms: at,
                parent_message_id: None,
                conversation_id: None,
            },
        )?;
        write_private(store, &record_key(&id)?, &record, at)?;
        write_private(store, &format!("{PRIVATE_PREFIX}run/{id}"), &id, at)?;
        let source_root_key = format!("{PRIVATE_PREFIX}source-root/{}", input.source_run_id);
        if read_private::<_, String>(store, &source_root_key)?.is_none() {
            write_private(
                store,
                &source_root_key,
                &record.material.view.root_exchange_id,
                at,
            )?;
        }
        write_private(
            store,
            &format!(
                "{PRIVATE_PREFIX}slot/{}",
                scope.target.context_conversation_id
            ),
            &id,
            at,
        )?;
        write_private(
            store,
            &call_key(
                &input.source_context_id,
                &input.source_run_id,
                &input.call_id,
            )?,
            &CallIndex {
                operation: "request".into(),
                input: serde_json::to_value(&input).map_err(|e| e.to_string())?,
                exchange_id: id,
                message_id: None,
            },
            at,
        )?;
        Ok(record.material)
    })
}

pub fn load<S: ApplicationCommandRepositories>(
    store: &mut S,
    room: &str,
    id: &str,
    authority: &HostExpertTemplateAuthority,
) -> Result<PeerMaterial, String> {
    let record = load_record(store, id, authority)?;
    let view = &record.material.view;
    let association = crate::agent_room::for_conversation(store, &view.parent_conversation_id)?
        .ok_or("expert_peer_parent_association_missing")?;
    if view.room_id != room || association.room_id != room {
        return Err("expert_peer_load_scope_mismatch".into());
    }
    Ok(record.material)
}
pub fn load_for_submission<S: ApplicationCommandRepositories>(
    store: &mut S,
    room: &str,
    id: &str,
    authority: &HostExpertTemplateAuthority,
) -> Result<Option<PeerMaterial>, String> {
    authority.validate()?;
    if store
        .read_expert_peer_setting(&record_key(id)?)
        .map_err(|e| e.to_string())?
        .is_none()
    {
        return Ok(None);
    }
    load(store, room, id, authority).map(Some)
}
pub fn clarify<S: ApplicationCommandRepositories>(
    store: &mut S,
    input: PeerClarificationInput,
    authority: &HostExpertTemplateAuthority,
    at: i64,
) -> Result<PeerView, String> {
    let _ = (store, input, authority, at);
    Err("experts_return_results_through_their_task".into())
}

fn visible_record(
    record: PeerRecord,
    parent: &str,
    context: Option<&str>,
) -> Result<PeerView, String> {
    authorize(
        &record,
        &HostExpertTemplateAuthority {
            actor: record.capture.actor.clone(),
            request_host_id: record.capture.request_host_id.clone(),
        },
    )?;
    let view = record.material.view;
    if view.parent_conversation_id != parent
        || context.is_some_and(|context| context != view.target.context_conversation_id)
        || record.material.entry.child_user_message_id != format!("{}:user", view.exchange_id)
    {
        return Err("expert_peer_public_scope_mismatch".into());
    }
    Ok(view)
}
fn public_room<
    R: RoomPresentationRepository + ConversationQueryRepository + ConversationProjectionRepository,
>(
    store: &R,
    parent: &str,
) -> Result<Option<String>, String> {
    if store
        .conversation_summary(parent)
        .map_err(|e| e.to_string())?
        .is_none()
        || store
            .read_room_context_owner(parent)
            .map_err(|e| e.to_string())?
            .is_some()
        || store
            .conversation_execution_ownership(parent)
            .map_err(|e| e.to_string())?
            != nola_domain::ConversationExecutionOwnership::User
        || store
            .conversation_source_owner(parent)
            .map_err(|e| e.to_string())?
            != ConversationSourceOwner::Local
    {
        return Err("expert_peer_public_parent_invalid".into());
    }
    let Some(forward) = store
        .read_room_presentation_setting(RoomPresentationKey::ConversationAssociation(parent))
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let link: serde_json::Value = serde_json::from_str(&forward.value_json)
        .map_err(|_| "expert_peer_parent_association_invalid")?;
    let room = link["room_id"]
        .as_str()
        .ok_or("expert_peer_parent_association_invalid")?;
    let reverse = store
        .read_room_presentation_setting(RoomPresentationKey::RoomAssociation(room))
        .map_err(|e| e.to_string())?
        .ok_or("expert_peer_reverse_association_missing")?;
    let row = store
        .read_room_presentation_setting(RoomPresentationKey::Room(room))
        .map_err(|e| e.to_string())?
        .ok_or("expert_peer_room_missing")?;
    Ok(Some(
        crate::agent_room::validated_association(
            parent,
            &forward,
            &reverse,
            crate::agent_room::room_from_record(room, &row)?,
        )?
        .room_id,
    ))
}

pub fn list_views<
    R: RoomPresentationRepository + ConversationQueryRepository + ConversationProjectionRepository,
>(
    store: &R,
    parent: &str,
    limit: usize,
    offset: usize,
) -> Result<ExpertPeerViews, String> {
    views_for_context_inner(store, parent, None, limit, offset)
}
pub fn views_for_context<
    R: RoomPresentationRepository + ConversationQueryRepository + ConversationProjectionRepository,
>(
    store: &R,
    parent: &str,
    context: &str,
    limit: usize,
    offset: usize,
) -> Result<ExpertPeerViews, String> {
    views_for_context_inner(store, parent, Some(context), limit, offset)
}
fn views_for_context_inner<
    R: RoomPresentationRepository + ConversationQueryRepository + ConversationProjectionRepository,
>(
    store: &R,
    parent: &str,
    context: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<ExpertPeerViews, String> {
    if !(1..=200).contains(&limit) || offset > (i64::MAX as usize).saturating_sub(limit) {
        return Err("expert_peer_invalid_page".into());
    }
    let mut result = PeerExchanges {
        version: 1,
        conversation_id: parent.into(),
        exchanges: Vec::new(),
        has_more: false,
        next_offset: None,
    };
    let Some(room) = public_room(store, parent)? else {
        return Ok(result);
    };
    if let Some(context) = context {
        let owner = store
            .read_room_context_owner(context)
            .map_err(|e| e.to_string())?
            .ok_or("expert_peer_child_owner_missing")?;
        if owner.room_id != room {
            return Err("expert_peer_child_scope_mismatch".into());
        }
    }
    let mut rows = store
        .list_expert_peer_records(parent, context, limit + 1, offset)
        .map_err(|e| e.to_string())?;
    result.has_more = rows.len() > limit;
    rows.truncate(limit);
    for row in rows {
        let record: PeerRecord = serde_json::from_str(&row.value_json)
            .map_err(|_| "expert_peer_private_record_invalid")?;
        if row.key != record_key(&record.material.view.exchange_id)?
            || record.material.view.room_id != room
        {
            return Err("expert_peer_public_record_identity_changed".into());
        }
        result
            .exchanges
            .push(visible_record(record, parent, context)?);
    }
    result.next_offset = result.has_more.then_some(offset + result.exchanges.len());
    Ok(result)
}
pub fn view_for_child_run<
    R: RoomPresentationRepository + ConversationQueryRepository + ConversationProjectionRepository,
>(
    store: &R,
    parent: &str,
    child: &str,
    run_id: &str,
) -> Result<Option<ExpertPeerView>, String> {
    let Some(record): Option<PeerRecord> = read_private(store, &record_key(run_id)?)? else {
        return Ok(None);
    };
    let room = public_room(store, parent)?.ok_or("expert_peer_parent_association_missing")?;
    if record.material.view.room_id != room {
        return Err("expert_peer_child_scope_mismatch".into());
    }
    Ok(Some(visible_record(record, parent, Some(child))?))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerDirectoryMember {
    pub participant: ExpertParticipantReference,
    pub display_name: String,
    pub description: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerDirectory {
    pub version: u32,
    pub parent_conversation_id: String,
    pub room_id: String,
    pub room_revision: i64,
    pub source: nola_domain::expert_task_target::ExpertTaskSourceV1,
    pub members: Vec<PeerDirectoryMember>,
}
pub fn directory<S: ApplicationCommandRepositories>(
    store: &mut S,
    source_context_id: &str,
    source_run_id: &str,
    authority: &HostExpertTemplateAuthority,
) -> Result<PeerDirectory, String> {
    directory_inner(store,source_context_id,source_run_id,authority,false)
}
pub(crate) fn recovery_directory<S:ApplicationCommandRepositories>(store:&mut S,context:&str,run:&str,authority:&HostExpertTemplateAuthority)->Result<PeerDirectory,String>{
    directory_inner(store,context,run,authority,true)
}
fn directory_inner<S:ApplicationCommandRepositories>(store:&mut S,source_context_id:&str,source_run_id:&str,authority:&HostExpertTemplateAuthority,legacy:bool)->Result<PeerDirectory,String>{
    store.with_consistent_read(|reader| {
        let binding = if legacy {legacy_source_binding(reader,source_context_id,source_run_id,authority)?} else {source_binding(reader, source_context_id, source_run_id, authority)?};
        Ok(PeerDirectory {
            version: 1,
            parent_conversation_id: binding.parent,
            room_id: binding.room.room.id().into(),
            room_revision: binding.room.revision,
            source: binding.source,
            members: binding
                .room
                .room
                .participants()
                .iter()
                .filter(|p| p.active)
                .map(|p| PeerDirectoryMember {
                    participant: reference(p),
                    display_name: p.display_name.clone(),
                    description: p.description.clone(),
                })
                .collect(),
        })
    })
}

/// A trusted Host may derive the actual originating capture actor from an exact
/// started expert tool effect. Renderer JSON has no alias for this operation.
pub fn derive_authority<S:ApplicationCommandRepositories>(store:&mut S,request:&nola_agent_engine::agent::host::HostRequestEnvelope,trusted:Option<HostExpertTemplateAuthority>,now:i64)
    ->Result<HostExpertTemplateAuthority,String> {
    let (run,owner,_)=crate::started_host_effect::validate(store,request,"execute_tool",now)?;
    let nola_agent_engine::agent::host::HostRequest::ExecuteTool{capability_id,..}=&request.payload else {return Err("expert_peer_requires_tool_effect".into())};
    if capability_id=="delegate_expert" {
        let authority=trusted.ok_or("main_expert_requires_trusted_host_authority")?;
        source_binding(store,&owner,&run,&authority)?;
        return Ok(authority);
    }
    if capability_id!="expert_exchange" {return Err("expert_peer_requires_exact_tool".into());}
    let authority=if let Some(exchange)=read_private::<_,String>(store,&format!("{PRIVATE_PREFIX}run/{run}"))? {
        let record:PeerRecord=read_private(store,&record_key(&exchange)?)?.ok_or("expert_peer_source_capture_missing")?;
        HostExpertTemplateAuthority{actor:record.capture.actor,request_host_id:record.capture.request_host_id}
    } else {
        let manifest=store.expert_message_for_child(&owner,&run).map_err(|e|e.to_string())?.ok_or("expert_peer_source_capture_missing")?;
        let raw=store.expert_template_capture_fact(&manifest.input.conversation_id,&manifest.input.submission_id).map_err(|e|e.to_string())?.ok_or("expert_peer_source_capture_missing")?;
        let fact:ExpertTemplateCaptureFact=serde_json::from_str(&raw).map_err(|_|"expert_peer_source_capture_invalid")?;
        HostExpertTemplateAuthority{actor:fact.actor,request_host_id:fact.request_host_id}
    };
    legacy_source_binding(store,&owner,&run,&authority)?;
    Ok(authority)
}

pub(crate) fn freeze_task_configuration<S:ApplicationCommandRepositories>(store:&mut S,id:&str,config:&nola_agent_engine::agent::AgentCoreConfig,
    definitions:&[ExpertPromptDefinition],authority:&HostExpertTemplateAuthority,now:i64)->Result<(),String> {
    let mut record=load_record(store,id,authority)?;
    if record.material.prompt_definitions!=definitions {return Err("expert_task_frozen_prompt_changed".into());}
    let config=serde_json::to_value(config).map_err(|e|e.to_string())?;
    if record.task_configuration.as_ref().is_some_and(|original|original!=&config) {return Err("expert_task_frozen_configuration_changed".into());}
    if record.task_exchange_context.is_none() {record.task_exchange_context=Some(record.material.view.clone());}
    record.task_configuration=Some(config);write_private(store,&record_key(id)?,&record,now)
}

/// Recover only the frozen request for this exact started Host effect.
pub fn load_for_effect<S: crate::repositories::ApplicationCommandRepositories>(store:&mut S,request:&nola_agent_engine::agent::host::HostRequestEnvelope,authority:&HostExpertTemplateAuthority,now:i64)->Result<Option<PeerMaterial>,String> {
    let (run,owner,_)=crate::started_host_effect::validate(store,request,"execute_tool",now)?;
    let nola_agent_engine::agent::host::HostRequest::ExecuteTool{capability_id,call_id,input,..}=&request.payload else {return Err("expert_task_requires_tool_effect".into())};
    if !matches!(capability_id.as_str(),"delegate_expert"|"expert_exchange") || input["action"]!="request" || input.as_object().is_none_or(|args|args.keys().any(|key|!matches!(key.as_str(),"action"|"target_participant_id"|"text"|"in_reply_to"))) {return Err("expert_task_requires_exact_request".into());}
    let input=PeerExchangeInput{source_context_id:owner,source_run_id:run,call_id:call_id.clone(),target_participant_id:input.get("target_participant_id").and_then(serde_json::Value::as_str).ok_or("expert_task_target_missing")?.into(),kind:"request".into(),text:input.get("text").and_then(serde_json::Value::as_str).ok_or("expert_task_text_missing")?.into(),in_reply_to:input.get("in_reply_to").and_then(serde_json::Value::as_str).map(str::to_owned)};
    input.validate()?;
    Ok(prior_call(store,&input.source_context_id,&input.source_run_id,&input.call_id,"request",&input,authority)?.map(|(record,_)|record.material))
}
