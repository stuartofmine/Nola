// SPDX-License-Identifier: Apache-2.0
//! Typed execution choices are task conditions, not permission grants or routing.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionConstraintDomain { BrowserTarget }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserExecutionTarget { Managed, Attached }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionTargetSelection {
    Required { target: BrowserExecutionTarget },
    Unconstrained,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTargetRole { Action, Diagnostic }

/// Host-owned semantics for one concrete, schema-validated invocation.
/// The capability name and natural language are not inputs to this projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionTargetProjection {
    pub domain: ExecutionConstraintDomain,
    pub argument: String,
    pub default_target: BrowserExecutionTarget,
    pub role: ExecutionTargetRole,
}

impl ExecutionTargetProjection {
    pub fn resolve(&self, input: &serde_json::Value) -> Result<BrowserExecutionTarget, String> {
        if self.argument.is_empty() || self.argument.trim() != self.argument {
            return Err("invalid Host execution target projection".into());
        }
        match input.get(&self.argument) {
            None => Ok(self.default_target),
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|_| "invocation does not select a valid declared execution target".into()),
        }
    }
}
