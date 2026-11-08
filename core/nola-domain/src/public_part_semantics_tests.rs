// SPDX-License-Identifier: Apache-2.0
use super::*;
use crate::{ActivityState, CapabilityId, MessagePartView, StructuredValue};

fn tool(output: Option<StructuredValue>) -> MessagePartView {
    MessagePartView::Tool {
        execution_source: Default::default(),
        part_id: MessagePartId("part".into()),
        revision: 7,
        tool_call_id: ToolCallId("call".into()),
        capability_id: CapabilityId("cap".into()),
        state: ActivityState::Completed,
        display_key: None,
        title: "动态标题 汉🙂".into(),
        summary: Some("".into()),
        origin: HostCapabilityOrigin::default(),
        permission: Some(ToolPermissionDisplay {
            request_id: "request".into(),
            permission_domain: "filesystem".into(),
            rationale: "只读说明".into(),
            decision: Some("allow".into()),
        }),
        input: StructuredValue::Null,
        output,
        output_preview: Some("独立预览".into()),
        full_output: Some(ToolOutputReference {
            message_id: MessageId("message".into()),
            part_id: MessagePartId("part".into()),
            tool_call_id: ToolCallId("call".into()),
        }),
        artifacts: vec![ArtifactReference {
            uri: "file:///artifact".into(),
            name: "报告".into(),
            mime_type: None,
            operation: Some("write".into()),
        }],
        started_at_ms: Some(0),
        finished_at_ms: Some(u64::MAX),
        affected_resources: vec![],
    }
}

#[test]
fn public_code_tool_source_roundtrips_and_rejects_invalid_identity() {
    let mut part = tool(None);
    let source = crate::code_mode::CodeItemSource::CodeCell {
        cell: crate::code_mode::CodeCellIdentity {
            host_id:"h".into(), conversation_id:"c".into(), turn_id:"t".into(),
            parent_call_id:"parent".into(), cell_id:"cell".into(), generation:"g".into(),
        },
    };
    if let MessagePartView::Tool { execution_source, .. } = &mut part {
        *execution_source = source;
    }
    let wire = serde_json::to_value(&part).unwrap();
    let restored: MessagePartView = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(restored, part);
    assert!(restored.validate_public_semantics(&MessageId("message".into())).is_ok());
    let mut invalid = wire.clone();
    invalid["execution_source"]["cell"]["generation"] = serde_json::json!("");
    let invalid: MessagePartView = serde_json::from_value(invalid).unwrap();
    assert!(invalid.validate_public_semantics(&MessageId("message".into())).is_err());
    let mut absent = wire;
    absent.as_object_mut().unwrap().remove("execution_source");
    assert!(serde_json::from_value::<MessagePartView>(absent).is_err());
}

#[test]
fn absent_null_empty_results_and_independent_summary_preview_roundtrip() {
    for output in [
        None,
        Some(StructuredValue::Null),
        Some(StructuredValue::String("".into())),
        Some(StructuredValue::Array(vec![])),
        Some(StructuredValue::Object(Default::default())),
    ] {
        let part = tool(output.clone());
        let wire = serde_json::to_value(&part).unwrap();
        match &output {
            None => assert!(!wire.as_object().unwrap().contains_key("output")),
            Some(value) => assert_eq!(wire["output"], serde_json::to_value(value).unwrap()),
        }
        assert_eq!(wire["summary"], "");
        assert_eq!(wire["output_preview"], "独立预览");
        assert_eq!(wire["finished_at_ms"].as_u64(), Some(u64::MAX));
        assert_eq!(
            serde_json::from_value::<MessagePartView>(wire).unwrap(),
            part
        );
        part.validate_public_semantics(&MessageId("message".into()))
            .unwrap();
    }
}

#[test]
fn permission_display_and_output_reference_cannot_import_execution_authority() {
    let part = tool(None);
    let wire = serde_json::to_value(&part).unwrap();
    for field in ["raw_input", "full_output_ref", "lease", "grant"] {
        let mut foreign = wire.clone();
        foreign[field] = serde_json::json!("private runtime material");
        assert!(
            serde_json::from_value::<MessagePartView>(foreign).is_err(),
            "{field}"
        );
    }
    for field in ["path", "token", "lease", "grant"] {
        let mut foreign = wire.clone();
        foreign["full_output"][field] = serde_json::json!("private runtime material");
        assert!(
            serde_json::from_value::<MessagePartView>(foreign).is_err(),
            "{field}"
        );
        let mut foreign = wire.clone();
        foreign["permission"][field] = serde_json::json!("private runtime material");
        assert!(
            serde_json::from_value::<MessagePartView>(foreign).is_err(),
            "{field}"
        );
    }
    for field in ["message_id", "part_id", "tool_call_id"] {
        let mut foreign = wire.clone();
        foreign["full_output"][field] = serde_json::json!("foreign owner");
        let foreign: MessagePartView = serde_json::from_value(foreign).unwrap();
        assert_eq!(
            foreign.validate_public_semantics(&MessageId("message".into())),
            Err(crate::PublicPartContractError::ToolOutputOwnerMismatch)
        );
    }
}

#[test]
fn review_status_is_preserved_without_revealing_withheld_text() {
    for review in [
        None,
        Some(TextReviewState::Approved),
        Some(TextReviewState::Pending),
        Some(TextReviewState::Rejected),
        Some(TextReviewState::Interrupted),
    ] {
        let mut part = MessagePartView::Text {
            part_id: MessagePartId("text".into()),
            revision: 3,
            markdown: String::new(),
            is_final: true,
            review,
        };
        let wire = serde_json::to_value(&part).unwrap();
        assert_eq!(
            serde_json::from_value::<MessagePartView>(wire).unwrap(),
            part
        );
        assert!(part
            .validate_public_semantics(&MessageId("message".into()))
            .is_ok());
        if let MessagePartView::Text { markdown, .. } = &mut part {
            *markdown = "正文".into();
        }
        assert_eq!(
            part.validate_public_semantics(&MessageId("message".into()))
                .is_ok(),
            review.is_none() || review == Some(TextReviewState::Approved)
        );
    }
}

#[test]
fn source_evidence_retains_all_redacted_facts_but_rejects_memory_body() {
    let evidence = SourceEvidence::Memory {
        usage: MemoryUsage {
            record_id: "record".into(),
            component_id: "component".into(),
            scope: Some(MemoryScope::Conversation),
            namespace: Some(MemoryNamespace::Custom("汉🙂空间".into())),
            provenance_kind: Some(MemoryProvenanceKind::ToolResult),
            source: Some("file:///source".into()),
            title: Some("".into()),
            observed_at: Some("observed".into()),
            source_role: Some(MemorySourceRole::Tool),
            event_time: Some("event".into()),
            range_start: Some("start".into()),
            range_end: Some("end".into()),
            requires_current_state_verification: true,
            truncated: true,
            supersedes: vec!["older".into()],
            usage_count: u64::MAX,
            last_used_at_epoch_ms: Some(i64::MIN),
        },
    };
    let wire = serde_json::to_value(&evidence).unwrap();
    assert_eq!(
        serde_json::from_value::<SourceEvidence>(wire.clone()).unwrap(),
        evidence
    );
    for field in ["content", "summary", "credential", "grant"] {
        let mut invalid = wire.clone();
        invalid["usage"][field] = serde_json::json!("private content");
        assert!(
            serde_json::from_value::<SourceEvidence>(invalid).is_err(),
            "{field}"
        );
    }
}

#[test]
fn memory_namespace_variants_reject_undeclared_private_fields() {
    for namespace in [
        MemoryNamespace::Personal,
        MemoryNamespace::Workspace,
        MemoryNamespace::Conversation,
        MemoryNamespace::Custom("汉🙂".into()),
    ] {
        let wire = serde_json::to_value(&namespace).unwrap();
        assert_eq!(
            serde_json::from_value::<MemoryNamespace>(wire.clone()).unwrap(),
            namespace
        );
        for field in ["content", "credential", "grant"] {
            let mut invalid = wire.clone();
            invalid[field] = serde_json::json!("private");
            assert!(serde_json::from_value::<MemoryNamespace>(invalid).is_err());
        }
        if namespace != MemoryNamespace::Custom("汉🙂".into()) {
            let mut invalid = wire;
            invalid["name"] = serde_json::Value::Null;
            assert!(serde_json::from_value::<MemoryNamespace>(invalid).is_err());
        }
    }
}

#[test]
fn citation_identity_mime_section_and_absent_or_empty_location_remain_distinct() {
    for location in [
        None,
        Some(SourceLocation::default()),
        Some(SourceLocation {
            page: Some(u64::MAX),
            line_start: Some(0),
            line_end: None,
            section: Some("节 汉🙂".repeat(2000)),
        }),
    ] {
        let part = MessagePartView::Citation {
            part_id: MessagePartId("citation-part".into()),
            revision: 9,
            citation_id: "citation-identity-not-part-id".into(),
            source_id: "source".into(),
            title: "title".into(),
            uri: "file:///source".into(),
            mime_type: Some("".into()),
            origin_tool_call_id: "call".into(),
            page: location.as_ref().and_then(|value| value.page),
            line_start: location.as_ref().and_then(|value| value.line_start),
            line_end: location.as_ref().and_then(|value| value.line_end),
            section: location.as_ref().and_then(|value| value.section.clone()),
            location,
            range_start: Some(0),
            range_end: Some(u64::MAX),
        };
        let wire = serde_json::to_value(&part).unwrap();
        assert_eq!(
            serde_json::from_value::<MessagePartView>(wire.clone()).unwrap(),
            part
        );
        assert!(part
            .validate_public_semantics(&MessageId("message".into()))
            .is_ok());
        let mut foreign = wire;
        foreign["section"] = serde_json::json!("different");
        let foreign: MessagePartView = serde_json::from_value(foreign).unwrap();
        assert_eq!(
            foreign.validate_public_semantics(&MessageId("message".into())),
            Err(crate::PublicPartContractError::CitationLocationMismatch)
        );
    }
}
