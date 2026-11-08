// SPDX-License-Identifier: Apache-2.0
//! Restore an already-public Part from the Store's bounded, verified fields.
//!
//! This is a presentation adapter, not historical tool replay or permission
//! restoration. Deferred tokens remain historical; neither the original JSON
//! body nor Core's private AgentPart schema is read or guessed here.

use nola_domain::{
    HistoricalPartProjection, HistoricalPartSchema, HistoricalProjectionStatus, MessageId,
    MessagePartId, MessagePartView, PartBodyField, SemanticIdentityAssessment, SemanticValue,
    StructuredValue,
};

pub(crate) fn project(projection: HistoricalPartProjection, revision: u64) -> MessagePartView {
    let can_restore = projection.schema == Some(HistoricalPartSchema::MessagePartView)
        && projection.binding == Some(SemanticIdentityAssessment::Canonical)
        && projection.status == HistoricalProjectionStatus::Supported
        && projection
            .fields
            .values()
            .all(|value| matches!(value, SemanticValue::Complete { .. }));
    if !can_restore {
        return historical(projection, revision);
    }

    // Binding is descriptive canonical evidence supplied by Store, never an
    // authorization grant. Reject inconsistent bounded evidence as a whole.
    match projection.source.as_ref() {
        Some(source)
            if source.owner() == &projection.owner
                && source.field() == PartBodyField::ContentJson => {}
        _ => return unsupported(projection, revision, "public_part_source_unverified"),
    }
    if string_field(&projection, "part_id") != Some(projection.owner.part_id()) {
        return unsupported(projection, revision, "public_part_owner_mismatch");
    }
    if string_field(&projection, "kind") != Some(projection.canonical_kind.as_str()) {
        return unsupported(projection, revision, "public_part_kind_mismatch");
    }

    // Missing keys stay missing, and an admitted JSON null is serialized as
    // null. The strict Domain decoder, including all nested read-only types,
    // is the single public contract; no hand-written lossy fallback is used.
    // These are only the already-budgeted Complete fields, not original JSON.
    let fields: Result<serde_json::Map<String, serde_json::Value>, _> = projection
        .fields
        .iter()
        .map(|(name, value)| {
            let SemanticValue::Complete { value } = value else {
                unreachable!("the complete-fields precondition was checked")
            };
            serde_json::to_value(value).map(|value| (name.clone(), value))
        })
        .collect();
    let candidate = fields.and_then(|fields| {
        serde_json::from_value::<MessagePartView>(serde_json::Value::Object(fields))
    });
    let Ok(mut part) = candidate else {
        return unsupported(projection, revision, "invalid_public_part_contract");
    };
    // A public Historical wrapper is not an original public Part that can be
    // recursively promoted. Owner-bound output and citation conveniences must
    // agree with the canonical public facts before exposing a typed Part.
    if matches!(part, MessagePartView::Historical { .. })
        || part
            .validate_public_semantics(&MessageId(projection.owner.message_id().to_owned()))
            .is_err()
    {
        return unsupported(projection, revision, "invalid_public_part_contract");
    }

    // The stored source's clock remains in its historical interpretation. A
    // local projection uses its own revision, never a remote source clock.
    match &mut part {
        MessagePartView::Historical { .. } => unreachable!("historical was rejected above"),
        MessagePartView::UiAction { revision: value, .. }
        | MessagePartView::UiSurface { revision: value, .. }
        | MessagePartView::Text {
            revision: value, ..
        }
        | MessagePartView::Reasoning {
            revision: value, ..
        }
        | MessagePartView::Tool {
            revision: value, ..
        }
        | MessagePartView::Source {
            revision: value, ..
        }
        | MessagePartView::Citation {
            revision: value, ..
        }
        | MessagePartView::Attachment {
            revision: value, ..
        }
        | MessagePartView::Task {
            revision: value, ..
        }
        | MessagePartView::Skill {
            revision: value, ..
        }
        | MessagePartView::ModelAttempt {
            revision: value, ..
        }
        | MessagePartView::Compaction {
            revision: value, ..
        }
        | MessagePartView::Error {
            revision: value, ..
        } => *value = revision,
    }
    part
}

fn string_field<'a>(projection: &'a HistoricalPartProjection, name: &str) -> Option<&'a str> {
    match projection.fields.get(name) {
        Some(SemanticValue::Complete {
            value: StructuredValue::String(value),
        }) => Some(value),
        _ => None,
    }
}

fn unsupported(
    mut projection: HistoricalPartProjection,
    revision: u64,
    code: &str,
) -> MessagePartView {
    // Historical inspectors show Complete fields even for an Unsupported
    // envelope. A failed public contract therefore retains identity and source
    // evidence only; it must not disclose a private grant or withheld candidate
    // via its fallback. Original bytes remain in Store, not in this view.
    projection.fields.clear();
    projection.canonical_text = None;
    projection.status = HistoricalProjectionStatus::Unsupported {
        code: code.to_owned(),
    };
    historical(projection, revision)
}

fn historical(projection: HistoricalPartProjection, revision: u64) -> MessagePartView {
    MessagePartView::Historical {
        part_id: MessagePartId(projection.owner.part_id().to_owned()),
        revision,
        projection: Box::new(projection),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_domain::{
        CanonicalPartOwner, HistoricalNotLoadedReason, HistoricalTextPreview,
        SemanticSourceIdentity, SemanticSourceKey, SemanticTokenKind, SemanticTokenLocator,
        TextReviewState,
    };
    use serde_json::{json, Value};

    fn owner(message: &str, part: &str) -> CanonicalPartOwner {
        CanonicalPartOwner::new("c".into(), message.into(), part.into()).unwrap()
    }

    fn source(owner: CanonicalPartOwner) -> SemanticSourceKey {
        SemanticSourceKey::new(
            owner,
            PartBodyField::ContentJson,
            SemanticSourceIdentity::InlineSha256 {
                digest: "a".repeat(64),
                total_utf8_bytes: 65536,
            },
        )
        .unwrap()
    }

    fn projection(raw: Value) -> HistoricalPartProjection {
        let fields = raw.as_object().unwrap();
        HistoricalPartProjection {
            owner: owner("m", "p"),
            canonical_revision: 123,
            source: Some(source(owner("m", "p"))),
            schema: Some(HistoricalPartSchema::MessagePartView),
            canonical_kind: fields["kind"].as_str().unwrap().into(),
            binding: Some(SemanticIdentityAssessment::Canonical),
            canonical_text: None,
            status: HistoricalProjectionStatus::Supported,
            fields: fields
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        SemanticValue::Complete {
                            value: serde_json::from_value(value.clone()).unwrap(),
                        },
                    )
                })
                .collect(),
        }
    }

    fn tool() -> Value {
        json!({
            "kind":"tool", "execution_source":{"kind":"model"}, "part_id":"p", "revision":999,
            "tool_call_id":"call", "capability_id":"file.read", "state":"completed",
            "title":"独立标题中😀", "summary":"",
            "origin":{"kind":"mcp","provider_id":"provider","provider_title":"来源中😀","namespace":"tools"},
            "permission":{"request_id":"request","permission_domain":"files","rationale":"显示而非授权","decision":"allow_once"},
            "input":null, "output_preview":"独立预览中😀",
            "full_output":{"message_id":"m","part_id":"p","tool_call_id":"call"},
            "artifacts":[{"uri":"file:///report.pdf","name":"文档中😀","mime_type":"application/pdf","operation":null}],
            "started_at_ms":0, "finished_at_ms":u64::MAX,
            "affected_resources":[{"resource_type":"file","uri":"file:///report.pdf","operation":"read"}]
        })
    }

    fn assert_historical_preserved(projection: HistoricalPartProjection) {
        let expected = projection.clone();
        let MessagePartView::Historical {
            part_id,
            revision,
            projection,
        } = project(projection, 7)
        else {
            panic!("incomplete or noncanonical evidence was promoted")
        };
        assert_eq!(part_id.0, expected.owner.part_id());
        assert_eq!(revision, 7);
        assert_eq!(*projection, expected);
    }

    fn assert_invalid(projection: HistoricalPartProjection) {
        let expected = projection.clone();
        let MessagePartView::Historical {
            part_id,
            revision,
            projection,
        } = project(projection, 7)
        else {
            panic!("invalid public contract became an ordinary Part")
        };
        assert_eq!(part_id.0, expected.owner.part_id());
        assert_eq!(revision, 7);
        assert_eq!(projection.owner, expected.owner);
        assert_eq!(projection.source, expected.source);
        assert_eq!(projection.schema, expected.schema);
        assert_eq!(projection.binding, expected.binding);
        assert_eq!(projection.canonical_revision, expected.canonical_revision);
        assert_eq!(projection.canonical_kind, expected.canonical_kind);
        assert!(projection.fields.is_empty());
        assert!(projection.canonical_text.is_none());
        assert!(matches!(
            projection.status,
            HistoricalProjectionStatus::Unsupported { .. }
        ));
    }

    #[test]
    fn complete_tool_preserves_independent_fields_and_missing_null_empty_results() {
        for output in [
            None,
            Some(Value::Null),
            Some(json!("")),
            Some(json!([])),
            Some(json!({})),
            Some(json!({"unicode":"完整结构化结果中😀","large_integer":u64::MAX})),
        ] {
            let mut raw = tool();
            if let Some(output) = output.clone() {
                raw["output"] = output;
            }
            let mut expected = raw.clone();
            expected["revision"] = 7.into();
            let actual = project(projection(raw), 7);
            assert_eq!(
                actual,
                serde_json::from_value::<MessagePartView>(expected).unwrap()
            );
            let MessagePartView::Tool {
                input,
                output: actual_output,
                summary,
                output_preview,
                finished_at_ms,
                ..
            } = actual
            else {
                panic!("complete verified public tool was not restored")
            };
            assert_eq!(input, StructuredValue::Null);
            assert_eq!(summary.as_deref(), Some(""));
            assert_eq!(output_preview.as_deref(), Some("独立预览中😀"));
            assert_eq!(finished_at_ms, Some(u64::MAX));
            assert_eq!(
                actual_output,
                output.map(|value| serde_json::from_value(value).unwrap())
            );
        }
    }

    #[test]
    fn complete_public_metadata_uses_the_exact_domain_contract_and_local_clock() {
        let fixtures = [
            json!({"kind":"reasoning","part_id":"p","revision":999,"state":"completed","title":"标题中😀","summary":"独立摘要中😀"}),
            json!({"kind":"source","part_id":"p","revision":999,"source_id":"source","title":"记忆证据","uri":"","mime_type":"text/plain","snippet":"独立片段","evidence":{"kind":"memory","usage":{"record_id":"record","component_id":"component","scope":"conversation","namespace":{"kind":"custom","name":"工作区中😀"},"provenance_kind":"explicit_user","source":"user","title":"证据标题","observed_at":"observed","source_role":"user","event_time":"event","range_start":"start","range_end":"end","requires_current_state_verification":true,"truncated":false,"supersedes":["old"],"usage_count":u64::MAX,"last_used_at_epoch_ms":-1}}}),
            json!({"kind":"attachment","part_id":"p","revision":999,"attachment_id":"attachment","name":"文档中😀","mime_type":"application/pdf","uri":"file:///report.pdf","operation":"write"}),
        ];
        for mut raw in fixtures {
            let actual = project(projection(raw.clone()), 7);
            raw["revision"] = 7.into();
            assert_eq!(
                actual,
                serde_json::from_value::<MessagePartView>(raw).unwrap()
            );
        }
    }

    #[test]
    fn citation_preserves_true_id_location_absence_empty_and_full_width_range() {
        for location in [
            None,
            Some(json!({})),
            Some(json!({"page":0,"line_start":u64::MAX,"line_end":0,"section":"节中😀"})),
        ] {
            let mut raw = json!({"kind":"citation","part_id":"p","revision":999,
                "citation_id":"true-citation-id","source_id":"source","title":"引文中😀","uri":"file:///report.pdf",
                "mime_type":"application/pdf","origin_tool_call_id":"origin-call","range_start":u64::MAX-1,"range_end":u64::MAX});
            if let Some(location) = location {
                for field in ["page", "line_start", "line_end", "section"] {
                    if let Some(value) = location.get(field) {
                        raw[field] = value.clone();
                    }
                }
                raw["location"] = location;
            }
            let actual = project(projection(raw.clone()), 7);
            raw["revision"] = 7.into();
            assert_eq!(
                actual,
                serde_json::from_value::<MessagePartView>(raw).unwrap()
            );
        }
        let base = json!({"kind":"citation","part_id":"p","revision":999,"citation_id":"cite","source_id":"source","title":"Title","uri":"","origin_tool_call_id":"call"});
        for (start, end) in [(Some(3), None), (None, Some(3)), (Some(4), Some(3))] {
            let mut raw = base.clone();
            if let Some(start) = start {
                raw["range_start"] = start.into();
            }
            if let Some(end) = end {
                raw["range_end"] = end.into();
            }
            assert_invalid(projection(raw));
        }
        let mut mismatch = base;
        mismatch["location"] = json!({"page":0});
        mismatch["page"] = 1.into();
        assert_invalid(projection(mismatch));
    }

    #[test]
    fn only_exact_canonical_public_complete_evidence_can_be_restored() {
        for binding in [
            None,
            Some(SemanticIdentityAssessment::VerifiedLocalDerivation),
            Some(SemanticIdentityAssessment::DerivationRequired),
            Some(SemanticIdentityAssessment::CanonicalTurnUnavailable),
        ] {
            let mut value = projection(tool());
            value.binding = binding;
            assert_historical_preserved(value);
        }
        for schema in [None, Some(HistoricalPartSchema::AgentPart)] {
            let mut value = projection(tool());
            value.schema = schema;
            assert_historical_preserved(value);
        }
        for status in [
            HistoricalProjectionStatus::Deferred,
            HistoricalProjectionStatus::NotLoaded {
                reason: HistoricalNotLoadedReason::SelectionBudget,
            },
            HistoricalProjectionStatus::Unsupported {
                code: "schema_unknown_kind".into(),
            },
        ] {
            let mut value = projection(tool());
            value.status = status;
            assert_historical_preserved(value);
        }
        let mut value = projection(tool());
        // Even an inconsistent Supported envelope cannot turn a deferred field
        // into missing, null, a preview, or a partially reconstructed Tool.
        value.fields.insert(
            "output".into(),
            SemanticValue::Deferred {
                locator: SemanticTokenLocator::new(
                    source(owner("m", "p")),
                    10,
                    100,
                    SemanticTokenKind::Object,
                )
                .unwrap(),
            },
        );
        assert_historical_preserved(value);
    }

    #[test]
    fn owner_kind_and_full_output_all_three_owners_are_revalidated() {
        let mut wrong_part = tool();
        wrong_part["part_id"] = "other-part".into();
        assert_invalid(projection(wrong_part));
        let mut wrong_kind = projection(tool());
        wrong_kind.canonical_kind = "text".into();
        assert_invalid(wrong_kind);
        let mut wrong_source = projection(tool());
        wrong_source.source = Some(source(owner("other-message", "p")));
        assert_invalid(wrong_source);
        for field in ["message_id", "part_id", "tool_call_id"] {
            let mut raw = tool();
            raw["full_output"][field] = "other-owner".into();
            assert_invalid(projection(raw));
        }
    }

    #[test]
    fn complete_public_part_requires_original_json_source_evidence() {
        let mut missing_source = projection(tool());
        missing_source.source = None;
        assert_invalid(missing_source);

        let mut non_json_source = projection(tool());
        non_json_source.source = Some(
            SemanticSourceKey::new(
                owner("m", "p"),
                PartBodyField::ContentText,
                SemanticSourceIdentity::InlineSha256 {
                    digest: "b".repeat(64),
                    total_utf8_bytes: 65536,
                },
            )
            .unwrap(),
        );
        assert_invalid(non_json_source);
    }

    #[test]
    fn unknown_private_fields_and_grants_never_become_ordinary_parts() {
        for (field, value) in [
            ("raw_input", json!("private model payload")),
            ("full_output_ref", json!("/private/result.json")),
            ("grant", json!({"allow":true})),
            ("future", json!(1)),
        ] {
            let mut raw = tool();
            raw[field] = value;
            assert_invalid(projection(raw));
        }
        for (field, value) in [
            (
                "permission",
                json!({"request_id":"request","permission_domain":"files","rationale":"read-only","grant":{"allow":true}}),
            ),
            (
                "full_output",
                json!({"message_id":"m","part_id":"p","tool_call_id":"call","uri":"file:///private/result.json"}),
            ),
            (
                "origin",
                json!({"kind":"mcp","provider_token":"private credential"}),
            ),
        ] {
            let mut raw = tool();
            raw[field] = value;
            assert_invalid(projection(raw));
        }
        let mut missing = tool();
        missing.as_object_mut().unwrap().remove("input");
        assert_invalid(projection(missing));
    }

    #[test]
    fn invalid_fallback_clears_fields_and_preview_before_historical_details_can_display_them() {
        let mut private_tool = tool();
        private_tool["grant"] = json!({"token":"PRIVATE_GRANT_MARKER"});
        let mut private_permission = tool();
        private_permission["permission"]["grant"] = json!("NESTED_PRIVATE_GRANT_MARKER");
        let withheld_text = json!({"kind":"text","part_id":"p","revision":999,
            "markdown":"WITHHELD_CANDIDATE_MARKER","is_final":false,"review":"pending"});
        for raw in [private_tool, private_permission, withheld_text] {
            let mut value = projection(raw);
            value.canonical_text = Some(HistoricalTextPreview {
                text: "FALLBACK_PREVIEW_MARKER".into(),
                total_utf8_bytes: 23,
            });
            let actual = project(value, 7);
            let MessagePartView::Historical { projection, .. } = &actual else {
                panic!("invalid public value was promoted")
            };
            assert!(projection.fields.is_empty());
            assert!(projection.canonical_text.is_none());
            assert!(matches!(
                projection.status,
                HistoricalProjectionStatus::Unsupported { .. }
            ));
            let serialized = serde_json::to_string(&actual).unwrap();
            for marker in [
                "PRIVATE_GRANT_MARKER",
                "NESTED_PRIVATE_GRANT_MARKER",
                "WITHHELD_CANDIDATE_MARKER",
                "FALLBACK_PREVIEW_MARKER",
            ] {
                assert!(
                    !serialized.contains(marker),
                    "invalid fallback exposed {marker}"
                );
            }
        }
    }

    #[test]
    fn approved_text_and_explicit_empty_review_survive_but_withheld_content_does_not() {
        for review in [
            None,
            Some("approved"),
            Some("pending"),
            Some("rejected"),
            Some("interrupted"),
        ] {
            let markdown = if matches!(review, None | Some("approved")) {
                "公开中😀"
            } else {
                ""
            };
            let mut raw = json!({"kind":"text","part_id":"p","revision":999,"markdown":markdown,"is_final":true});
            if let Some(review) = review {
                raw["review"] = review.into();
            }
            let actual = project(projection(raw.clone()), 7);
            raw["revision"] = 7.into();
            assert_eq!(
                actual,
                serde_json::from_value::<MessagePartView>(raw).unwrap()
            );
        }
        for review in [
            TextReviewState::Pending,
            TextReviewState::Rejected,
            TextReviewState::Interrupted,
        ] {
            assert_invalid(projection(
                json!({"kind":"text","part_id":"p","revision":999,
                "markdown":"非公开候选不能成为普通正文","is_final":false,"review":review}),
            ));
        }
        // Store disclosure may have removed a withheld candidate. Missing is
        // never synthesized as an empty, apparently complete public value.
        assert_invalid(projection(
            json!({"kind":"text","part_id":"p","revision":999,
            "is_final":false,"review":"pending"}),
        ));
    }
}
