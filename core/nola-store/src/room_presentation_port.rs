// SPDX-License-Identifier: Apache-2.0
//! Read on the existing connection/snapshot. No Room repair or authority mutation.
use crate::{NolaStore, StoreError, OptionalExtension};
use nola_store_port::room_presentation::{RoomPresentationKey, RoomPresentationRepository};
use nola_store_port::product_setting::StoredProductSetting;
use nola_domain::agent_room::{ContextOwner, CONTEXT_OWNER_FIELD, ROOM_KEY_PREFIX};

impl RoomPresentationRepository for NolaStore {
    type Error = StoreError;
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
