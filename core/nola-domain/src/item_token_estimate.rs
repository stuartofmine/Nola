// SPDX-License-Identifier: Apache-2.0
//! Borrowed item content for pure token estimation; no materialized transcript.
#[derive(Debug, Clone, Copy)]
pub enum ItemTokenEstimateFacts<'a> {
    Text(&'a str),
    Attachment {
        uri: &'a str,
        name: &'a str,
        mime_type: Option<&'a str>,
    },
    Reasoning {
        summary: &'a str,
        content: &'a str,
    },
    NamedJson {
        name: &'a str,
        value: &'a serde_json::Value,
    },
}
