// SPDX-License-Identifier: Apache-2.0
//! Model-visible UI arguments contain no authority, conversation, Run or Part IDs.
use crate::generative_ui::{UiError, UI_MAX_MESSAGE_BYTES};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const UI_TOOL_ID: &str = "render_ui";
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiToolInput {
    pub expected_revision: u64,
    pub messages: Vec<Value>,
}
impl UiToolInput {
    pub fn validate(&self) -> Result<(), UiError> {
        if self.messages.is_empty() || self.messages.len() > 32 {
            return Err(UiError::ResourceLimit);
        }
        if serde_json::to_vec(self).map_err(|_| UiError::InvalidMessage)?.len() > UI_MAX_MESSAGE_BYTES {
            return Err(UiError::ResourceLimit);
        }
        Ok(())
    }
    pub fn input_schema() -> Value {
        json!({"type":"object","additionalProperties":false,
            "description":"Create or update a conversation UI using A2UI v0.9.1. Use only the advertised Nola catalog. Supply complete protocol messages for one surface. expected_revision is 0 for creation, otherwise the last committed revision returned by this tool. UI publication is not task completion or permission approval. Editable template list items must have unique stable string or numeric id values. Input bindings must not modify those ids or their parent data. Read-only template lists do not require ids. Do not embed HTML, JavaScript or external catalogs.",
            "properties":{
                "expected_revision":{"type":"integer","minimum":0},
                "messages":{"type":"array","minItems":1,"maxItems":32,"items":{"type":"object"}}
            },"required":["expected_revision","messages"]})
    }
}
