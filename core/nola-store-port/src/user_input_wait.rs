// SPDX-License-Identifier: Apache-2.0
//! Committed questions for presentation and exact answer-owner validation.
//! Writes remain part of the canonical Core transition, never a second runtime.
use nola_domain::user_input_wait::UserInputWait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredUserInputWait {
    pub conversation_id: String,
    pub run_id: String,
    pub wait: UserInputWait,
}

/// A parent's read view retains the child's original wait owner; it is not a
/// new parent question or execution/permission grant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnedTaskUserInputWait {
    pub task_id: String,
    #[serde(flatten)]
    pub stored: StoredUserInputWait,
}

pub trait UserInputWaitRepository {
    type Error: std::fmt::Display;
    /// Read within the response command's write unit of work. A committed Task
    /// cancellation prevents new answers, but does not erase prior receipts.
    fn user_input_wait_task_cancel_requested(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<bool, Self::Error>;
    fn owned_task_user_input_waits(
        &self,
        owner_conversation_id: &str,
        task_id: Option<&str>,
    ) -> Result<Vec<OwnedTaskUserInputWait>, Self::Error>;
    fn load_user_input_wait(
        &self,
        conversation_id: &str,
        wait_id: &str,
    ) -> Result<Option<StoredUserInputWait>, Self::Error>;
    fn open_user_input_waits(
        &self,
        conversation_id: &str,
        run_id: Option<&str>,
    ) -> Result<Vec<StoredUserInputWait>, Self::Error>;
}
