// SPDX-License-Identifier: Apache-2.0
//! Redact a provider-bound copy. Protocol identifiers, authentication headers
//! and opaque attachment bytes are owned by their respective adapters.

use regex::{Captures, Regex};
use serde_json::Value;
use std::{ffi::CString, os::raw::c_char, ptr, sync::LazyLock};

static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"[A-Za-z0-9._%+\-]+@[A-Za-z0-9\-]+(?:\.[A-Za-z0-9\-]+)+"#)
        .expect("valid email pattern")
});
static PRIVATE_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r#"(?s)-----BEGIN (?:[A-Z0-9 ]+ )?PRIVATE KEY-----.*?(?:-----END (?:[A-Z0-9 ]+ )?PRIVATE KEY-----|\z)"#
).expect("valid privateKey pattern")
});
static ACCESS_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r#"(?-u:\b)(?:sk-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9_]{16,}|github_pat_[A-Za-z0-9_]{16,}|AIza[A-Za-z0-9_-]{35}|xox[baprs]-[A-Za-z0-9-]{12,}|(?:AKIA|ASIA)[A-Z0-9]{16}(?-u:\b))"#
).expect("valid accessKey pattern")
});
static USERINFO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)([a-z][a-z0-9+.-]*://)([^/\s@?#]+)@"#).expect("valid urlUserinfo pattern")
});
static BEARER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)((?-u:\b)(?:Bearer|Authorization:[ \t]*Basic)[ \t]+)[A-Za-z0-9._~+/=-]{8,}"#)
        .expect("valid bearer pattern")
});
static ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r#"(?-u:\b)[1-9][0-9]{5}(?:19|20)[0-9]{2}(?:0[1-9]|1[0-2])(?:0[1-9]|[12][0-9]|3[01])[0-9]{3}[0-9Xx](?-u:\b)"#
).expect("valid id pattern")
});
static PHONE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r#"(?:\+(?:86[ -]?)?1[3-9][0-9][ -]?[0-9]{4}[ -]?[0-9]{4}|(?:\+1[ -]?)?\([2-9][0-9]{2}\)[ -]?[2-9][0-9]{2}[ -][0-9]{4}|\+[1-9][0-9]{7,14}|(?-u:\b)1[3-9][0-9][ -]?[0-9]{4}[ -]?[0-9]{4})(?-u:\b)"#
).expect("valid phone pattern")
});
static LABELED_SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r#"(?i)((?-u:\b)(?:[a-z][a-z0-9]*[_-])*(?:api[_-]?key|secret[_-]?access[_-]?key|access[_-]?token|refresh[_-]?token|client[_-]?secret|password|passwd|token)["']?\s*[:=]\s*)(?:\[(?:CREDENTIAL|REDACTED|redacted)\]|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|[^\s,;&"'<>}\]]+)"#
).expect("valid labeledSecret pattern")
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedactionOptions {
    pub redact_pii: bool,
}

impl Default for RedactionOptions {
    fn default() -> Self {
        Self { redact_pii: true }
    }
}

pub fn is_loopback_endpoint(endpoint: &str) -> bool {
    let trimmed = endpoint.trim();
    if trimmed.is_empty() {
        return false;
    }
    let after_scheme = match trimmed.find("://") {
        Some(idx) => &trimmed[idx + 3..],
        None => trimmed,
    };
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let host_port = match authority.rsplit_once('@') {
        Some((_, rest)) => rest,
        None => authority,
    };
    let host = if let Some(rest) = host_port.strip_prefix('[') {
        match rest.split_once(']') {
            Some((inside, _)) => inside,
            None => return false,
        }
    } else if host_port == "::1" {
        "::1"
    } else {
        match host_port.rsplit_once(':') {
            Some((h, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
            _ => host_port,
        }
    };
    let host = host.trim_end_matches('.');
    host.eq_ignore_ascii_case("localhost") || matches!(host, "127.0.0.1" | "::1" | "0.0.0.0")
}

pub fn should_redact_pii(endpoint: &str) -> bool {
    if let Ok(raw) = std::env::var("NOLA_REDACT_PII") {
        let trimmed = raw.trim();
        if trimmed == "0" || trimmed.eq_ignore_ascii_case("false") {
            return false;
        }
        if trimmed == "1" || trimmed.eq_ignore_ascii_case("true") {
            return true;
        }
    }
    !is_loopback_endpoint(endpoint)
}

fn is_bind_placeholder(value: &str) -> bool {
    matches!(value.as_bytes().first(), Some(b'$' | b'?'))
        && value[1..]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

pub fn redact_secrets_text(input: &str, placeholder: &str) -> (String, u32) {
    if input.is_empty() {
        return (String::new(), 0);
    }
    let mut text = input.to_string();
    let mut count = 0_u32;

    for pattern in [&*PRIVATE_KEY, &*ACCESS_KEY] {
        let matches = pattern.find_iter(&text).count() as u32;
        if matches > 0 {
            count = count.saturating_add(matches);
            if let std::borrow::Cow::Owned(result) = pattern.replace_all(&text, placeholder) {
                text = result;
            }
        }
    }

    let mut userinfo_count = 0_u32;
    if let std::borrow::Cow::Owned(result) = USERINFO.replace_all(&text, |c: &Captures<'_>| {
        if &c[2] == placeholder || &c[2] == "[CREDENTIAL]" {
            c[0].to_string()
        } else {
            userinfo_count = userinfo_count.saturating_add(1);
            format!("{}{placeholder}@", &c[1])
        }
    }) {
        text = result;
    }
    count = count.saturating_add(userinfo_count);

    let bearer_matches = BEARER.find_iter(&text).count() as u32;
    if bearer_matches > 0 {
        count = count.saturating_add(bearer_matches);
        if let std::borrow::Cow::Owned(result) = BEARER.replace_all(&text, |c: &Captures<'_>| {
            format!("{}{placeholder}", &c[1])
        }) {
            text = result;
        }
    }

    let mut labeled_count = 0_u32;
    if let std::borrow::Cow::Owned(result) =
        LABELED_SECRET.replace_all(&text, |c: &Captures<'_>| {
            let value = &c[0][c[1].len()..];
            let quote = match value.as_bytes().first() {
                Some(b'"') => "\"",
                Some(b'\'') => "'",
                _ => "",
            };
            let inner = if !quote.is_empty() && value.len() >= 2 && value.ends_with(quote) {
                &value[1..value.len() - 1]
            } else {
                value
            };
            if inner == placeholder
                || inner == "[CREDENTIAL]"
                || (quote.is_empty() && is_bind_placeholder(inner))
            {
                return c[0].to_string();
            }
            labeled_count = labeled_count.saturating_add(1);
            format!("{}{quote}{placeholder}{quote}", &c[1])
        })
    {
        text = result;
    }
    count = count.saturating_add(labeled_count);

    (text, count)
}

fn redact(text: &mut String, credential: &str, options: RedactionOptions) {
    if !credential.is_empty()
        && (text == credential || (credential.len() >= 8 && text.contains(credential)))
    {
        *text = text.replace(credential, "[CREDENTIAL]");
    }
    for (pattern, replacement) in [
        (&*PRIVATE_KEY, "[CREDENTIAL]"),
        (&*ACCESS_KEY, "[CREDENTIAL]"),
        (&*USERINFO, "${1}[CREDENTIAL]@"),
        (&*BEARER, "${1}[CREDENTIAL]"),
    ] {
        if let std::borrow::Cow::Owned(result) = pattern.replace_all(text, replacement) {
            *text = result;
        }
    }
    if options.redact_pii {
        for (pattern, replacement) in [
            (&*EMAIL, "[EMAIL]"),
            (&*ID, "[ID]"),
            (&*PHONE, "[PHONE]"),
        ] {
            if let std::borrow::Cow::Owned(result) = pattern.replace_all(text, replacement) {
                *text = result;
            }
        }
    }
    if let std::borrow::Cow::Owned(result) = LABELED_SECRET.replace_all(text, |c: &Captures<'_>| {
        let value = &c[0][c[1].len()..];
        let quote = match value.as_bytes().first() {
            Some(b'"') => "\"",
            Some(b'\'') => "'",
            _ => "",
        };
        format!("{}{quote}[CREDENTIAL]{quote}", &c[1])
    }) {
        *text = result;
    }
}

fn secret_field(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let field = lower.replace('-', "_");
    if [
        "_api_key",
        "_access_token",
        "_refresh_token",
        "_client_secret",
        "_secret_access_key",
        "_password",
        "_token",
    ]
    .iter()
    .any(|suffix| field.ends_with(suffix))
    {
        return true;
    }
    matches!(
        field.replace('_', "").as_str(),
        "apikey"
            | "xapikey"
            | "authorization"
            | "password"
            | "passwd"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "clientsecret"
            | "cookie"
            | "setcookie"
            | "privatekey"
    )
}

// Inside tool input/output, field names describe application data, not provider controls.
fn tool_data(
    value: &mut Value,
    credential: &str,
    options: RedactionOptions,
    embedded_depth: usize,
) {
    match value {
        Value::Object(fields) => {
            let mut renames = Vec::new();
            for (key, child) in fields.iter_mut() {
                if secret_field(key) {
                    *child = Value::from("[CREDENTIAL]");
                } else {
                    tool_data(child, credential, options, embedded_depth);
                }
                let mut masked = key.clone();
                redact(&mut masked, credential, options);
                if masked != *key {
                    renames.push((key.clone(), masked));
                }
            }
            // Distinct data keys can become the same placeholder. Keep all their values.
            for (key, masked) in renames {
                let mut destination = masked.clone();
                let mut index = 2;
                while fields.contains_key(&destination) {
                    destination = format!("{masked} {index}");
                    index += 1;
                }
                if let Some(child) = fields.remove(&key) {
                    fields.insert(destination, child);
                }
            }
        }
        Value::Array(list) => {
            for child in list {
                tool_data(child, credential, options, embedded_depth);
            }
        }
        Value::String(text) => {
            if embedded_depth < 3 && text.trim_start().starts_with(['{', '[']) {
                if let Ok(mut decoded) = serde_json::from_str::<Value>(text) {
                    let before = decoded.clone();
                    tool_data(&mut decoded, credential, options, embedded_depth + 1);
                    if decoded != before {
                        *text = decoded.to_string();
                    }
                    return;
                }
            }
            redact(text, credential, options);
        }
        Value::Number(number) => {
            let original = number.to_string();
            let mut text = original.clone();
            redact(&mut text, credential, options);
            if text != original {
                *value = Value::String(text);
            }
        }
        _ => {}
    }
}

fn payload(value: &mut Value, credential: &str, options: RedactionOptions) {
    tool_data(value, credential, options, 0);
}

fn content(value: &mut Value, credential: &str, options: RedactionOptions) {
    match value {
        Value::String(_) => payload(value, credential, options),
        Value::Array(list) => {
            for item in list {
                content(item, credential, options);
            }
        }
        Value::Object(obj) => {
            for key in [
                "content",
                "text",
                "refusal",
                "instructions",
                "reasoning_content",
                "summary",
                "filename",
            ] {
                if let Some(child) = obj.get_mut(key) {
                    content(child, credential, options);
                }
            }
            for key in ["arguments", "output"] {
                if let Some(child) = obj.get_mut(key) {
                    payload(child, credential, options);
                }
            }
            if obj.get("type").and_then(Value::as_str) == Some("tool_use") {
                if let Some(input) = obj.get_mut("input") {
                    payload(input, credential, options);
                }
            }
            if let Some(calls) = obj.get_mut("tool_calls").and_then(Value::as_array_mut) {
                for call in calls {
                    if let Some(args) = call.pointer_mut("/function/arguments") {
                        payload(args, credential, options);
                    }
                }
            }
            if let Some(args) = obj
                .get_mut("function_call")
                .and_then(|f| f.get_mut("arguments"))
            {
                payload(args, credential, options);
            }
        }
        _ => {}
    }
}

// Only human-readable schema annotations may be changed. Property names,
// required fields, tool names and enum values are part of the call contract.
fn tool_schema(value: &mut Value, credential: &str, options: RedactionOptions) {
    match value {
        Value::Array(items) => {
            for item in items {
                tool_schema(item, credential, options);
            }
        }
        Value::Object(fields) => {
            for (field, child) in fields {
                match field.as_str() {
                    "description" | "title" if child.is_string() => {
                        payload(child, credential, options)
                    }
                    "examples" | "default" => payload(child, credential, options),
                    "properties" | "$defs" | "definitions" | "patternProperties" => {
                        if let Some(properties) = child.as_object_mut() {
                            for schema in properties.values_mut() {
                                tool_schema(schema, credential, options);
                            }
                        }
                    }
                    "function"
                    | "parameters"
                    | "input_schema"
                    | "items"
                    | "oneOf"
                    | "anyOf"
                    | "allOf"
                    | "additionalProperties"
                    | "tools" => tool_schema(child, credential, options),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

pub fn redact_model_request_with_options(
    body: &mut Value,
    credential: &str,
    options: RedactionOptions,
) {
    if let Some(tools) = body.get_mut("tools") {
        tool_schema(tools, credential, options);
    }
    for key in ["messages", "input", "system", "instructions", "prompt"] {
        if let Some(value) = body.get_mut(key) {
            content(value, credential, options);
        }
    }
}

pub fn redact_model_request(body: &mut Value, credential: &str) {
    redact_model_request_with_options(body, credential, RedactionOptions::default());
}

/// # Safety
/// Both buffers must remain readable for the duration of this call. An empty
/// credential may be null. Free the returned JSON with nola_model_privacy_free.
#[no_mangle]
pub unsafe extern "C" fn nola_core_redact_model_request_json(
    input: *const u8,
    size: usize,
    key: *const u8,
    key_size: usize,
) -> *mut c_char {
    if input.is_null() || size == 0 || (key_size > 0 && key.is_null()) {
        return ptr::null_mut();
    }
    let Ok(mut body) = serde_json::from_slice::<Value>(std::slice::from_raw_parts(input, size))
    else {
        return ptr::null_mut();
    };
    if !body.is_object() {
        return ptr::null_mut();
    }
    let credential = if key_size == 0 {
        ""
    } else {
        let Ok(key) = std::str::from_utf8(std::slice::from_raw_parts(key, key_size)) else {
            return ptr::null_mut();
        };
        key
    };
    redact_model_request(&mut body, credential);
    CString::new(body.to_string()).map_or(ptr::null_mut(), CString::into_raw)
}

/// Releases the result of nola_core_redact_model_request_json in the allocating library.
/// # Safety
/// The pointer must be null or a live string returned by that function.
#[no_mangle]
pub unsafe extern "C" fn nola_model_privacy_free(value: *mut c_char) {
    if !value.is_null() {
        drop(CString::from_raw(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn model_privacy_covers_prompt_history_calls_and_results_across_protocols() {
        let person = "用户anne+test@example.org，电话13800138000，证件11010519990101123X";
        let input = json!({
            "model":"keep@example.org", "tools":[{"name":"lookup","parameters":{"type":"object"}}],
            "instructions": person, "system":[{"type":"text","text":person}],
            "messages":[{"role":"user","content":[{"type":"text","text":person}]},
                {"role":"assistant","tool_calls":[{"id":"call_1","function":{"name":"lookup",
                     "arguments":r#"{"accessToken":"opaque value","email":"anne+test@example.org"}"#}}]},
                {"role":"tool","tool_call_id":"call_1","content":r#"{"nested":{"Cookie":"session=example","phone":13800138000}}"#},
                {"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":person},
                    {"type":"tool_use","name":"lookup","id":"call_2","input":{"passwd":"my secret"}}]}],
            "input":[{"type":"function_call_output","call_id":"call_2","output":"Bearer fixture-bearer-value"},
                {"type":"reasoning","encrypted_content":"opaque continuation"},
                {"role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,AAAA"}]}]
        });
        let mut wire = input.clone();
        redact_model_request(&mut wire, "");
        assert_eq!(wire["instructions"], "用户[EMAIL]，电话[PHONE]，证件[ID]");
        for raw in [
            "anne+",
            "13800138000",
            "1101051999",
            "fixture-bearer",
            "my secret",
            "session=example",
            "opaque value",
        ] {
            assert!(!wire.to_string().contains(raw), "{raw}");
        }
        assert_eq!(wire["messages"][1]["tool_calls"][0]["id"], "call_1");
        assert_eq!(wire["input"][1], input["input"][1]);
        assert_eq!(wire["input"][2], input["input"][2]);
        assert_eq!(wire["tools"], input["tools"]);
        assert_eq!(wire["model"], input["model"]);
        assert_eq!(input["instructions"], person);
        let once = wire.clone();
        redact_model_request(&mut wire, "");
        assert_eq!(once, wire);
    }

    #[test]
    fn tool_schemas_keep_identifiers_and_nested_data_keeps_colliding_keys() {
        let mut request = json!({"tools":[{"type":"function","function":{
        "name":"lookup","description":"contact person@example.org","parameters":{
            "type":"object","properties":{"title":{"description":"call +1 (212) 555-0100"}},
            "required":["title"]}}}],
        "input":[{"type":"function_call_output","call_id":"live","output":{
            "first@example.org":1, "second@example.org":2,
            "serialized":r#"{"Cookie":"raw cookie value"}"#
        }}]});
        redact_model_request(&mut request, "");
        assert_eq!(
            request.pointer("/tools/0/function/description").unwrap(),
            "contact [EMAIL]"
        );
        assert_eq!(
            request
                .pointer("/tools/0/function/parameters/properties/title/description")
                .unwrap(),
            "call [PHONE]"
        );
        assert_eq!(
            request
                .pointer("/tools/0/function/parameters/required/0")
                .unwrap(),
            "title"
        );
        let result = &request["input"][0]["output"];
        assert_eq!(result["[EMAIL]"], 1);
        assert_eq!(result["[EMAIL] 2"], 2);
        assert_eq!(result["serialized"], r#"{"Cookie":"[CREDENTIAL]"}"#);
    }

    #[test]
    fn model_privacy_handles_text_credentials_and_preserves_ordinary_content() {
        let mut body = json!({"input":"文档 v1.2.3，日期 2026-11-23，订单 12345678901234\npassword='multi word' token=sample-url-value&x=1 NOLA_HOST_TOKEN=freeformvalue\n-----BEGIN PRIVATE KEY-----\nprivate fixture\n-----END PRIVATE KEY-----\n配置 fixture-secret-key"});
        redact_model_request(&mut body, "fixture-secret-key");
        assert_eq!(body["input"], "文档 v1.2.3，日期 2026-11-23，订单 12345678901234\npassword='[CREDENTIAL]' token=[CREDENTIAL]&x=1 NOLA_HOST_TOKEN=[CREDENTIAL]\n[CREDENTIAL]\n配置 [CREDENTIAL]");
        let once = body.clone();
        redact_model_request(&mut body, "fixture-secret-key");
        assert_eq!(once, body);
    }

    #[test]
    fn redacts_aws_access_keys_in_requests_and_text() {
        let prod_key = concat!("AKIA", "IOSFODNN7EXAMPLE");
        let temp_key = concat!("ASIA", "1234567890ABCDEF");
        let mut body = json!({
            "input": format!("prod {prod_key} and temp {temp_key}")
        });
        redact_model_request(&mut body, "");
        assert_eq!(body["input"], "prod [CREDENTIAL] and temp [CREDENTIAL]");

        let text = format!("keys {prod_key} and {temp_key}");
        let (redacted, count) = redact_secrets_text(&text, "[REDACTED]");
        assert_eq!(count, 2);
        assert_eq!(redacted, "keys [REDACTED] and [REDACTED]");
    }

    #[test]
    fn redaction_options_can_preserve_pii_while_still_redacting_secrets() {
        let prod_key = concat!("AKIA", "IOSFODNN7EXAMPLE");
        let input = json!({
            "instructions": format!("Email anne+test@example.org, phone 13800138000, id 11010519990101123X, key {prod_key}"),
            "messages": [
                {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_1",
                        "function": {
                            "name": "lookup",
                            "arguments": r#"{"accessToken":"secret-val","email":"anne+test@example.org"}"#
                        }
                    }]
                }
            ]
        });
        let mut wire = input.clone();
        redact_model_request_with_options(&mut wire, "my-live-cred-1234", RedactionOptions { redact_pii: false });
        assert_eq!(
            wire["instructions"],
            "Email anne+test@example.org, phone 13800138000, id 11010519990101123X, key [CREDENTIAL]"
        );
        let args = wire["messages"][0]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap();
        assert!(args.contains("anne+test@example.org"));
        assert!(!args.contains("secret-val"));
        assert!(args.contains("[CREDENTIAL]"));
    }

    #[test]
    fn loopback_endpoint_detection_handles_local_and_remote_urls() {
        for local in [
            "http://localhost:11434/v1",
            "https://LOCALHOST/v1/chat/completions",
            "http://127.0.0.1:8080/v1",
            "http://[::1]:8000/v1",
            "::1",
            "http://0.0.0.0:11434",
            "http://user:pass@127.0.0.1:8080/v1",
        ] {
            assert!(is_loopback_endpoint(local), "expected loopback: {local}");
        }
        for remote in [
            "",
            "https://api.openai.com/v1",
            "https://api.anthropic.com/v1",
            "http://localhost.evil.com/v1",
            "http://127.0.0.1.evil.com/v1",
            "https://example.com/localhost",
        ] {
            assert!(!is_loopback_endpoint(remote), "expected non-loopback: {remote}");
        }
    }

    #[test]
    fn redact_secrets_text_preserves_pii_and_is_idempotent() {
        let input = "email alice@example.com phone 13800138000 url https://u:p@git.example.com/r.git token=abc123456789";
        let (first, count) = redact_secrets_text(input, "[REDACTED]");
        assert_eq!(count, 2);
        assert_eq!(
            first,
            "email alice@example.com phone 13800138000 url https://[REDACTED]@git.example.com/r.git token=[REDACTED]"
        );
        let (second, second_count) = redact_secrets_text(&first, "[REDACTED]");
        assert_eq!(second_count, 0);
        assert_eq!(second, first);
    }
}
