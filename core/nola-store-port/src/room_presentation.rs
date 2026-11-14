// SPDX-License-Identifier: Apache-2.0
//! Narrow immutable Room membership/association input for presentation reads.
use nola_domain::agent_room::ContextOwner;
use crate::product_setting::StoredProductSetting;

pub enum RoomPresentationKey<'a> {
    Room(&'a str),
    ConversationAssociation(&'a str),
    RoomAssociation(&'a str),
}
pub trait RoomPresentationRepository {
    type Error: std::fmt::Display;
    /// Ownership-filtered metadata page, including retired contexts after rejoin.
    fn list_room_contexts(&self, room_id: &str, limit: usize, offset: usize)
        -> Result<Vec<crate::conversation_query::ConversationSummary>, Self::Error>;
    fn read_expert_peer_setting(&self, key: &str) -> Result<Option<StoredProductSetting>, Self::Error>;
    fn list_expert_peer_records(&self, parent: &str, context: Option<&str>, limit: usize, offset: usize)
        -> Result<Vec<StoredProductSetting>, Self::Error>;
    /// Pending common Agent Tasks reserve the same persistent expert context.
    fn expert_context_has_pending_task(&self, context_id: &str) -> Result<bool, Self::Error>;
    /// Canonical failed/cancelled Task receipt with a Core-frozen expert target and no Run.
    fn expert_task_is_terminal_without_run(&self, _context_id:&str, _task_id:&str, _at_ms:i64)->Result<bool,Self::Error>{Ok(false)}
    /// Absence must also prove no Room still names this context.
    fn read_room_context_owner(&self, context_id: &str) -> Result<Option<ContextOwner>, Self::Error>;
    fn read_room_presentation_setting(&self, key: RoomPresentationKey<'_>) -> Result<Option<StoredProductSetting>, Self::Error>;
}
