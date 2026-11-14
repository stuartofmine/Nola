// SPDX-License-Identifier: Apache-2.0
//! Read on the existing connection/snapshot. No Room repair or authority mutation.
use crate::{NolaStore, StoreError, OptionalExtension};
use nola_store_port::room_presentation::{RoomPresentationKey, RoomPresentationRepository};
use nola_store_port::product_setting::StoredProductSetting;
use nola_domain::agent_room::{ContextOwner, CONTEXT_OWNER_FIELD, ROOM_KEY_PREFIX};

impl RoomPresentationRepository for NolaStore {
    type Error = StoreError;
    fn read_expert_peer_setting(&self, key: &str) -> Result<Option<StoredProductSetting>, StoreError> {
        if !key.starts_with("expert_peer_private/v1/") || key.len() > 4096 || key.chars().any(char::is_control) {
            return Err(StoreError::Invariant("invalid expert peer key".into()));
        }
        self.product_setting(key)
    }
    fn list_expert_peer_records(&self, parent: &str, context: Option<&str>, limit: usize, offset: usize)
        -> Result<Vec<StoredProductSetting>, StoreError> {
        crate::require_id("expert parent", parent)?;
        if limit == 0 || limit > 201 || offset > i64::MAX as usize {
            return Err(StoreError::Invariant("invalid expert peer page bounds".into()));
        }
        let prefix="expert_peer_private/v1/exchange/";
        let mut statement=self.connection.prepare("SELECT key,value_json,revision,updated_at_ms FROM product_settings
            WHERE substr(key,1,length(?1))=?1 AND json_extract(value_json,'$.material.view.parent_conversation_id')=?2
            AND (?3 IS NULL OR json_extract(value_json,'$.material.view.target.context_conversation_id')=?3)
            ORDER BY json_extract(value_json,'$.material.view.created_at_ms') DESC,key ASC LIMIT ?4 OFFSET ?5")?;
        let values=statement.query_map(rusqlite::params![prefix,parent,context,limit as i64,offset as i64],|row|Ok(
            StoredProductSetting{key:row.get(0)?,value_json:row.get(1)?,revision:row.get(2)?,updated_at_ms:row.get(3)?}
        ))?.collect::<Result<Vec<_>,_>>()?;
        Ok(values)
    }
    fn list_room_contexts(&self, room_id: &str, limit: usize, offset: usize)
        -> Result<Vec<nola_store_port::conversation_query::ConversationSummary>, StoreError> {
        crate::require_id("Room", room_id)?;
        if limit == 0 || limit > 201 || offset > i64::MAX as usize {
            return Err(StoreError::Invariant("invalid expert child page bounds".into()));
        }
        // Filter before paging and never read message bodies for a directory.
        let mut query = self.connection.prepare("SELECT c.id,c.title,c.revision,
            (SELECT count(*) FROM messages m WHERE m.conversation_id=c.id),c.created_at_ms,c.updated_at_ms,
            json_extract(s.settings_json,'$.project_root')
            FROM conversations c JOIN conversation_settings s ON s.conversation_id=c.id
            WHERE json_extract(s.settings_json,'$.agentRoomOwner.room_id')=?1
            ORDER BY c.updated_at_ms DESC,c.created_at_ms DESC,c.id ASC LIMIT ?2 OFFSET ?3")?;
        let values = query.query_map(rusqlite::params![room_id,limit as i64,offset as i64], |row| Ok(
            nola_store_port::conversation_query::ConversationSummary {
                id:row.get(0)?,title:row.get(1)?,revision:row.get(2)?,message_count:row.get(3)?,
                created_at_ms:row.get(4)?,updated_at_ms:row.get(5)?,project_root:row.get(6)?,
                first_user_message:None,last_user_message_at_ms:None,
            }))?.collect::<Result<Vec<_>,_>>()?;
        Ok(values)
    }
    fn expert_context_has_pending_task(&self, context_id: &str) -> Result<bool, StoreError> {
        crate::require_id("Expert context", context_id)?;
        NolaStore::expert_context_has_pending_task(self, context_id)
    }
    fn expert_task_is_terminal_without_run(&self,context_id:&str,task_id:&str,at_ms:i64)->Result<bool,StoreError>{
        NolaStore::expert_task_is_terminal_without_run(self,context_id,task_id,at_ms)
    }
    fn read_room_context_owner(&self, context_id: &str) -> Result<Option<ContextOwner>, StoreError> {
        crate::require_id("Room context", context_id)?;
        let raw: Option<String> = self.connection.query_row(
            "SELECT settings_json FROM conversation_settings WHERE conversation_id=?1", [context_id], |row| row.get(0)).optional()?;
        let fields: serde_json::Value = raw.map(|raw| serde_json::from_str(&raw)).transpose()?.unwrap_or(serde_json::json!({}));
        let fields = fields.as_object().ok_or_else(|| StoreError::Invariant("Room context settings must be an object".into()))?;
        if let Some(owner) = fields.get(CONTEXT_OWNER_FIELD) {
            return Ok(Some(serde_json::from_value(owner.clone())?));
        }
        crate::agent_room_admission::require_unowned_context(&self.connection, context_id)?;
        Ok(None)
    }
    fn read_room_presentation_setting(&self, key: RoomPresentationKey<'_>) -> Result<Option<StoredProductSetting>, StoreError> {
        let key = match key {
            RoomPresentationKey::Room(id) => { crate::require_id("Room", id)?; format!("{ROOM_KEY_PREFIX}{id}") },
            RoomPresentationKey::ConversationAssociation(id) => { crate::require_id("Room conversation", id)?; format!("agent_chat_conversation_room/{id}") },
            RoomPresentationKey::RoomAssociation(id) => { crate::require_id("Room", id)?; format!("agent_chat_room_conversation/{id}") },
        };
        self.product_setting(&key)
    }
}
