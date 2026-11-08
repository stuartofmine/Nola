// SPDX-License-Identifier: Apache-2.0
//! Diagnostic previews must not mutate or retain the original cumulative body.
use super::*;
use nola_agent_engine::agent::parts::{PartUpdate, TextPart};

#[test]
fn event_preview_unicode_edges_and_budget_boundary_are_stable() {
    assert_eq!(bounded_event_text("中🙂ab", 4), "中🙂ab");
    assert_eq!(bounded_event_text("", 0), "");
    assert_eq!(
        bounded_event_text("中🙂abcdef终😀", 4),
        "中🙂\n...[event snapshot omitted]...\n终😀"
    );
    assert_eq!(
        bounded_event_text("x", 0),
        "\n...[event snapshot omitted]...\n"
    );
}

#[test]
fn large_created_and_replaced_events_only_serialize_bounded_previews() {
    let body = format!("开头🙂{}末尾😀", "正文".repeat(1_400_000));
    let part = AgentPart::Text(TextPart {
        id: "p".into(),
        ownership: PartOwnership::for_turn("t"),
        review: None,
        text: body.clone(),
        is_complete: true,
    });
    for update in [
        PartUpdate::Created { part: part.clone() },
        PartUpdate::TextReplaced {
            part_id: "p".into(),
            text: body.clone(),
            is_complete: true,
        },
    ] {
        let event = AgentEvent::PartUpdated {
            turn_id: nola_agent_engine::agent::TurnId::from_string("t".into()),
            projection_revision: 17,
            update,
            part: part.clone(),
        };
        let payload = event_payload_for_persistence(&event).unwrap();
        let preview = payload.pointer("/part/text").unwrap().as_str().unwrap();
        assert!(preview.starts_with("开头🙂"));
        assert!(preview.ends_with("末尾😀"));
        assert_eq!(payload["projection_revision"], 17);
        assert_eq!(payload["part"]["is_complete"], true);
        assert!(serde_json::to_vec(&payload).unwrap().len() < 20_000);
        if payload["update"]["type"] == "created" {
            assert_eq!(payload["update"]["snapshot_in"], "part");
            assert!(payload["update"].get("part").is_none());
        } else {
            assert_eq!(payload["update"]["text"].as_str().unwrap(), preview);
        }
        assert_eq!(payload["diagnostic_schema"], "nola.part_event.v1");
        assert!(
            serde_json::to_vec(&payload).unwrap().len() < event_diagnostics::MAX_PART_EVENT_BYTES
        );
        let AgentEvent::PartUpdated {
            part: AgentPart::Text(original),
            ..
        } = &event
        else {
            unreachable!()
        };
        assert_eq!(original.text, body);
    }
}

#[test]
fn aggregate_tool_event_budget_includes_unicode_escaping_and_metadata() {
    use nola_agent_engine::agent::parts::{ToolPart, ToolPartState};
    let large = "\u{0}\n\\\"中🙂".repeat(40_000);
    let part = AgentPart::Tool(ToolPart {
        execution_source: Default::default(),
        id: "part".into(),
        ownership: PartOwnership::for_turn("turn"),
        call_id: "call".into(),
        capability_id: "tool".into(),
        title: large.clone(),
        summary: Some(large.clone()),
        origin: Default::default(),
        state: ToolPartState::Completed,
        input: serde_json::json!({"nested":large}),
        raw_input: large.clone(),
        permission: None,
        output_preview: Some(large.clone()),
        structured_output: Some(serde_json::json!({"result":large})),
        full_output_ref: Some("/tmp/owned-output".into()),
        artifacts: vec![],
        started_at_ms: Some(10),
        finished_at_ms: Some(20),
    });
    for update in [
        PartUpdate::Created { part: part.clone() },
        PartUpdate::ToolState {
            part_id: "part".into(),
            state: ToolPartState::Completed,
            title: Some(large.clone()),
            summary: Some(large.clone()),
            output_preview: Some(large.clone()),
            structured_output: Some(serde_json::json!({"result":large})),
            full_output_ref: Some("/tmp/owned-output".into()),
            artifacts: vec![],
            timestamp_ms: Some(20),
        },
        PartUpdate::ToolInputReplaced {
            part_id: "part".into(),
            input: serde_json::json!({"nested":large}),
            raw_input: large.clone(),
            state: ToolPartState::Completed,
        },
    ] {
        let event = AgentEvent::PartUpdated {
            turn_id: nola_agent_engine::agent::TurnId::from_string("turn".into()),
            projection_revision: 23,
            update,
            part: part.clone(),
        };
        let payload = event_payload_for_persistence(&event).unwrap();
        let encoded = serde_json::to_vec(&payload).unwrap();
        assert!(
            encoded.len() < event_diagnostics::MAX_PART_EVENT_BYTES,
            "{} encoded bytes",
            encoded.len()
        );
        assert_eq!(payload["part"]["full_output_ref"], "/tmp/owned-output");
        assert_eq!(payload["projection_revision"], 23);
        assert!(payload["part"].get("raw_input").is_none());
        let AgentEvent::PartUpdated {
            part: AgentPart::Tool(original),
            ..
        } = event
        else {
            unreachable!()
        };
        assert_eq!(original.input["nested"], large);
        assert_eq!(original.structured_output.unwrap()["result"], large);
        assert_eq!(original.output_preview.unwrap(), large);
        assert_eq!(original.summary.unwrap(), large);
    }
}

#[test]
fn tool_diagnostic_json_prefix_is_bounded_without_changing_canonical_input_or_null() {
    let input =
        serde_json::json!({"body":"输入🙂".repeat(1_000_000),"late":"must remain in original"});
    let preview = bounded_event_json(&input);
    assert_eq!(preview["truncated"], true);
    assert!(preview["input_preview"].as_str().unwrap().len() <= 4096);
    assert!(preview["input_preview"]
        .as_str()
        .unwrap()
        .starts_with("{\"body\":\""));
    assert_eq!(input["late"], "must remain in original");
    for value in [
        Value::Null,
        serde_json::json!(""),
        serde_json::json!([]),
        serde_json::json!({}),
        serde_json::json!({"small":true}),
    ] {
        assert_eq!(bounded_event_json(&value), value);
    }
}

#[test]
fn actual_source_citation_artifact_and_tool_metadata_events_share_the_bounded_encoder() {
    let huge = "unescaped metadata 中🙂".repeat(40_000);
    let mut values = vec![
        serde_json::json!({"kind":"source", "source_id":"source", "title":huge, "uri":huge,
            "evidence":{"kind":"memory", "usage":{"record_id":"record", "component_id":"component", "source":huge, "title":huge, "supersedes":[huge]}}}),
        serde_json::json!({"kind":"citation", "citation_id":"citation", "source_id":"source", "title":huge, "uri":huge,
            "origin_tool_call_id":"call", "location":{"section":huge}}),
        serde_json::json!({"kind":"artifact", "artifact":{"uri":huge, "name":huge, "mime_type":huge, "operation":huge}}),
        serde_json::json!({"kind":"tool", "call_id":"call", "capability_id":"files", "title":"Title", "state":"running", "input":{}, "structured_output":null,
            "execution_source":{"kind":"model"},
            "origin":{"kind":"mcp", "provider_id":huge, "provider_title":huge, "namespace":huge},
            "permission":{"request_id":"request", "permission_domain":"files", "rationale":huge, "decision":null},
            "artifacts":[{"uri":huge, "name":huge}]}),
    ];
    for value in &mut values {
        value["id"] = serde_json::json!("part");
        value["message_id"] = serde_json::json!("assistant");
        value["turn_id"] = serde_json::json!("turn");
        let part: AgentPart = serde_json::from_value(value.clone()).unwrap();
        let mut updates = vec![PartUpdate::Created { part: part.clone() }];
        if matches!(&part, AgentPart::Tool(_)) {
            updates.push(PartUpdate::ToolProgress {
                part_id: "part".into(),
                title: Some(huge.clone()),
                detail: Some(huge.clone()),
                output_delta: Some(huge.clone()),
            });
        }
        for update in updates {
            let event = AgentEvent::PartUpdated {
                turn_id: nola_agent_engine::agent::TurnId::from_string("turn".into()),
                projection_revision: 31,
                update,
                part: part.clone(),
            };
            let payload = event_payload_for_persistence(&event).unwrap();
            assert!(
                serde_json::to_vec(&payload).unwrap().len()
                    < event_diagnostics::MAX_PART_EVENT_BYTES
            );
            let diagnostic = &payload["part"];
            let field = match &part {
                AgentPart::Source(_) | AgentPart::Citation(_) => "uri",
                AgentPart::Artifact(_) => "artifact",
                AgentPart::Tool(_) => "origin",
                _ => unreachable!(),
            };
            assert_eq!(diagnostic[field]["truncated"], true);
            if matches!(&part, AgentPart::Tool(_)) {
                assert!(diagnostic
                    .as_object()
                    .unwrap()
                    .contains_key("structured_output"));
                assert_eq!(diagnostic["structured_output"], Value::Null);
                if payload["update"].get("type").is_none() {
                    assert_eq!(payload["update"]["truncated"], true);
                }
            }
            let AgentEvent::PartUpdated { part: original, .. } = event else {
                unreachable!()
            };
            assert_eq!(original, part);
        }
    }
}
