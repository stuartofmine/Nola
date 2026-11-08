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
    /// Absence must also prove no Room still names this context.
    fn read_room_context_owner(&self, context_id: &str) -> Result<Option<ContextOwner>, Self::Error>;
    fn read_room_presentation_setting(&self, key: RoomPresentationKey<'_>) -> Result<Option<StoredProductSetting>, Self::Error>;
}
