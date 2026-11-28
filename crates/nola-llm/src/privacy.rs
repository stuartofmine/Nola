// SPDX-License-Identifier: Apache-2.0
//! Full wire privacy redactor.
//!
//! Policy invariants:
//! - Credential redaction is ALWAYS active across ALL endpoints (including loopback & private LAN).
//! - PII redaction (`[EMAIL]`, `[PHONE]`, `[ID]`) is controlled by `should_redact_pii(endpoint)`:
//!   honors `NOLA_REDACT_PII=0|1|false|true`, defaulting to `false` on local/LAN endpoints and
//!   `true` on public/cloud endpoints.
//! - Round-2 `LABELED_SECRET` matches single-line assignments only (`[ \t]*[:=][ \t]*`) and
//!   preserves bind placeholders (`$VAR`, `${...}`, `{{...}}`) and non-secret schema/algorithm
//!   identifiers (`sha256`, `mask_secret`, `hmac`, `bcrypt`, `argon2`, `pbkdf2`, `none`, etc.).

use std::borrow::Cow;
use std::net::IpAddr;
use std::sync::LazyLock;

use nola_types::{HistoryMessage, StructuredModelRequest, ToolModelPayload};
use regex::{Captures, Regex};
use serde_json::Value;

static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"[A-Za-z0-9._%+\-]+@[A-Za-z0-9\-]+(?:\.[A-Za-z0-9\-]+)+"#)
        .expect("valid email pattern")
});
static PRIVATE_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)-----BEGIN (?:[A-Z0-9 ]+ )?PRIVATE KEY-----.*?(?:-----END (?:[A-Z0-9 ]+ )?PRIVATE KEY-----|\z)"#,
    )
    .expect("valid privateKey pattern")
});
static ACCESS_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?-u:\b)(?:sk-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9_]{16,}|github_pat_[A-Za-z0-9_]{16,}|AIza[A-Za-z0-9_-]{35}|xox[baprs]-[A-Za-z0-9-]{12,}|(?:AKIA|ASIA)[A-Z0-9]{16}(?-u:\b))"#,
    )
    .expect("valid accessKey pattern")
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
        r#"(?-u:\b)[1-9][0-9]{5}(?:19|20)[0-9]{2}(?:0[1-9]|1[0-2])(?:0[1-9]|[12][0-9]|3[01])[0-9]{3}[0-9Xx](?-u:\b)"#,
    )
    .expect("valid id pattern")
});
static PHONE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:\+(?:86[ -]?)?1[3-9][0-9][ -]?[0-9]{4}[ -]?[0-9]{4}|(?:\+1[ -]?)?\([2-9][0-9]{2}\)[ -]?[2-9][0-9]{2}[ -][0-9]{4}|\+[1-9][0-9]{7,14}|(?-u:\b)1[3-9][0-9][ -]?[0-9]{4}[ -]?[0-9]{4})(?-u:\b)"#,
    )
    .expect("valid phone pattern")
});
static LABELED_SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)((?-u:\b)(?:[a-z][a-z0-9]*[_-])*(?:api[_-]?key|secret[_-]?access[_-]?key|access[_-]?token|refresh[_-]?token|client[_-]?secret|password|passwd|token)["']?[ \t]*[:=][ \t]*)(?:\[(?:CREDENTIAL|REDACTED|redacted|REDACTED_[A-Z_]+)\]|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|[^\s,;&"'<>}\]]+)"#,
    )
    .expect("valid labeledSecret pattern")
});

// Legacy fine-grained patterns preserved for backward-compatible `PrivacyRedactor::redact_text`
// and `PrivacyRedactor::redact_json_value` callers (e.g. FFI / TUI / memory logs).
static LEGACY_SK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bsk-[A-Za-z0-9_\-]{16,}\b").expect("valid sk pattern"));
static LEGACY_GH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:gh[pousr]_[A-Za-z0-9_]{16,}|github_pat_[A-Za-z0-9_]{16,})\b")
        .expect("valid gh pattern")
});
static LEGACY_AWS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b").expect("valid aws pattern"));
static LEGACY_OTHER_KEYS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?-u:\b)(?:AIza[A-Za-z0-9_-]{35}|xox[baprs]-[A-Za-z0-9-]{12,})")
        .expect("valid other key pattern")
});

/// Options governing wire privacy redaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedactionOptions {
    pub redact_pii: bool,
}

impl Default for RedactionOptions {
    fn default() -> Self {
        Self { redact_pii: true }
    }
}

fn extract_endpoint_host(endpoint: &str) -> Option<&str> {
    let trimmed = endpoint.trim();
    if trimmed.is_empty() {
        return None;
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
            None => return None,
        }
    } else if host_port == "::" || host_port == "::1" || host_port.matches(':').count() > 1 {
        host_port
    } else {
        match host_port.rsplit_once(':') {
            Some((h, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
            _ => host_port,
        }
    };
    let host = host.trim_end_matches('.');
    (!host.is_empty()).then_some(host)
}

/// Returns true for loopback, unspecified (`0.0.0.0`, `::`), RFC1918 private LAN,
/// RFC6598 CGNAT/Tailscale (`100.64.0.0/10`), link-local, IPv6 ULA (`fc00::/7`),
/// mDNS `.local`, `.localhost`, and container host bridges (`host.docker.internal`,
/// `gateway.docker.internal`, `host.containers.internal`).
pub fn is_local_or_lan_endpoint(endpoint: &str) -> bool {
    let Some(host) = extract_endpoint_host(endpoint) else {
        return false;
    };
    let lower = host.to_ascii_lowercase();
    if lower == "localhost"
        || lower.ends_with(".localhost")
        || lower.ends_with(".local")
        || matches!(
            lower.as_str(),
            "host.docker.internal" | "gateway.docker.internal" | "host.containers.internal"
        )
    {
        return true;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => {
                let [a, b, ..] = v4.octets();
                v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified()
                    || (a == 100 && (64..=127).contains(&b))
            }
            IpAddr::V6(v6) => {
                let seg0 = v6.segments()[0];
                v6.is_loopback()
                    || v6.is_unspecified()
                    || (seg0 & 0xfe00) == 0xfc00
                    || (seg0 & 0xffc0) == 0xfe80
            }
        };
    }
    false
}

/// Controls whether PII (`[EMAIL]`, `[PHONE]`, `[ID]`) should be redacted for `endpoint`.
///
/// Honors `NOLA_REDACT_PII=0|false` and `NOLA_REDACT_PII=1|true` overrides;
/// otherwise defaults to `false` on local/LAN endpoints and `true` on public/cloud endpoints.
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
    !is_local_or_lan_endpoint(endpoint)
}

fn is_bind_placeholder(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    if matches!(value.as_bytes().first(), Some(b'$' | b'?')) {
        if value[1..]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return value.len() > 1;
        }
        // Shell brace/command variable expansions such as ${VAR}, ${#VAR}, ${VAR:-fallback}.
        if value.starts_with("${") && value.ends_with('}') && value.len() >= 4 {
            return true;
        }
    }
    // Template placeholders such as {{VAR}}.
    if value.starts_with("{{") && value.ends_with("}}") && value.len() >= 5 {
        return true;
    }
    false
}

/// Distinguishes ordinary schema/config/code values (e.g. `card_token: sha256`,
/// `session_token: mask_secret`, `TOKEN=$API_TOKEN`, `password: bcrypt`,
/// `api_key: os.environ["X"]`) from literal secrets so wire privacy redaction
/// does not corrupt source files or YAML policies.
fn is_non_secret_labeled_value(inner: &str, quoted: bool) -> bool {
    if inner.is_empty()
        || inner == "[CREDENTIAL]"
        || inner == "[REDACTED]"
        || inner == "[redacted]"
        || (inner.starts_with("[REDACTED_") && inner.ends_with(']'))
        || is_bind_placeholder(inner)
    {
        return true;
    }
    if quoted {
        return false;
    }
    let lower = inner.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "null"
            | "none"
            | "nil"
            | "true"
            | "false"
            | "string"
            | "str"
            | "int"
            | "integer"
            | "number"
            | "float"
            | "boolean"
            | "bool"
            | "object"
            | "array"
            | "bytes"
            | "text"
            | "uuid"
            | "hex"
            | "base64"
            | "sha256"
            | "sha384"
            | "sha512"
            | "sha1"
            | "md5"
            | "blake2b"
            | "hmac"
            | "bcrypt"
            | "argon2"
            | "argon2id"
            | "pbkdf2"
            | "scrypt"
            | "required"
            | "optional"
            | "default"
            | "auto"
            | "unset"
            | "empty"
            | "redact"
            | "redacted"
            | "mask"
            | "masked"
            | "hash"
            | "fake"
            | "omit"
            | "omitted"
            | "unknown"
            | "invalid"
            | "valid"
            | "ok"
            | "error"
            | "failed"
            | "passed"
            | "env"
    ) {
        return true;
    }
    // Unquoted lowercase snake_case config/schema identifiers (e.g. `mask_secret`,
    // `redact_text`, `from_env`). Real prefixed credentials (`ghp_...`,
    // `github_pat_...`, `sk-...`, `AKIA...`) are already redacted by ACCESS_KEY
    // before LABELED_SECRET runs.
    if inner.contains('_')
        && !inner.starts_with('_')
        && !inner.ends_with('_')
        && inner
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && inner.bytes().any(|b| b.is_ascii_lowercase())
        && inner.split('_').all(|seg| !seg.is_empty() && seg.len() <= 16)
    {
        return true;
    }
    // Unquoted code expressions referencing environment or variables.
    if inner.starts_with("os.environ")
        || inner.starts_with("process.env")
        || inner.starts_with("std::env")
        || inner.starts_with("env::")
    {
        return true;
    }
    false
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

fn redact_string_in_place(
    text: &mut String,
    extra_secrets: &[&str],
    options: RedactionOptions,
) -> usize {
    if text.is_empty() {
        return 0;
    }
    let mut count: usize = 0;

    for raw_secret in extra_secrets {
        let cred = raw_secret.trim();
        if !cred.is_empty() && (*text == cred || (cred.len() >= 8 && text.contains(cred))) {
            let hits = text.matches(cred).count();
            if hits > 0 {
                count = count.saturating_add(hits);
                *text = text.replace(cred, "[CREDENTIAL]");
            }
        }
    }

    for pattern in [&*PRIVATE_KEY, &*ACCESS_KEY] {
        let hits = pattern.find_iter(text).count();
        if hits > 0 {
            count = count.saturating_add(hits);
            if let Cow::Owned(result) = pattern.replace_all(text, "[CREDENTIAL]") {
                *text = result;
            }
        }
    }

    let mut userinfo_hits = 0usize;
    if let Cow::Owned(result) = USERINFO.replace_all(text, |c: &Captures<'_>| {
        if &c[2] == "[CREDENTIAL]" || &c[2] == "[REDACTED]" {
            c[0].to_string()
        } else {
            userinfo_hits = userinfo_hits.saturating_add(1);
            format!("{}[CREDENTIAL]@", &c[1])
        }
    }) {
        *text = result;
    }
    count = count.saturating_add(userinfo_hits);

    let bearer_hits = BEARER.find_iter(text).count();
    if bearer_hits > 0 {
        count = count.saturating_add(bearer_hits);
        if let Cow::Owned(result) = BEARER.replace_all(text, |c: &Captures<'_>| {
            format!("{}[CREDENTIAL]", &c[1])
        }) {
            *text = result;
        }
    }

    if options.redact_pii {
        for (pattern, replacement) in [
            (&*EMAIL, "[EMAIL]"),
            (&*ID, "[ID]"),
            (&*PHONE, "[PHONE]"),
        ] {
            let hits = pattern.find_iter(text).count();
            if hits > 0 {
                count = count.saturating_add(hits);
                if let Cow::Owned(result) = pattern.replace_all(text, replacement) {
                    *text = result;
                }
            }
        }
    }

    let mut labeled_hits = 0usize;
    if let Cow::Owned(result) = LABELED_SECRET.replace_all(text, |c: &Captures<'_>| {
        let label = &c[1];
        let trimmed_label = label.trim_end();
        // Avoid false positives on natural prose such as "Return only this exact ASCII token: FOO_BAR"
        // where bare unquoted `token:` is a noun label rather than a credential assignment
        // (`token=abc`, `"token": "abc"`, `access_token: abc`, `real_token: abc`).
        let is_bare_unquoted_prose_token = trimmed_label.ends_with(':')
            && trimmed_label[..trimmed_label.len() - 1]
                .trim_end()
                .eq_ignore_ascii_case("token");
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
        if (is_bare_unquoted_prose_token && quote.is_empty())
            || is_non_secret_labeled_value(inner, !quote.is_empty())
        {
            return c[0].to_string();
        }
        labeled_hits = labeled_hits.saturating_add(1);
        format!("{}{quote}[CREDENTIAL]{quote}", &c[1])
    }) {
        *text = result;
    }
    count = count.saturating_add(labeled_hits);

    count
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JsonContext {
    WireRequest,
    ResponseText,
    Schema,
    Data,
}

fn redact_tool_data(
    value: &mut Value,
    secrets: &[&str],
    options: RedactionOptions,
    depth: usize,
) -> usize {
    redact_value(value, secrets, options, depth, JsonContext::Data)
}

// A property named "password" in a tool schema describes a field. The same
// name in a tool result contains a value that must be removed.
fn redact_value(
    value: &mut Value,
    extra_secrets: &[&str],
    options: RedactionOptions,
    embedded_depth: usize,
    context: JsonContext,
) -> usize {
    match value {
        Value::Object(fields) => {
            let mut count = 0usize;
            let mut renames = Vec::new();
            for (key, child) in fields.iter_mut() {
                if context != JsonContext::Schema && secret_field(key) {
                    if child.as_str() != Some("[CREDENTIAL]") {
                        *child = Value::from("[CREDENTIAL]");
                        count = count.saturating_add(1);
                    }
                } else {
                    let child_context = match (context, key.as_str()) {
                        (JsonContext::WireRequest, "text") => JsonContext::ResponseText,
                        (JsonContext::WireRequest, "tools" | "response_format")
                        | (JsonContext::ResponseText, "format") => JsonContext::Schema,
                        (
                            JsonContext::Schema,
                            "default" | "examples" | "example" | "const" | "enum",
                        ) => JsonContext::Data,
                        (JsonContext::Schema, _) => JsonContext::Schema,
                        _ => JsonContext::Data,
                    };
                    count = count.saturating_add(redact_value(
                        child,
                        extra_secrets,
                        options,
                        embedded_depth,
                        child_context,
                    ));
                }
                let mut masked = key.clone();
                let key_hits = redact_string_in_place(&mut masked, extra_secrets, options);
                if key_hits > 0 && masked != *key {
                    count = count.saturating_add(key_hits);
                    renames.push((key.clone(), masked));
                }
            }
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
            count
        }
        Value::Array(list) => {
            let mut count = 0usize;
            for child in list {
                count = count.saturating_add(redact_value(
                    child,
                    extra_secrets,
                    options,
                    embedded_depth,
                    context,
                ));
            }
            count
        }
        Value::String(text) => {
            if embedded_depth < 3 && text.trim_start().starts_with(['{', '[']) {
                if let Ok(mut decoded) = serde_json::from_str::<Value>(text) {
                    let sub_count =
                        redact_tool_data(&mut decoded, extra_secrets, options, embedded_depth + 1);
                    if sub_count > 0 {
                        *text = decoded.to_string();
                    }
                    return sub_count;
                }
            }
            redact_string_in_place(text, extra_secrets, options)
        }
        Value::Number(number) => {
            let original = number.to_string();
            let mut text = original.clone();
            let hits = redact_string_in_place(&mut text, extra_secrets, options);
            if hits > 0 && text != original {
                *value = Value::String(text);
                hits
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// Wire privacy redactor enforcing always-on credential redaction across all endpoints
/// and endpoint-aware PII redaction (`[EMAIL]`, `[PHONE]`, `[ID]`).
#[derive(Debug, Default, Clone, Copy)]
pub struct PrivacyRedactor;

impl PrivacyRedactor {
    /// Backward-compatible high-confidence secret redactor for logs, UI summaries, and FFI.
    /// Preserves legacy placeholder names (`[REDACTED_API_KEY]`, `[REDACTED_GITHUB_TOKEN]`,
    /// `[REDACTED_AWS_KEY]`, `[REDACTED_PRIVATE_KEY]`) while also catching `AIza`, `xox*`,
    /// URL userinfo, and Bearer/Basic tokens.
    pub fn redact_text(input: &str) -> String {
        if input.is_empty() {
            return String::new();
        }
        let mut out = input.to_string();
        for (re, replacement) in [
            (&*PRIVATE_KEY, "[REDACTED_PRIVATE_KEY]"),
            (&*LEGACY_SK, "[REDACTED_API_KEY]"),
            (&*LEGACY_GH, "[REDACTED_GITHUB_TOKEN]"),
            (&*LEGACY_AWS, "[REDACTED_AWS_KEY]"),
            (&*LEGACY_OTHER_KEYS, "[CREDENTIAL]"),
        ] {
            if let Cow::Owned(result) = re.replace_all(&out, replacement) {
                out = result;
            }
        }
        if let Cow::Owned(result) = USERINFO.replace_all(&out, |c: &Captures<'_>| {
            if &c[2] == "[CREDENTIAL]" || &c[2] == "[REDACTED]" {
                c[0].to_string()
            } else {
                format!("{}[CREDENTIAL]@", &c[1])
            }
        }) {
            out = result;
        }
        if let Cow::Owned(result) = BEARER.replace_all(&out, |c: &Captures<'_>| {
            format!("{}[CREDENTIAL]", &c[1])
        }) {
            out = result;
        }
        out
    }

    /// Backward-compatible recursive JSON string redactor.
    pub fn redact_json_value(val: &mut Value) -> bool {
        match val {
            Value::String(s) => {
                let red = Self::redact_text(s);
                if red != *s {
                    *s = red;
                    true
                } else {
                    false
                }
            }
            Value::Array(arr) => {
                let mut changed = false;
                for item in arr {
                    if Self::redact_json_value(item) {
                        changed = true;
                    }
                }
                changed
            }
            Value::Object(map) => {
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

    /// Full wire redaction over arbitrary JSON values for a target `endpoint`.
    ///
    /// Credential redaction (keys, bearer, userinfo, secret field names, single-line
    /// labeled secrets, and `extra_secrets`) is ALWAYS active, including on LAN/local
    /// endpoints. PII redaction (`[EMAIL]`, `[PHONE]`, `[ID]`) follows `should_redact_pii(endpoint)`.
    /// Returns the total number of redactions performed.
    pub fn redact_json_value_for_endpoint(
        &self,
        value: &mut Value,
        endpoint: &str,
        extra_secrets: &[&str],
    ) -> usize {
        let options = RedactionOptions {
            redact_pii: should_redact_pii(endpoint),
        };
        redact_value(value, extra_secrets, options, 0, JsonContext::WireRequest)
    }

    /// Full wire text redaction for a target `endpoint` with explicit `extra_secrets`.
    pub fn redact_text_for_endpoint(
        &self,
        input: &str,
        endpoint: &str,
        extra_secrets: &[&str],
    ) -> (String, usize) {
        let options = RedactionOptions {
            redact_pii: should_redact_pii(endpoint),
        };
        let mut out = input.to_string();
        let count = redact_string_in_place(&mut out, extra_secrets, options);
        (out, count)
    }

    /// Sanitize a `StructuredModelRequest` before sending to `endpoint`.
    /// Credential redaction is ALWAYS enforced (even on loopback/LAN); PII redaction
    /// follows `should_redact_pii(endpoint)`.
    pub fn sanitize_request(endpoint: &str, req: &StructuredModelRequest) -> StructuredModelRequest {
        Self::sanitize_request_with_secrets(endpoint, req, &[])
    }

    /// Sanitize a `StructuredModelRequest` before sending to `endpoint`, also scrubbing
    /// any live `extra_secrets` (such as the active route's `api_key`).
    pub fn sanitize_request_with_secrets(
        endpoint: &str,
        req: &StructuredModelRequest,
        extra_secrets: &[&str],
    ) -> StructuredModelRequest {
        let options = RedactionOptions {
            redact_pii: should_redact_pii(endpoint),
        };
        let mut out = req.clone();
        for tool in &mut out.prefix_stable_tools {
            redact_string_in_place(&mut tool.description, extra_secrets, options);
            redact_value(
                &mut tool.parameters,
                extra_secrets,
                options,
                0,
                JsonContext::Schema,
            );
        }
        if let Some(schema) = &mut out.json_schema {
            redact_value(schema, extra_secrets, options, 0, JsonContext::Schema);
        }
        for block in &mut out.frozen_system_prefix {
            redact_string_in_place(&mut block.text, extra_secrets, options);
        }
        for rem in &mut out.ephemeral_tail_reminders {
            redact_string_in_place(&mut rem.text, extra_secrets, options);
        }
        for msg in &mut out.history {
            if let HistoryMessage::User { media, .. } | HistoryMessage::Tool { media, .. } = msg {
                for file in media {
                    redact_string_in_place(&mut file.filename, extra_secrets, options);
                }
            }
            match msg {
                HistoryMessage::User { content, .. } => {
                    redact_string_in_place(content, extra_secrets, options);
                }
                HistoryMessage::Assistant {
                    content,
                    reasoning,
                    tool_calls,
                    continuation,
                } => {
                    redact_string_in_place(content, extra_secrets, options);
                    if let Some(r) = reasoning {
                        redact_string_in_place(r, extra_secrets, options);
                    }
                    for tc in tool_calls {
                        redact_tool_data(&mut tc.arguments, extra_secrets, options, 0);
                    }
                    if let Some(cont) = continuation {
                        let mut mutated = false;
                        for item in &mut cont.items {
                            // Preserve opaque encrypted reasoning items untouched.
                            let is_encrypted_reasoning = item
                                .get("type")
                                .and_then(Value::as_str)
                                == Some("reasoning")
                                && item.get("encrypted_content").is_some();
                            if is_encrypted_reasoning {
                                continue;
                            }
                            if redact_tool_data(item, extra_secrets, options, 0) > 0 {
                                mutated = true;
                            }
                        }
                        // If a signed Anthropic thinking block was modified by redaction, drop
                        // continuation so Anthropic does not reject an invalidated signature.
                        if mutated
                            && cont.protocol == nola_types::ModelProtocol::AnthropicMessages
                        {
                            *continuation = None;
                        }
                    }
                }
                HistoryMessage::Tool { payload, .. } => match payload {
                    ToolModelPayload::Text(t) => {
                        if t.trim_start().starts_with(['{', '[']) {
                            if let Ok(mut decoded) = serde_json::from_str::<Value>(t) {
                                if redact_tool_data(&mut decoded, extra_secrets, options, 0) > 0 {
                                    *t = decoded.to_string();
                                }
                                continue;
                            }
                        }
                        redact_string_in_place(t, extra_secrets, options);
                    }
                    ToolModelPayload::Structured {
                        model_data,
                        ui_summary,
                    } => {
                        redact_string_in_place(ui_summary, extra_secrets, options);
                        redact_tool_data(model_data, extra_secrets, options, 0);
                    }
                },
            }
        }
        out
    }
}
