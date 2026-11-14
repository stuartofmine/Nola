// SPDX-License-Identifier: Apache-2.0
//! Metadata-only children; Room ownership survives explicit member replacement.
use super::*;
use nola_store_port::{consistent_read::ConsistentReadRepository, conversation_query::ConversationQueryRepository,
    conversation_history::ConversationProjectionRepository, room_presentation::{RoomPresentationRepository, RoomPresentationKey}};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExpertChildConversation {
    pub conversation_id: String,
    pub revision: i64,
    pub title: String,
    pub participant_id: String,
    pub agent_id: AgentId,
    pub agent_revision: Option<i64>,
    pub is_current: bool,
    pub active: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub message_count: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExpertChildConversations {
    pub version: u32,
    pub conversation_id: String,
    pub room_id: Option<String>,
    pub room_revision: Option<i64>,
    pub children: Vec<ExpertChildConversation>,
    pub has_more: bool,
    pub next_offset: Option<usize>,
}
pub fn child_conversations<S>(store:&mut S, conversation_id:String, limit:usize, offset:usize)
    -> Result<ExpertChildConversations,String>
where S: ConsistentReadRepository + RoomPresentationRepository + ConversationQueryRepository + ConversationProjectionRepository {
    if !(1..=200).contains(&limit) || offset > (i64::MAX as usize).saturating_sub(limit) {
        return Err("invalid_expert_child_page_bounds".into());
    }
    store.with_consistent_read(|reader| {
        if reader.conversation_summary(&conversation_id).map_err(|e|e.to_string())?.is_none() {
            return Err("conversation_not_found".into());
        }
        if reader.read_room_context_owner(&conversation_id).map_err(|e|e.to_string())?.is_some()
            || reader.conversation_execution_ownership(&conversation_id).map_err(|e|e.to_string())? != nola_domain::ConversationExecutionOwnership::User {
            return Err("expert_children_require_root_conversation".into());
        }
        let source=reader.conversation_source_owner(&conversation_id).map_err(|e|e.to_string())?;
        if source == nola_domain::ConversationSourceOwner::Unavailable {return Err("expert_child_source_unavailable".into());}
        let mut result=ExpertChildConversations{version:1,conversation_id:conversation_id.clone(),room_id:None,room_revision:None,
            children:Vec::new(),has_more:false,next_offset:None};
        let Some(forward)=reader.read_room_presentation_setting(RoomPresentationKey::ConversationAssociation(&conversation_id)).map_err(|e|e.to_string())? else {return Ok(result)};
        let link=decode_link(&forward.value_json)?;
        let reverse=reader.read_room_presentation_setting(RoomPresentationKey::RoomAssociation(&link.room_id)).map_err(|e|e.to_string())?.ok_or("expert_child_reverse_association_missing")?;
        let record=reader.read_room_presentation_setting(RoomPresentationKey::Room(&link.room_id)).map_err(|e|e.to_string())?.ok_or("expert_child_room_missing")?;
        let association=validated_association(&conversation_id,&forward,&reverse,room_from_record(&link.room_id,&record)?)?;
        let mut page=reader.list_room_contexts(&link.room_id,limit+1,offset).map_err(|e|e.to_string())?;
        result.has_more=page.len()>limit;
        page.truncate(limit);
        for context in page {
            let owner=reader.read_room_context_owner(&context.id).map_err(|e|e.to_string())?.ok_or("expert_child_owner_missing")?;
            if owner.room_id!=link.room_id || context.id==conversation_id
                || reader.conversation_source_owner(&context.id).map_err(|e|e.to_string())? != source {
                return Err("expert_child_owner_or_source_mismatch".into());
            }
            let current=association.room.room.participants().iter().find(|member|member.id==owner.participant_id);
            if current.is_some_and(|member|member.context_conversation_id!=context.id || member.agent_id!=owner.agent_id) {
                return Err("expert_child_member_binding_mismatch".into());
            }
            result.children.push(ExpertChildConversation {conversation_id:context.id,revision:context.revision,title:context.title,
                participant_id:owner.participant_id,agent_id:owner.agent_id,agent_revision:current.map(|m|m.agent_revision),
                is_current:current.is_some(),active:current.is_some_and(|m|m.active),created_at_ms:context.created_at_ms,
                updated_at_ms:context.updated_at_ms,message_count:context.message_count});
        }
        result.room_id=Some(link.room_id);
        result.room_revision=Some(association.room.revision);
        result.next_offset=result.has_more.then_some(offset+result.children.len());
        Ok(result)
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpertChildHistory {
    pub version: u32,
    pub conversation_id: String,
    pub child_conversation_id: String,
    pub room_id: String,
    pub room_revision: i64,
    pub source_owner: nola_domain::ConversationSourceOwner,
    pub window: crate::ConversationProjectionWindowV1,
    pub expert_exchanges: Vec<crate::expert_peer::ExpertPeerView>,
}
pub fn child_history<S>(store:&mut S, parent:String, child:String, before:Option<i64>, limit:usize, bytes:usize)
    -> Result<ExpertChildHistory,String>
where S: ConsistentReadRepository + RoomPresentationRepository + ConversationQueryRepository + ConversationProjectionRepository {
    if !(1..=200).contains(&limit) || !(1..=2_000_000).contains(&bytes) || before.is_some_and(|v|v<=0) {
        return Err("invalid_expert_history_window_bounds".into());
    }
    store.with_consistent_read(|reader| {
        if parent==child || reader.read_room_context_owner(&parent).map_err(|e|e.to_string())?.is_some()
            || reader.conversation_execution_ownership(&parent).map_err(|e|e.to_string())? != nola_domain::ConversationExecutionOwnership::User {
            return Err("expert_history_requires_root_parent".into());
        }
        let owner=reader.read_room_context_owner(&child).map_err(|e|e.to_string())?.ok_or("expert_history_child_owner_missing")?;
        let forward=reader.read_room_presentation_setting(RoomPresentationKey::ConversationAssociation(&parent)).map_err(|e|e.to_string())?.ok_or("expert_history_parent_association_missing")?;
        let reverse=reader.read_room_presentation_setting(RoomPresentationKey::RoomAssociation(&owner.room_id)).map_err(|e|e.to_string())?.ok_or("expert_history_reverse_association_missing")?;
        let row=reader.read_room_presentation_setting(RoomPresentationKey::Room(&owner.room_id)).map_err(|e|e.to_string())?.ok_or("expert_history_room_missing")?;
        let association=validated_association(&parent,&forward,&reverse,room_from_record(&owner.room_id,&row)?)?;
        if let Some(member)=association.room.room.participants().iter().find(|p|p.id==owner.participant_id) {
            if member.context_conversation_id!=child || member.agent_id!=owner.agent_id {return Err("expert_history_child_binding_mismatch".into());}
        }
        let source=reader.conversation_source_owner(&parent).map_err(|e|e.to_string())?;
        if source==nola_domain::ConversationSourceOwner::Unavailable
            || reader.conversation_source_owner(&child).map_err(|e|e.to_string())?!=source {
            return Err("expert_history_source_mismatch".into());
        }
        let window=crate::load_conversation_projection_window_v1(reader,&child,before,limit,bytes).map_err(|e|e.to_string())?;
        let mut exchanges=Vec::new();
        for message in &window.projection.messages {
            if message.role!=nola_domain::MessageRole::User {continue;}
            let Some(run)=message.message_id.0.as_str().strip_suffix(":user") else {continue};
            if uuid::Uuid::parse_str(run).is_err() {continue;}
            if let Some(view)=crate::expert_peer::view_for_child_run(reader,&parent,&child,run)? {
                exchanges.push(view);
            }
        }
        Ok(ExpertChildHistory {version:1,conversation_id:parent,child_conversation_id:child,room_id:owner.room_id,
            room_revision:association.room.revision,source_owner:source,window,expert_exchanges:exchanges})
    })
}
