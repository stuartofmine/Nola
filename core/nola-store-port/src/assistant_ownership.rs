// SPDX-License-Identifier: Apache-2.0
//! Canonical ownership facts read with the same presentation snapshot.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantOwner {
    pub conversation_id: String,
    pub run_id: String,
    pub assistant_message_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantPartOwner {
    pub part_id: String,
    pub turn_id: Option<String>,
    pub owner: AssistantOwner,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantOwnershipFacts {
    pub activities: Vec<AssistantActivityOwner>,
    pub admissions: Vec<AssistantOwner>,
    pub parts: Vec<AssistantPartOwner>,
    pub turns: Vec<AssistantTurnOwner>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantTurnOwner {
    pub turn_id: String,
    pub owner: AssistantOwner,
    pub admission: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantActivityOwner {
    pub kind: String, pub id: String, pub turn_id: String, pub owner: AssistantOwner,
}

/// Narrow changed-item planning reads and same-source durable reservations.
/// Read provenance is not publication or execution authority.
pub trait AssistantJournalPlanningRepository {
    type Error: std::fmt::Display;
    fn bind_tool_source(
        &mut self,
        stamp: &crate::assistant_journal::AssistantSourceCommitStamp,
        identity: &nola_domain::assistant_journal::AssistantJournalIdentity,
        input: &crate::tool_source::ToolSourceInput,
    ) -> Result<(), Self::Error>;
    /// Resolve a real Core Turn without guessing an active/latest Run. Ambiguous
    /// persisted Run membership is an error, including historical collisions.
    fn assistant_owner_for_core_turn(
        &self, conversation_id: &str, turn_id: &str,
    ) -> Result<Option<AssistantOwner>, Self::Error>;
    fn assistant_owner_for_turn(
        &self,
        conversation_id: &str,
        run_id: &str,
        turn_id: &str,
    ) -> Result<Option<AssistantOwner>, Self::Error>;
    fn assistant_owner_for_activity(&self, conversation_id: &str, run_id: &str, kind: &str, id: &str)
        -> Result<Option<AssistantOwner>, Self::Error>;
    fn assistant_owner_for_effect(&self, conversation_id: &str, run_id: &str, effect_id: &str)
        -> Result<Option<AssistantOwner>, Self::Error>;
    fn assistant_owner_for_part(
        &self,
        conversation_id: &str,
        run_id: &str,
        part_id: &str,
    ) -> Result<Option<AssistantPartOwner>, Self::Error>;
    fn reserve_assistant_journal_order(
        &mut self,
        stamp: &crate::assistant_journal::AssistantSourceCommitStamp,
        identity: &nola_domain::assistant_journal::AssistantJournalIdentity,
        section: nola_domain::assistant_journal_operations::JournalSection,
        part_id: &str,
    ) -> Result<nola_domain::assistant_journal_operations::JournalOrderKey, Self::Error>;
    fn model_attempt_successor(
        &self,
        run_id: &str,
        effect_id: &str,
        invocation_id: &str,
        predecessor_id: &str,
    ) -> Result<Option<nola_domain::ModelAttemptRecord>, Self::Error>;
}
