// SPDX-License-Identifier: Apache-2.0
//! Bind token extraction to one visible canonical owner and read snapshot.
use crate::published_part_body::ResolvedPartField;
use crate::semantic_json_projection::{TokenKind, TokenProjection};
use crate::{params, NolaStore, StoreError, StoreResult};
use nola_domain::{
    CanonicalPartOwner, PartBodyField, SemanticSourceIdentity, SemanticSourceKey,
    SemanticTokenKind, SemanticTokenLocator,
};

pub struct OwnedJsonTokens {
    pub source: SemanticSourceKey,
    pub tokens: TokenProjection,
}

impl OwnedJsonTokens {
    /// Only extracted field spans can become locators through this adapter.
    pub fn locator(&self, field_index: usize) -> StoreResult<Option<SemanticTokenLocator>> {
        let Some(field) = self
            .tokens
            .fields
            .iter()
            .find(|field| field.field_index == field_index)
        else {
            return Ok(None);
        };
        if field.duplicate_known_key {
            return Err(StoreError::Invariant(
                "ambiguous JSON field has no semantic locator".into(),
            ));
        }
        let kind = match field.span.kind {
            TokenKind::Null => SemanticTokenKind::Null,
            TokenKind::Bool => SemanticTokenKind::Boolean,
            TokenKind::Number => SemanticTokenKind::Number,
            TokenKind::String => SemanticTokenKind::String,
            TokenKind::Array => SemanticTokenKind::Array,
            TokenKind::Object => SemanticTokenKind::Object,
        };
        SemanticTokenLocator::new(
            self.source.clone(),
            field.span.start_bytes,
            field.span.end_bytes,
            kind,
        )
        .map(Some)
        .map_err(|error| StoreError::Invariant(error.to_string()))
    }
}

pub struct OwnedPartSemanticDraft {
    pub source: SemanticSourceKey,
    /// Recognition is not ownership attestation or permission to execute.
    pub draft: crate::semantic_part_draft::SemanticPartDraft,
    pub identity: Option<nola_domain::SemanticIdentityAssessment>,
}

fn assess_draft_identity(
    store: &NolaStore,
    draft: &crate::semantic_part_draft::SemanticPartDraft,
    owner: &CanonicalPartOwner,
    run_id: Option<&str>,
) -> StoreResult<Option<nola_domain::SemanticIdentityAssessment>> {
    use crate::semantic_json_projection::{ExtractedValue, ScalarValue};
    use crate::semantic_part_draft::{DraftStatus, HistoricalSchema};
    if draft.status != DraftStatus::Recognized {
        return Ok(None);
    }
    let field = |name| match draft.field(name).map(|field| &field.value) {
        Some(ExtractedValue::Complete(ScalarValue::String(value))) => Ok(value.as_str()),
        _ => Err(StoreError::Invariant(
            "recognized semantic owner is incomplete".into(),
        )),
    };
    let claim = match draft.schema {
        Some(HistoricalSchema::MessagePartView) => nola_kernel::SemanticOwnerClaim::Part {
            part_id: field("part_id")?,
        },
        Some(HistoricalSchema::AgentPart) => nola_kernel::SemanticOwnerClaim::AgentPart {
            part_id: field("id")?,
            message_id: field("message_id")?,
            turn_id: field("turn_id")?,
        },
        None => {
            return Err(StoreError::Invariant(
                "recognized semantic schema is unavailable".into(),
            ))
        }
    };
    let direct = nola_kernel::assess_semantic_identity(owner, run_id, claim);
    if direct == nola_domain::SemanticIdentityAssessment::Canonical {
        return Ok(Some(direct));
    }
    // The outer extraction snapshot also covers target metadata, both raw
    // fingerprints, immutable provenance and the historical root declaration.
    let record = store.part_derivation_record(owner)?;
    if let Some(stored) = &record {
        let fingerprint = stored.derivation.target.fields[1].as_ref();
        if !fingerprint.is_some_and(|value| {
            value.field == PartBodyField::ContentJson && value.sha256 == draft.tokens.source_sha256
        }) {
            return Err(StoreError::Conflict(
                "semantic draft and derivation name different original JSON".into(),
            ));
        }
    }
    Ok(Some(nola_kernel::assess_semantic_identity_with_derivation(
        owner,
        run_id,
        claim,
        record.as_ref(),
    )))
}

fn source_chunks<'a>(
    store: &'a NolaStore,
    owner: &'a CanonicalPartOwner,
    total: u64,
) -> impl Iterator<Item = Result<(u64, String), String>> + 'a {
    let mut offset = 0;
    let mut failed = false;
    std::iter::from_fn(move || {
        if failed || offset >= total {
            return None;
        }
        match crate::published_part_body::field_range(
            &store.connection,
            owner.conversation_id(),
            owner.message_id(),
            owner.part_id(),
            PartBodyField::ContentJson,
            offset,
            32768,
        ) {
            Ok((_, range)) => {
                let start = offset;
                offset = range.next_offset_bytes;
                Some(Ok((start, range.text)))
            }
            Err(error) => {
                failed = true;
                Some(Err(error.to_string()))
            }
        }
    })
}

impl NolaStore {
    /// Preparation seam for the shared semantic projection. It never publishes
    /// a message, changes a field, or treats a client descriptor as authority.
    pub fn inspect_part_json_tokens(
        &self,
        owner: &CanonicalPartOwner,
        expected_revision: i64,
        known_fields: &[&str],
        maximum_output_bytes: usize,
    ) -> StoreResult<Option<OwnedJsonTokens>> {
        self.inspect_json_source(owner, expected_revision, |total| {
            let tokens = crate::semantic_json_projection::extract_top_level(
                || source_chunks(self, owner, total),
                total,
                known_fields,
                maximum_output_bytes,
            )?;
            let digest = tokens.source_sha256.clone();
            Ok((tokens, digest))
        })
        .map(|value| value.map(|(source, tokens)| OwnedJsonTokens { source, tokens }))
    }

    pub fn inspect_part_semantic_draft(
        &self,
        owner: &CanonicalPartOwner,
        expected_revision: i64,
        maximum_output_bytes: usize,
    ) -> StoreResult<Option<OwnedPartSemanticDraft>> {
        self.inspect_json_source(owner, expected_revision, |total| {
            let kind: String = self.connection.query_row(
                "SELECT CASE WHEN length(CAST(kind AS BLOB))<=128 THEN kind END FROM message_parts WHERE id=?1",
                params![owner.part_id()], |row| row.get(0))?;
            let draft = crate::semantic_part_draft::extract_draft(
                || source_chunks(self, owner, total), total, &kind, maximum_output_bytes)?;
            let run_id: Option<String> = self.connection.query_row(
                "SELECT CASE WHEN length(CAST(run_id AS BLOB))<=128 THEN run_id END,
                 COALESCE(length(CAST(run_id AS BLOB)),0) FROM messages WHERE id=?1",
                params![owner.message_id()], |row| {
                    if row.get::<_, i64>(1)? > 128 { return Err(rusqlite::Error::InvalidQuery); }
                    row.get(0)
                })?;
            let identity = assess_draft_identity(self, &draft, owner, run_id.as_deref())?;
            let digest = draft.tokens.source_sha256.clone();
            Ok(((draft, identity), digest))
        }).map(|value| value.map(|(source, (draft, identity))| OwnedPartSemanticDraft { source, draft, identity }))
    }

    fn inspect_json_source<T>(
        &self,
        owner: &CanonicalPartOwner,
        expected_revision: i64,
        extract: impl FnOnce(u64) -> StoreResult<(T, String)>,
    ) -> StoreResult<Option<(SemanticSourceKey, T)>> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let current: i64 = self.connection.query_row(
            "SELECT revision FROM conversations WHERE id=?1",
            params![owner.conversation_id()],
            |row| row.get(0),
        )?;
        if expected_revision < 1 || current != expected_revision {
            return Err(StoreError::Conflict(
                "semantic source revision changed".into(),
            ));
        }
        let resolved = crate::published_part_body::resolve_field(
            &self.connection,
            owner.conversation_id(),
            owner.message_id(),
            owner.part_id(),
            PartBodyField::ContentJson,
        )?;
        let Some(total) = resolved.total_utf8_bytes() else {
            return Ok(None);
        };
        let (output, digest) = extract(total)?;
        let identity = match resolved {
            ResolvedPartField::Referenced(reference) => {
                SemanticSourceIdentity::Immutable { reference }
            }
            ResolvedPartField::Inline { .. } => SemanticSourceIdentity::InlineSha256 {
                digest,
                total_utf8_bytes: total,
            },
            ResolvedPartField::Chunked(_) => {
                return Err(StoreError::Invariant(
                    "text stream cannot be reinterpreted as a semantic JSON source".into(),
                ));
            }
            ResolvedPartField::Absent => unreachable!("absence returned before extraction"),
        };
        let source = SemanticSourceKey::new(owner.clone(), PartBodyField::ContentJson, identity)
            .map_err(|error| StoreError::Invariant(error.to_string()))?;
        Ok(Some((source, output)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (NolaStore, CanonicalPartOwner) {
        let store = NolaStore::open(":memory:").unwrap();
        store
            .connection
            .execute_batch(
                "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','title',1,1,1);
            INSERT INTO messages VALUES('m','c',NULL,'assistant','completed',1,1,1);
            INSERT INTO message_parts VALUES('p','m','tool',1,'completed','',NULL,NULL,1,1);",
            )
            .unwrap();
        (
            store,
            CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap(),
        )
    }
    #[test]
    fn original_tokens_keep_owner_and_revision_and_do_not_reencode_strings() {
        let (store, owner) = fixture();
        let json = r#"{ "input": {"x":"\u754c"}, "kind":"tool", "output":null }"#;
        store
            .connection
            .execute("UPDATE message_parts SET content_json=?1", params![json])
            .unwrap();
        let result = store
            .inspect_part_json_tokens(&owner, 1, &["kind", "input", "output"], 4096)
            .unwrap()
            .unwrap();
        let locator = result.locator(1).unwrap().unwrap();
        assert_eq!(
            &json[locator.start_bytes() as usize..locator.end_bytes() as usize],
            r#"{"x":"\u754c"}"#
        );
        assert_eq!(locator.source().owner(), &owner);
        assert_eq!(locator.token_kind(), SemanticTokenKind::Object);
        assert!(store
            .inspect_part_json_tokens(&owner, 2, &["kind"], 4096)
            .is_err());
        let wrong = CanonicalPartOwner::new("c".into(), "other".into(), "p".into()).unwrap();
        assert!(store
            .inspect_part_json_tokens(&wrong, 1, &["kind"], 4096)
            .is_err());
        let count: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM message_part_body_references",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
    #[test]
    fn absent_empty_and_ambiguous_json_remain_distinct() {
        let (store, owner) = fixture();
        assert!(store
            .inspect_part_json_tokens(&owner, 1, &["kind"], 4096)
            .unwrap()
            .is_none());
        store
            .connection
            .execute("UPDATE message_parts SET content_json=''", [])
            .unwrap();
        assert!(store
            .inspect_part_json_tokens(&owner, 1, &["kind"], 4096)
            .is_err());
        store
            .connection
            .execute(
                "UPDATE message_parts SET content_json=?1",
                params![r#"{"kind":"tool","\u006bind":"text"}"#],
            )
            .unwrap();
        let result = store
            .inspect_part_json_tokens(&owner, 1, &["kind"], 4096)
            .unwrap()
            .unwrap();
        assert!(result.locator(0).is_err());
    }
    #[test]
    fn prepared_object_is_not_a_visible_source_until_canonical_binding_exists() {
        use nola_store_port::canonical_body::CanonicalBodyRepository;
        let (mut store, owner) = fixture();
        let json = r#"{"input":[1,2],"kind":"tool"}"#;
        let reference = nola_domain::CanonicalBodyReference::for_field(
            "body".into(),
            "c".into(),
            "m".into(),
            "p".into(),
            PartBodyField::ContentJson,
            json.len() as u64,
            1,
        )
        .unwrap();
        store
            .prepare_body(
                &reference,
                &mut std::iter::once(Ok((0, json.into()))),
                32768,
            )
            .unwrap();
        assert!(store
            .inspect_part_json_tokens(&owner, 1, &["input"], 4096)
            .unwrap()
            .is_none());
        store
            .connection
            .execute(
                "INSERT INTO message_part_body_references VALUES('p','content_json',?1)",
                params![serde_json::to_string(&reference).unwrap()],
            )
            .unwrap();
        let result = store
            .inspect_part_json_tokens(&owner, 1, &["input"], 4096)
            .unwrap()
            .unwrap();
        assert_eq!(
            result.source.identity(),
            &SemanticSourceIdentity::Immutable { reference }
        );
        let locator = result.locator(0).unwrap().unwrap();
        assert_eq!(
            &json[locator.start_bytes() as usize..locator.end_bytes() as usize],
            "[1,2]"
        );
        store
            .connection
            .execute("DELETE FROM messages WHERE id='m'", [])
            .unwrap();
        assert!(store
            .inspect_part_json_tokens(&owner, 1, &["input"], 4096)
            .is_err());
    }
    #[test]
    fn schema_draft_uses_canonical_kind_without_promoting_unrecognized_json() {
        use crate::semantic_part_draft::{DraftStatus, HistoricalSchema};
        let (store, owner) = fixture();
        let original =
            r#"{"kind":"text","part_id":"p","revision":1,"markdown":"hello","is_final":true}"#;
        store
            .connection
            .execute(
                "UPDATE message_parts SET kind='text',content_json=?1",
                params![original],
            )
            .unwrap();
        let result = store
            .inspect_part_semantic_draft(&owner, 1, 8192)
            .unwrap()
            .unwrap();
        assert_eq!(result.draft.schema, Some(HistoricalSchema::MessagePartView));
        assert_eq!(result.draft.status, DraftStatus::Recognized);
        assert_eq!(
            result.identity,
            Some(nola_domain::SemanticIdentityAssessment::Canonical)
        );
        assert_eq!(result.source.owner(), &owner);
        store
            .connection
            .execute("UPDATE message_parts SET kind='tool'", [])
            .unwrap();
        let result = store
            .inspect_part_semantic_draft(&owner, 1, 8192)
            .unwrap()
            .unwrap();
        assert!(matches!(result.draft.status, DraftStatus::Unsupported(_)));
        let persisted: String = store
            .connection
            .query_row(
                "SELECT content_json FROM message_parts WHERE id='p'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(persisted, original);
        assert!(store.inspect_part_semantic_draft(&owner, 2, 8192).is_err());
    }
    #[test]
    fn embedded_agent_identity_cannot_override_the_canonical_message_or_run() {
        use nola_domain::SemanticIdentityAssessment::*;
        let (store, owner) = fixture();
        for (part, message, turn, canonical_run, expected) in [
            ("p", "m", "run", Some("run"), Canonical),
            ("old", "m", "run", Some("run"), DerivationRequired),
            ("p", "old", "run", Some("run"), DerivationRequired),
            ("p", "m", "old", Some("run"), DerivationRequired),
            ("p", "m", "run", None, CanonicalTurnUnavailable),
        ] {
            let raw = serde_json::json!({"kind":"text","id":part,"message_id":message,
                "turn_id":turn,"text":"original","is_complete":true})
            .to_string();
            store
                .connection
                .execute(
                    "UPDATE message_parts SET kind='text',content_json=?1",
                    params![raw],
                )
                .unwrap();
            store
                .connection
                .execute("UPDATE messages SET run_id=?1", params![canonical_run])
                .unwrap();
            let result = store
                .inspect_part_semantic_draft(&owner, 1, 8192)
                .unwrap()
                .unwrap();
            assert_eq!(result.identity, Some(expected));
            let persisted: String = store
                .connection
                .query_row(
                    "SELECT content_json FROM message_parts WHERE id='p'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(persisted, raw);
        }
    }
    #[test]
    fn local_two_generation_binding_survives_source_deletion_but_never_promotes_external_claims() {
        use nola_domain::{
            CanonicalBranchIntent, CanonicalBranchTarget, SemanticIdentityAssessment::*,
        };
        for raw in [
            r#"{"kind":"text","part_id":"p","revision":1,"markdown":"raw","is_final":true}"#,
            r#"{"kind":"text","id":"p","message_id":"m","turn_id":"run","text":"raw","is_complete":true}"#,
        ] {
            let (mut store, _) = fixture();
            store
                .connection
                .execute(
                    "UPDATE message_parts SET kind='text',content_json=?1",
                    params![raw],
                )
                .unwrap();
            store
                .connection
                .execute("UPDATE messages SET run_id='run'", [])
                .unwrap();
            let mut source = "c".to_string();
            let mut boundary = "m".to_string();
            let mut revision = 1;
            for target in ["branch", "second"] {
                store
                    .create_conversation_branch(&CanonicalBranchIntent {
                        operation_id: format!("op-{target}"),
                        source_conversation_id: source.clone(),
                        expected_source_revision: revision,
                        through_message_id: boundary,
                        conversation: CanonicalBranchTarget {
                            id: target.into(),
                            title: target.into(),
                            settings_json: None,
                            created_at_ms: 3,
                            updated_at_ms: 3,
                        },
                        committed_at_ms: 3,
                    })
                    .unwrap();
                let (message, part): (String, String) = store.connection.query_row(
                    "SELECT m.id,p.id FROM messages m JOIN message_parts p ON p.message_id=m.id WHERE m.conversation_id=?1",
                    params![target], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
                let owner =
                    CanonicalPartOwner::new(target.into(), message.clone(), part.clone()).unwrap();
                let read = |store: &NolaStore| {
                    store
                        .inspect_part_semantic_draft(&owner, 2, 8192)
                        .unwrap()
                        .unwrap()
                };
                assert_eq!(read(&store).identity, Some(VerifiedLocalDerivation));
                store.connection.execute("UPDATE message_parts SET content_text='source changed later' WHERE message_id IN (SELECT id FROM messages WHERE conversation_id=?1)", params![source]).unwrap();
                assert_eq!(read(&store).identity, Some(VerifiedLocalDerivation));
                assert!(store.delete_conversation(&source).unwrap());
                assert_eq!(read(&store).identity, Some(VerifiedLocalDerivation));
                let persisted: String = store
                    .connection
                    .query_row(
                        "SELECT content_json FROM message_parts WHERE id=?1",
                        params![part],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(persisted, raw);
                if target == "second" {
                    // Isolated fixture substitutes receiver-owned origin; production
                    // replica/archive ingress has independent cross-database tests.
                    let stored = store.part_derivation_record(&owner).unwrap().unwrap();
                    for provenance in ["replica", "archive"] {
                        store
                            .connection
                            .execute(
                                "DELETE FROM message_part_derivations WHERE target_part_id=?1",
                                params![part],
                            )
                            .unwrap();
                        store
                            .connection
                            .execute(
                                "INSERT INTO message_part_derivations VALUES(?1,?2,?3,?4)",
                                params![
                                    part,
                                    stored.derivation.operation_id,
                                    serde_json::to_string(&stored.derivation).unwrap(),
                                    provenance
                                ],
                            )
                            .unwrap();
                        assert_eq!(read(&store).identity, Some(DerivationRequired));
                    }
                    store
                        .connection
                        .execute(
                            "UPDATE message_parts SET content_text='changed' WHERE id=?1",
                            params![part],
                        )
                        .unwrap();
                    assert!(store.inspect_part_semantic_draft(&owner, 2, 8192).is_err());
                }
                source = target.into();
                boundary = message;
                revision = 2;
            }
        }
    }
}
