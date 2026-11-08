// SPDX-License-Identifier: Apache-2.0
//! Atomic conversation deletion contract, independent of the database adapter.

pub trait ConversationDeletionRepository {
    type Error: std::fmt::Display;

    /// Returns false when absent. Rejects any nonterminal Run in the same
    /// write unit that deletes the conversation and publishes its tombstone.
    /// An earlier idle check is not a reservation. Publication failure must
    /// leave the conversation, its children and its revision unchanged.
    fn delete_conversation(&mut self, conversation_id: &str) -> Result<bool, Self::Error>;
}
