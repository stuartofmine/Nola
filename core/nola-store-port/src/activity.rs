// SPDX-License-Identifier: Apache-2.0
//! Bounded execution metadata reads. Neither a trace nor an execution grant.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityCursor { pub created_at_ms: i64, pub run_id: String }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityQuery {
    pub source: Option<String>, pub needs_response: bool, pub query: String,
    pub cursor: Option<ActivityCursor>, pub limit: usize,
}
#[derive(Debug, Clone)]
pub struct ActivityRecord {
    pub run_id: String, pub conversation_id: String, pub assistant_message_id: String,
    pub source: String, pub title: String, pub created_at_ms: i64, pub completed_at_ms: Option<i64>,
    pub run_status: String, pub needs_response: bool, pub result_json: Option<String>,
    pub seal_json: Option<String>, pub is_current: bool, pub seal_sources:i64,
    pub execution_host_id: Option<String>,
}
pub trait ActivityRepository {
    type Error: std::fmt::Display;
    fn activity_records(&self, query: &ActivityQuery) -> Result<Vec<ActivityRecord>, Self::Error>;
    fn activity_result_text(&self, conversation_id: &str, run_id: &str, answer_parts: &[String])
        -> Result<(Option<String>, bool), Self::Error>;
    fn activity_error_text(&self, conversation_id: &str, run_id: &str, error_ref: &str)
        -> Result<Option<String>, Self::Error>;
    fn activity_record(&self, conversation_id: &str, run_id: &str) -> Result<Option<ActivityRecord>, Self::Error>;
}
