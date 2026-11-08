// SPDX-License-Identifier: Apache-2.0
//! One owner-bound historical interpretation for presentation and model context.
use crate::semantic_json_projection::{TokenKind, TokenSpan};
use crate::semantic_nested_draft::{NestedKind, NestedStatus};
use crate::semantic_part_draft::{DraftStatus, HistoricalSchema};
use crate::{params, NolaStore, StoreError, StoreResult};
use nola_domain::{
    CanonicalPartOwner, HistoricalPartProjection, HistoricalPartRead, HistoricalPartSchema,
    HistoricalProjectionStatus, HistoricalTextPreview, PartBodyField, SemanticIdentityAssessment,
    SemanticTokenKind, SemanticTokenLocator, SemanticValue, StructuredValue,
};
use std::collections::BTreeMap;
fn unsupported(code: impl Into<String>) -> HistoricalProjectionStatus {
    HistoricalProjectionStatus::Unsupported { code: code.into() }
}
fn encoded_size(value: &impl serde::Serialize) -> StoreResult<usize> {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    serde_json::to_writer(&mut count, value)?;
    Ok(count.0)
}
fn finish(value: HistoricalPartProjection, budget: usize) -> StoreResult<HistoricalPartRead> {
    if encoded_size(&value)? > budget {
        return Ok(HistoricalPartRead::BudgetDeferred {
            canonical_kind: value.canonical_kind,
        });
    }
    Ok(HistoricalPartRead::Available(Box::new(value)))
}
fn token_kind(kind: TokenKind) -> SemanticTokenKind {
    match kind {
        TokenKind::Null => SemanticTokenKind::Null,
        TokenKind::Bool => SemanticTokenKind::Boolean,
        TokenKind::Number => SemanticTokenKind::Number,
        TokenKind::String => SemanticTokenKind::String,
        TokenKind::Array => SemanticTokenKind::Array,
        TokenKind::Object => SemanticTokenKind::Object,
    }
}
impl NolaStore {
    pub fn read_historical_part(
        &self,
        owner: &CanonicalPartOwner,
        expected_revision: i64,
        maximum_output_bytes: usize,
    ) -> StoreResult<Option<HistoricalPartProjection>> {
        match self.read_historical_part_with_budget(
            owner,
            expected_revision,
            maximum_output_bytes,
        )? {
            HistoricalPartRead::Absent => Ok(None),
            HistoricalPartRead::Available(value) => Ok(Some(*value)),
            HistoricalPartRead::BudgetDeferred { .. } => Err(StoreError::Invariant(
                "historical projection exceeds encoded output budget".into(),
            )),
        }
    }
    fn finish_historical(
        &self,
        mut value: HistoricalPartProjection,
        budget: usize,
    ) -> StoreResult<HistoricalPartRead> {
        // Historical interpretation must preserve the same disclosure policy
        // as ordinary text/range views. Neither decoded JSON text nor a fallback
        // preview may revive a withheld candidate in the chat timeline.
        if value.canonical_kind == "text"
            && !crate::canonical_text_disclosure::allows_product_text(
                &self.connection,
                value.owner.conversation_id(),
                value.owner.message_id(),
                value.owner.part_id(),
            )?
        {
            value.fields.remove("text");
            let already_public_empty = value.schema == Some(HistoricalPartSchema::MessagePartView)
                && matches!(value.fields.get("markdown"), Some(SemanticValue::Complete {
                    value: StructuredValue::String(text)
                }) if text.is_empty());
            if !already_public_empty {
                value.fields.remove("markdown");
            }
            value.canonical_text = None;
            return finish(value, budget);
        }
        let base = encoded_size(&value)?;
        if base > budget {
            return finish(value, budget);
        }
        let owner = &value.owner;
        let total = crate::published_part_body::resolve_field(
            &self.connection,
            owner.conversation_id(),
            owner.message_id(),
            owner.part_id(),
            PartBodyField::ContentText,
        )?
        .total_utf8_bytes();
        // Reserve the preview envelope and worst-case JSON escaping (six bytes
        // per raw byte). Never displace semantic fields to add optional prose.
        let limit = budget.saturating_sub(base).saturating_sub(128) / 6;
        if let Some(total) = total.filter(|total| *total > 0) {
            if limit >= 4 {
                let (_, range) = crate::published_part_body::field_range(
                    &self.connection,
                    owner.conversation_id(),
                    owner.message_id(),
                    owner.part_id(),
                    PartBodyField::ContentText,
                    0,
                    limit.min(1024),
                )?;
                value.canonical_text = Some(HistoricalTextPreview {
                    text: range.text,
                    total_utf8_bytes: total,
                });
                if encoded_size(&value)? > budget {
                    value.canonical_text = None;
                }
            }
        }
        finish(value, budget)
    }
    fn historical_token(&self, owner: &CanonicalPartOwner, span: TokenSpan) -> StoreResult<String> {
        let length = span.end_bytes - span.start_bytes;
        if length > 32768 {
            return Err(StoreError::Invariant(
                "historical token exceeds bounded read".into(),
            ));
        }
        let mut text = String::with_capacity(length as usize);
        let mut offset = span.start_bytes;
        while offset < span.end_bytes {
            let (_, range) = crate::published_part_body::field_range(
                &self.connection,
                owner.conversation_id(),
                owner.message_id(),
                owner.part_id(),
                PartBodyField::ContentJson,
                offset,
                (span.end_bytes - offset).clamp(4, 32768) as usize,
            )?;
            let count = range.text.len().min((span.end_bytes - offset) as usize);
            if count == 0 || !range.text.is_char_boundary(count) {
                return Err(StoreError::Invariant(
                    "historical token did not make UTF-8 progress".into(),
                ));
            }
            text.push_str(&range.text[..count]);
            offset += count as u64;
        }
        Ok(text)
    }
    pub fn read_historical_part_with_budget(
        &self,
        owner: &CanonicalPartOwner,
        expected_revision: i64,
        maximum_output_bytes: usize,
    ) -> StoreResult<HistoricalPartRead> {
        if maximum_output_bytes > 32768 {
            return Err(StoreError::Invariant(
                "invalid historical projection budget".into(),
            ));
        }
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        // Preparation has its own fixed 32 KiB ceiling; output is independently
        // charged against the caller's exact serialized response budget.
        let Some(owned) = self.inspect_part_semantic_draft(owner, expected_revision, 32768)? else {
            return Ok(HistoricalPartRead::Absent);
        };
        let canonical_kind:String=self.connection.query_row("SELECT CASE WHEN length(CAST(kind AS BLOB))<=128 THEN kind END FROM message_parts WHERE id=?1",params![owner.part_id()],|r|r.get(0))?;
        let mut projection = HistoricalPartProjection {
            owner: owner.clone(),
            canonical_revision: expected_revision,
            source: Some(owned.source.clone()),
            schema: owned.draft.schema.map(|schema| match schema {
                HistoricalSchema::AgentPart => HistoricalPartSchema::AgentPart,
                HistoricalSchema::MessagePartView => HistoricalPartSchema::MessagePartView,
            }),
            canonical_kind,
            binding: owned.identity,
            canonical_text: None,
            status: HistoricalProjectionStatus::Supported,
            fields: BTreeMap::new(),
        };
        if let DraftStatus::Unsupported(reason) = &owned.draft.status {
            projection.status = unsupported(format!("schema_{:?}", reason.code).to_lowercase());
            return self.finish_historical(projection, maximum_output_bytes);
        }
        if !matches!(
            owned.identity,
            Some(
                SemanticIdentityAssessment::Canonical
                    | SemanticIdentityAssessment::VerifiedLocalDerivation
            )
        ) {
            projection.status = unsupported("semantic_identity_unverified");
            return self.finish_historical(projection, maximum_output_bytes);
        }
        for field in &owned.draft.pending_nested_validation {
            let kind = match *field {
                "origin" => Some(NestedKind::Origin),
                "permission" => Some(NestedKind::Permission),
                "artifact" => Some(NestedKind::Artifact),
                "artifacts" => Some(NestedKind::Artifacts),
                "affected_resources" => Some(NestedKind::AffectedResources),
                "evidence" => Some(NestedKind::Evidence),
                "location" => Some(NestedKind::Location),
                "range" => Some(NestedKind::Range),
                "retained_item_ids" => Some(NestedKind::RetainedItemIds),
                "full_output" => Some(NestedKind::ToolOutputReference),
                _ => None,
            };
            if let Some(kind) = kind {
                let nested = self
                    .inspect_part_nested_draft(owner, expected_revision, kind, 32768)?
                    .ok_or_else(|| {
                        StoreError::Invariant("declared nested field disappeared".into())
                    })?;
                match nested.nested.status {
                    NestedStatus::Validated => {}
                    NestedStatus::Deferred => {
                        projection.status = HistoricalProjectionStatus::Deferred
                    }
                    NestedStatus::Unsupported(_) => {
                        projection.status = unsupported(format!("invalid_nested_{field}"));
                        return self.finish_historical(projection, maximum_output_bytes);
                    }
                }
                if kind == NestedKind::ToolOutputReference {
                    let token = owned.draft.field(field).expect("pending field exists");
                    let raw = self.historical_token(owner, token.span)?;
                    let reference: nola_domain::ToolOutputReference = serde_json::from_str(&raw)?;
                    let call_id = owned
                        .draft
                        .field("tool_call_id")
                        .and_then(|field| match &field.value {
                            crate::semantic_json_projection::ExtractedValue::Complete(
                                crate::semantic_json_projection::ScalarValue::String(value),
                            ) => Some(value.as_str()),
                            _ => None,
                        });
                    if reference.message_id.0 != owner.message_id()
                        || reference.part_id.0 != owner.part_id()
                        || Some(reference.tool_call_id.0.as_str()) != call_id
                    {
                        projection.status = unsupported("invalid_tool_output_owner");
                        return self.finish_historical(projection, maximum_output_bytes);
                    }
                }
            } else {
                // Empty collections contain no unvalidated element. This does
                // not validate nonempty artifacts/retained IDs by guessing shape.
                let token = owned.draft.field(field).expect("pending field exists");
                let empty = matches!(*field, "artifacts" | "retained_item_ids")
                    && token.span.end_bytes - token.span.start_bytes <= 32768
                    && {
                        let raw = self.historical_token(owner, token.span)?;
                        raw.bytes()
                            .filter(|b| !b.is_ascii_whitespace())
                            .eq(b"[]".iter().copied())
                    };
                if !empty {
                    projection.status = unsupported(format!("unsupported_nested_{field}"));
                    return self.finish_historical(projection, maximum_output_bytes);
                }
            }
        }
        let Some(mut remaining) = maximum_output_bytes.checked_sub(encoded_size(&projection)?)
        else {
            return Ok(HistoricalPartRead::BudgetDeferred {
                canonical_kind: projection.canonical_kind,
            });
        };
        let mut alternatives = Vec::new();
        // Control scalars already identified by the draft always precede body
        // values; within each class use the fixed schema catalog, not wire order.
        let mut ordered: Vec<_> = owned.draft.tokens.fields.iter().collect();
        ordered.sort_by_key(|token| {
            (
                matches!(
                    token.value,
                    crate::semantic_json_projection::ExtractedValue::Deferred
                ),
                token.field_index,
            )
        });
        for token in ordered {
            if token.duplicate_known_key {
                return Err(StoreError::Invariant(
                    "recognized historical field is ambiguous".into(),
                ));
            }
            let name = owned.draft.field_names[token.field_index].to_string();
            if name == "raw_input" || name == "full_output_ref" {
                // Private execution material never becomes a public semantic
                // field or a deferred locator. A retained output is represented
                // solely by the already verified canonical owner below.
                continue;
            }
            let locator = SemanticTokenLocator::new(
                owned.source.clone(),
                token.span.start_bytes,
                token.span.end_bytes,
                token_kind(token.span.kind),
            )
            .map_err(|e| StoreError::Invariant(e.to_string()))?;
            let length = token.span.end_bytes - token.span.start_bytes;
            let mut value = SemanticValue::Deferred {
                locator: locator.clone(),
            };
            if length <= remaining as u64 {
                let raw = self.historical_token(owner, token.span)?;
                // Bounded raw bytes plus a bounded node admission prevent tiny
                // array syntax from creating an unbounded in-memory Value tree.
                if bounded_nodes(&raw) {
                    let complete = SemanticValue::Complete {
                        value: serde_json::from_str::<StructuredValue>(&raw)?,
                    };
                    let cost = encoded_size(&complete)? + name.len() + 4;
                    if cost <= remaining {
                        remaining -= cost;
                        value = complete;
                    }
                }
            }
            if matches!(value, SemanticValue::Deferred { .. }) {
                remaining = remaining.saturating_sub(encoded_size(&value)? + name.len() + 4);
                projection.status = HistoricalProjectionStatus::Deferred;
            }
            alternatives.push((name.clone(), locator));
            projection.fields.insert(name, value);
        }
        if owned.draft.schema == Some(HistoricalSchema::AgentPart)
            && owned.draft.kind.as_deref() == Some("tool")
            && owned
                .draft
                .field("full_output_ref")
                .is_some_and(|field| field.span.kind == TokenKind::String)
        {
            let call_id = owned
                .draft
                .field("call_id")
                .and_then(|field| match &field.value {
                    crate::semantic_json_projection::ExtractedValue::Complete(
                        crate::semantic_json_projection::ScalarValue::String(value),
                    ) => Some(value.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    StoreError::Invariant("retained tool output has no call owner".into())
                })?;
            projection.fields.insert(
                "full_output".into(),
                SemanticValue::Complete {
                    value: serde_json::from_value(serde_json::to_value(
                        nola_domain::ToolOutputReference {
                            message_id: nola_domain::MessageId(owner.message_id().into()),
                            part_id: nola_domain::MessagePartId(owner.part_id().into()),
                            tool_call_id: nola_domain::ToolCallId(call_id),
                        },
                    )?)?,
                },
            );
        }
        // JSON escaping and repeated locator metadata are included in the final
        // byte count. Downgrade only when doing so saves space, never truncate.
        if encoded_size(&projection)? > maximum_output_bytes {
            for (name, locator) in alternatives {
                let deferred = SemanticValue::Deferred { locator };
                if projection.fields.get(&name).is_some_and(|old| {
                    encoded_size(old).unwrap_or(usize::MAX)
                        > encoded_size(&deferred).unwrap_or(usize::MAX)
                }) {
                    projection.fields.insert(name, deferred);
                    projection.status = HistoricalProjectionStatus::Deferred;
                    if encoded_size(&projection)? <= maximum_output_bytes {
                        break;
                    }
                }
            }
        }
        self.finish_historical(projection, maximum_output_bytes)
    }
}
impl nola_store_port::historical_part_projection::HistoricalPartProjectionRepository for NolaStore {
    type Error = StoreError;
    fn read_historical_part_with_budget(
        &self,
        owner: &CanonicalPartOwner,
        expected_revision: i64,
        maximum_output_bytes: usize,
    ) -> StoreResult<HistoricalPartRead> {
        NolaStore::read_historical_part_with_budget(
            self,
            owner,
            expected_revision,
            maximum_output_bytes,
        )
    }

    fn read_historical_part(
        &self,
        owner: &CanonicalPartOwner,
        expected_revision: i64,
        maximum_output_bytes: usize,
    ) -> StoreResult<Option<HistoricalPartProjection>> {
        NolaStore::read_historical_part(self, owner, expected_revision, maximum_output_bytes)
    }
}
/// Count JSON values before building StructuredValue. The input itself is already
/// validated and bounded; serde's borrowed string visitor avoids string copies.
fn bounded_nodes(raw: &str) -> bool {
    use serde::de::{DeserializeSeed, Error, IgnoredAny, MapAccess, SeqAccess, Visitor};
    struct Seed<'a>(&'a mut usize);
    struct Count<'a>(&'a mut usize);
    impl<'de> DeserializeSeed<'de> for Seed<'_> {
        type Value = ();
        fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
            if *self.0 >= 256 {
                return Err(D::Error::custom("historical field node budget"));
            }
            *self.0 += 1;
            d.deserialize_any(Count(self.0))
        }
    }
    impl<'de> Visitor<'de> for Count<'_> {
        type Value = ();
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("bounded JSON value")
        }
        fn visit_unit<E: Error>(self) -> Result<(), E> {
            Ok(())
        }
        fn visit_bool<E: Error>(self, _: bool) -> Result<(), E> {
            Ok(())
        }
        fn visit_i64<E: Error>(self, _: i64) -> Result<(), E> {
            Ok(())
        }
        fn visit_u64<E: Error>(self, _: u64) -> Result<(), E> {
            Ok(())
        }
        fn visit_f64<E: Error>(self, _: f64) -> Result<(), E> {
            Ok(())
        }
        fn visit_str<E: Error>(self, _: &str) -> Result<(), E> {
            Ok(())
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<(), A::Error> {
            while a.next_element_seed(Seed(self.0))?.is_some() {}
            Ok(())
        }
        fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<(), A::Error> {
            while a.next_key::<IgnoredAny>()?.is_some() {
                a.next_value_seed(Seed(self.0))?;
            }
            Ok(())
        }
    }
    let mut count = 0;
    Seed(&mut count)
        .deserialize(&mut serde_json::Deserializer::from_str(raw))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(kind: &str, json: Option<&str>) -> (NolaStore, CanonicalPartOwner) {
        let store = NolaStore::open(":memory:").unwrap();
        store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','title',1,1,1);INSERT INTO messages VALUES('m','c',NULL,'assistant','completed',1,1,1);INSERT INTO message_parts VALUES('p','m','text',1,'completed','plain',NULL,NULL,1,1);").unwrap();
        store
            .connection
            .execute(
                "UPDATE message_parts SET kind=?1,content_json=?2 WHERE id='p'",
                params![kind, json],
            )
            .unwrap();
        crate::canonical_text_disclosure::flush(&store.connection).unwrap();
        (
            store,
            CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap(),
        )
    }
    #[test]
    fn actual_store_task_history_preserves_sealed_and_waiting_input_without_claiming_completion() {
        for state in ["sealed","waiting_for_input"] {
            let raw=serde_json::json!({"kind":"task","part_id":"p","revision":1,"task_id":"task",
                "display_key":"Task","state":state,"cancel_requested":false});
            let (store,owner)=fixture("task",Some(&raw.to_string()));
            let value=store.read_historical_part(&owner,1,8192).unwrap().unwrap();
            assert_eq!(value.status,HistoricalProjectionStatus::Supported);
            assert!(matches!(&value.fields["state"],SemanticValue::Complete {value:nola_domain::StructuredValue::String(actual)} if actual==state));
            assert_ne!(value.fields["state"],SemanticValue::Complete {value:nola_domain::StructuredValue::String("completed".into())});
        }
        let raw=serde_json::json!({"kind":"task","part_id":"p","revision":1,"task_id":"task",
            "display_key":"Task","state":"invented_future_state","cancel_requested":false});
        let (store,owner)=fixture("task",Some(&raw.to_string()));
        let value=store.read_historical_part(&owner,1,8192).unwrap().unwrap();
        assert!(matches!(value.status,HistoricalProjectionStatus::Unsupported {..}));
        assert!(value.fields.is_empty());
    }
    #[test]
    fn production_attachment_append_and_opaque_text_keep_canonical_description() {
        use crate::{AppendMessagesInput, CreateConversationInput, MessageInput, MessagePartInput};
        let mut store = NolaStore::open(":memory:").unwrap();
        store
            .create_conversation(&CreateConversationInput {
                id: "c".into(),
                title: "attachments".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        // Exact canonical kind / AgentPart JSON pairing emitted by
        // Application::ensure_applied_user_messages for AgentItem::Attachment.
        let raw = r#"{"kind":"artifact","id":"p","message_id":"m","turn_id":"r","artifact":{"uri":"file:///report.pdf","name":"report.pdf","mime_type":"application/pdf","operation":"attach"}}"#;
        store
            .append_messages(&AppendMessagesInput {
                conversation_id: "c".into(),
                expected_message_count: 0,
                updated_at_ms: 2,
                messages: vec![MessageInput {
                    id: "m".into(),
                    run_id: Some("r".into()),
                    role: "user".into(),
                    status: "completed".into(),
                    ordinal: 1,
                    created_at_ms: 1,
                    completed_at_ms: Some(2),
                    parts: vec![MessagePartInput {
                        id: "p".into(),
                        kind: "attachment".into(),
                        ordinal: 1,
                        state: "completed".into(),
                        content_text: "report.pdf".into(),
                        content_json: Some(raw.into()),
                        content_digest: None,
                        created_at_ms: 1,
                        updated_at_ms: 2,
                    }],
                }],
            })
            .unwrap();
        let owner = CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap();
        let value = store
            .read_historical_part(&owner, 2, 8192)
            .unwrap()
            .unwrap();
        assert_eq!(value.status, HistoricalProjectionStatus::Supported);
        assert_eq!(value.canonical_kind, "attachment");
        assert_eq!(value.binding, Some(SemanticIdentityAssessment::Canonical));
        assert!(matches!(
            value.fields["artifact"],
            SemanticValue::Complete { .. }
        ));
        assert_eq!(value.canonical_text.unwrap().text, "report.pdf");
        // A wrong embedded owner must not become trusted through the alias.
        store
            .connection
            .execute(
                "UPDATE message_parts SET content_json=?1",
                params![raw.replace("\"id\":\"p\"", "\"id\":\"foreign\"")],
            )
            .unwrap();
        let value = store
            .read_historical_part(&owner, 2, 8192)
            .unwrap()
            .unwrap();
        assert_eq!(value.status, unsupported("semantic_identity_unverified"));
        assert!(value.fields.is_empty());
        assert_eq!(value.canonical_text.unwrap().text, "report.pdf");
        store
            .connection
            .execute(
                "UPDATE message_parts SET content_json=?1,content_text=?2",
                params![
                    r#"{"legacy":{"unknown":true}}"#,
                    "旧格式正文\n仍可阅读😀".repeat(200)
                ],
            )
            .unwrap();
        let value = store
            .read_historical_part(&owner, 2, 1600)
            .unwrap()
            .unwrap();
        assert!(matches!(
            value.status,
            HistoricalProjectionStatus::Unsupported { .. }
        ));
        assert!(value.fields.is_empty());
        assert!(encoded_size(&value).unwrap() <= 1600);
        let text = value.canonical_text.unwrap();
        assert!(text.text.starts_with("旧格式正文"));
        assert!(text.total_utf8_bytes > text.text.len() as u64);
        assert!(
            matches!(store.read_historical_part_with_budget(&owner, 2, 0).unwrap(), HistoricalPartRead::BudgetDeferred { canonical_kind } if canonical_kind == "attachment")
        );
        assert!(store
            .read_historical_part_with_budget(&owner, 3, 0)
            .is_err());
        let wrong = CanonicalPartOwner::new("other".into(), "m".into(), "p".into()).unwrap();
        assert!(store
            .read_historical_part_with_budget(&wrong, 2, 0)
            .is_err());
        store
            .connection
            .execute("UPDATE message_parts SET content_json=NULL", [])
            .unwrap();
        assert_eq!(
            store
                .read_historical_part_with_budget(&owner, 2, 0)
                .unwrap(),
            HistoricalPartRead::Absent
        );
    }
    #[test]
    fn complete_fields_decode_original_strings_structures_and_share_canonical_identity() {
        let raw = r#"{"kind":"tool","execution_source":{"kind":"model"},"part_id":"p","revision":1,"tool_call_id":"call","capability_id":"read","title":"Read","origin":{},"artifacts":[],"state":"completed","input":{"path":"中😀","flags":[true,null,1]},"output":"line\nquote\"","affected_resources":[{"resource_type":"file","uri":"a","operation":"read"}]}"#;
        let (store, owner) = fixture("tool", Some(raw));
        let value = store
            .read_historical_part(&owner, 1, 8192)
            .unwrap()
            .unwrap();
        assert_eq!(value.status, HistoricalProjectionStatus::Supported);
        assert_eq!(value.binding, Some(SemanticIdentityAssessment::Canonical));
        assert_eq!(value.owner, owner);
        assert_eq!(value.schema, Some(HistoricalPartSchema::MessagePartView));
        assert_eq!(
            value.fields["output"],
            SemanticValue::Complete {
                value: StructuredValue::String("line\nquote\"".into())
            }
        );
        let original: serde_json::Value = serde_json::from_str(raw).unwrap();
        for (name, field) in &value.fields {
            let SemanticValue::Complete { value } = field else {
                panic!("unexpected deferred {name}")
            };
            assert_eq!(serde_json::to_value(value).unwrap(), original[name]);
        }
        assert!(encoded_size(&value).unwrap() <= 8192);
        assert!(store.read_historical_part(&owner, 2, 8192).is_err());
        let wrong = CanonicalPartOwner::new("c".into(), "other".into(), "p".into()).unwrap();
        assert!(store.read_historical_part(&wrong, 1, 8192).is_err());
    }

    #[test]
    fn current_public_schema_retains_independent_metadata_and_null_presence() {
        let evidence = serde_json::json!({"kind":"memory", "usage":{"record_id":"record", "component_id":"component", "source_role":"tool", "event_time":"event", "range_start":"start", "range_end":"end"}});
        for (kind, mut raw, fields) in [
            (
                "reasoning",
                serde_json::json!({"state":"completed", "title":"title only", "summary":""}),
                vec!["title", "summary"],
            ),
            (
                "tool",
                serde_json::json!({"tool_call_id":"call","capability_id":"read","title":"human title","summary":"","origin":{"kind":"mcp","provider_title":"source title"},"permission":{"request_id":"request","permission_domain":"files","rationale":"approval display","decision":"allow_once"},"state":"completed","input":{},"output":null,"output_preview":"independent preview","full_output":{"message_id":"m","part_id":"p","tool_call_id":"call"},"artifacts":[{"uri":"file:///a.pdf","name":"PDF","mime_type":"application/pdf","operation":null}],"started_at_ms":0,"finished_at_ms":u64::MAX,"affected_resources":[]}),
                vec![
                    "title",
                    "summary",
                    "origin",
                    "permission",
                    "output",
                    "output_preview",
                    "full_output",
                    "artifacts",
                    "started_at_ms",
                    "finished_at_ms",
                ],
            ),
            (
                "source",
                serde_json::json!({"source_id":"source","title":"Source","uri":"memory://record","evidence":evidence}),
                vec!["evidence"],
            ),
            (
                "citation",
                serde_json::json!({"citation_id":"not the Part ID","source_id":"source","title":"Quote","uri":"file:///a.pdf","mime_type":"application/pdf","origin_tool_call_id":"call","location":{},"section":null,"page":null,"line_start":null,"line_end":null,"range_start":0,"range_end":u64::MAX}),
                vec![
                    "citation_id",
                    "mime_type",
                    "location",
                    "section",
                    "range_start",
                    "range_end",
                ],
            ),
            (
                "attachment",
                serde_json::json!({"attachment_id":"attachment","name":"PDF","uri":"file:///a.pdf","mime_type":"application/pdf","operation":"write"}),
                vec!["operation"],
            ),
        ] {
            raw["kind"] = serde_json::json!(kind);
            if kind == "tool" {
                raw["execution_source"] = serde_json::json!({"kind":"model"});
            }
            raw["part_id"] = serde_json::json!("p");
            raw["revision"] = serde_json::json!(1);
            let (store, owner) = fixture(kind, Some(&raw.to_string()));
            let projection = store
                .read_historical_part(&owner, 1, 32768)
                .unwrap()
                .unwrap();
            assert_eq!(
                projection.status,
                HistoricalProjectionStatus::Supported,
                "{kind}"
            );
            for name in fields {
                assert_eq!(
                    projection.fields[name],
                    SemanticValue::Complete {
                        value: serde_json::from_value(raw[name].clone()).unwrap()
                    },
                    "{kind}/{name}"
                );
            }
        }
    }

    #[test]
    fn historical_readonly_objects_reject_embedded_authority_and_wrong_output_owner() {
        let base = serde_json::json!({"kind":"tool","execution_source":{"kind":"model"},"part_id":"p","revision":1,"tool_call_id":"call","capability_id":"read","title":"Read","origin":{},"artifacts":[],"state":"completed","input":{},"affected_resources":[]});
        for (field, nested, expected) in [
            (
                "origin",
                serde_json::json!({"kind":"mcp","token":"PRIVATE_TOKEN"}),
                "invalid_nested_origin",
            ),
            (
                "permission",
                serde_json::json!({"request_id":"request","permission_domain":"files","rationale":"display","grant":"PRIVATE_GRANT"}),
                "invalid_nested_permission",
            ),
            (
                "full_output",
                serde_json::json!({"message_id":"m","part_id":"p","tool_call_id":"call","path":"PRIVATE_PATH"}),
                "invalid_nested_full_output",
            ),
            (
                "full_output",
                serde_json::json!({"message_id":"other","part_id":"p","tool_call_id":"call"}),
                "invalid_tool_output_owner",
            ),
        ] {
            let mut raw = base.clone();
            raw[field] = nested;
            let (store, owner) = fixture("tool", Some(&raw.to_string()));
            let projection = store
                .read_historical_part(&owner, 1, 8192)
                .unwrap()
                .unwrap();
            assert_eq!(projection.status, unsupported(expected));
            assert!(projection.fields.is_empty());
            let wire = serde_json::to_string(&projection).unwrap();
            assert!(!wire.contains("PRIVATE_"));
        }
        let raw = serde_json::json!({"kind":"source","part_id":"p","revision":1,"source_id":"source","title":"Source","uri":"memory://record",
            "evidence":{"kind":"memory","usage":{"record_id":"record","component_id":"component","content":"PRIVATE_MEMORY_BODY"}}});
        let (store, owner) = fixture("source", Some(&raw.to_string()));
        let projection = store
            .read_historical_part(&owner, 1, 8192)
            .unwrap()
            .unwrap();
        assert_eq!(projection.status, unsupported("invalid_nested_evidence"));
        assert!(projection.fields.is_empty());
    }
    #[test]
    fn unsupported_identity_schema_and_nested_values_never_produce_executable_fields() {
        for (kind, raw, code) in [
            (
                "text",
                r#"{"kind":"text","part_id":"other","revision":1,"markdown":"x","is_final":true}"#,
                "semantic_identity_unverified",
            ),
            (
                "text",
                r#"{"kind":"text","id":"p","message_id":"m","turn_id":"r","text":"x","is_complete":true}"#,
                "semantic_identity_unverified",
            ),
            (
                "tool",
                r#"{"kind":"tool","execution_source":{"kind":"model"},"part_id":"p","revision":1,"tool_call_id":"call","capability_id":"read","title":"Read","origin":{},"artifacts":[],"state":"completed","input":{},"affected_resources":[{"bad":true}]}"#,
                "invalid_nested_affected_resources",
            ),
            (
                "compaction",
                r#"{"kind":"compaction","part_id":"p","revision":1,"summary":"s","retained_item_ids":[1],"source_fingerprint":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","original_token_count":1,"compacted_token_count":1}"#,
                "invalid_nested_retained_item_ids",
            ),
        ] {
            let (store, owner) = fixture(kind, Some(raw));
            let value = store
                .read_historical_part(&owner, 1, 8192)
                .unwrap()
                .unwrap();
            assert_eq!(value.status, unsupported(code));
            assert!(value.fields.is_empty());
            assert!(value.source.is_some());
        }
        let (store, owner) = fixture("text", Some("null"));
        let value = store
            .read_historical_part(&owner, 1, 8192)
            .unwrap()
            .unwrap();
        assert!(matches!(
            value.status,
            HistoricalProjectionStatus::Unsupported { .. }
        ));
        assert!(value.fields.is_empty());
        let (store, owner) = fixture("text", None);
        assert!(store.read_historical_part(&owner, 1, 0).unwrap().is_none());
    }
    #[test]
    fn ten_megabyte_json_reference_defers_original_body_without_lying_about_text() {
        use nola_store_port::canonical_body::CanonicalBodyRepository;
        let (mut store, owner) = fixture("text", None);
        store.begin_write_unit_of_work().unwrap();
        store
            .connection
            .execute("UPDATE message_parts SET content_text='' WHERE id='p'", [])
            .unwrap();
        let prefix = "{\"kind\":\"text\",\"part_id\":\"p\",\"revision\":1,\"markdown\":\"";
        let suffix = "\",\"is_final\":true}";
        let body = 10_000_000u64;
        let total = prefix.len() as u64 + body + suffix.len() as u64;
        let reference = nola_domain::CanonicalBodyReference::for_field(
            "json-body".into(),
            "c".into(),
            "m".into(),
            "p".into(),
            PartBodyField::ContentJson,
            total,
            total.div_ceil(32768) as u32,
        )
        .unwrap();
        let mut position = 0;
        let mut source = std::iter::from_fn(|| {
            if position == total {
                return None;
            }
            let offset = position;
            let length = (total - position).min(32768) as usize;
            let mut text = String::with_capacity(length);
            for i in 0..length {
                let at = offset + i as u64;
                let byte = if at < prefix.len() as u64 {
                    prefix.as_bytes()[at as usize]
                } else if at >= prefix.len() as u64 + body {
                    suffix.as_bytes()[(at - prefix.len() as u64 - body) as usize]
                } else {
                    b'x'
                };
                text.push(char::from(byte));
            }
            position += length as u64;
            Some(Ok((offset, text)))
        });
        store
            .prepare_body(&reference, &mut source, 20_000_000)
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO message_part_body_references VALUES('p','content_json',?1)",
                params![serde_json::to_string(&reference).unwrap()],
            )
            .unwrap();
        // Publish the changed backing and disclosure generation through the
        // same production transaction boundary used by canonical writers.
        store.commit_write_unit_of_work().unwrap();
        let window = store
            .load_conversation_message_window("c", None, 10, 4096)
            .unwrap()
            .unwrap();
        assert_eq!(window.conversation.messages.len(), 1);
        assert_eq!(window.conversation.messages[0].parts.len(), 1);
        assert!(window.conversation.messages[0].parts[0]
            .content_json
            .is_none());
        let value = store
            .read_historical_part(&owner, 1, 8192)
            .unwrap()
            .unwrap();
        assert_eq!(value.status, HistoricalProjectionStatus::Deferred);
        assert!(encoded_size(&value).unwrap() <= 8192);
        let SemanticValue::Deferred { locator } = &value.fields["markdown"] else {
            panic!("10MB markdown must be deferred")
        };
        assert_eq!(locator.end_bytes() - locator.start_bytes(), body + 2);
        assert_eq!(locator.source().owner(), &owner);
        let (_, range) = crate::published_part_body::field_range(
            &store.connection,
            "c",
            "m",
            "p",
            PartBodyField::ContentJson,
            locator.end_bytes() - 4,
            4,
        )
        .unwrap();
        assert_eq!(range.text, "xxx\"");
    }
    #[test]
    fn exact_response_budget_and_node_limits_defer_without_truncated_json() {
        let input = format!("[{}]", vec!["0"; 300].join(","));
        let raw=format!("{{\"kind\":\"tool\",\"execution_source\":{{\"kind\":\"model\"}},\"part_id\":\"p\",\"revision\":1,\"tool_call_id\":\"call\",\"capability_id\":\"read\",\"title\":\"Read\",\"origin\":{{}},\"artifacts\":[],\"state\":\"completed\",\"input\":{input},\"affected_resources\":[]}}");
        let (store, owner) = fixture("tool", Some(&raw));
        let value = store
            .read_historical_part(&owner, 1, 8192)
            .unwrap()
            .unwrap();
        assert_eq!(value.status, HistoricalProjectionStatus::Deferred);
        assert!(matches!(
            value.fields["input"],
            SemanticValue::Deferred { .. }
        ));
        assert!(store.read_historical_part(&owner, 1, 64).is_err());
        assert!(store.read_historical_part(&owner, 1, 32769).is_err());
        let raw = r#"{"kind":"compaction","part_id":"p","revision":1,"summary":"s","retained_item_ids":[ ],"source_fingerprint":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","original_token_count":2,"compacted_token_count":1}"#;
        let (store, owner) = fixture("compaction", Some(raw));
        let value = store
            .read_historical_part(&owner, 1, 8192)
            .unwrap()
            .unwrap();
        assert_eq!(value.status, HistoricalProjectionStatus::Supported);
    }
    #[test]
    fn input_wire_order_cannot_starve_complete_owner_and_tool_controls() {
        let controls = r#""kind":"tool","execution_source":{"kind":"model"},"part_id":"p","revision":1,"tool_call_id":"call","capability_id":"read","title":"Read","origin":{},"artifacts":[],"state":"completed","affected_resources":[]"#;
        let input = format!("\"{}\"", "x".repeat(1500));
        let mut statuses = Vec::new();
        for raw in [
            format!("{{\"input\":{input},{controls}}}"),
            format!("{{{controls},\"input\":{input}}}"),
        ] {
            let (store, owner) = fixture("tool", Some(&raw));
            let projected = store
                .read_historical_part(&owner, 1, 2048)
                .unwrap()
                .unwrap();
            assert!(encoded_size(&projected).unwrap() <= 2048);
            assert_eq!(
                projected.fields["part_id"],
                SemanticValue::Complete {
                    value: StructuredValue::String("p".into())
                }
            );
            assert_eq!(
                projected.fields["tool_call_id"],
                SemanticValue::Complete {
                    value: StructuredValue::String("call".into())
                }
            );
            statuses.push(projected.status);
        }
        assert_eq!(statuses[0], statuses[1]);
    }
}
