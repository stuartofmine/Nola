// SPDX-License-Identifier: Apache-2.0
//! Nested semantic preparation from a verified original token range.
use crate::semantic_json_projection::TokenKind;
use crate::semantic_nested_draft::{NestedDraft, NestedKind};
use crate::semantic_part_draft::{DraftStatus, HistoricalSchema};
use crate::{NolaStore, StoreError, StoreResult};
use nola_domain::{
    CanonicalPartOwner, PartBodyField, SemanticIdentityAssessment, SemanticTokenKind,
    SemanticTokenLocator,
};

pub struct OwnedNestedDraft {
    pub locator: SemanticTokenLocator,
    pub identity: Option<SemanticIdentityAssessment>,
    pub nested: NestedDraft,
}

impl NolaStore {
    /// Returns structural validation separately from parent ownership. Even a
    /// validated historical permission object is not a live permission request.
    pub fn inspect_part_nested_draft(
        &self,
        owner: &CanonicalPartOwner,
        expected_revision: i64,
        kind: NestedKind,
        maximum_output_bytes: usize,
    ) -> StoreResult<Option<OwnedNestedDraft>> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let Some(parent) =
            self.inspect_part_semantic_draft(owner, expected_revision, maximum_output_bytes)?
        else {
            return Ok(None);
        };
        if parent.draft.status != DraftStatus::Recognized {
            return Err(StoreError::Invariant(
                "nested semantics require a recognized parent schema".into(),
            ));
        }
        let (schema, part_kind, field, token_kind) = match kind {
            NestedKind::Artifact => (
                HistoricalSchema::AgentPart,
                "artifact",
                "artifact",
                SemanticTokenKind::Object,
            ),
            NestedKind::Artifacts => (
                parent.draft.schema.expect("recognized Part schema"),
                "tool",
                "artifacts",
                SemanticTokenKind::Array,
            ),
            NestedKind::Origin => (
                parent.draft.schema.expect("recognized Part schema"),
                "tool",
                "origin",
                SemanticTokenKind::Object,
            ),
            NestedKind::Permission => (
                parent.draft.schema.expect("recognized Part schema"),
                "tool",
                "permission",
                SemanticTokenKind::Object,
            ),
            NestedKind::AffectedResources => (
                HistoricalSchema::MessagePartView,
                "tool",
                "affected_resources",
                SemanticTokenKind::Array,
            ),
            NestedKind::Evidence => (
                parent.draft.schema.expect("recognized Part schema"),
                "source",
                "evidence",
                SemanticTokenKind::Object,
            ),
            NestedKind::Location => (
                parent.draft.schema.expect("recognized Part schema"),
                "citation",
                "location",
                SemanticTokenKind::Object,
            ),
            NestedKind::Range => (
                HistoricalSchema::AgentPart,
                "citation",
                "range",
                SemanticTokenKind::Object,
            ),
            NestedKind::RetainedItemIds => (
                parent.draft.schema.ok_or_else(|| {
                    StoreError::Invariant("compaction nested field has no schema".into())
                })?,
                "compaction",
                "retained_item_ids",
                SemanticTokenKind::Array,
            ),
            NestedKind::ToolOutputReference => (
                HistoricalSchema::MessagePartView,
                "tool",
                "full_output",
                SemanticTokenKind::Object,
            ),
        };
        if parent.draft.schema != Some(schema) || parent.draft.kind.as_deref() != Some(part_kind) {
            return Err(StoreError::Invariant(
                "nested field does not belong to this Part schema".into(),
            ));
        }
        let Some(token) = parent.draft.field(field) else {
            return Ok(None);
        };
        if token.span.kind == TokenKind::Null {
            return Ok(None);
        }
        let start = token.span.start_bytes;
        let total = token.span.end_bytes - start;
        let locator = SemanticTokenLocator::new(
            parent.source.clone(),
            start,
            token.span.end_bytes,
            token_kind,
        )
        .map_err(|error| StoreError::Invariant(error.to_string()))?;
        let identity = parent.identity;
        drop(parent);
        let nested = crate::semantic_nested_draft::validate_nested(
            || {
                let mut offset = 0;
                let mut failed = false;
                std::iter::from_fn(move || {
                    if failed || offset >= total {
                        return None;
                    }
                    match crate::published_part_body::field_range(
                        &self.connection,
                        owner.conversation_id(),
                        owner.message_id(),
                        owner.part_id(),
                        PartBodyField::ContentJson,
                        start + offset,
                        32768,
                    ) {
                        Ok((_, range)) => {
                            let mut text = range.text;
                            let length = text.len().min((total - offset) as usize);
                            if length == 0 || !text.is_char_boundary(length) {
                                failed = true;
                                return Some(Err(
                                    "nested token range made invalid UTF-8 progress".into()
                                ));
                            }
                            text.truncate(length);
                            let previous = offset;
                            offset += length as u64;
                            Some(Ok((previous, text)))
                        }
                        Err(error) => {
                            failed = true;
                            Some(Err(error.to_string()))
                        }
                    }
                })
            },
            total,
            kind,
            maximum_output_bytes,
        )?;
        Ok(Some(OwnedNestedDraft {
            locator,
            identity,
            nested,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params;
    use crate::semantic_nested_draft::NestedStatus;
    #[test]
    fn typed_resource_span_is_bound_to_the_visible_parent_and_revision() {
        let store = NolaStore::open(":memory:").unwrap();
        store
            .connection
            .execute_batch(
                "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','title',1,1,1);
            INSERT INTO messages VALUES('m','c',NULL,'assistant','completed',1,1,1);
            INSERT INTO message_parts VALUES('p','m','tool',1,'completed','',NULL,NULL,1,1);",
            )
            .unwrap();
        let raw = r#"{"kind":"tool","execution_source":{"kind":"model"},"part_id":"p","revision":1,"tool_call_id":"call","capability_id":"file","title":"Read","origin":{},"artifacts":[],"state":"completed","input":{},"output":null,"affected_resources":[{"resource_type":"file","uri":"原文","operation":"read"}]}"#;
        store
            .connection
            .execute("UPDATE message_parts SET content_json=?1", params![raw])
            .unwrap();
        let owner = CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap();
        let result = store
            .inspect_part_nested_draft(&owner, 1, NestedKind::AffectedResources, 8192)
            .unwrap()
            .unwrap();
        assert_eq!(result.identity, Some(SemanticIdentityAssessment::Canonical));
        assert_eq!(result.nested.status, NestedStatus::Validated);
        assert_eq!(result.nested.resources[0].value.uri, "原文");
        assert_eq!(
            &raw[result.locator.start_bytes() as usize..result.locator.end_bytes() as usize],
            r#"[{"resource_type":"file","uri":"原文","operation":"read"}]"#
        );
        assert!(store
            .inspect_part_nested_draft(&owner, 2, NestedKind::AffectedResources, 8192)
            .is_err());
        assert!(store
            .inspect_part_nested_draft(&owner, 1, NestedKind::Permission, 8192)
            .unwrap()
            .is_none());
        assert!(store
            .inspect_part_nested_draft(&owner, 1, NestedKind::Range, 8192)
            .is_err());
        // Public permission-display facts are optional, not a live grant.
        let with_permission = raw.replace(
            "\"affected_resources\":",
            "\"permission\":{\"request_id\":\"request\",\"permission_domain\":\"files\",\"rationale\":\"Historical display\",\"decision\":null},\"affected_resources\":",
        );
        store
            .connection
            .execute(
                "UPDATE message_parts SET content_json=?1",
                params![with_permission],
            )
            .unwrap();
        let display = store
            .inspect_part_nested_draft(&owner, 1, NestedKind::Permission, 8192)
            .unwrap()
            .unwrap();
        assert_eq!(
            display.identity,
            Some(SemanticIdentityAssessment::Canonical)
        );
        assert_eq!(display.nested.status, NestedStatus::Validated);
        store
            .connection
            .execute("DELETE FROM messages WHERE id='m'", [])
            .unwrap();
        assert!(store
            .inspect_part_nested_draft(&owner, 1, NestedKind::AffectedResources, 8192)
            .is_err());
    }
    #[test]
    fn citation_nested_fields_use_their_actual_schema_and_original_token_locators() {
        let store = NolaStore::open(":memory:").unwrap();
        let raw = r#"{"kind":"citation","id":"p","message_id":"m","turn_id":"r","citation_id":"cite","source_id":"source","title":"title","uri":"file:///original","origin_tool_call_id":"call","location":{"page":2,"section":"\u4e2d"},"range":{"start":9,"end":2}}"#;
        store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','title',1,1,1);
            INSERT INTO messages VALUES('m','c','r','assistant','completed',1,1,1);
            INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
            VALUES('p','m','citation',1,'completed','',NULL,1,1);").unwrap();
        store
            .connection
            .execute("UPDATE message_parts SET content_json=?1", params![raw])
            .unwrap();
        let owner = CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap();
        for (kind, original) in [
            (NestedKind::Location, r#"{"page":2,"section":"\u4e2d"}"#),
            (NestedKind::Range, r#"{"start":9,"end":2}"#),
        ] {
            let result = store
                .inspect_part_nested_draft(&owner, 1, kind, 8192)
                .unwrap()
                .unwrap();
            assert_eq!(result.nested.status, NestedStatus::Validated);
            assert_eq!(result.identity, Some(SemanticIdentityAssessment::Canonical));
            assert_eq!(
                &raw[result.locator.start_bytes() as usize..result.locator.end_bytes() as usize],
                original
            );
            assert_eq!(result.locator.source().owner(), &owner);
            assert!(store
                .inspect_part_nested_draft(&owner, 2, kind, 8192)
                .is_err());
        }
        assert!(store
            .inspect_part_nested_draft(&owner, 1, NestedKind::Evidence, 8192)
            .is_err());
    }
}
