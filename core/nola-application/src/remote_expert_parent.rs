// SPDX-License-Identifier: Apache-2.0
//! Authenticated transport receiver for a committed expert-only parent user message.
//! No Run, assistant terminal, prompt definition or execution authority is synthesized.
use crate::repositories::ApplicationCommandRepositories;
use nola_domain::expert_message::AdmitExpertConversationMessage;
use nola_domain::remote_expert_parent::{
    source_key, RemoteExpertParentSource, REMOTE_EXPERT_PARENT_RECEIPT_PREFIX,
};
use nola_domain::{MessagePartView, MessageRole, MessageState, MessageView, TextReviewState};
use nola_store_port::protocol_journal::CommandTransactionRepository;
use nola_store_port::conversation_profile::ConversationProfileRepository;
use nola_store_port::{
    product_setting::ProductSettingRepository,
    transcript_write::{AppendMessagesInput, MessageInput, TranscriptWriteRepository},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteExpertParentAdmission {
    pub version: u32,
    pub source_host_id: String,
    pub source_store_epoch: String,
    pub conversation_id: String,
    pub source_revision: i64,
    #[serde(deserialize_with = "required_nullable_predecessor")]
    pub source_predecessor_message_id: Option<String>,
    pub input: AdmitExpertConversationMessage,
    pub message: MessageView,
}
fn required_nullable_predecessor<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteExpertParentReceipt {
    pub version: u32,
    pub source_host_id: String,
    pub source_store_epoch: String,
    pub conversation_id: String,
    pub message_id: String,
    pub source_revision: i64,
    pub local_revision: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredAdmission {
    admission: RemoteExpertParentAdmission,
    receipt: RemoteExpertParentReceipt,
}
impl RemoteExpertParentAdmission {
    pub fn validate(&self) -> Result<(), String> {
        self.input.validate()?;
        self.source()
            .validate(&self.conversation_id)
            .map_err(str::to_owned)?;
        if self.version != 1
            || self.source_revision <= self.input.expected_revision
            || self.input.conversation_id != self.conversation_id
            || self.message.message_id.0 != self.input.user_message_id
            || self.message.role != MessageRole::User
            || self.message.state != MessageState::Completed
            || self.message.revision == 0 || self.message.revision > self.source_revision as u64
            || self.message.created_at_ms != self.input.created_at_ms
        {
            return Err("remote_expert_parent_admission_owner_mismatch".into());
        }
        match self.message.parts.as_slice() {
            [MessagePartView::Text {
                part_id,
                revision,
                markdown,
                is_final,
                review,
            }] if part_id.0 == format!("{}:text", self.input.user_message_id)
                && *revision > 0 && *revision <= self.source_revision as u64
                && *is_final
                && markdown == &self.input.text
                && matches!(review, None | Some(TextReviewState::Approved)) => {}
            _ => return Err("remote_expert_parent_requires_exact_final_user_text".into()),
        }
        Ok(())
    }
    fn source(&self) -> RemoteExpertParentSource {
        RemoteExpertParentSource {
            version: 1,
            source_host_id: self.source_host_id.clone(),
            source_store_epoch: self.source_store_epoch.clone(),
            conversation_id: self.conversation_id.clone(),
            source_revision: self.source_revision,
        }
    }
}
fn receipt_key(parent: &str, message: &str) -> Result<String, String> {
    Ok(format!(
        "{REMOTE_EXPERT_PARENT_RECEIPT_PREFIX}{}",
        serde_json::to_string(&(parent, message)).map_err(|e| e.to_string())?
    ))
}

pub fn export<S: ApplicationCommandRepositories>(
    store: &mut S,
    parent: String,
    message: String,
    host: String,
    epoch: String,
) -> Result<RemoteExpertParentAdmission, String> {
    store.with_consistent_read(|reader| {
        if reader.assistant_source_epoch().map_err(|e| e.to_string())? != epoch {
            return Err("remote_expert_parent_export_epoch_changed".into());
        }
        if reader
            .conversation_source_owner(&parent)
            .map_err(|e| e.to_string())?
            != nola_domain::ConversationSourceOwner::Local
            || reader
                .read_room_context_owner(&parent)
                .map_err(|e| e.to_string())?
                .is_some()
        {
            return Err("remote_expert_parent_export_requires_owning_host".into());
        }
        let manifest = reader
            .expert_message_manifest(&parent, &message)
            .map_err(|e| e.to_string())?
            .ok_or("expert_manifest_not_found")?;
        let summary = reader
            .conversation_summary(&parent)
            .map_err(|e| e.to_string())?
            .ok_or("conversation_not_found")?;
        // Bound each page; the usual post-admission export finds its user in the tail.
        let mut before = None;
        loop {
            let window = crate::load_conversation_projection_window_v1(
                reader, &parent, before, 50, 2_000_000,
            )
            .map_err(|e| e.to_string())?;
            if let Some(index) = window
                .projection
                .messages
                .iter()
                .position(|m| m.message_id.0 == message)
            {
                let predecessor = if index > 0 {
                    Some(window.projection.messages[index - 1].message_id.0.clone())
                } else if window.has_more {
                    crate::load_conversation_projection_window_v1(
                        reader,
                        &parent,
                        window.before_ordinal,
                        1,
                        262144,
                    )
                    .map_err(|e| e.to_string())?
                    .projection
                    .messages
                    .last()
                    .map(|m| m.message_id.0.clone())
                } else {
                    None
                };
                let value = RemoteExpertParentAdmission {
                    version: 1,
                    source_host_id: host.clone(),
                    source_store_epoch: epoch.clone(),
                    conversation_id: parent.clone(),
                    source_revision: summary.revision,
                    source_predecessor_message_id: predecessor,
                    input: manifest.input.clone(),
                    message: window.projection.messages[index].clone(),
                };
                value.validate()?;
                return Ok(value);
            }
            if !window.has_more {
                return Err("expert_parent_canonical_message_missing".into());
            }
            let next = window
                .before_ordinal
                .ok_or("expert_parent_history_cursor_missing")?;
            if before.is_some_and(|old| next >= old) {
                return Err("expert_parent_history_cursor_not_advancing".into());
            }
            before = Some(next);
        }
    })
}

pub fn receive<S: ApplicationCommandRepositories>(
    store: &mut S,
    expected_host: String,
    expected_epoch: String,
    admission: RemoteExpertParentAdmission,
) -> Result<RemoteExpertParentReceipt, String> {
    admission.validate()?;
    if admission.source_host_id != expected_host || admission.source_store_epoch != expected_epoch {
        return Err("remote_expert_parent_transport_source_mismatch".into());
    }
    store
        .command_protocol_repository()
        .begin_command_mutations()
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let key = receipt_key(&admission.conversation_id, &admission.input.user_message_id)?;
        let previous_receipt = { store.product_setting_repository().product_setting(&key).map_err(|e|e.to_string())? };
        if let Some(row) = previous_receipt {
            let prior: StoredAdmission = serde_json::from_str(&row.value_json)
                .map_err(|_| "remote_expert_parent_receipt_corrupt")?;
            if store
                .conversation_source_owner(&admission.conversation_id)
                .map_err(|e| e.to_string())?
                != admission.source().owner()
            {
                return Err("remote_expert_parent_replay_source_changed".into());
            }
            let mut expected = admission.clone();
            expected.source_revision = prior.admission.source_revision;
            // Source-only user projections may advance their presentation revision
            // with the parent; immutable user identity/body must still agree.
            expected.message.revision = prior.admission.message.revision;
            if let ([MessagePartView::Text { revision, .. }], [MessagePartView::Text { revision: old, .. }]) =
                (expected.message.parts.as_mut_slice(), prior.admission.message.parts.as_slice()) { *revision = *old; }

            if prior.admission != expected {
                return Err("remote_expert_parent_idempotency_conflict".into());
            }
            return Ok(prior.receipt);
        }
        let marker_key = source_key(&admission.conversation_id);
        let marker = store
            .product_setting_repository()
            .product_setting(&marker_key)
            .map_err(|e| e.to_string())?
            .map(|row| {
                serde_json::from_str::<RemoteExpertParentSource>(&row.value_json)
                    .map_err(|_| "remote_expert_parent_source_corrupt".to_owned())
            })
            .transpose()?;
        let summary = store
            .conversation_summary(&admission.conversation_id)
            .map_err(|e| e.to_string())?
            .ok_or("conversation_not_found")?;
        if store
            .read_room_context_owner(&admission.conversation_id)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err("remote_expert_parent_requires_root".into());
        }
        let settings = store
            .load_conversation_settings(&admission.conversation_id)
            .map_err(|e| e.to_string())?
            .ok_or("conversation_settings_missing")?;
        if serde_json::from_str::<serde_json::Value>(&settings)
            .map_err(|e| e.to_string())?
            .get("executionHostBinding")
            .filter(|v| v.is_object())
            .is_none()
        {
            return Err("remote_expert_parent_requires_bound_host".into());
        }
        let source = store
            .conversation_source_owner(&admission.conversation_id)
            .map_err(|e| e.to_string())?;
        if let Some(old) = marker {
            old.validate(&admission.conversation_id)
                .map_err(str::to_owned)?;
            if old.owner() != admission.source().owner()
                || source != old.owner()
                || admission.source_revision <= old.source_revision
            {
                return Err("remote_expert_parent_source_or_revision_conflict".into());
            }
        } else if source != admission.source().owner()
            && (source != nola_domain::ConversationSourceOwner::Local
                || summary.message_count != 0
                || store
                    .conversation_has_admitted_history(&admission.conversation_id)
                    .map_err(|e| e.to_string())?)
        {
            return Err("remote_expert_parent_cannot_relabel_existing_history".into());
        }
        let predecessor = store
            .load_conversation_tail_snapshot(&admission.conversation_id, 1, 1, 1)
            .map_err(|e| e.to_string())?
            .and_then(|tail| tail.conversation.messages.last().map(|m| m.id.clone()));
        if predecessor != admission.source_predecessor_message_id {
            return Err("remote_expert_parent_predecessor_conflict_sync_required".into());
        }
        let (count, last) = store
            .conversation_write_repository()
            .message_frontier(&admission.conversation_id)
            .map_err(|e| e.to_string())?;
        let append = store
            .conversation_write_repository()
            .append_messages(&AppendMessagesInput {
                conversation_id: admission.conversation_id.clone(),
                expected_message_count: count,
                updated_at_ms: admission.input.created_at_ms,
                messages: vec![MessageInput {
                    id: admission.input.user_message_id.clone(),
                    run_id: None,
                    role: "user".into(),
                    status: "completed".into(),
                    ordinal: last
                        .checked_add(1)
                        .ok_or("remote_expert_parent_ordinal_overflow")?,
                    created_at_ms: admission.input.created_at_ms,
                    completed_at_ms: Some(admission.input.created_at_ms),
                    parts: vec![nola_domain::MessagePartInput {
                        id: format!("{}:text", admission.input.user_message_id),
                        kind: "text".into(),
                        ordinal: 1,
                        state: "completed".into(),
                        content_text: admission.input.text.clone(),
                        content_json: None,
                        content_digest: None,
                        created_at_ms: admission.input.created_at_ms,
                        updated_at_ms: admission.input.created_at_ms,
                    }],
                }],
            })
            .map_err(|e| e.to_string())?;
        let receipt = RemoteExpertParentReceipt {
            version: 1,
            source_host_id: admission.source_host_id.clone(),
            source_store_epoch: admission.source_store_epoch.clone(),
            conversation_id: admission.conversation_id.clone(),
            message_id: admission.input.user_message_id.clone(),
            source_revision: admission.source_revision,
            local_revision: append.conversation_revision,
        };
        let stored = StoredAdmission {
            admission: admission.clone(),
            receipt: receipt.clone(),
        };
        store
            .product_setting_repository()
            .put_product_setting(
                &marker_key,
                &serde_json::to_string(&admission.source()).map_err(|e| e.to_string())?,
                admission.input.created_at_ms,
            )
            .map_err(|e| e.to_string())?;
        ensure_context_mirrors(store, &admission, &settings)?;
        store
            .product_setting_repository()
            .put_product_setting(
                &key,
                &serde_json::to_string(&stored).map_err(|e| e.to_string())?,
                admission.input.created_at_ms,
            )
            .map_err(|e| e.to_string())?;
        Ok(receipt)
    })();
    match result {
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
                .map_err(|e| format!("{error}; rollback: {e}"))?;
            Err(error)
        }
    }
}

fn ensure_context_mirrors<S:ApplicationCommandRepositories>(store:&mut S,admission:&RemoteExpertParentAdmission,parent_settings:&str)->Result<(),String>{
    use nola_domain::remote_expert_parent::{RemoteExpertContextSource,context_source_key};
    let settings:serde_json::Value=serde_json::from_str(parent_settings).map_err(|e|e.to_string())?;
    for participant in &admission.input.selected_participants {
        let id=&participant.context_conversation_id;
        let owner=nola_domain::agent_room::ContextOwner {room_id:admission.input.room_id.clone(),participant_id:participant.participant_id.clone(),agent_id:participant.agent_id.clone()};
        let marker=RemoteExpertContextSource{version:1,source_host_id:admission.source_host_id.clone(),source_store_epoch:admission.source_store_epoch.clone(),conversation_id:id.clone(),parent_conversation_id:admission.conversation_id.clone(),room_id:admission.input.room_id.clone(),participant_id:participant.participant_id.clone(),agent_id:participant.agent_id.as_str().into(),agent_revision:participant.agent_revision};
        marker.validate(id).map_err(str::to_owned)?;
        let key=context_source_key(id);
        let previous=store.product_setting_repository().product_setting(&key).map_err(|e|e.to_string())?;
        if let Some(previous)=previous{
            let prior:RemoteExpertContextSource=serde_json::from_str(&previous.value_json).map_err(|_|"remote_expert_context_source_corrupt")?;
            if prior!=marker || store.read_room_context_owner(id).map_err(|e|e.to_string())?.as_ref()!=Some(&owner){return Err("remote_expert_context_source_conflict".into());}
            continue;
        }
        if store.conversation_summary(id).map_err(|e|e.to_string())?.is_some(){return Err("remote_expert_context_cannot_adopt_unproven_existing_context".into());}
        store.conversation_write_repository().create_conversation(&nola_store_port::conversation_profile::CreateConversationInput{
            id:id.clone(),title:"Expert".into(),revision:1,settings_json:Some(serde_json::json!({"agentRoomOwner":owner,"executionHostBinding":settings["executionHostBinding"]}).to_string()),created_at_ms:admission.input.created_at_ms,updated_at_ms:admission.input.created_at_ms,
        }).map_err(|e|e.to_string())?;
        store.product_setting_repository().put_product_setting(&key,&serde_json::to_string(&marker).map_err(|e|e.to_string())?,admission.input.created_at_ms).map_err(|e|e.to_string())?;
    }
    Ok(())
}
