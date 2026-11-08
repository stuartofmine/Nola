// SPDX-License-Identifier: Apache-2.0
//! Dormant schema recognition for original AgentPart / MessagePartView JSON.
//! Recognition is neither trusted ownership nor a runnable historical tool.
use crate::semantic_json_projection::{
    extract_top_level, extract_top_level_with_scalar_fields, ExtractedValue, ScalarValue,
    TokenKind, TokenProjection,
};
use crate::{StoreError, StoreResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoricalSchema {
    AgentPart,
    MessagePartView,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsupportedCode {
    NonObject,
    MissingField,
    DuplicateField,
    AmbiguousIdentity,
    UnknownField,
    UnknownKind,
    KindMismatch,
    WrongType,
    DeferredControl,
    InvalidControl,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported {
    pub code: UnsupportedCode,
    pub field: Option<&'static str>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DraftStatus {
    Recognized,
    Unsupported(Unsupported),
}
#[derive(Debug)]
pub struct SemanticPartDraft {
    pub schema: Option<HistoricalSchema>,
    /// Original wire kind; no artifact/step to attachment/task identity rewriting.
    pub kind: Option<String>,
    pub status: DraftStatus,
    pub field_names: Vec<&'static str>,
    pub tokens: TokenProjection,
    /// Typed nested values still need a bounded schema adapter before consumption.
    pub pending_nested_validation: Vec<&'static str>,
}
impl SemanticPartDraft {
    pub fn field(&self, name: &str) -> Option<&crate::semantic_json_projection::FieldToken> {
        let index = self.field_names.iter().position(|field| *field == name)?;
        self.tokens
            .fields
            .iter()
            .find(|field| field.field_index == index)
    }
}
#[derive(Clone, Copy)]
enum Shape {
    Any,
    String,
    Bool,
    Unsigned,
    U32,
    Number,
    Array,
    Object,
}
struct Rule {
    name: &'static str,
    shape: Shape,
    required: bool,
    nullable: bool,
    control: bool,
    nested: bool,
}
impl Rule {
    fn required(name: &'static str, shape: Shape) -> Self {
        Self {
            name,
            shape,
            required: true,
            nullable: false,
            control: false,
            nested: false,
        }
    }
    fn optional(name: &'static str, shape: Shape) -> Self {
        Self {
            name,
            shape,
            required: false,
            nullable: true,
            control: false,
            nested: false,
        }
    }
    fn defaulted(name: &'static str, shape: Shape) -> Self {
        Self {
            name,
            shape,
            required: false,
            nullable: false,
            control: false,
            nested: false,
        }
    }
    fn control(mut self) -> Self {
        self.control = true;
        self
    }
    fn nested(mut self) -> Self {
        self.nested = true;
        self
    }
}
const ID_FIELDS: &[&str] = &["kind", "id", "part_id", "message_id", "turn_id"];
fn unsupported(code: UnsupportedCode, field: Option<&'static str>) -> Unsupported {
    Unsupported { code, field }
}
fn token<'a>(
    tokens: &'a TokenProjection,
    names: &[&str],
    name: &str,
) -> Option<&'a crate::semantic_json_projection::FieldToken> {
    let index = names.iter().position(|field| *field == name)?;
    tokens
        .fields
        .iter()
        .find(|field| field.field_index == index)
}
fn string_control<'a>(
    tokens: &'a TokenProjection,
    names: &[&str],
    name: &'static str,
) -> Result<&'a str, Unsupported> {
    let field = token(tokens, names, name)
        .ok_or_else(|| unsupported(UnsupportedCode::MissingField, Some(name)))?;
    if field.duplicate_known_key {
        return Err(unsupported(UnsupportedCode::DuplicateField, Some(name)));
    }
    match &field.value {
        ExtractedValue::Complete(ScalarValue::String(value))
            if !value.trim().is_empty() && value.len() <= 256 =>
        {
            Ok(value)
        }
        ExtractedValue::Complete(ScalarValue::String(_)) => {
            Err(unsupported(UnsupportedCode::InvalidControl, Some(name)))
        }
        ExtractedValue::Deferred if field.span.kind == TokenKind::String => {
            Err(unsupported(UnsupportedCode::DeferredControl, Some(name)))
        }
        _ => Err(unsupported(UnsupportedCode::WrongType, Some(name))),
    }
}

/// Two bounded extraction stages: identity first, then the selected schema's
/// fixed fields. Each stage verifies syntax and immutable source identity; their
/// digests must also match. No entire AgentPart or MessagePartView is deserialized.
/// `expected_part_kind` is trusted DB metadata, not a replacement JSON kind.
pub fn extract_draft<F, I>(
    mut open: F,
    total_bytes: u64,
    expected_part_kind: &str,
    maximum_output_bytes: usize,
) -> StoreResult<SemanticPartDraft>
where
    F: FnMut() -> I,
    I: Iterator<Item = Result<(u64, String), String>>,
{
    let header = extract_top_level(&mut open, total_bytes, ID_FIELDS, maximum_output_bytes)?;
    let mut draft = SemanticPartDraft {
        schema: None,
        kind: None,
        status: DraftStatus::Recognized,
        field_names: ID_FIELDS.to_vec(),
        tokens: header,
        pending_nested_validation: Vec::new(),
    };
    let selection = (|| {
        if draft.tokens.root_kind != TokenKind::Object {
            return Err(unsupported(UnsupportedCode::NonObject, None));
        }
        for name in ID_FIELDS {
            if token(&draft.tokens, ID_FIELDS, name).is_some_and(|field| field.duplicate_known_key)
            {
                return Err(unsupported(UnsupportedCode::DuplicateField, Some(name)));
            }
        }
        let kind = string_control(&draft.tokens, ID_FIELDS, "kind")?.to_owned();
        draft.kind = Some(kind.clone());
        let agent = token(&draft.tokens, ID_FIELDS, "id").is_some();
        let view = token(&draft.tokens, ID_FIELDS, "part_id").is_some();
        if agent && view {
            return Err(unsupported(UnsupportedCode::AmbiguousIdentity, None));
        }
        let schema = if agent {
            HistoricalSchema::AgentPart
        } else if view {
            HistoricalSchema::MessagePartView
        } else {
            return Err(unsupported(
                UnsupportedCode::MissingField,
                Some("id/part_id"),
            ));
        };
        draft.schema = Some(schema);
        if schema == HistoricalSchema::MessagePartView
            && (token(&draft.tokens, ID_FIELDS, "message_id").is_some()
                || token(&draft.tokens, ID_FIELDS, "turn_id").is_some())
        {
            return Err(unsupported(UnsupportedCode::AmbiguousIdentity, None));
        }
        let rules = rules(schema, &kind)
            .ok_or_else(|| unsupported(UnsupportedCode::UnknownKind, Some("kind")))?;
        if kind != expected_part_kind
            && !(schema == HistoricalSchema::AgentPart
                && kind == "artifact"
                && expected_part_kind == "attachment")
        {
            return Err(unsupported(UnsupportedCode::KindMismatch, Some("kind")));
        }
        for name in if schema == HistoricalSchema::AgentPart {
            &["id", "message_id", "turn_id"][..]
        } else {
            &["part_id"][..]
        } {
            string_control(&draft.tokens, ID_FIELDS, name)?;
        }
        Ok(rules)
    })();
    let rules = match selection {
        Ok(rules) => rules,
        Err(error) => {
            draft.status = DraftStatus::Unsupported(error);
            return Ok(draft);
        }
    };
    let names: Vec<_> = rules.iter().map(|rule| rule.name).collect();
    // Body strings and arbitrary input/output scalars never spend the control
    // budget. Key order cannot make a valid identity unavailable after a title.
    let scalar_fields: Vec<_> = rules
        .iter()
        .map(|rule| {
            rule.control || matches!(rule.shape, Shape::Unsigned | Shape::U32 | Shape::Number)
        })
        .collect();
    let tokens = extract_top_level_with_scalar_fields(
        &mut open,
        total_bytes,
        &names,
        &scalar_fields,
        maximum_output_bytes,
    )?;
    if tokens.source_sha256 != draft.tokens.source_sha256 {
        return Err(StoreError::Conflict(
            "semantic source changed between schema recognition stages".into(),
        ));
    }
    draft.field_names = names;
    draft.tokens = tokens;
    for rule in &rules {
        if let Err(error) = validate_rule(&draft, rule) {
            draft.status = DraftStatus::Unsupported(error);
            return Ok(draft);
        }
        if rule.nested
            && draft
                .field(rule.name)
                .is_some_and(|field| field.span.kind != TokenKind::Null)
        {
            draft.pending_nested_validation.push(rule.name);
        }
    }
    if let Err(error) = validate_enums(&draft) {
        draft.status = DraftStatus::Unsupported(error);
    } else if draft.schema == Some(HistoricalSchema::MessagePartView)
        && draft.tokens.has_unknown_fields
    {
        // A bounded public value cannot become strict merely by discarding
        // fields that its actual Domain wire contract rejects.
        draft.status = DraftStatus::Unsupported(unsupported(UnsupportedCode::UnknownField, None));
    }
    Ok(draft)
}
fn validate_rule(draft: &SemanticPartDraft, rule: &Rule) -> Result<(), Unsupported> {
    let Some(field) = draft.field(rule.name) else {
        return if rule.required {
            Err(unsupported(UnsupportedCode::MissingField, Some(rule.name)))
        } else {
            Ok(())
        };
    };
    let error = |code| unsupported(code, Some(rule.name));
    if field.duplicate_known_key {
        return Err(error(UnsupportedCode::DuplicateField));
    }
    if rule.nullable && field.span.kind == TokenKind::Null {
        return Ok(());
    }
    let valid = match rule.shape {
        Shape::Any => true,
        Shape::String => field.span.kind == TokenKind::String,
        Shape::Bool => field.span.kind == TokenKind::Bool,
        Shape::Unsigned | Shape::U32 | Shape::Number => field.span.kind == TokenKind::Number,
        Shape::Array => field.span.kind == TokenKind::Array,
        Shape::Object => field.span.kind == TokenKind::Object,
    };
    if !valid {
        return Err(error(UnsupportedCode::WrongType));
    }
    if matches!(rule.shape, Shape::Unsigned | Shape::U32 | Shape::Number) {
        let ExtractedValue::Complete(ScalarValue::Number(value)) = &field.value else {
            return Err(error(UnsupportedCode::DeferredControl));
        };
        if matches!(rule.shape, Shape::Unsigned) && value.as_u64().is_none()
            || matches!(rule.shape, Shape::U32)
                && value.as_u64().is_none_or(|v| v > u32::MAX as u64)
        {
            return Err(error(UnsupportedCode::InvalidControl));
        }
    }
    if rule.control {
        match rule.shape {
            Shape::String => {
                string_control(&draft.tokens, &draft.field_names, rule.name)?;
            }
            _ => {
                if matches!(field.value, ExtractedValue::Deferred) {
                    return Err(error(UnsupportedCode::DeferredControl));
                }
            }
        }
    }
    Ok(())
}
fn validate_enums(draft: &SemanticPartDraft) -> Result<(), Unsupported> {
    fn accepts<T: serde::de::DeserializeOwned>(value: &str) -> bool {
        T::deserialize(serde::de::value::StrDeserializer::<serde::de::value::Error>::new(value)).is_ok()
    }
    if draft.field("state").is_some() {
        let value = string_control(&draft.tokens, &draft.field_names, "state")?;
        let valid = match (draft.schema, draft.kind.as_deref()) {
            (Some(HistoricalSchema::MessagePartView), Some("reasoning" | "tool" | "task" | "skill")) =>
                accepts::<nola_domain::ActivityState>(value),
            (Some(HistoricalSchema::AgentPart), Some("tool")) => accepts::<nola_domain::ToolPartState>(value),
            _ => false,
        };
        if !valid { return Err(unsupported(UnsupportedCode::InvalidControl, Some("state"))); }
    }
    if draft.field("status").is_some() {
        let value = string_control(&draft.tokens, &draft.field_names, "status")?;
        if draft.schema != Some(HistoricalSchema::AgentPart) || draft.kind.as_deref() != Some("step")
            || !accepts::<nola_domain::StepStatus>(value) {
            return Err(unsupported(UnsupportedCode::InvalidControl, Some("status")));
        }
    }
    if draft
        .field("review")
        .is_some_and(|field| field.span.kind != TokenKind::Null)
    {
        let value = string_control(&draft.tokens, &draft.field_names, "review")?;
        if !["pending", "approved", "rejected", "interrupted"].contains(&value) {
            return Err(unsupported(UnsupportedCode::InvalidControl, Some("review")));
        }
    }
    Ok(())
}
fn rules(schema: HistoricalSchema, kind: &str) -> Option<Vec<Rule>> {
    use Rule as R;
    use Shape::*;
    let mut rules = vec![R::required("kind", String).control()];
    match schema {
        HistoricalSchema::AgentPart => rules.extend([
            R::required("id", String).control(),
            R::required("message_id", String).control(),
            R::required("turn_id", String).control(),
        ]),
        HistoricalSchema::MessagePartView => rules.extend([
            R::required("part_id", String).control(),
            R::required("revision", Unsigned).control(),
        ]),
    }
    let fields = match (schema, kind) {
        (HistoricalSchema::AgentPart, "text") => vec![
            R::required("text", String),
            R::required("is_complete", Bool).control(),
            R::optional("review", String).control(),
        ],
        (HistoricalSchema::AgentPart, "reasoning") => vec![
            R::required("title", String),
            R::required("detail", String),
            R::required("is_complete", Bool).control(),
        ],
        (HistoricalSchema::AgentPart, "tool") => vec![
            R::required("execution_source", Object),
            R::required("call_id", String).control(),
            R::required("capability_id", String).control(),
            R::required("title", String),
            R::optional("summary", String),
            R::defaulted("origin", Object).nested(),
            R::required("state", String).control(),
            R::required("input", Any),
            R::defaulted("raw_input", String),
            R::optional("permission", Object).nested(),
            R::optional("output_preview", String),
            R::optional("structured_output", Any),
            R::optional("full_output_ref", String),
            R::required("artifacts", Array).nested(),
            R::optional("started_at_ms", Unsigned),
            R::optional("finished_at_ms", Unsigned),
        ],
        (HistoricalSchema::AgentPart, "source") => vec![
            R::required("source_id", String).control(),
            R::required("title", String),
            R::required("uri", String),
            R::optional("mime_type", String),
            R::optional("evidence", Object).nested(),
        ],
        (HistoricalSchema::AgentPart, "citation") => vec![
            R::required("citation_id", String).control(),
            R::required("source_id", String).control(),
            R::required("title", String),
            R::required("uri", String),
            R::optional("mime_type", String),
            R::optional("location", Object).nested(),
            R::optional("range", Object).nested(),
            R::required("origin_tool_call_id", String).control(),
        ],
        (HistoricalSchema::AgentPart, "artifact") => vec![R::required("artifact", Object).nested()],
        (HistoricalSchema::AgentPart, "step") => vec![
            R::required("title", String),
            R::required("status", String).control(),
        ],
        (_, "compaction") => vec![
            R::required("summary", String),
            R::required("retained_item_ids", Array).nested(),
            R::required("source_fingerprint", String).control(),
            R::required("original_token_count", Unsigned),
            R::required("compacted_token_count", Unsigned),
        ],
        (HistoricalSchema::MessagePartView, "ui_action") => vec![
            R::required("surface", Object),
            // Opaque model context, like tool input; not an executable nested
            // tool contract. The public UiAction decoder validates its identity
            // and A2UI shape after bounded complete-field reconstruction.
            R::required("envelope", Object),
            R::required("summary", String),
        ],
        (HistoricalSchema::MessagePartView, "ui_surface") => vec![
            R::required("surface", Object),
            R::required("summary", String),
        ],
        (HistoricalSchema::MessagePartView, "text") => vec![
            R::required("markdown", String),
            R::required("is_final", Bool).control(),
            R::optional("review", String).control(),
        ],
        (HistoricalSchema::MessagePartView, "reasoning") => vec![
            R::required("state", String).control(),
            R::required("title", String),
            R::optional("summary", String),
        ],
        (HistoricalSchema::MessagePartView, "tool") => vec![
            R::required("execution_source", Object),
            R::required("tool_call_id", String).control(),
            R::required("capability_id", String).control(),
            R::required("state", String).control(),
            R::optional("display_key", String),
            R::required("title", String),
            R::optional("summary", String),
            R::required("origin", Object).nested(),
            R::optional("permission", Object).nested(),
            R::required("input", Any),
            R::optional("output", Any),
            R::optional("output_preview", String),
            R::optional("full_output", Object).nested(),
            R::required("artifacts", Array).nested(),
            R::optional("started_at_ms", Unsigned),
            R::optional("finished_at_ms", Unsigned),
            R::required("affected_resources", Array).nested(),
        ],
        (HistoricalSchema::MessagePartView, "source") => vec![
            R::required("source_id", String).control(),
            R::required("title", String),
            R::required("uri", String),
            R::optional("mime_type", String),
            R::optional("snippet", String),
            R::optional("evidence", Object).nested(),
        ],
        (HistoricalSchema::MessagePartView, "citation") => vec![
            R::required("citation_id", String).control(),
            R::required("source_id", String).control(),
            R::required("title", String),
            R::required("uri", String),
            R::required("origin_tool_call_id", String).control(),
            R::optional("mime_type", String),
            R::optional("location", Object).nested(),
            R::optional("section", String),
            R::optional("page", Unsigned),
            R::optional("line_start", Unsigned),
            R::optional("line_end", Unsigned),
            R::optional("range_start", Unsigned),
            R::optional("range_end", Unsigned),
        ],
        (HistoricalSchema::MessagePartView, "attachment") => vec![
            R::required("attachment_id", String).control(),
            R::required("name", String),
            R::optional("mime_type", String),
            R::required("uri", String),
            R::optional("operation", String),
        ],
        (HistoricalSchema::MessagePartView, "task") => vec![
            R::required("task_id", String).control(),
            R::required("display_key", String),
            R::required("state", String).control(),
            R::optional("progress", Number),
            R::optional("current_step", String),
            R::optional("result_ref", String),
            R::required("cancel_requested", Bool).control(),
        ],
        (HistoricalSchema::MessagePartView, "skill") => vec![
            R::required("run_id", String).control(),
            R::required("skill_id", String).control(),
            R::required("display_name", String),
            R::required("mode", String).control(),
            R::required("state", String).control(),
            R::required("model_turns", U32),
            R::required("tool_calls", U32),
            R::required("elapsed_millis", Unsigned),
            R::optional("result_ref", String),
            R::optional("failure", String),
        ],
        (HistoricalSchema::MessagePartView, "error") => vec![
            R::required("code", String).control(),
            R::required("display_key", String),
            R::required("recoverable", Bool).control(),
        ],
        _ => return None,
    };
    rules.extend(fields);
    Some(rules)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(input: &str, kind: &str) -> SemanticPartDraft {
        extract_draft(
            || {
                // ASCII fixtures can split every escape and scalar at any byte.
                input
                    .as_bytes()
                    .chunks(7)
                    .enumerate()
                    .map(|(i, b)| Ok((i as u64 * 7, String::from_utf8(b.to_vec()).unwrap())))
            },
            input.len() as u64,
            kind,
            8192,
        )
        .unwrap()
    }
    fn fixtures(schema: HistoricalSchema) -> Vec<(&'static str, serde_json::Value)> {
        use serde_json::json;
        let fields = match schema {
            HistoricalSchema::AgentPart => vec![
                ("text", json!({"text":"hello","is_complete":true})),
                (
                    "reasoning",
                    json!({"title":"thinking","detail":"detail","is_complete":true}),
                ),
                (
                    "tool",
                    json!({"call_id":"call","capability_id":"read","title":"Read","state":"completed","input":{"path":"a"},"artifacts":[]}),
                ),
                (
                    "source",
                    json!({"source_id":"s","title":"Source","uri":"file://a"}),
                ),
                (
                    "citation",
                    json!({"citation_id":"cite","source_id":"s","title":"Source","uri":"file://a","origin_tool_call_id":"call"}),
                ),
                (
                    "artifact",
                    json!({"artifact":{"uri":"file://a","name":"a"}}),
                ),
                ("step", json!({"title":"Step","status":"started"})),
                (
                    "compaction",
                    json!({"summary":"s","retained_item_ids":["p"],"source_fingerprint":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","original_token_count":100,"compacted_token_count":10}),
                ),
            ],
            HistoricalSchema::MessagePartView => vec![
                ("text", json!({"markdown":"hello","is_final":true})),
                (
                    "reasoning",
                    json!({"state":"completed","title":"Thinking","summary":"detail"}),
                ),
                (
                    "tool",
                    json!({"tool_call_id":"call","capability_id":"read","title":"Read","origin":{"kind":"builtin"},"state":"completed","input":{"path":"a"},"output":null,"artifacts":[],"affected_resources":[]}),
                ),
                (
                    "source",
                    json!({"source_id":"s","title":"Source","uri":"file://a"}),
                ),
                (
                    "citation",
                    json!({"citation_id":"cite","source_id":"s","title":"Source","uri":"file://a","origin_tool_call_id":"call"}),
                ),
                (
                    "attachment",
                    json!({"attachment_id":"a","name":"a","uri":"file://a"}),
                ),
                (
                    "task",
                    json!({"task_id":"t","display_key":"work","state":"completed","cancel_requested":false}),
                ),
                (
                    "skill",
                    json!({"run_id":"r","skill_id":"s","display_name":"Skill","mode":"run","state":"completed","model_turns":1,"tool_calls":1,"elapsed_millis":10}),
                ),
                (
                    "compaction",
                    json!({"summary":"s","retained_item_ids":["p"],"source_fingerprint":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","original_token_count":100,"compacted_token_count":10}),
                ),
                (
                    "error",
                    json!({"code":"error","display_key":"oops","recoverable":true}),
                ),
            ],
        };
        fields
            .into_iter()
            .map(|(kind, mut value)| {
                value["kind"] = kind.into();
                if kind == "tool" {
                    value["execution_source"] = serde_json::json!({"kind":"model"});
                }
                match schema {
                    HistoricalSchema::AgentPart => {
                        value["id"] = "p".into();
                        value["message_id"] = "m".into();
                        value["turn_id"] = "r".into();
                    }
                    HistoricalSchema::MessagePartView => {
                        value["part_id"] = "p".into();
                        value["revision"] = 1.into();
                    }
                }
                (kind, value)
            })
            .collect()
    }
    #[test]
    fn recognizes_all_actual_wire_kinds_without_collapsing_identity_or_nested_validation() {
        for schema in [
            HistoricalSchema::AgentPart,
            HistoricalSchema::MessagePartView,
        ] {
            for (kind, value) in fixtures(schema) {
                if schema == HistoricalSchema::MessagePartView {
                    // Production Domain decoder is the oracle for these shapes.
                    serde_json::from_value::<nola_domain::projection::MessagePartView>(
                        value.clone(),
                    )
                    .unwrap();
                }
                let draft = parse(&value.to_string(), kind);
                assert_eq!(
                    draft.status,
                    DraftStatus::Recognized,
                    "schema={schema:?} kind={kind}"
                );
                assert_eq!(draft.schema, Some(schema));
                assert_eq!(draft.kind.as_deref(), Some(kind));
                if kind == "tool" {
                    assert!(draft.pending_nested_validation.contains(&if schema
                        == HistoricalSchema::AgentPart
                    {
                        "artifacts"
                    } else {
                        "affected_resources"
                    }));
                }
                if kind == "artifact" {
                    assert_eq!(draft.pending_nested_validation, vec!["artifact"]);
                }
                if kind == "compaction" {
                    assert_eq!(draft.pending_nested_validation, vec!["retained_item_ids"]);
                }
            }
        }
    }
    #[test]
    fn public_schema_rejects_unknown_source_fields_before_bounded_materialization() {
        for (kind, mut value) in fixtures(HistoricalSchema::MessagePartView) {
            value["private_grant"] = serde_json::json!({"token":"must not disappear"});
            assert!(serde_json::from_value::<nola_domain::MessagePartView>(value.clone()).is_err());
            let draft = parse(&value.to_string(), kind);
            assert!(
                matches!(
                    draft.status,
                    DraftStatus::Unsupported(Unsupported {
                        code: UnsupportedCode::UnknownField,
                        ..
                    })
                ),
                "{kind}: {:?}",
                draft.status
            );
            assert!(draft.tokens.has_unknown_fields);
            assert!(!draft.tokens.source_sha256.is_empty());
        }
    }
    #[test]
    fn unsupported_is_explicit_preserves_evidence_and_never_falls_back_between_schemas() {
        let base = r#"{"kind":"text","id":"p","message_id":"m","turn_id":"r","text":"hello","is_complete":true}"#;
        let mut value: serde_json::Value = serde_json::from_str(base).unwrap();
        value["part_id"] = "p".into();
        let draft = parse(&value.to_string(), "text");
        assert!(matches!(
            draft.status,
            DraftStatus::Unsupported(Unsupported {
                code: UnsupportedCode::AmbiguousIdentity,
                ..
            })
        ));
        assert!(draft.field("id").is_some());
        for (input, expected) in [
            (
                r#"{"kind":"text","part_id":"p","revision":1,"markdown":"a","is_final":true,"message_id":"m"}"#,
                UnsupportedCode::AmbiguousIdentity,
            ),
            (
                r#"{"kind":"text","part_id":"p","revision":1,"markdown":"a","is_final":true,"\u0069s_final":false}"#,
                UnsupportedCode::DuplicateField,
            ),
            (
                r#"{"kind":"text","part_id":"p","revision":-1,"markdown":"a","is_final":true}"#,
                UnsupportedCode::InvalidControl,
            ),
            (
                r#"{"kind":"text","part_id":"p","revision":1,"markdown":"a","is_final":"true"}"#,
                UnsupportedCode::WrongType,
            ),
            (
                r#"{"kind":"text","id":"p","message_id":"m","text":"a","is_complete":true}"#,
                UnsupportedCode::MissingField,
            ),
            (
                r#"{"kind":"future","id":"p","message_id":"m","turn_id":"r"}"#,
                UnsupportedCode::UnknownKind,
            ),
            (r#"[]"#, UnsupportedCode::NonObject),
        ] {
            let draft = parse(input, "text");
            assert!(
                matches!(draft.status,DraftStatus::Unsupported(Unsupported{code,..}) if code==expected),
                "{input}: {:?}",
                draft.status
            );
            assert!(!draft.tokens.source_sha256.is_empty());
        }
        assert!(matches!(
            parse(base, "tool").status,
            DraftStatus::Unsupported(Unsupported {
                code: UnsupportedCode::KindMismatch,
                ..
            })
        ));
        value = serde_json::from_str(base).unwrap();
        value["id"] = "x".repeat(10000).into();
        assert!(matches!(
            parse(&value.to_string(), "text").status,
            DraftStatus::Unsupported(Unsupported {
                code: UnsupportedCode::DeferredControl,
                ..
            })
        ));
        let invalid = r#"{"kind":"text","text":"\uD800"}"#;
        assert!(extract_draft(
            || vec![Ok((0, invalid.into()))].into_iter(),
            invalid.len() as u64,
            "text",
            8192
        )
        .is_err());
    }
    #[test]
    fn current_activity_and_tool_states_use_their_schema_contracts() {
        for schema in [HistoricalSchema::AgentPart, HistoricalSchema::MessagePartView] {
            for (kind, mut raw) in fixtures(schema) {
                if !matches!(kind, "tool" | "reasoning" | "task" | "skill")
                    || schema == HistoricalSchema::AgentPart && kind != "tool" { continue; }
                for state in ["sealed", "waiting_for_input"] {
                    raw["state"] = state.into();
                    assert_eq!(parse(&raw.to_string(),kind).status,DraftStatus::Recognized,"{schema:?}/{kind}/{state}");
                }
                raw["state"]="invented_future_state".into();
                assert!(matches!(parse(&raw.to_string(),kind).status,DraftStatus::Unsupported(_)));
            }
        }
        let (_,mut step)=fixtures(HistoricalSchema::AgentPart).into_iter().find(|(kind,_)|*kind=="step").unwrap();
        for status in ["started","completed","failed","cancelled"] {
            step["status"]=status.into();assert_eq!(parse(&step.to_string(),"step").status,DraftStatus::Recognized);
        }
        for status in ["sealed","waiting_for_input","invented_future_state"] {
            step["status"]=status.into();assert!(matches!(parse(&step.to_string(),"step").status,DraftStatus::Unsupported(_)));
        }
    }
    #[test]
    fn controls_types_defaults_and_states_match_the_supported_top_level_contract() {
        let mut tool = fixtures(HistoricalSchema::AgentPart)
            .into_iter()
            .find(|(kind, _)| *kind == "tool")
            .unwrap()
            .1;
        for state in [
            "pending",
            "running",
            "waiting_for_permission",
            "completed",
            "failed",
            "cancelled",
        ] {
            tool["state"] = state.into();
            assert_eq!(
                parse(&tool.to_string(), "tool").status,
                DraftStatus::Recognized
            );
        }
        tool["state"] = "done".into();
        assert!(matches!(
            parse(&tool.to_string(), "tool").status,
            DraftStatus::Unsupported(Unsupported {
                code: UnsupportedCode::InvalidControl,
                ..
            })
        ));
        tool["state"] = "completed".into();
        tool["permission"] = serde_json::json!({"request_id":"request","permission_domain":"fs","rationale":"reason"});
        tool["origin"] = serde_json::json!({});
        let draft = parse(&tool.to_string(), "tool");
        assert_eq!(draft.status, DraftStatus::Recognized);
        assert!(draft.pending_nested_validation.contains(&"permission"));
        assert!(draft.pending_nested_validation.contains(&"origin"));
        tool["raw_input"] = serde_json::Value::Null;
        assert!(matches!(
            parse(&tool.to_string(), "tool").status,
            DraftStatus::Unsupported(Unsupported {
                code: UnsupportedCode::WrongType,
                ..
            })
        ));
        let mut skill = fixtures(HistoricalSchema::MessagePartView)
            .into_iter()
            .find(|(kind, _)| *kind == "skill")
            .unwrap()
            .1;
        skill["tool_calls"] = serde_json::json!(u32::MAX as u64 + 1);
        assert!(matches!(
            parse(&skill.to_string(), "skill").status,
            DraftStatus::Unsupported(Unsupported {
                code: UnsupportedCode::InvalidControl,
                ..
            })
        ));
        let text = r#"{"kind":"text","part_id":"p","revision":1,"markdown":"x","is_final":true}"#;
        let mut opens = 0;
        let error = extract_draft(
            || {
                opens += 1;
                vec![Ok((
                    0,
                    if opens <= 2 {
                        text.to_owned()
                    } else {
                        text.replace("\"p\"", "\"q\"")
                    },
                ))]
                .into_iter()
            },
            text.len() as u64,
            "text",
            8192,
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("between schema recognition stages"));
    }
    #[test]
    fn body_order_never_spends_the_control_scalar_budget() {
        for schema in [
            HistoricalSchema::AgentPart,
            HistoricalSchema::MessagePartView,
        ] {
            for (kind, mut value) in fixtures(schema)
                .into_iter()
                .filter(|(kind, _)| matches!(*kind, "text" | "reasoning" | "tool"))
            {
                let body = match (schema, kind) {
                    (HistoricalSchema::AgentPart, "text") => "text",
                    (HistoricalSchema::MessagePartView, "text") => "markdown",
                    (HistoricalSchema::MessagePartView, "reasoning") => "summary",
                    (HistoricalSchema::MessagePartView, "tool") => "display_key",
                    _ => "title",
                };
                let rule_count = rules(schema, kind).unwrap().len();
                let budget = 4096;
                let metadata = rule_count
                    * std::mem::size_of::<crate::semantic_json_projection::FieldToken>()
                    + std::mem::size_of::<TokenProjection>()
                    + 64;
                // This complete raw body would fit by itself, yet leave too
                // little room for kind/owner/call IDs with encounter-order decoding.
                let body_text = "x".repeat(budget - metadata - 3);
                value.as_object_mut().unwrap().remove(body);
                let controls = value.to_string();
                let controls = &controls[1..controls.len() - 1];
                for input in [
                    format!("{{\"{body}\":\"{body_text}\",{controls}}}"),
                    format!("{{{controls},\"{body}\":\"{body_text}\"}}"),
                ] {
                    let draft = extract_draft(
                        || vec![Ok((0, input.clone()))].into_iter(),
                        input.len() as u64,
                        kind,
                        budget,
                    )
                    .unwrap();
                    assert_eq!(
                        draft.status,
                        DraftStatus::Recognized,
                        "schema={schema:?} kind={kind}"
                    );
                    assert_eq!(draft.field(body).unwrap().value, ExtractedValue::Deferred);
                    let id = if schema == HistoricalSchema::AgentPart {
                        "id"
                    } else {
                        "part_id"
                    };
                    assert_eq!(
                        draft.field(id).unwrap().value,
                        ExtractedValue::Complete(ScalarValue::String("p".into()))
                    );
                    if kind == "tool" {
                        let call = if schema == HistoricalSchema::AgentPart {
                            "call_id"
                        } else {
                            "tool_call_id"
                        };
                        assert_eq!(
                            draft.field(call).unwrap().value,
                            ExtractedValue::Complete(ScalarValue::String("call".into()))
                        );
                    }
                }
            }
        }
    }
}
