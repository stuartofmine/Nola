// SPDX-License-Identifier: Apache-2.0
//! Cloud PII/secret redactor with automatic bypass for loopback & private LAN endpoints.

use std::net::IpAddr;
use std::sync::OnceLock;

use nola_types::{HistoryMessage, StructuredModelRequest, ToolModelPayload};
use regex::Regex;
use url::Url;

/// Returns true when the target URL resolves to loopback or RFC1918/ULA private LAN.
pub fn is_local_or_lan_endpoint(endpoint: &str) -> bool {
    let Ok(url) = Url::parse(endpoint) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let lower = host.trim_end_matches('.').to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") || lower.ends_with(".local") {
        return true;
    }
    let clean = lower.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = clean.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
            IpAddr::V6(v6) => {
                v6.is_loopback()
                    || (v6.segments()[0] & 0xfe00) == 0xfc00
                    || (v6.segments()[0] & 0xffc0) == 0xfe80
            }
        };
    }
    false
}

/// Redacts high-confidence API keys and credentials when targeting public cloud endpoints.
pub struct PrivacyRedactor;

impl PrivacyRedactor {
    pub fn redact_text(input: &str) -> String {
        static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
        let rules = PATTERNS.get_or_init(|| {
            vec![
                (
                    Regex::new(r"\bsk-[A-Za-z0-9_\-]{20,}\b").unwrap(),
                    "[REDACTED_API_KEY]",
                ),
                (
                    Regex::new(r"\bgh[pousr]_[A-Za-z0-9]{20,}\b").unwrap(),
                    "[REDACTED_GITHUB_TOKEN]",
                ),
                (
                    Regex::new(r"\bAKIA[0-9A-Z]{16}\b").unwrap(),
                    "[REDACTED_AWS_KEY]",
                ),
                (
                    Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----").unwrap(),
                    "[REDACTED_PRIVATE_KEY]",
                ),
            ]
        });
        let mut out = input.to_string();
        for (re, replacement) in rules {
            if re.is_match(&out) {
                out = re.replace_all(&out, *replacement).into_owned();
            }
        }
        out
    }

    /// Recursively redact every string leaf inside a JSON `Value` without ever breaking JSON structure.
    pub fn redact_json_value(val: &mut serde_json::Value) -> bool {
        match val {
            serde_json::Value::String(s) => {
                let red = Self::redact_text(s);
                if red != *s {
                    *s = red;
                    true
                } else {
                    false
                }
            }
            serde_json::Value::Array(arr) => {
                let mut changed = false;
                for item in arr {
                    if Self::redact_json_value(item) {
                        changed = true;
                    }
                }
                changed
            }
            serde_json::Value::Object(map) => {
                let mut changed = false;
                for (_, v) in map.iter_mut() {
                    if Self::redact_json_value(v) {
                        changed = true;
                    }
                }
                changed
            }
            _ => false,
        }
    }

    pub fn sanitize_request(endpoint: &str, req: &StructuredModelRequest) -> StructuredModelRequest {
        if is_local_or_lan_endpoint(endpoint) {
            return req.clone();
        }
        let mut out = req.clone();
        for block in &mut out.frozen_system_prefix {
            block.text = Self::redact_text(&block.text);
        }
        for rem in &mut out.ephemeral_tail_reminders {
            rem.text = Self::redact_text(&rem.text);
        }
        for msg in &mut out.history {
            match msg {
                HistoryMessage::User { content, .. } => {
                    *content = Self::redact_text(content);
                }
                HistoryMessage::Assistant {
                    content,
                    reasoning,
                    tool_calls,
                    continuation,
                } => {
                    *content = Self::redact_text(content);
                    if let Some(r) = reasoning {
                        *r = Self::redact_text(r);
                    }
                    for tc in tool_calls {
                        Self::redact_json_value(&mut tc.arguments);
                    }
                    if let Some(cont) = continuation {
                        let mut mutated = false;
                        for item in &mut cont.items {
                            if Self::redact_json_value(item) {
                                mutated = true;
                            }
                        }
                        // If a signed Anthropic thinking block was modified by redaction, drop
                        // continuation so Anthropic does not reject an invalidated signature.
                        if mutated && cont.protocol == nola_types::ModelProtocol::AnthropicMessages {
                            *continuation = None;
                        }
                    }
                }
                HistoryMessage::Tool { payload, .. } => match payload {
                    ToolModelPayload::Text(t) => *t = Self::redact_text(t),
                    ToolModelPayload::Structured {
                        model_data,
                        ui_summary,
                    } => {
                        *ui_summary = Self::redact_text(ui_summary);
                        Self::redact_json_value(model_data);
                    }
                },
            }
        }
        out
    }
}
