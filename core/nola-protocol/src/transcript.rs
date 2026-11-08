// SPDX-License-Identifier: Apache-2.0
//! Shared finalized transcript wire values. No Store or Application dependency.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TranscriptMessageInput {
    pub id: String,
    pub run_id: Option<String>,
    pub role: String,
    pub status: String,
    pub ordinal: i64,
    pub created_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub parts: Vec<TranscriptPartInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TranscriptPartInput {
    pub id: String,
    pub kind: String,
    pub ordinal: i64,
    pub state: String,
    pub content_text: String,
    pub content_json: Option<String>,
    pub content_digest: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl TranscriptMessageInput {
    pub fn validate_finalized(&self) -> Result<(), String> {
        validate_identity("message", &self.id)?;
        if let Some(run_id) = &self.run_id {
            validate_identity("source Run", run_id)?;
        }
        if !matches!(self.role.as_str(), "user" | "assistant" | "system") {
            return Err(format!(
                "unsupported replacement message role {}",
                self.role
            ));
        }
        if self.status != "completed" || self.completed_at_ms.is_none() {
            return Err("replacement message must be completed before it is committed".into());
        }
        if self
            .completed_at_ms
            .is_some_and(|time| time < self.created_at_ms)
        {
            return Err("message completion cannot precede creation".into());
        }
        if self.parts.is_empty() || self.parts.iter().any(|part| part.state != "completed") {
            return Err("replacement message Parts must be non-empty and completed".into());
        }
        if self.parts.len() > 512 {
            return Err("finalized message exceeds the 512 Part limit".into());
        }
        let mut identities = std::collections::HashSet::new();
        for (offset, part) in self.parts.iter().enumerate() {
            validate_identity("Part", &part.id)?;
            if !identities.insert(part.id.as_str()) {
                return Err("finalized message Part IDs must be unique".into());
            }
            if part.ordinal != offset as i64 + 1 {
                return Err("finalized message Part ordinals must be contiguous".into());
            }
            if part.updated_at_ms < part.created_at_ms {
                return Err("Part update cannot precede creation".into());
            }
        }
        Ok(())
    }
}

fn validate_identity(kind: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > 128
        || value.chars().any(char::is_control)
    {
        return Err(format!("invalid finalized {kind} identity"));
    }
    Ok(())
}
