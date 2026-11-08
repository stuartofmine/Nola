// SPDX-License-Identifier: Apache-2.0
// A model response is an atomic planning batch, not an unbounded execution queue.

/// Bounds one response, not the number of tools allowed in a user turn and not
/// scheduler concurrency. Further work must be planned using earlier results.
pub const MAX_MODEL_TOOL_CALLS_PER_BATCH: usize = 32;
pub const MODEL_TOOL_BATCH_LIMIT_EXCEEDED: &str = "model_tool_batch_limit_exceeded";

pub fn validate_model_tool_batch_size(count: usize) -> Result<(), &'static str> {
    if count > MAX_MODEL_TOOL_CALLS_PER_BATCH {
        Err(MODEL_TOOL_BATCH_LIMIT_EXCEEDED)
    } else {
        Ok(())
    }
}

pub fn model_tool_batch_guidance() -> String {
    format!(
        "Issue at most {MAX_MODEL_TOOL_CALLS_PER_BATCH} native tool calls in one model response. \
         Plan a bounded set of independent operations, inspect their actual results, then plan the next batch. \
         A response exceeding this capacity is rejected as a whole before execution; none of its calls ran. \
         Do not move an oversized batch into a shell loop or another wrapper to bypass this boundary."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_capacity_has_an_inclusive_boundary() {
        assert!(validate_model_tool_batch_size(0).is_ok());
        assert!(validate_model_tool_batch_size(MAX_MODEL_TOOL_CALLS_PER_BATCH).is_ok());
        assert_eq!(validate_model_tool_batch_size(MAX_MODEL_TOOL_CALLS_PER_BATCH + 1),
            Err(MODEL_TOOL_BATCH_LIMIT_EXCEEDED));
    }
}
