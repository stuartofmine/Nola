// SPDX-License-Identifier: Apache-2.0
//! Immutable scheduled occurrence presentation; no lease or execution capability.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueuedAutomationFact {
    pub occurrence_id: String,
    pub automation_id: String,
    pub definition_revision: i64,
    /// Present only when the currently stored definition is the admitted revision.
    pub title: Option<String>,
    pub scheduled_for_ms: i64,
}

pub trait AutomationFactsRepository {
    type Error: std::fmt::Display;
    /// Apply queued filtering before ordering/limiting. Join the caller's snapshot.
    fn queued_automation_facts(&self, limit: usize) -> Result<Vec<QueuedAutomationFact>, Self::Error>;
}
