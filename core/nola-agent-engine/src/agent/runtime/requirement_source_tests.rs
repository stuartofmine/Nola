// SPDX-License-Identifier: Apache-2.0
//! Request-time source mapping: admitted identity, bounded text, no new authority.

use super::*;
use crate::agent::protocol::AgentInputOrigin;

fn metadata(runtime: &AgentRuntime, turn: &TurnId) -> Value {
    let Some(AgentItem::Message { content, .. }) = runtime.requirement_context(turn).unwrap() else {
        panic!("missing requirement context");
    };
    serde_json::from_str(content.split_once("<requirements_state>").unwrap().1
        .strip_suffix("</requirements_state>").unwrap()).unwrap()
}

fn add_items(runtime: &mut AgentRuntime, turn: &TurnId, admission: &str,
    origin: AgentInputOrigin, items: Vec<AgentItem>, new_scope: bool) {
    let batch = runtime.session.execution.input_queue.admit(admission.into(), None,
        origin, AgentInputDelivery::CurrentTurn, None, "a".repeat(64), items.clone()).unwrap();
    let applied = runtime.session.execution.input_queue.apply(&batch, turn.as_str()).unwrap();
    runtime.session.record_turn_items(turn, items.clone());
    runtime.apply_requirement_inputs(turn, &[applied], &items, new_scope, &mut vec![]).unwrap();
}

#[test]
fn requirement_refs_resolve_to_exact_multilingual_input_not_abstract_description() {
    let (mut runtime, turn) = super::tests::fixture();
    for (number, text) in ["请用自然语言解释这个结果。", "Réponds uniquement au format demandé.",
        "その説明を短くしてください。", "Please continue with the same answer format."]
        .into_iter().enumerate() {
        let id = format!("follow-up-{number}");
        add_items(&mut runtime, &turn, &format!("admission-{number}"),
            AgentInputOrigin::InteractiveUser,
            vec![AgentItem::user_message(ItemId::from_string(id.clone()), text)], false);
        let context = metadata(&runtime, &turn);
        let requirement = context["requirements"].as_array().unwrap().iter()
            .find(|requirement| requirement["source_refs"] == json!([id])).unwrap();
        assert_eq!(requirement["description"], "Deliver the accepted input scope referenced by this requirement.");
        let source = context["source_inputs"]["items"].as_array().unwrap().iter()
            .find(|source| source["ref"] == id).unwrap();
        assert_eq!(source["role"], "user");
        assert_eq!(source["origin"], "interactive_user");
        assert_eq!(source["turn_id"], turn.as_str());
        assert_eq!(source["text_excerpt"], text);
        assert_eq!(source["truncated"], false);
    }
    assert!(runtime.session.execution.requirements.assessments.is_empty());
    assert!(runtime.session.execution.requirements.proposed.is_none());
    assert!(runtime.session.host.pending_effects.iter().next().is_none());
}

#[test]
fn prior_receipts_survive_new_scope_and_restore_without_current_evidence_authority() {
    use nola_domain::execution_evidence::*;
    let (mut runtime, old_turn) = super::tests::fixture();
    let observation = ExecutionEvidenceObservation {
        evidence_id:"earlier-observation".into(),call_id:"earlier-call".into(),tool_name:"read_file".into(),
        source:ExecutionEvidenceSource { effect_id:"effect".into(),request_id:"request".into(),host_id:"host".into(),
            attempt_id:None,response_sequence:JournalNumber(1),catalog_revision:JournalNumber(1),availability_epoch:JournalNumber(0) },
        outcome:nola_domain::host_effect::HostToolOutcome::Succeeded,
        receipt:Some(ExecutionReceipt {local_stop:None,state:ExecutionState::Completed,effects:vec![],checks:vec![]}),
    };
    runtime.session.execution.requirements.observations.insert(observation.evidence_id.clone(),
        ObservedEvidenceReference::from((&observation, old_turn.as_str(), 1)));
    let turn = TurnId::from_string("follow-up".into());
    add_items(&mut runtime, &turn, "follow-up-admission", AgentInputOrigin::InteractiveUser,
        vec![AgentItem::user_message(ItemId::from_string("follow-up-input".into()), "Continue with the requested work")], true);
    runtime.session.conversation.model_context.clear();
    runtime.session.execution.requirements = serde_json::from_value(
        serde_json::to_value(&runtime.session.execution.requirements).unwrap()).unwrap();
    let binding = runtime.requirement_binding().unwrap().unwrap();
    assert!(binding.evidence.is_empty());
    assert_eq!(binding.sources.len(), 1);
    let context = metadata(&runtime, &turn);
    assert_eq!(context["historical_evidence"]["items"][0]["turn_id"], old_turn.as_str());
    assert_eq!(context["historical_evidence"]["items"][0]["call_id"], "earlier-call");
    assert_eq!(context["requirements"][0]["resource_scope"]["kind"], "undeclared");
    assert!(runtime.session.execution.requirements.assessments.is_empty());
}

#[test]
fn new_scope_excludes_old_run_and_preserves_developer_trigger_role() {
    let (mut runtime, old_turn) = super::tests::fixture();
    let turn = TurnId::from_string("new-run".into());
    add_items(&mut runtime, &turn, "continuation", AgentInputOrigin::GoalContinuation,
        vec![AgentItem::Message { id: ItemId::from_string("trigger".into()),
            role: AgentRole::Developer, content: "Continue the admitted Goal context.".into() }], true);
    let context = metadata(&runtime, &turn);
    assert_eq!(context["requirements"][0]["source_refs"], json!(["trigger"]));
    assert_eq!(context["source_inputs"]["source_count"], 1);
    let source = &context["source_inputs"]["items"][0];
    assert_eq!(source["role"], "developer");
    assert_eq!(source["origin"], "goal_continuation");
    assert_eq!(source["turn_id"], turn.as_str());
    assert_ne!(source["turn_id"], old_turn.as_str());
    assert!(!context.to_string().contains("Explain the result"));

    // Even an inconsistent foreign reference cannot borrow a transcript body.
    runtime.session.execution.requirements.scope.as_mut().unwrap().requirements[0]
        .source_refs = vec!["input".into()];
    let foreign = metadata(&runtime, &turn);
    assert_eq!(foreign["source_inputs"]["items"][0]["availability"], "unavailable");
    assert_eq!(foreign["source_inputs"]["items"][0]["reason"], "not_in_current_scope");
    assert!(!foreign.to_string().contains("Explain the result"));
}

#[test]
fn tool_result_and_attachment_sources_never_expose_output_or_locator_as_task_text() {
    let (mut runtime, turn) = super::tests::fixture();
    add_items(&mut runtime, &turn, "non-message", AgentInputOrigin::SystemRecovery, vec![
        AgentItem::ToolResult { id: ItemId::from_string("tool-source".into()),
            call_id: CallId::from_string("call".into()), name: "read_file".into(),
            output: json!({"text":"UNTRUSTED_TOOL_BODY"}), is_error: false,
            full_output_ref: Some("UNTRUSTED_OUTPUT_REF".into()), artifacts: vec![] },
        AgentItem::Attachment { id: ItemId::from_string("attachment-source".into()),
            message_id: ItemId::from_string("message".into()),
            uri: "file:///UNTRUSTED_ATTACHMENT_LOCATOR".into(), name: "data".into(),
            mime_type: None, byte_count: 1 },
    ], false);
    let context = metadata(&runtime, &turn);
    for (id, kind) in [("tool-source", "tool_result"), ("attachment-source", "attachment")] {
        let source = context["source_inputs"]["items"].as_array().unwrap().iter()
            .find(|source| source["ref"] == id).unwrap();
        assert_eq!(source["kind"], kind);
        assert_eq!(source["origin"], "system_recovery");
        assert_eq!(source["availability"], "non_message");
        assert!(source.get("text_excerpt").is_none());
        assert!(source.get("role").is_none());
    }
    for forbidden in ["UNTRUSTED_TOOL_BODY", "UNTRUSTED_OUTPUT_REF", "UNTRUSTED_ATTACHMENT_LOCATOR"] {
        assert!(!context.to_string().contains(forbidden));
    }
}

#[test]
fn missing_foreign_or_ambiguous_transcript_identity_is_explicitly_unavailable() {
    let (mut runtime, turn) = super::tests::fixture();
    let original = runtime.session.conversation.transcript.clone();
    runtime.session.conversation.transcript.clear();
    assert_eq!(metadata(&runtime, &turn)["source_inputs"]["items"][0]["reason"], "missing_transcript_item");
    runtime.session.conversation.transcript = original.clone();
    runtime.session.conversation.transcript.extend(original);
    assert_eq!(metadata(&runtime, &turn)["source_inputs"]["items"][0]["reason"], "ambiguous_transcript_item");
    runtime.session.conversation.transcript.truncate(1);
    runtime.session.item_ownership.get_mut("input").unwrap().turn_id = "foreign-run".into();
    let context = metadata(&runtime, &turn);
    assert_eq!(context["source_inputs"]["items"][0]["reason"], "missing_or_foreign_turn_owner");
    assert!(!context.to_string().contains("Explain the result"));
}

#[test]
fn shared_source_index_deduplicates_and_enforces_global_unicode_budget() {
    let (mut runtime, turn) = super::tests::fixture();
    // Each body is longer than its excerpt; all rN still carry exact references.
    let items = (0..20).map(|number| AgentItem::user_message(
        ItemId::from_string(format!("long-source-{number}")), "界".repeat(2000))).collect();
    add_items(&mut runtime, &turn, "large-input", AgentInputOrigin::InteractiveUser, items, true);
    let scope = runtime.session.execution.requirements.scope.as_mut().unwrap();
    let mut same_sources = scope.requirements[0].clone();
    same_sources.requirement_id = "second-requirement".into();
    scope.requirements.push(same_sources);
    let context = metadata(&runtime, &turn);
    assert_eq!(context["requirements"][0]["source_refs"].as_array().unwrap().len(), 20);
    assert_eq!(context["requirements"][1]["source_refs"], context["requirements"][0]["source_refs"]);
    let index = &context["source_inputs"];
    assert_eq!(index["source_count"], 20);
    assert_eq!(index["items_omitted"], 4);
    let sources = index["items"].as_array().unwrap();
    assert_eq!(sources.len(), MODEL_SOURCE_ITEMS);
    let text_chars = sources.iter().map(|source| source["text_excerpt"].as_str().unwrap()
        .chars().count()).sum::<usize>();
    assert_eq!(text_chars, MODEL_SOURCE_TOTAL_CHARS);
    assert_eq!(index["text_chars_projected"], text_chars);
    assert!(sources.iter().all(|source| source["truncated"] == true && source["text_chars"] == 2000));
    assert_eq!(sources[0]["text_excerpt"].as_str().unwrap().chars().count(), MODEL_SOURCE_ITEM_CHARS);
    assert_eq!(sources.last().unwrap()["availability"], "omitted");
    // Source remains lossless; budgeting changes neither state nor references.
    assert_eq!(runtime.session.conversation.transcript.last().unwrap(),
        &AgentItem::user_message(ItemId::from_string("long-source-19".into()), "界".repeat(2000)));
}

#[test]
fn cold_checkpoint_rebuilds_same_mapping_even_when_source_is_not_in_model_context() {
    let (mut runtime, turn) = super::tests::fixture();
    runtime.session.conversation.model_context.clear();
    let before = metadata(&runtime, &turn);
    let checkpoint = runtime.checkpoint(CheckpointReason::Manual);
    let bytes = serde_json::to_vec(&checkpoint).unwrap();
    let decoded = serde_json::from_slice(&bytes).unwrap();
    let restored = AgentRuntime::restore(runtime.config.clone(), decoded).unwrap();
    assert_eq!(metadata(&restored, &turn), before);
    assert_eq!(before["source_inputs"]["items"][0]["text_excerpt"], "Explain the result");
}

#[test]
fn answered_schema_and_control_accept_text_deliverables_without_execution_evidence() {
    let description = tool_schema()["function"]["description"].as_str().unwrap().to_owned();
    assert!(description.contains("Direct replies need no tool, including after execution"));
    assert!(description.contains("completed requires a nonempty resource scope"));
    assert!(description.contains("An undeclared scope is unknown, not covered"));
    for answer in ["Une explication courte.", "指定形式の回答", "A natural continuation."] {
        let (mut runtime, turn) = super::tests::fixture();
        let binding = runtime.requirement_binding().unwrap().unwrap();
        let bound = nola_domain::requirement_control::bind_model_task_result(
            &json!({"outcome":"answered", "answer":answer}), &binding).unwrap();
        let (_, delivered) = runtime.apply_requirement_control(&turn,
            &CallId::from_string("text-answer".into()), &bound, &mut vec![]).unwrap();
        assert_eq!(delivered.as_deref(), Some(answer));
        assert!(matches!(runtime.session.execution.requirements.proposed, Some(RunResultValue::Answered { .. })));
        assert!(runtime.session.execution.requirements.observations.is_empty());
        assert!(runtime.session.host.pending_effects.iter().next().is_none());
    }
}

#[test]
fn source_projection_does_not_overwrite_model_negative_assessment_or_incomplete_result() {
    let (mut runtime, turn) = super::tests::fixture();
    let binding = runtime.requirement_binding().unwrap().unwrap();
    let bound = nola_domain::requirement_control::bind_model_task_result(&json!({
        "outcome":"incomplete", "answer":"The requested scope remains unassessed.",
        "assessments":[{"requirement":"r1", "verdict":"insufficient_evidence",
            "rationale":"Model judgment remains negative.", "evidence":[]}]
    }), &binding).unwrap();
    runtime.apply_requirement_control(&turn, &CallId::from_string("negative".into()),
        &bound, &mut vec![]).unwrap();
    let before = serde_json::to_value(&runtime.session.execution.requirements).unwrap();
    let context = metadata(&runtime, &turn);
    assert_eq!(context["requirements"][0]["verdict"], "insufficient_evidence");
    assert_eq!(context["source_inputs"]["items"][0]["text_excerpt"], "Explain the result");
    assert!(matches!(runtime.session.execution.requirements.proposed, Some(RunResultValue::Incomplete { .. })));
    assert_eq!(serde_json::to_value(&runtime.session.execution.requirements).unwrap(), before);
}

#[test]
fn admitted_ui_action_retains_user_values_in_bounded_requirement_sources() {
    let (mut runtime, turn) = super::tests::fixture();
    let identity = json!({"owner":{"authority_id":"host","conversation_id":"conversation","participant_id":"agent",
        "context_id":"conversation","originating_run_id":"run","message_id":"assistant","part_id":"tool"},
        "surface_id":"form","generation":1});
    let action: nola_domain::generative_ui::UiActionContent = serde_json::from_value(json!({
        "surface":{"identity":identity,"revision":2},"summary":"Submitted form",
        "envelope":{"type":"ui_action","surface":identity,"message":{"version":"v0.9.1",
            "action":{"name":"submit","sourceComponentId":"submit","surfaceId":"form",
                "timestamp":"2026-10-25T00:00:00Z","context":{"name":"ORCHID 用户"}}}}
    })).unwrap();
    let envelope = action.model_envelope().to_string();
    let mut large_action = action.clone();
    large_action.envelope["message"]["action"]["context"]["name"] = json!("字".repeat(8000));
    add_items(&mut runtime, &turn, "ui-admission", AgentInputOrigin::InteractiveUser,
        vec![AgentItem::UiAction { id: ItemId::from_string("form-submit".into()), action }], true);
    let context = metadata(&runtime, &turn);
    let source = &context["source_inputs"]["items"][0];
    assert_eq!(source["kind"], "ui_action");
    assert_eq!(source["role"], "user");
    assert_eq!(source["origin"], "interactive_user");
    assert_eq!(source["availability"], "available");
    assert_eq!(source["text_excerpt"], envelope);
    let projected: serde_json::Value = serde_json::from_str(source["text_excerpt"].as_str().unwrap()).unwrap();
    assert_eq!(projected["surface_revision"], 2);
    assert_eq!(projected["message"]["version"], "v0.9.1");
    assert_eq!(source["truncated"], false);
    add_items(&mut runtime, &turn, "large-ui-admission", AgentInputOrigin::InteractiveUser,
        vec![AgentItem::UiAction { id: ItemId::from_string("large-form-submit".into()), action: large_action }], true);
    let bounded = metadata(&runtime, &turn);
    let excerpt = &bounded["source_inputs"]["items"][0];
    assert_eq!(excerpt["truncated"], true);
    assert!(excerpt["text_excerpt"].as_str().unwrap().chars().count() <= 4096);
    assert!(runtime.session.execution.requirements.assessments.is_empty());
}
