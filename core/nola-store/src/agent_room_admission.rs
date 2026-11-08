// SPDX-License-Identifier: Apache-2.0
//! Recheck room membership inside the actual new-turn admission transaction.
use crate::{Connection, OptionalExtension, StoreError, StoreResult};
use nola_domain::agent_room::{AgentRoom, ContextOwner, CONTEXT_OWNER_FIELD, ROOM_KEY_PREFIX};

/// Room membership retains history even after a participant leaves. Generic
/// deletion, replacement and archive restore cannot detach that history.
pub(crate) fn require_unowned_context(connection: &Connection, context_id: &str) -> StoreResult<()> {
    if is_owned_context(connection, context_id)? {
        return Err(StoreError::Conflict("room participant context cannot be deleted or replaced".into()));
    }
    Ok(())
}

fn is_owned_context(connection: &Connection, context_id: &str) -> StoreResult<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM conversation_settings WHERE conversation_id=?1
            AND json_type(settings_json,'$.agentRoomOwner') IS NOT NULL)
         OR EXISTS(SELECT 1 FROM product_settings s,json_each(s.value_json,'$.participants') p
            WHERE substr(s.key,1,length(?2))=?2
            AND json_extract(p.value,'$.context_conversation_id')=?1)",
        rusqlite::params![context_id, ROOM_KEY_PREFIX], |row| row.get(0))?)
}

fn require_no_missing_owner(connection: &Connection, context_id: &str) -> StoreResult<()> {
    if is_owned_context(connection, context_id)? {
        return Err(StoreError::Conflict("participant context owner missing".into()));
    }
    Ok(())
}

pub(crate) fn require_active_context(connection: &Connection, context_id: &str) -> StoreResult<()> {
    let raw: Option<String> = connection.query_row(
        "SELECT settings_json FROM conversation_settings WHERE conversation_id=?1",
        [context_id], |row| row.get(0)).optional()?;
    let Some(raw) = raw else { return require_no_missing_owner(connection, context_id); };
    let settings: serde_json::Value = serde_json::from_str(&raw)?;
    let settings = settings.as_object().ok_or_else(||
        StoreError::Invariant("conversation settings must be an object".into()))?;
    // Ordinary single-agent conversations do not have room membership.
    let Some(owner) = settings.get(CONTEXT_OWNER_FIELD) else { return require_no_missing_owner(connection, context_id); };
    let owner: ContextOwner = serde_json::from_value(owner.clone())?;
    let room: Option<String> = connection.query_row(
        "SELECT value_json FROM product_settings WHERE key=?1",
        [format!("{ROOM_KEY_PREFIX}{}", owner.room_id)], |row| row.get(0)).optional()?;
    let room: AgentRoom = serde_json::from_str(&room.ok_or_else(||
        StoreError::Conflict("participant room no longer exists".into()))?)?;
    let member = room.recipient(&owner.participant_id).map_err(StoreError::Conflict)?;
    if room.id() != owner.room_id || member.context_conversation_id != context_id
        || member.agent_id != owner.agent_id {
        return Err(StoreError::Conflict("participant context owner mismatch".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NolaStore;
    use nola_domain::agent_room::RoomParticipant;
    use serde_json::json;

    #[test]
    fn admission_rechecks_current_membership_and_context_not_a_stale_ui_selection() {
        let mut store = NolaStore::open(":memory:").unwrap();
        store.create_conversation(&crate::CreateConversationInput {
            id: "context".into(), title: "A".into(), revision: 1,
            settings_json: Some(json!({CONTEXT_OWNER_FIELD:{"room_id":"room", "participant_id":"a", "agent_id":"agent"}}).to_string()),
            created_at_ms: 1, updated_at_ms: 1,
        }).unwrap();
        assert!(require_active_context(&store.connection, "context").is_err());
        let room = AgentRoom::new("room".into(), "Room".into(), vec![RoomParticipant {
            id: "a".into(), agent_id: "agent".to_string().try_into().unwrap(), agent_revision: 1,
            display_name: "A".into(), description: "Research".into(), context_conversation_id: "context".into(), active: true,
        }]).unwrap();
        store.put_product_setting("agent_chat_room/room", &serde_json::to_string(&room).unwrap(), 1).unwrap();
        require_active_context(&store.connection, "context").unwrap();
        assert!(store.delete_conversation("context").unwrap_err().to_string().contains("cannot be deleted"));
        let expanded = room.add(RoomParticipant {
            id: "b".into(), agent_id: "other-agent".to_string().try_into().unwrap(),
            agent_revision: 1, display_name: "B".into(), description: "Research".into(), context_conversation_id: "b-context".into(), active: true,
        }).unwrap();
        store.put_product_setting("agent_chat_room/room", &serde_json::to_string(&expanded).unwrap(), 2).unwrap();
        // A room-wide revision change is not a change to A's recipient/configuration.
        require_active_context(&store.connection, "context").unwrap();
        let mut foreign = room.participants()[0].clone();
        foreign.context_conversation_id = "other-context".into();
        let mismatched = AgentRoom::new("room".into(), "Room".into(), vec![foreign]).unwrap();
        store.put_product_setting("agent_chat_room/room", &serde_json::to_string(&mismatched).unwrap(), 2).unwrap();
        assert!(require_active_context(&store.connection, "context").unwrap_err().to_string().contains("owner mismatch"));
        let inactive = room.deactivate("a").unwrap();
        store.put_product_setting("agent_chat_room/room", &serde_json::to_string(&inactive).unwrap(), 2).unwrap();
        assert!(require_active_context(&store.connection, "context").unwrap_err().to_string().contains("inactive"));
        assert!(store.delete_conversation("context").is_err());
        assert!(store.load_conversation("context").unwrap().is_some());
        // Ordinary contexts remain valid; an inactive room context is not
        // silently transformed into an ordinary one.
        require_active_context(&store.connection, "ordinary").unwrap();
        require_unowned_context(&store.connection, "ordinary").unwrap();
        // The forward membership also protects history if reverse settings
        // are missing or corrupted; deletion must not silently repair it.
        store.connection.execute("UPDATE conversation_settings SET settings_json='{}' WHERE conversation_id='context'", []).unwrap();
        assert!(store.delete_conversation("context").is_err());
        assert!(require_active_context(&store.connection, "context").unwrap_err().to_string().contains("owner missing"));
        store.connection.execute("UPDATE conversation_settings SET settings_json='[]' WHERE conversation_id='context'", []).unwrap();
        assert!(require_active_context(&store.connection, "context").unwrap_err().to_string().contains("must be an object"));
    }
}
