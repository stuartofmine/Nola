// SPDX-License-Identifier: Apache-2.0
//! Model evidence is not a browser/UI control channel. No IO or authorization.
use serde_json::Value;

/// The only projection of Host output eligible for model context and recall.
/// Raw Host output remains available to its owning UI/receipt adapter.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelToolEvidence(Value);

impl ModelToolEvidence {
    pub fn from_host_output(output: &Value) -> Self {
        Self(project(output, 0))
    }

    pub fn into_value(self) -> Value { self.0 }
    pub fn as_value(&self) -> &Value { &self.0 }

    pub fn text(text: &str) -> String {
        match project(&Value::String(text.to_owned()), 0) {
            Value::String(text) => text,
            _ => unreachable!("text projection preserves its type"),
        }
    }
}

fn project(value: &Value, depth: usize) -> Value {
    if depth >= 64 {
        return Value::String("[evidence nesting limit; inspect a bounded resource range]".into());
    }
    match value {
        Value::Object(object) => Value::Object(object.iter()
            .filter(|(key, _)| !matches!(key.as_str(), "preview_url" | "_nola_control" | "_nola_ui_control"))
            .map(|(key, value)| (key.clone(), project(value, depth + 1))).collect()),
        Value::Array(values) => Value::Array(values.iter().map(|value| project(value, depth + 1)).collect()),
        Value::String(text) => {
            // Tool adapters sometimes serialize structured results inside text.
            // Preserve byte-for-byte text when there is no control information.
            if let Ok(parsed @ (Value::Object(_) | Value::Array(_))) = serde_json::from_str::<Value>(text) {
                let safe = project(&parsed, depth + 1);
                if safe != parsed { return Value::String(safe.to_string()); }
            }
            Value::String(without_local_control_urls(text))
        }
        value => value.clone(),
    }
}

fn without_local_control_urls(text: &str) -> String {
    let mut result = String::new();
    let mut tail = text;
    while let Some(start) = tail.find("http") {
        result.push_str(&tail[..start]);
        tail = &tail[start..];
        let end = tail.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`'))
            .unwrap_or(tail.len());
        let end = end.max(1);
        let candidate = &tail[..end];
        let is_control = url::Url::parse(candidate).ok().is_some_and(|url| {
            let local = match url.host() {
                Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
                Some(url::Host::Ipv4(host)) => host.is_loopback(),
                Some(url::Host::Ipv6(host)) => host.is_loopback(),
                None => false,
            };
            local && url.query_pairs().any(|(key, _)| key == "token")
        });
        result.push_str(if is_control { "[local preview control omitted]" } else { candidate });
        tail = &tail[end..];
    }
    result.push_str(tail);
    result
}

/// A browser resource's semantic observation excludes capture/transport noise.
/// Real URL, DOM, accessibility, element and viewport changes remain visible.
pub fn semantic_tool_observation(output: &Value) -> Value {
    fn semantic(value: Value) -> Value {
        match value {
            Value::Object(mut object) => {
                let page_observation = ["accessibility", "elements", "interactive_elements", "visible_text", "page_html"]
                    .iter().any(|key| object.contains_key(*key));
                for key in ["full_output_ref", "preview_end_byte_offset", "original_bytes", "original_chars"] {
                    object.remove(key);
                }
                if page_observation {
                    for key in ["revision", "generation", "page_id", "screenshot", "screenshot_data_url", "screenshot_path", "captured_at", "captured_at_ms"] {
                        object.remove(key);
                    }
                    if object.get("text").and_then(Value::as_str)
                        .is_some_and(|text| serde_json::from_str::<Value>(text).is_ok()) {
                        object.remove("text");
                    }
                    // Element handles are transport references to this snapshot,
                    // not changes to the underlying DOM/accessibility content.
                    if let Some(Value::Array(elements)) = object.get_mut("elements") {
                        for element in elements {
                            if let Some(fields) = element.as_object_mut() { fields.remove("ref"); }
                        }
                    }
                }
                Value::Object(object.into_iter().map(|(key, value)| (key, semantic(value))).collect())
            }
            Value::Array(values) => Value::Array(values.into_iter().map(semantic).collect()),
            Value::String(text) => match serde_json::from_str::<Value>(&text) {
                Ok(value @ (Value::Object(_) | Value::Array(_))) => semantic(value),
                _ => Value::String(text),
            },
            value => value,
        }
    }
    semantic(ModelToolEvidence::from_host_output(output).into_value())
}

/// Capture size and image preferences change presentation, not the page being
/// investigated. Other tools retain their full semantic argument identity.
pub fn semantic_observation_arguments(tool_name: &str, arguments: &Value) -> Value {
    let mut arguments = arguments.clone();
    if matches!(tool_name, "browser_snapshot" | "browser_dom_snapshot" | "browser_accessibility_snapshot"
        | "chrome_native_inspect" | "chrome_native_dom_snapshot" | "chrome_native_accessibility_snapshot") {
        if let Some(fields) = arguments.as_object_mut() {
            for key in ["screenshot", "max_elements", "max_chars", "timeout_ms"] { fields.remove(key); }
        }
    }
    arguments
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn control_data_never_enters_nested_or_text_evidence() {
        let control = "http://127.0.0.1:1234/browser/preview?token=secret-fixture";
        let raw = json!({"text": json!({"preview_url":control,"title":"Page"}).to_string(),
            "data":{"preview_url":control,"url":"https://example.org"},
            "_nola_control":{"preview_url":control},
            "note":format!("Preview: {control}")});
        let safe = ModelToolEvidence::from_host_output(&raw).into_value();
        assert!(!safe.to_string().contains("secret-fixture"));
        assert_eq!(safe["data"]["url"], "https://example.org");
        assert!(raw.to_string().contains("secret-fixture"), "UI source is not mutated");
    }

    #[test]
    fn ordinary_text_and_remote_source_links_remain_exact() {
        for text in ["hello 世界", "http is a protocol", "https://example.org/paper?token=public-reference"] {
            assert_eq!(ModelToolEvidence::text(text), text);
        }
    }

    #[test]
    fn browser_capture_noise_is_not_semantic_progress() {
        let a = json!({"full_output_ref":"one", "data":{"url":"https://example.org", "elements":[{"ref":"e1","text":"Read"}], "revision":4,"screenshot":"one"}});
        let mut b = a.clone();
        b["full_output_ref"] = json!("two");
        b["data"]["revision"] = json!(5);
        b["data"]["screenshot"] = json!("two");
        b["data"]["elements"][0]["ref"] = json!("e-new-generation-5-1");
        b["data"]["generation"] = json!("reconnected-runtime");
        b["data"]["page_id"] = json!("same-url-new-document");
        assert_eq!(semantic_tool_observation(&a), semantic_tool_observation(&b));
        b["data"]["elements"][0]["text"] = json!("Changed");
        assert_ne!(semantic_tool_observation(&a), semantic_tool_observation(&b));
    }
}
