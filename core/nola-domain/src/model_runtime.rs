// SPDX-License-Identifier: Apache-2.0
//! Platform-neutral declared model execution capabilities.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeModelCapabilities {
    pub instruction_profile: Option<String>,
    pub native_tool_calls: Option<bool>,
    pub parallel_tool_calls: Option<bool>,
    pub reasoning: Option<bool>,
    /// Explicit opt-in support for the provider public-summary request parameter.
    pub reasoning_summary_supported: Option<bool>,
    /// Full declared controls for explicit catalog initialization, never inferred
    /// from this turn's selected effort or budget.
    pub reasoning_declaration: Option<crate::reasoning_tiers::ReasoningCapabilityDeclaration>,
    pub reasoning_continuity: Option<bool>,
    pub vision_input: Option<bool>,
    pub native_pdf_input: Option<bool>,
    pub request_continuation: Option<bool>,
    pub max_request_continuations: Option<u8>,
    pub stream_text: Option<bool>,
    pub stream_reasoning: Option<bool>,
    pub stream_tool_calls: Option<bool>,
}

impl RuntimeModelCapabilities {
    pub fn is_empty(&self) -> bool {
        self.instruction_profile.is_none()
            && self.native_tool_calls.is_none()
            && self.parallel_tool_calls.is_none()
            && self.reasoning.is_none()
            && self.reasoning_summary_supported.is_none()
            && self.reasoning_declaration.is_none()
            && self.reasoning_continuity.is_none()
            && self.vision_input.is_none()
            && self.native_pdf_input.is_none()
            && self.request_continuation.is_none()
            && self.max_request_continuations.is_none()
            && self.stream_text.is_none()
            && self.stream_reasoning.is_none()
            && self.stream_tool_calls.is_none()
    }
}
