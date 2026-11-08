// SPDX-License-Identifier: Apache-2.0
//! Bounded typed nested preparation; no owner authority or live tool permission.
use crate::semantic_json_projection::{
    extract_array_items, extract_top_level, ExtractedValue, ScalarValue, TokenKind,
    TokenProjection, TokenSpan,
};
use crate::{StoreError, StoreResult};
use serde::Deserialize;
mod evidence;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NestedKind {
    Origin,
    Permission,
    Artifact,
    Artifacts,
    AffectedResources,
    Evidence,
    Location,
    Range,
    RetainedItemIds,
    ToolOutputReference,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NestedIssueCode {
    WrongType,
    MissingField,
    DuplicateField,
    InvalidEnum,
    InvalidResource,
    DuplicateItem,
    UnknownField,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NestedIssue {
    pub code: NestedIssueCode,
    pub field: Option<&'static str>,
    pub item_index: Option<usize>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NestedStatus {
    Validated,
    Deferred,
    Unsupported(NestedIssue),
}
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AffectedResource {
    pub resource_type: String,
    pub uri: String,
    pub operation: String,
}
use nola_domain::ArtifactReference;
#[derive(Debug)]
pub struct ResourceItem {
    pub span: TokenSpan,
    pub value: AffectedResource,
}
#[derive(Debug)]
pub struct NestedDraft {
    pub kind: NestedKind,
    pub status: NestedStatus,
    pub source_sha256: String,
    /// Object schemas retain decoded bounded scalars once, in original token evidence.
    pub object_tokens: Option<TokenProjection>,
    /// Only present when the complete array has been validated. Never a prefix.
    pub resources: Vec<ResourceItem>,
    pub total_items: Option<u64>,
    /// Applies only to origin; omitted kind has the actual Core default Unknown.
    pub effective_origin_kind: Option<&'static str>,
}
pub const ARTIFACT_FIELDS: &[&str] = &["uri", "name", "mime_type", "operation"];
pub const ORIGIN_FIELDS: &[&str] = &["kind", "provider_id", "provider_title", "namespace"];
pub const PERMISSION_FIELDS: &[&str] =
    &["request_id", "permission_domain", "rationale", "decision"];
const TOOL_OUTPUT_FIELDS: &[&str] = &["message_id", "part_id", "tool_call_id"];
const ORIGIN_KINDS: &[&str] = &[
    "unknown",
    "builtin",
    "skill",
    "mcp",
    "device",
    "extension",
    "internal",
];
fn issue(
    code: NestedIssueCode,
    field: Option<&'static str>,
    item_index: Option<usize>,
) -> NestedStatus {
    NestedStatus::Unsupported(NestedIssue {
        code,
        field,
        item_index,
    })
}
/// The caller supplies an already owner-bound token subsource. Offsets returned
/// here are relative to that token; the owning adapter must rebase locators.
/// Null/absent optional parents are handled by the outer draft, not invented here.
pub fn validate_nested<F, I>(
    mut open: F,
    total_bytes: u64,
    kind: NestedKind,
    maximum_output_bytes: usize,
) -> StoreResult<NestedDraft>
where
    F: FnMut() -> I,
    I: Iterator<Item = Result<(u64, String), String>>,
{
    let reserved = std::mem::size_of::<NestedDraft>()
        + if kind == NestedKind::AffectedResources {
            16 * std::mem::size_of::<ResourceItem>()
        } else {
            0
        };
    let budget = maximum_output_bytes
        .checked_sub(reserved)
        .filter(|_| maximum_output_bytes <= 32768)
        .ok_or_else(|| {
            StoreError::Invariant("nested draft budget cannot hold fixed metadata".into())
        })?;
    if matches!(
        kind,
        NestedKind::Evidence | NestedKind::Location | NestedKind::Range
    ) {
        return evidence::validate(&mut open, total_bytes, kind, budget);
    }
    if kind == NestedKind::RetainedItemIds {
        let array = extract_array_items(&mut open, total_bytes, 64, budget)?;
        let mut draft = NestedDraft {
            kind,
            status: NestedStatus::Validated,
            source_sha256: array.source_sha256,
            object_tokens: None,
            resources: Vec::new(),
            total_items: Some(array.total_items),
            effective_origin_kind: None,
        };
        if array.root_kind != TokenKind::Array {
            draft.status = issue(NestedIssueCode::WrongType, None, None);
            return Ok(draft);
        }
        if array.total_items > 64 || array.items.iter().any(|item| item.raw_json.is_none()) {
            draft.status = NestedStatus::Deferred;
            return Ok(draft);
        }
        let mut unique = std::collections::BTreeSet::new();
        for (index, item) in array.items.into_iter().enumerate() {
            if item.span.kind != TokenKind::String {
                draft.status = issue(NestedIssueCode::WrongType, None, Some(index));
                return Ok(draft);
            }
            let raw = item.raw_json.expect("bounded retained item id");
            let id: String = serde_json::from_str(&raw)?;
            if id.is_empty() {
                draft.status = issue(NestedIssueCode::WrongType, None, Some(index));
                return Ok(draft);
            }
            if !unique.insert(id) {
                draft.status = issue(NestedIssueCode::DuplicateItem, None, Some(index));
                return Ok(draft);
            }
        }
        return Ok(draft);
    }
    if matches!(kind, NestedKind::Artifacts | NestedKind::AffectedResources) {
        let array = extract_array_items(&mut open, total_bytes, 16, budget)?;
        let mut draft = NestedDraft {
            kind,
            status: NestedStatus::Validated,
            source_sha256: array.source_sha256,
            object_tokens: None,
            resources: Vec::new(),
            total_items: Some(array.total_items),
            effective_origin_kind: None,
        };
        if array.root_kind != TokenKind::Array {
            draft.status = issue(NestedIssueCode::WrongType, None, None);
            return Ok(draft);
        }
        if array.total_items > 16 || array.items.iter().any(|item| item.raw_json.is_none()) {
            draft.status = NestedStatus::Deferred;
            return Ok(draft);
        }
        let mut resources = Vec::with_capacity(array.items.len());
        for (index, item) in array.items.into_iter().enumerate() {
            if item.span.kind != TokenKind::Object {
                draft.status = issue(NestedIssueCode::WrongType, None, Some(index));
                return Ok(draft);
            }
            // Entire item has already fit the shared bounded raw budget. Using
            // serde here cannot materialize a large item or the complete array.
            let raw = item
                .raw_json
                .expect("all retained items have bounded raw bytes");
            if kind == NestedKind::Artifacts {
                if serde_json::from_str::<ArtifactReference>(&raw).is_err() {
                    draft.status = issue(NestedIssueCode::InvalidResource, None, Some(index));
                    return Ok(draft);
                }
            } else {
                match serde_json::from_str::<AffectedResource>(&raw) {
                    Ok(value) => resources.push(ResourceItem {
                        span: item.span,
                        value,
                    }),
                    Err(_) => {
                        draft.status = issue(NestedIssueCode::InvalidResource, None, Some(index));
                        return Ok(draft);
                    }
                }
            }
        }
        draft.resources = resources;
        return Ok(draft);
    }
    let fields = if kind == NestedKind::Origin {
        ORIGIN_FIELDS
    } else if kind == NestedKind::Artifact {
        ARTIFACT_FIELDS
    } else if kind == NestedKind::ToolOutputReference {
        TOOL_OUTPUT_FIELDS
    } else {
        PERMISSION_FIELDS
    };
    let tokens = extract_top_level(&mut open, total_bytes, fields, budget)?;
    let mut status = NestedStatus::Validated;
    let mut effective_origin_kind = None;
    if tokens.root_kind != TokenKind::Object {
        status = issue(NestedIssueCode::WrongType, None, None);
    } else if tokens.has_unknown_fields {
        status = issue(NestedIssueCode::UnknownField, None, None);
    } else {
        for (index, name) in fields.iter().enumerate() {
            let Some(field) = tokens
                .fields
                .iter()
                .find(|field| field.field_index == index)
            else {
                if (kind == NestedKind::Permission && index < 3)
                    || (kind == NestedKind::Artifact && index < 2)
                    || kind == NestedKind::ToolOutputReference
                {
                    status = issue(NestedIssueCode::MissingField, Some(name), None);
                    break;
                }
                if kind == NestedKind::Origin && index == 0 {
                    effective_origin_kind = Some("unknown");
                }
                continue;
            };
            if field.duplicate_known_key {
                status = issue(NestedIssueCode::DuplicateField, Some(name), None);
                break;
            }
            let nullable = if kind == NestedKind::Origin {
                index > 0
            } else if kind == NestedKind::Artifact {
                index >= 2
            } else if kind == NestedKind::ToolOutputReference {
                false
            } else {
                index == 3
            };
            if nullable && field.span.kind == TokenKind::Null {
                continue;
            }
            if field.span.kind != TokenKind::String {
                status = issue(NestedIssueCode::WrongType, Some(name), None);
                break;
            }
            match &field.value {
                ExtractedValue::Deferred => {
                    status = NestedStatus::Deferred;
                }
                ExtractedValue::Complete(ScalarValue::String(value)) => {
                    if kind == NestedKind::Origin && index == 0 {
                        let Some(known) = ORIGIN_KINDS.iter().find(|known| **known == value) else {
                            status = issue(NestedIssueCode::InvalidEnum, Some(name), None);
                            break;
                        };
                        effective_origin_kind = Some(*known);
                    }
                }
                _ => {
                    status = issue(NestedIssueCode::WrongType, Some(name), None);
                    break;
                }
            }
        }
    }
    let source_sha256 = tokens.source_sha256.clone();
    Ok(NestedDraft {
        kind,
        status,
        source_sha256,
        object_tokens: Some(tokens),
        resources: Vec::new(),
        total_items: None,
        effective_origin_kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artifact_reference_uses_actual_required_optional_and_duplicate_rules() {
        for (raw, expected) in [
            (
                r#"{"uri":"file:///a","name":"a","mime_type":null,"operation":"attach"}"#,
                NestedStatus::Validated,
            ),
            (
                r#"{"uri":"file:///a","name":"a","mime_type":null,"operation":"attach","extension":true}"#,
                issue(NestedIssueCode::UnknownField, None, None),
            ),
            (
                r#"{"uri":"a"}"#,
                issue(NestedIssueCode::MissingField, Some("name"), None),
            ),
            (
                r#"{"uri":"a","name":"a","name":"b"}"#,
                issue(NestedIssueCode::DuplicateField, Some("name"), None),
            ),
            (
                r#"{"uri":null,"name":"a"}"#,
                issue(NestedIssueCode::WrongType, Some("uri"), None),
            ),
        ] {
            assert_eq!(
                serde_json::from_str::<nola_domain::ArtifactReference>(raw).is_ok(),
                expected == NestedStatus::Validated,
                "shared production artifact wire: {raw}"
            );
            let draft = validate_nested(
                || chunks(raw, 7).into_iter(),
                raw.len() as u64,
                NestedKind::Artifact,
                8192,
            )
            .unwrap();
            assert_eq!(draft.status, expected, "{raw}");
        }
        let prefix = r#"{"uri":"file:///a","name":""#;
        let suffix = "\"}";
        let length = 10_000_000usize;
        let draft = validate_nested(
            || {
                let mut offset = 0u64;
                std::iter::once(prefix.to_owned())
                    .chain(
                        (0..length.div_ceil(32768))
                            .map(move |index| "x".repeat((length - index * 32768).min(32768))),
                    )
                    .chain(std::iter::once(suffix.to_owned()))
                    .map(move |text| {
                        let start = offset;
                        offset += text.len() as u64;
                        Ok((start, text))
                    })
            },
            (prefix.len() + length + suffix.len()) as u64,
            NestedKind::Artifact,
            8192,
        )
        .unwrap();
        assert_eq!(draft.status, NestedStatus::Deferred);
    }

    #[test]
    fn tool_artifacts_validate_the_actual_agent_part_array_shape() {
        let valid = r#"[{"uri":"file:///tmp/report.docx","name":"report.docx","mime_type":"application/octet-stream","operation":"create"}]"#;
        assert_eq!(
            parse(valid, NestedKind::Artifacts).status,
            NestedStatus::Validated
        );
        assert!(matches!(
            parse(
                r#"[{"uri":"file:///tmp/report.docx"}]"#,
                NestedKind::Artifacts
            )
            .status,
            NestedStatus::Unsupported(NestedIssue {
                code: NestedIssueCode::InvalidResource,
                ..
            })
        ));
    }
    fn chunks(input: &str, size: usize) -> Vec<Result<(u64, String), String>> {
        let mut start = 0;
        let mut values = Vec::new();
        while start < input.len() {
            let mut end = (start + size).min(input.len());
            while !input.is_char_boundary(end) {
                end += 1;
            }
            values.push(Ok((start as u64, input[start..end].to_string())));
            start = end;
        }
        values
    }
    fn parse(input: &str, kind: NestedKind) -> NestedDraft {
        validate_nested(
            || chunks(input, 1).into_iter(),
            input.len() as u64,
            kind,
            8192,
        )
        .unwrap()
    }
    #[test]
    fn origin_defaults_exact_enum_nullable_fields_and_duplicates_match_actual_shape() {
        let draft = parse("{}", NestedKind::Origin);
        assert_eq!(draft.status, NestedStatus::Validated);
        assert_eq!(draft.effective_origin_kind, Some("unknown"));
        for kind in ORIGIN_KINDS {
            let input=serde_json::json!({"kind":kind,"provider_id":"id","provider_title":"title","namespace":null}).to_string();
            let draft = parse(&input, NestedKind::Origin);
            assert_eq!(draft.status, NestedStatus::Validated);
            assert_eq!(draft.effective_origin_kind, Some(*kind));
        }
        for (input, code) in [
            (r#"{"kind":"custom"}"#, NestedIssueCode::InvalidEnum),
            (r#"{"kind":null}"#, NestedIssueCode::WrongType),
            (r#"{"provider_title":1}"#, NestedIssueCode::WrongType),
            (
                r#"{"kind":"mcp","\u006bind":"device"}"#,
                NestedIssueCode::DuplicateField,
            ),
        ] {
            assert!(
                matches!(parse(input,NestedKind::Origin).status,NestedStatus::Unsupported(NestedIssue{code:actual,..}) if actual==code)
            );
        }
        assert_eq!(
            parse(
                r#"{"future":{"large":[1,2]},"provider_id":null}"#,
                NestedKind::Origin
            )
            .status,
            issue(NestedIssueCode::UnknownField, None, None)
        );
        assert!(serde_json::from_str::<nola_domain::HostCapabilityOrigin>(
            r#"{"future":{"large":[1,2]},"provider_id":null}"#
        )
        .is_err());
        assert_eq!(
            parse(r#"{"provider_id":null}"#, NestedKind::Origin).status,
            NestedStatus::Validated
        );
        assert!(serde_json::from_str::<nola_domain::HostCapabilityOrigin>(
            r#"{"provider_id":null}"#
        )
        .is_ok());
    }
    #[test]
    fn permission_preserves_request_fields_without_granting_execution_or_inventing_decisions() {
        let input = r#"{"request_id":"r","permission_domain":"fs","rationale":"中😀\nreason","decision":null}"#;
        let draft = parse(input, NestedKind::Permission);
        assert_eq!(draft.status, NestedStatus::Validated);
        let tokens = draft.object_tokens.unwrap();
        let rationale = tokens
            .fields
            .iter()
            .find(|field| field.field_index == 2)
            .unwrap();
        assert_eq!(
            rationale.value,
            ExtractedValue::Complete(ScalarValue::String("中😀\nreason".into()))
        );
        assert_eq!(
            &input[rationale.span.start_bytes as usize..rationale.span.end_bytes as usize],
            r#""中😀\nreason""#
        );
        for input in [
            r#"{"request_id":"r","permission_domain":"fs"}"#,
            r#"{"request_id":1,"permission_domain":"fs","rationale":"x"}"#,
            r#"{"request_id":"r","request_id":"other","permission_domain":"fs","rationale":"x"}"#,
        ] {
            assert!(matches!(
                parse(input, NestedKind::Permission).status,
                NestedStatus::Unsupported(_)
            ));
        }
        let huge = format!(
            "{{\"rationale\":\"{}\",\"request_id\":\"r\",\"permission_domain\":\"fs\"}}",
            "x".repeat(10000)
        );
        assert_eq!(
            parse(&huge, NestedKind::Permission).status,
            NestedStatus::Deferred
        );
    }
    #[test]
    fn resources_validate_every_retained_object_against_domain_and_preserve_exact_spans() {
        let input = r#" [ {"resource_type":"file","uri":"中😀","operation":"read"}, {"operation":"write","uri":"b","resource_type":"file"} ] "#;
        let expected: Vec<nola_domain::projection::AffectedResourceView> =
            serde_json::from_str(input).unwrap();
        for size in [1, 7, 32768] {
            let draft = validate_nested(
                || chunks(input, size).into_iter(),
                input.len() as u64,
                NestedKind::AffectedResources,
                8192,
            )
            .unwrap();
            assert_eq!(draft.status, NestedStatus::Validated);
            assert_eq!(draft.total_items, Some(2));
            assert_eq!(draft.resources.len(), 2);
            for (item, expected) in draft.resources.iter().zip(&expected) {
                assert_eq!(item.value.uri, expected.uri);
                assert_eq!(item.value.resource_type, expected.resource_type);
                assert_eq!(item.value.operation, expected.operation);
                let raw = &input[item.span.start_bytes as usize..item.span.end_bytes as usize];
                let actual: nola_domain::projection::AffectedResourceView =
                    serde_json::from_str(raw).unwrap();
                assert_eq!(&actual, expected);
            }
        }
        for input in [
            r#"[{}]"#,
            r#"[{"resource_type":"file","uri":"a","operation":"read","extra":null}]"#,
            r#"[{"resource_type":"file","uri":"a","\u0075ri":"b","operation":"read"}]"#,
            r#"[{"resource_type":null,"uri":"a","operation":"read"}]"#,
        ] {
            assert!(
                serde_json::from_str::<Vec<nola_domain::projection::AffectedResourceView>>(input)
                    .is_err()
            );
            let draft = parse(input, NestedKind::AffectedResources);
            assert!(matches!(draft.status, NestedStatus::Unsupported(_)));
            assert!(draft.resources.is_empty());
        }
        assert_eq!(
            parse("[]", NestedKind::AffectedResources).status,
            NestedStatus::Validated
        );
        let many = format!(
            "[{}]",
            vec![r#"{"resource_type":"file","uri":"a","operation":"read"}"#; 17].join(",")
        );
        let draft = parse(&many, NestedKind::AffectedResources);
        assert_eq!(draft.status, NestedStatus::Deferred);
        assert_eq!(draft.total_items, Some(17));
        assert!(draft.resources.is_empty());
    }

    #[test]
    fn retained_compaction_ids_are_complete_bounded_unique_strings() {
        let input = r#"["message:1","tool:2","中😀"]"#;
        let draft = parse(input, NestedKind::RetainedItemIds);
        assert_eq!(draft.status, NestedStatus::Validated);
        assert_eq!(draft.total_items, Some(3));
        for input in [r#"["a","a"]"#, r#"["a",1]"#, r#"[""]"#] {
            assert!(matches!(
                parse(input, NestedKind::RetainedItemIds).status,
                NestedStatus::Unsupported(_)
            ));
        }
        let many = format!(
            "[{}]",
            (0..65)
                .map(|index| format!("\"item-{index}\""))
                .collect::<Vec<_>>()
                .join(",")
        );
        assert_eq!(
            parse(&many, NestedKind::RetainedItemIds).status,
            NestedStatus::Deferred
        );
    }
    #[test]
    fn source_mutation_syntax_and_oversized_items_never_validate_as_complete() {
        let large = format!(
            "[{{\"resource_type\":\"file\",\"uri\":\"{}\",\"operation\":\"read\"}}]",
            "x".repeat(10000)
        );
        let draft = parse(&large, NestedKind::AffectedResources);
        assert_eq!(draft.status, NestedStatus::Deferred);
        assert!(draft.resources.is_empty());
        for (kind, input) in [
            (NestedKind::AffectedResources, "[{},]"),
            (NestedKind::Origin, r#"{"provider_id":"\uD800"}"#),
            (NestedKind::Permission, "{} garbage"),
        ] {
            assert!(validate_nested(
                || chunks(input, 1).into_iter(),
                input.len() as u64,
                kind,
                8192
            )
            .is_err());
        }
        let mut pass = 0;
        let input = r#"[{"resource_type":"file","uri":"a","operation":"read"}]"#;
        assert!(validate_nested(
            || {
                pass += 1;
                chunks(
                    &if pass == 1 {
                        input.to_string()
                    } else {
                        input.replace("\"a\"", "\"b\"")
                    },
                    1,
                )
                .into_iter()
            },
            input.len() as u64,
            NestedKind::AffectedResources,
            8192
        )
        .unwrap_err()
        .to_string()
        .contains("changed"));
        assert!(validate_nested(
            || chunks("[]", 1).into_iter(),
            2,
            NestedKind::AffectedResources,
            16
        )
        .is_err());
    }
}
