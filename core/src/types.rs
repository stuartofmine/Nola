// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements types logic in the Rust core module.
// - Interfaces:
//   - RawEvent
//   - ProcessedEvent
//   - RulesConfig
//   - NotificationCandidate
//   - SummaryItem
//   - ModelOutputRust
//   - DecisionResult
//   - ParsedTask
// - APIs:
//   - none
//
// Core data types used across the crate

use serde::{Deserialize, Serialize};

/// Minimal RawEvent equivalent for passing JSON back to Swift
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RawEvent {
    pub id: String,
    pub source: String,
    pub title: String,
    pub date: Option<String>,
    pub notes: Option<String>,
    pub metadata: std::collections::HashMap<String, String>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProcessedEvent {
    pub id: String,
    pub source: String,
    pub source_type: String,
    pub source_id: String,
    pub title: String,
    pub date: Option<String>,
    pub start: Option<String>,
    pub due: Option<String>,
    pub priority: f64,
    pub tags: Option<Vec<String>>,
    pub dedupe_key: String,
    pub provenance: Option<std::collections::HashMap<String, String>>,
    pub metadata: std::collections::HashMap<String, String>,
    pub notes: Option<String>,
    pub hash: i64,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RulesConfig {
    pub priority_threshold: f64,
    pub quiet_hours_start: i32,
    pub quiet_hours_end: i32,
    pub throttle_window_hours: i32,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NotificationCandidate {
    pub event: ProcessedEvent,
    pub dedupe_key: String,
    pub text: String,
    pub action_type: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SummaryItem {
    pub text: String,
    pub priority: f64,
    pub dedupe_key: String,
    pub action: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelOutputRust {
    pub items: Vec<SummaryItem>,
    pub explain: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DecisionResult {
    pub save_summary: bool,
    pub summary_text: Option<String>,
    pub summary_event: Option<ProcessedEvent>,
    pub candidates: Vec<NotificationCandidate>,
}

/// JSON schema for input similar to Swift's URLDataResponse
#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct URLDataResponse {
    pub events: Vec<JSONEvent>,
    pub version: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct JSONEvent {
    pub id: String,
    pub r#type: String,
    pub title: String,
    pub date: String,
    pub description: Option<String>,
    pub priority: Option<f64>,
    pub metadata: Option<JSONEventMetadata>,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct JSONEventMetadata {
    pub tags: Option<Vec<String>>,
    pub location: Option<String>,
}

// Task parsing result structure returned to Swift for command parsing
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ParsedTask {
    pub task_type: String,
    pub title: String,
    pub description: Option<String>,
    pub target_date: Option<String>,
    pub is_recurring: Option<bool>,
    pub recurrence_rule: Option<String>,
    pub related_person: Option<String>,
    pub amount: Option<f64>,
    pub currency: Option<String>,
    pub confidence: Option<f64>,
}
