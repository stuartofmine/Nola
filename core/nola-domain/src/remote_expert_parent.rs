// SPDX-License-Identifier: Apache-2.0
//! Source ownership of expert-only parent messages which deliberately have no Run.
use serde::{Deserialize, Serialize};
pub const REMOTE_EXPERT_PARENT_SOURCE_PREFIX: &str = "remote_expert_parent_source/v1/";
pub const REMOTE_EXPERT_PARENT_RECEIPT_PREFIX: &str = "remote_expert_parent_admission/v1/";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteExpertParentSource {
    pub version: u32,
    pub source_host_id: String,
    pub source_store_epoch: String,
    pub conversation_id: String,
    pub source_revision: i64,
}
impl RemoteExpertParentSource {
    pub fn validate(&self, conversation: &str) -> Result<(), &'static str> {
        fn id(value: &str) -> bool {
            !value.is_empty()
                && value.trim() == value
                && value.len() <= 512
                && !value.chars().any(char::is_control)
        }
        if self.version != 1
            || self.conversation_id != conversation
            || !id(conversation)
            || !id(&self.source_host_id)
            || !id(&self.source_store_epoch)
            || self.source_revision < 1
        {
            return Err("invalid_remote_expert_parent_source");
        }
        Ok(())
    }
    pub fn owner(&self) -> crate::ConversationSourceOwner {
        crate::ConversationSourceOwner::Remote {
            source_host_id: self.source_host_id.clone(),
            source_store_epoch: self.source_store_epoch.clone(),
        }
    }
}
pub fn source_key(conversation: &str) -> String {
    format!("{REMOTE_EXPERT_PARENT_SOURCE_PREFIX}{conversation}")
}
pub fn owns_setting(key: &str) -> bool {
    key.starts_with(REMOTE_EXPERT_CONTEXT_SOURCE_PREFIX) || key.starts_with(REMOTE_EXPERT_PARENT_SOURCE_PREFIX)
        || key.starts_with(REMOTE_EXPERT_PARENT_RECEIPT_PREFIX)
}

pub const REMOTE_EXPERT_CONTEXT_SOURCE_PREFIX:&str="remote_expert_context_source/v1/";
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteExpertContextSource {
    pub version:u32,pub source_host_id:String,pub source_store_epoch:String,pub conversation_id:String,
    pub parent_conversation_id:String,pub room_id:String,pub participant_id:String,pub agent_id:String,pub agent_revision:i64,
}
impl RemoteExpertContextSource {
    pub fn validate(&self,conversation:&str)->Result<(),&'static str>{
        if self.version!=1||self.conversation_id!=conversation||self.parent_conversation_id==conversation||self.agent_revision<1
            ||[conversation,self.source_host_id.as_str(),self.source_store_epoch.as_str(),self.parent_conversation_id.as_str(),self.room_id.as_str(),self.participant_id.as_str(),self.agent_id.as_str()].iter().any(|v|v.is_empty()||v.trim()!=*v||v.len()>512||v.chars().any(char::is_control)) {return Err("invalid_remote_expert_context_source");}Ok(())
    }
    pub fn owner(&self)->crate::ConversationSourceOwner{crate::ConversationSourceOwner::Remote{source_host_id:self.source_host_id.clone(),source_store_epoch:self.source_store_epoch.clone()}}
}
pub fn context_source_key(conversation:&str)->String{format!("{REMOTE_EXPERT_CONTEXT_SOURCE_PREFIX}{conversation}")}
