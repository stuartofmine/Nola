// SPDX-License-Identifier: Apache-2.0
//! Rules taken from agent::parts and memory::MemoryUsage, not a new wire schema.
use super::*;
use sha2::{Digest, Sha256};
const EVIDENCE: &[&str] = &["kind", "usage"];
const LOCATION: &[&str] = &["page", "line_start", "line_end", "section"];
const RANGE: &[&str] = &["start", "end"];
const USAGE: &[&str] = &[
    "record_id",
    "component_id",
    "scope",
    "namespace",
    "provenance_kind",
    "source",
    "title",
    "observed_at",
    "source_role",
    "event_time",
    "range_start",
    "range_end",
    "requires_current_state_verification",
    "truncated",
    "supersedes",
    "usage_count",
    "last_used_at_epoch_ms",
];
const NAMESPACE: &[&str] = &["kind", "name"];
#[derive(Clone, Copy)]
enum Shape {
    String,
    Bool,
    U64,
    I64,
    Object,
    Array,
}
fn field<'a>(
    tokens: &'a TokenProjection,
    names: &[&str],
    name: &str,
) -> Option<&'a crate::semantic_json_projection::FieldToken> {
    let index = names.iter().position(|v| *v == name)?;
    tokens.fields.iter().find(|v| v.field_index == index)
}
fn fail(code: NestedIssueCode, name: &'static str) -> NestedStatus {
    issue(code, Some(name), None)
}
fn check(
    tokens: &TokenProjection,
    names: &[&str],
    name: &'static str,
    shape: Shape,
    required: bool,
    nullable: bool,
) -> NestedStatus {
    let Some(value) = field(tokens, names, name) else {
        return if required {
            fail(NestedIssueCode::MissingField, name)
        } else {
            NestedStatus::Validated
        };
    };
    if value.duplicate_known_key {
        return fail(NestedIssueCode::DuplicateField, name);
    }
    if nullable && value.span.kind == TokenKind::Null {
        return NestedStatus::Validated;
    }
    let wanted = match shape {
        Shape::String => TokenKind::String,
        Shape::Bool => TokenKind::Bool,
        Shape::U64 | Shape::I64 => TokenKind::Number,
        Shape::Object => TokenKind::Object,
        Shape::Array => TokenKind::Array,
    };
    if value.span.kind != wanted {
        return fail(NestedIssueCode::WrongType, name);
    }
    if matches!(shape, Shape::Object | Shape::Array) {
        return NestedStatus::Validated;
    }
    match &value.value {
        ExtractedValue::Deferred => NestedStatus::Deferred,
        ExtractedValue::Complete(ScalarValue::Number(n))
            if matches!(shape, Shape::U64) && n.as_u64().is_none()
                || matches!(shape, Shape::I64) && n.as_i64().is_none() =>
        {
            fail(NestedIssueCode::WrongType, name)
        }
        _ => NestedStatus::Validated,
    }
}
fn merge(status: &mut NestedStatus, next: NestedStatus) {
    if matches!(next, NestedStatus::Unsupported(_)) || *status == NestedStatus::Validated {
        *status = next;
    }
}
fn enumeration(
    tokens: &TokenProjection,
    names: &[&str],
    name: &'static str,
    allowed: &[&str],
) -> NestedStatus {
    let Some(value) = field(tokens, names, name) else {
        return NestedStatus::Validated;
    };
    if let ExtractedValue::Complete(ScalarValue::String(value)) = &value.value {
        if !allowed.contains(&value.as_str()) {
            return fail(NestedIssueCode::InvalidEnum, name);
        }
    }
    NestedStatus::Validated
}
/// Every sub-pass hashes and drains the full parent source, including its suffix.
/// Moving/truncating owned chunks avoids a second resident source-sized buffer.
fn subsource<I>(
    mut input: I,
    span: TokenSpan,
    total: u64,
    expected_hash: String,
) -> impl Iterator<Item = Result<(u64, String), String>>
where
    I: Iterator<Item = Result<(u64, String), String>>,
{
    let mut received = 0u64;
    let mut hash = Sha256::new();
    let mut done = false;
    std::iter::from_fn(move || {
        if done {
            return None;
        }
        loop {
            match input.next() {
                Some(Ok((offset, mut text))) => {
                    let end = offset.checked_add(text.len() as u64);
                    if offset != received
                        || text.is_empty()
                        || text.len() > 32768
                        || end.is_none_or(|v| v > total)
                    {
                        done = true;
                        return Some(Err("invalid nested parent chunk geometry".into()));
                    }
                    let end = end.unwrap();
                    received = end;
                    hash.update(text.as_bytes());
                    let start = offset.max(span.start_bytes);
                    let stop = end.min(span.end_bytes);
                    if start < stop {
                        let lo = (start - offset) as usize;
                        let hi = (stop - offset) as usize;
                        if !text.is_char_boundary(lo) || !text.is_char_boundary(hi) {
                            done = true;
                            return Some(Err("nested token is not a UTF-8 boundary".into()));
                        }
                        text.truncate(hi);
                        text.drain(..lo);
                        return Some(Ok((start - span.start_bytes, text)));
                    }
                }
                Some(Err(error)) => {
                    done = true;
                    return Some(Err(error));
                }
                None => {
                    done = true;
                    if received != total
                        || format!("{:x}", hash.clone().finalize()) != expected_hash
                    {
                        return Some(Err("nested parent source changed".into()));
                    }
                    return None;
                }
            }
        }
    })
}
fn child_tokens<F, I>(
    open: &mut F,
    total: u64,
    parent_hash: &str,
    span: TokenSpan,
    names: &[&str],
    budget: usize,
) -> StoreResult<TokenProjection>
where
    F: FnMut() -> I,
    I: Iterator<Item = Result<(u64, String), String>>,
{
    extract_top_level(
        || subsource(open(), span, total, parent_hash.into()),
        span.end_bytes - span.start_bytes,
        names,
        budget,
    )
}
pub(super) fn validate<F, I>(
    open: &mut F,
    total: u64,
    kind: NestedKind,
    budget: usize,
) -> StoreResult<NestedDraft>
where
    F: FnMut() -> I,
    I: Iterator<Item = Result<(u64, String), String>>,
{
    let names = match kind {
        NestedKind::Evidence => EVIDENCE,
        NestedKind::Location => LOCATION,
        NestedKind::Range => RANGE,
        _ => unreachable!(),
    };
    // Reserve half for deeper known objects. Other shapes need only one output.
    let level_budget = if kind == NestedKind::Evidence {
        budget / 2
    } else {
        budget
    };
    let tokens = extract_top_level(&mut *open, total, names, level_budget)?;
    let mut status = NestedStatus::Validated;
    if tokens.root_kind != TokenKind::Object {
        status = issue(NestedIssueCode::WrongType, None, None);
    } else if tokens.has_unknown_fields {
        status = issue(NestedIssueCode::UnknownField, None, None);
    } else {
        match kind {
            NestedKind::Location => {
                for name in &LOCATION[..3] {
                    merge(
                        &mut status,
                        check(&tokens, names, name, Shape::U64, false, true),
                    );
                }
                merge(
                    &mut status,
                    check(&tokens, names, "section", Shape::String, false, true),
                );
            }
            NestedKind::Range => {
                for name in RANGE {
                    merge(
                        &mut status,
                        check(&tokens, names, name, Shape::U64, true, false),
                    );
                }
            }
            NestedKind::Evidence => {
                merge(
                    &mut status,
                    check(&tokens, names, "kind", Shape::String, true, false),
                );
                merge(
                    &mut status,
                    enumeration(&tokens, names, "kind", &["memory"]),
                );
                merge(
                    &mut status,
                    check(&tokens, names, "usage", Shape::Object, true, false),
                );
                if status == NestedStatus::Validated {
                    let usage = field(&tokens, names, "usage").unwrap().span;
                    status = usage_status(
                        open,
                        total,
                        &tokens.source_sha256,
                        usage,
                        budget - level_budget,
                    )?;
                }
            }
            _ => unreachable!(),
        }
    }
    Ok(NestedDraft {
        kind,
        status,
        source_sha256: tokens.source_sha256.clone(),
        object_tokens: Some(tokens),
        resources: Vec::new(),
        total_items: None,
        effective_origin_kind: None,
    })
}
fn usage_status<F, I>(
    open: &mut F,
    total: u64,
    hash: &str,
    span: TokenSpan,
    budget: usize,
) -> StoreResult<NestedStatus>
where
    F: FnMut() -> I,
    I: Iterator<Item = Result<(u64, String), String>>,
{
    let tokens = child_tokens(open, total, hash, span, USAGE, budget)?;
    if tokens.has_unknown_fields {
        return Ok(issue(NestedIssueCode::UnknownField, None, None));
    }
    let mut status = NestedStatus::Validated;
    for name in ["record_id", "component_id"] {
        merge(
            &mut status,
            check(&tokens, USAGE, name, Shape::String, true, false),
        );
    }
    for name in [
        "scope",
        "provenance_kind",
        "source",
        "title",
        "observed_at",
        "source_role",
        "event_time",
        "range_start",
        "range_end",
    ] {
        merge(
            &mut status,
            check(&tokens, USAGE, name, Shape::String, false, true),
        );
    }
    for name in ["requires_current_state_verification", "truncated"] {
        merge(
            &mut status,
            check(&tokens, USAGE, name, Shape::Bool, false, false),
        );
    }
    merge(
        &mut status,
        check(&tokens, USAGE, "usage_count", Shape::U64, false, false),
    );
    merge(
        &mut status,
        check(
            &tokens,
            USAGE,
            "last_used_at_epoch_ms",
            Shape::I64,
            false,
            true,
        ),
    );
    merge(
        &mut status,
        check(&tokens, USAGE, "namespace", Shape::Object, false, true),
    );
    merge(
        &mut status,
        check(&tokens, USAGE, "supersedes", Shape::Array, false, false),
    );
    merge(
        &mut status,
        enumeration(&tokens, USAGE, "scope", &["turn", "conversation", "global"]),
    );
    merge(
        &mut status,
        enumeration(
            &tokens,
            USAGE,
            "source_role",
            &["user", "assistant", "tool", "system", "other"],
        ),
    );
    merge(
        &mut status,
        enumeration(
            &tokens,
            USAGE,
            "provenance_kind",
            &[
                "explicit_user",
                "agent_inference",
                "tool_result",
                "imported",
                "synced",
                "other",
            ],
        ),
    );
    if matches!(status, NestedStatus::Unsupported(_)) {
        return Ok(status);
    }
    // Drop scalar storage before inspecting child collections under the same budget.
    let namespace = field(&tokens, USAGE, "namespace")
        .filter(|f| f.span.kind == TokenKind::Object)
        .map(|f| f.span);
    let supersedes = field(&tokens, USAGE, "supersedes").map(|f| f.span);
    drop(tokens);
    if let Some(relative) = namespace {
        let child = TokenSpan {
            start_bytes: span.start_bytes + relative.start_bytes,
            end_bytes: span.start_bytes + relative.end_bytes,
            kind: relative.kind,
        };
        let ns = child_tokens(open, total, hash, child, NAMESPACE, budget)?;
        if ns.has_unknown_fields {
            return Ok(issue(NestedIssueCode::UnknownField, None, None));
        }
        merge(
            &mut status,
            check(&ns, NAMESPACE, "kind", Shape::String, true, false),
        );
        merge(
            &mut status,
            enumeration(
                &ns,
                NAMESPACE,
                "kind",
                &["personal", "workspace", "conversation", "custom"],
            ),
        );
        if let Some(value) = field(&ns, NAMESPACE, "kind") {
            if let ExtractedValue::Complete(ScalarValue::String(value)) = &value.value {
                if value == "custom" {
                    merge(
                        &mut status,
                        check(&ns, NAMESPACE, "name", Shape::String, true, false),
                    );
                } else if let Some(name) = field(&ns, NAMESPACE, "name") {
                    if name.duplicate_known_key {
                        merge(&mut status, fail(NestedIssueCode::DuplicateField, "name"));
                    } else {
                        // Shared Core/Domain unit namespaces have a strict
                        // empty payload. Even name:null is an unknown field.
                        merge(&mut status, fail(NestedIssueCode::UnknownField, "name"));
                    }
                }
            }
        }
    }
    if let Some(relative) = supersedes {
        let child = TokenSpan {
            start_bytes: span.start_bytes + relative.start_bytes,
            end_bytes: span.start_bytes + relative.end_bytes,
            kind: relative.kind,
        };
        let values = extract_array_items(
            || subsource(open(), child, total, hash.into()),
            child.end_bytes - child.start_bytes,
            16,
            budget,
        )?;
        if values.total_items > 16 || values.items.iter().any(|item| item.raw_json.is_none()) {
            merge(&mut status, NestedStatus::Deferred);
        } else if values
            .items
            .iter()
            .any(|item| item.span.kind != TokenKind::String)
        {
            merge(&mut status, fail(NestedIssueCode::WrongType, "supersedes"));
        }
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chunks(value: &str) -> Vec<Result<(u64, String), String>> {
        value
            .as_bytes()
            .chunks(1)
            .enumerate()
            .map(|(i, b)| Ok((i as u64, String::from_utf8(b.to_vec()).unwrap())))
            .collect()
    }
    fn parse(value: &str, kind: NestedKind) -> NestedDraft {
        validate_nested(|| chunks(value).into_iter(), value.len() as u64, kind, 8192).unwrap()
    }
    #[test]
    fn citation_location_and_range_follow_optional_and_unsigned_wire_rules() {
        for value in [
            "{}",
            r#"{"page":null,"section":null}"#,
            r#"{"page":18446744073709551615,"line_start":0,"line_end":2,"section":"s"}"#,
        ] {
            assert!(serde_json::from_str::<nola_domain::SourceLocation>(value).is_ok());
            assert_eq!(
                parse(value, NestedKind::Location).status,
                NestedStatus::Validated
            );
        }
        for value in [
            r#"{"page":-1}"#,
            r#"{"page":1.0}"#,
            r#"{"line_start":1,"line_start":2}"#,
            r#"{"section":true}"#,
            r#"{"page":18446744073709551615,"line_start":0,"line_end":2,"section":"s","future":[]}"#,
        ] {
            assert!(serde_json::from_str::<nola_domain::SourceLocation>(value).is_err());
            assert!(matches!(
                parse(value, NestedKind::Location).status,
                NestedStatus::Unsupported(_)
            ));
        }
        // The actual CitationRange struct imposes types, not an ordering policy.
        for value in [
            r#"{"start":9,"end":1}"#,
            r#"{"start":0,"end":18446744073709551615}"#,
        ] {
            assert!(serde_json::from_str::<nola_domain::CitationRange>(value).is_ok());
            assert_eq!(
                parse(value, NestedKind::Range).status,
                NestedStatus::Validated
            );
        }
        for value in [
            "{}",
            r#"{"start":null,"end":1}"#,
            r#"{"start":0,"end":18446744073709551616}"#,
            r#"{"start":0,"\u0073tart":1,"end":2}"#,
            r#"{"start":0,"end":18446744073709551615,"extra":{}}"#,
        ] {
            assert!(serde_json::from_str::<nola_domain::CitationRange>(value).is_err());
            assert!(matches!(
                parse(value, NestedKind::Range).status,
                NestedStatus::Unsupported(_)
            ));
        }
    }
    #[test]
    fn memory_evidence_checks_usage_defaults_namespace_and_all_known_types() {
        for value in [
            r#"{"kind":"memory","usage":{"record_id":"r","component_id":"c"}}"#,
            r#"{"kind":"memory","usage":{"record_id":"r","component_id":"c","scope":"global","namespace":{"kind":"custom","name":"project"},"provenance_kind":"explicit_user","source":"s","title":null,"observed_at":"now","source_role":"tool","event_time":"event","range_start":"start","range_end":"end","requires_current_state_verification":true,"truncated":false,"supersedes":["old"],"usage_count":18446744073709551615,"last_used_at_epoch_ms":-9223372036854775808}}"#,
            r#"{"kind":"memory","usage":{"record_id":"r","component_id":"c","namespace":{"kind":"personal"}}}"#,
            r#"{"kind":"memory","usage":{"record_id":"r","component_id":"c","namespace":{"kind":"custom","name":""}}}"#,
        ] {
            assert!(serde_json::from_str::<nola_domain::SourceEvidence>(value).is_ok());
            assert_eq!(
                parse(value, NestedKind::Evidence).status,
                NestedStatus::Validated,
                "{value}"
            );
        }
        for usage in [
            r#"{"record_id":"r"}"#,
            r#"{"record_id":"r","component_id":"c","scope":"invalid"}"#,
            r#"{"record_id":"r","component_id":"c","scope":1}"#,
            r#"{"record_id":"r","component_id":"c","namespace":{"kind":"custom"}}"#,
            r#"{"record_id":"r","component_id":"c","namespace":{"kind":"personal","name":"bad"}}"#,
            r#"{"record_id":"r","component_id":"c","namespace":{"kind":"personal","name":null}}"#,
            r#"{"record_id":"r","component_id":"c","namespace":{"kind":"workspace","name":""}}"#,
            r#"{"record_id":"r","component_id":"c","namespace":{"kind":"conversation","name":"project"}}"#,
            r#"{"record_id":"r","component_id":"c","namespace":{"kind":"workspace","future":{}}}"#,
            r#"{"record_id":"r","component_id":"c","namespace":{"kind":"custom","name":"a","name":"b"}}"#,
            r#"{"record_id":"r","component_id":"c","supersedes":[1]}"#,
            r#"{"record_id":"r","component_id":"c","usage_count":null}"#,
            r#"{"record_id":"r","component_id":"c","truncated":null}"#,
            r#"{"record_id":"r","component_id":"c","last_used_at_epoch_ms":9223372036854775808}"#,
            r#"{"record_id":"r","component_id":"c","\u0063omponent_id":"other"}"#,
        ] {
            let value = format!("{{\"kind\":\"memory\",\"usage\":{usage}}}");
            assert!(serde_json::from_str::<nola_domain::SourceEvidence>(&value).is_err());
            assert!(
                matches!(
                    parse(&value, NestedKind::Evidence).status,
                    NestedStatus::Unsupported(_)
                ),
                "{value}"
            );
        }
        for value in [
            r#"{"kind":"other","usage":{}}"#,
            r#"{"kind":"memory","usage":null}"#,
            r#"{"kind":"memory","kind":"memory","usage":{}}"#,
            r#"{"kind":"memory","usage":{"record_id":"r","component_id":"c"},"future":[]}"#,
        ] {
            assert!(serde_json::from_str::<nola_domain::SourceEvidence>(value).is_err());
            assert!(matches!(
                parse(value, NestedKind::Evidence).status,
                NestedStatus::Unsupported(_)
            ));
        }
    }
    #[test]
    fn nested_source_suffix_mutation_is_rejected_and_large_values_remain_deferred() {
        // Keep the suffix a real, allowed field. Unknown extension fields are
        // rejected on the first pass and cannot test later source immutability.
        let input = r#"{"usage":{"record_id":"r","component_id":"c"},"kind":"memory"}"#;
        assert!(serde_json::from_str::<nola_domain::SourceEvidence>(input).is_ok());
        let mut pass = 0;
        let result = validate_nested(
            || {
                pass += 1;
                chunks(&if pass <= 2 {
                    input.into()
                } else {
                    input.replace("\"memory\"", "\"memorz\"")
                })
                .into_iter()
            },
            input.len() as u64,
            NestedKind::Evidence,
            8192,
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("parent source changed"));
        let usage=format!("{{\"kind\":\"memory\",\"usage\":{{\"record_id\":\"r\",\"component_id\":\"c\",\"title\":\"{}\"}}}}","x".repeat(10000));
        assert_eq!(
            parse(&usage, NestedKind::Evidence).status,
            NestedStatus::Deferred
        );
        let usage=format!("{{\"kind\":\"memory\",\"usage\":{{\"record_id\":\"r\",\"component_id\":\"c\",\"supersedes\":[{}]}}}}",vec!["\"old\"";17].join(","));
        assert_eq!(
            parse(&usage, NestedKind::Evidence).status,
            NestedStatus::Deferred
        );
    }

    #[test]
    fn namespace_unit_variants_reject_any_name_but_preserve_duplicate_diagnostics() {
        for kind in ["personal", "workspace", "conversation"] {
            for name in ["null", "\"\"", "\"project\""] {
                let value = format!(
                    "{{\"kind\":\"memory\",\"usage\":{{\"record_id\":\"r\",\"component_id\":\"c\",\"namespace\":{{\"kind\":\"{kind}\",\"name\":{name}}}}}}}"
                );
                assert!(serde_json::from_str::<nola_domain::SourceEvidence>(&value).is_err());
                assert_eq!(
                    parse(&value, NestedKind::Evidence).status,
                    issue(NestedIssueCode::UnknownField, Some("name"), None)
                );
            }
        }
        let duplicate = r#"{"kind":"memory","usage":{"record_id":"r","component_id":"c","namespace":{"kind":"personal","name":null,"name":null}}}"#;
        assert!(serde_json::from_str::<nola_domain::SourceEvidence>(duplicate).is_err());
        assert_eq!(
            parse(duplicate, NestedKind::Evidence).status,
            issue(NestedIssueCode::DuplicateField, Some("name"), None)
        );
    }

    #[test]
    fn bounded_validator_matches_shared_actual_core_wire_cases() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/semantic_nested_wire_cases.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let kind = match case["kind"].as_str().unwrap() {
                "location" => NestedKind::Location,
                "range" => NestedKind::Range,
                "evidence" => NestedKind::Evidence,
                _ => unreachable!(),
            };
            let draft = parse(case["raw"].as_str().unwrap(), kind);
            // Core's rich Parts reuse these exact public wire values. Compare
            // against actual typed deserialization, not just a hand-made flag.
            let raw = case["raw"].as_str().unwrap();
            let typed = match kind {
                NestedKind::Location => {
                    serde_json::from_str::<nola_domain::SourceLocation>(raw).is_ok()
                }
                NestedKind::Range => {
                    serde_json::from_str::<nola_domain::CitationRange>(raw).is_ok()
                }
                NestedKind::Evidence => {
                    serde_json::from_str::<nola_domain::SourceEvidence>(raw).is_ok()
                }
                _ => unreachable!(),
            };
            assert_eq!(
                typed,
                case["accepted"].as_bool().unwrap(),
                "actual shared/Core wire: {case}"
            );
            assert_eq!(
                draft.status == NestedStatus::Validated,
                case["accepted"].as_bool().unwrap(),
                "{case}: {:?}",
                draft.status
            );
        }
    }
}
