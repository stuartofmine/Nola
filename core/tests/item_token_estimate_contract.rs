// SPDX-License-Identifier: Apache-2.0
use nola_core::agent::{AgentItem, AgentRole, CallId, ItemId};
use nola_domain::generative_ui::{UiActionContent, UiSurfaceIdentity, UiSurfaceOwner, UiSurfaceRevisionRef};
use serde_json::json;

#[test]
fn real_agent_item_variants_match_original_character_accounting() {
    let id = || ItemId::from_string("id".into());
    let call_id = || CallId::from_string("call".into());
    let surface = UiSurfaceIdentity {
        owner: UiSurfaceOwner { authority_id:"host".into(), conversation_id:"chat".into(),
            participant_id:"assistant".into(), context_id:"chat".into(), originating_run_id:"run".into(),
            message_id:"message".into(), part_id:"part".into() },
        surface_id:"form".into(), generation:1,
    };
    let action = UiActionContent {
        surface:UiSurfaceRevisionRef { identity:surface.clone(), revision:1 },
        summary:"提交表单 🦀".into(),
        envelope:json!({"type":"ui_action","surface":surface,"message":{"version":"v0.9.1",
            "action":{"surfaceId":"form","sourceComponentId":"submit","name":"save",
                "timestamp":"2026-10-23T00:00:00Z","context":{"name":"用户 🦀","quoted":"\"\\\n"}}}}),
    };
    action.validate().unwrap();
    let items = vec![
        AgentItem::UiAction { id:id(), action },
        AgentItem::Message {
            id: id(),
            role: AgentRole::User,
            content: "用户 🦀\n\t".into(),
        },
        AgentItem::Attachment {
            id: id(),
            message_id: id(),
            uri: "file:///文档".into(),
            name: "报告🦀".into(),
            mime_type: Some("text/plain".into()),
            byte_count: u64::MAX,
        },
        AgentItem::Attachment {
            id: id(),
            message_id: id(),
            uri: "file:///a".into(),
            name: "a".into(),
            mime_type: None,
            byte_count: 0,
        },
        AgentItem::Reasoning {
            id: id(),
            summary: "摘要".into(),
            content: "原文é".into(),
            provider_continuation: None,
        },
        AgentItem::ToolCall {
            id: id(),
            call_id: call_id(),
            name: "工具".into(),
            arguments: json!({"引号":"\"\\\0\n🦀","nested":[[1,2],{"value":false}]}),
        },
        AgentItem::ToolResult {
            id: id(),
            call_id: call_id(),
            name: "工具".into(),
            output: json!([null,-123.5,"é",{"文":"\u{1f}"}]),
            is_error: true,
            full_output_ref: Some("ignored metadata".repeat(1000)),
            artifacts: Vec::new(),
        },
        AgentItem::Compaction {
            id: id(),
            summary: "摘要\nquoted: \\\"".into(),
            replaced_item_count: usize::MAX,
            source_item_ids: vec![],
            source_fingerprint: "fixture".into(),
            original_token_count: usize::MAX,
        },
    ];
    for item in items {
        // The independent character-count oracle materializes JSON only in this test.
        let chars = match &item {
            AgentItem::UiAction { action, .. } => "ui_action".chars().count() + action.envelope.to_string().chars().count(),
            AgentItem::Message { content, .. } => content.chars().count(),
            AgentItem::Attachment {
                uri,
                name,
                mime_type,
                ..
            } => {
                uri.chars().count()
                    + name.chars().count()
                    + mime_type.as_deref().unwrap_or_default().chars().count()
            }
            AgentItem::Reasoning {
                summary, content, ..
            } => summary.chars().count() + content.chars().count(),
            AgentItem::ToolCall {
                name, arguments, ..
            } => name.chars().count() + arguments.to_string().chars().count(),
            AgentItem::ToolResult { name, output, .. } => {
                name.chars().count() + output.to_string().chars().count()
            }
            AgentItem::Compaction { summary, .. } => summary.chars().count(),
        };
        assert_eq!(item.estimated_tokens(), (chars + 3) / 4 + 4);
        let roundtrip: AgentItem =
            serde_json::from_slice(&serde_json::to_vec(&item).unwrap()).unwrap();
        assert_eq!(roundtrip.estimated_tokens(), item.estimated_tokens());
    }
}
