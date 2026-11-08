// SPDX-License-Identifier: Apache-2.0
//! Bounded, non-authorizing guidance attached to a permission request.
//!
//! The value is shared by Host transport and persistence.  Keeping its
//! identity and validation in Domain prevents the SQLite adapter from
//! depending on the transport crate merely to validate a stored annotation.

use serde::{Deserialize, Serialize};

/// Supplemental user guidance; it never grants permission and never carries
/// tool business input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionInstruction {
    pub permission_request_id: String,
    pub instruction: String,
}

impl PermissionInstruction {
    /// Validates one annotation before it is appended to a pending request.
    pub fn validate_append(
        existing: &[Self],
        request_id: &str,
        instruction: &str,
    ) -> Result<(), String> {
        let instruction = instruction.trim();
        if instruction.is_empty() {
            return Err("permission instruction must not be empty".into());
        }
        let total = existing
            .iter()
            .fold(0usize, |sum, entry| {
                sum.saturating_add(entry.instruction.len())
                    .saturating_add(entry.permission_request_id.len())
            })
            .saturating_add(instruction.len())
            .saturating_add(request_id.len());
        if instruction.len() > 4096 || existing.len() >= 16 || total > 16384 {
            return Err("permission instruction exceeds the bounded guidance budget".into());
        }
        Ok(())
    }
}
