// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Classifies the terminal state of one provider model request.
// - Boundary: Never creates follow-up prompts or makes Agent decisions.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IncompleteModelOutput {
    OutputLimit,
    TransportInterrupted,
}

pub(crate) fn incomplete_model_output(finish_reason: &str) -> Option<IncompleteModelOutput> {
    match finish_reason.trim().to_ascii_lowercase().as_str() {
        "length" | "max_tokens" | "max_output_tokens" | "max_output_bytes" | "token_limit" => {
            Some(IncompleteModelOutput::OutputLimit)
        }
        "stream_incomplete" => Some(IncompleteModelOutput::TransportInterrupted),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{incomplete_model_output, IncompleteModelOutput};

    #[test]
    fn classifies_output_limit_without_requesting_a_continuation() {
        assert_eq!(
            incomplete_model_output("length"),
            Some(IncompleteModelOutput::OutputLimit)
        );
        assert_eq!(
            incomplete_model_output("max_tokens"),
            Some(IncompleteModelOutput::OutputLimit)
        );
        assert_eq!(
            incomplete_model_output("max_output_bytes"),
            Some(IncompleteModelOutput::OutputLimit)
        );
    }

    #[test]
    fn classifies_interrupted_transport_separately() {
        assert_eq!(
            incomplete_model_output("stream_incomplete"),
            Some(IncompleteModelOutput::TransportInterrupted)
        );
    }

    #[test]
    fn accepts_provider_terminal_completion_regardless_of_markdown_shape() {
        assert_eq!(incomplete_model_output("stop"), None);
        assert_eq!(incomplete_model_output("tool_calls"), None);
    }
}
