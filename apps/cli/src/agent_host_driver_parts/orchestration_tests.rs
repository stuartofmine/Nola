// SPDX-License-Identifier: Apache-2.0
// Mechanical extraction: included in the original module to preserve private access.

#[cfg(test)]
#[test]
fn code_memory_initialization_is_an_execution_prerequisite_through_real_host_dispatch() {
    use effect_delivery_start_barrier_tests::{test_context, test_llm, fenced_request};
    use crate::code_intelligence_provider::{NolaCodeMemoryProvider, CodeIntelligenceProvider, CodeIntelligenceCall};
    use nola_domain::permission_resource_plan::PermissionResolutionRequest;
    use nola_core::code_intelligence::CodeIntelligenceOperation;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let other = temp.path().join("other");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&other).unwrap();
    for index in 0..21 {
        for directory in [&root, &other] {
            fs::write(directory.join(format!("file_{index}.rs")),
                format!("fn visible_symbol_{index}() {{}}\n")).unwrap();
        }
    }
    let launch_directory = root.join("scripts");
    fs::create_dir(&launch_directory).unwrap();
    let provider = std::sync::Arc::new(NolaCodeMemoryProvider::open(&temp.path().join("index.db")).unwrap());
    let mut driver = CliAgentHostDriver::new("delivery-host",
        test_llm("http://127.0.0.1:1".into()), test_context(&root))
        .with_code_intelligence_provider(provider.clone())
        .with_code_memory_workspace(launch_directory.clone());
    let search = &driver.catalog.capabilities["codebase_search"];
    assert_eq!(search.title, "Search indexed symbols");
    let schema = discovery_tool_schema(search);
    assert!(schema["function"]["description"].as_str().unwrap().contains("repository-scale"));
    assert_ne!(schema["function"]["description"], search.title);
    let mut number = 0;
    let mut execute = |driver: &mut CliAgentHostDriver, capability: &str, input: Value, authorized: bool| {
        number += 1;
        let contract = &driver.catalog.capabilities[capability].input_normalization.permission_contract;
        let grant = contract.resource_plan_for(&input).and_then(|plan| {
        let resolution = PermissionResolutionRequest {
            resolution_id: format!("resolution-{number}"), execution_target: "delivery-host".into(),
            invocation_digest: PermissionResolutionRequest::input_digest(&input),
            capability: plan.capability.clone(), operation: plan.operation.clone(),
            demands: plan.extract(&input).unwrap(),
        };
        let facts = crate::tools::resolve_permission_resources(&driver.tool_context, &resolution, &input).unwrap();
        authorized.then(|| json!({"permission_resolution":{"request":resolution,"facts":facts}}))
        });
        let call = format!("call-{number}");
        let mut request = fenced_request(driver, HostRequest::ExecuteTool {
            capability_id: capability.into(), call_id: call, input, grant,
            permission_instructions: vec![], permission_profile: nola_domain::PermissionProfile::Cautious,
        });
        request.effect_id.0 = format!("effect-{number}");
        request.request_id = format!("request-{number}");
        request.delivery.as_mut().unwrap().attempt_id = format!("attempt-{number}");
        let expected_navigation: Vec<_> = ["codebase_architecture", "codebase_search", "codebase_read_symbol", "codebase_trace", "codebase_text_search"]
            .into_iter().filter(|id| driver.configured_tool_allowlist.as_ref()
                .is_none_or(|allowed| allowed.contains(*id))).collect();
        let mut completed = None;
        driver.dispatch(request, &mut |response| {
            if let HostResponse::ToolCompleted { output, outcome, activation, .. } = response.payload {
                if capability == "codebase_index" {
                    if outcome == HostToolOutcome::Failed || expected_navigation.is_empty() {
                        assert!(activation.is_none(), "failed indexing must not expose a ready workflow");
                    } else {
                        let activation = activation.expect("successful index exposes navigation schemas");
                        assert_eq!(activation.source, "code_memory");
                        for id in &expected_navigation {
                            assert!(activation.capability_ids.iter().any(|value| value == *id));
                        }
                        assert_eq!(activation.capability_descriptors.len(), expected_navigation.len());
                        assert!(output.to_string().contains("navigation"));
                        assert!(output.to_string().contains("fixture"));
                    }
                }
                completed = Some((output, outcome));
            }
            Ok(())
        }).unwrap();
        completed.expect("Host must produce a concrete result")
    };
    let query = json!({"root":root,"query":"visible_symbol_0","mode":"definition"});
    let (blocked, outcome) = execute(&mut driver, "code_search", query.clone(), true);
    assert_eq!(outcome, HostToolOutcome::Failed);
    assert!(blocked.to_string().contains("code_memory_initialization_required"));
    assert!(blocked.to_string().contains("not_started"));
    let presentation: nola_host_protocol::tool_result::ToolResult = serde_json::from_value(blocked.clone()).unwrap();
    assert!(presentation.presentation.summary.contains("this search has not run"));
    assert!(!presentation.presentation.summary.contains('{'));
    assert!(provider.execute(CodeIntelligenceCall { operation:CodeIntelligenceOperation::Projects,
        arguments:json!({}),workspace_roots:vec![temp.path().into()] }, &|| false).value.as_array().unwrap().is_empty(),
        "blocked search must not silently index");

    let (_, denied) = execute(&mut driver, "codebase_index", json!({"repo_path":root,"project":"fixture"}), false);
    assert_eq!(denied, HostToolOutcome::Failed, "initialization must not bypass resource authorization");
    assert!(execute(&mut driver,"code_search",query.clone(),true).0.to_string()
        .contains("code_memory_initialization_required"));
    let (index, outcome) = execute(&mut driver, "codebase_index", json!({"repo_path":root,"project":"fixture"}), true);
    assert!(!index.to_string().contains("code_intelligence_failed"), "{index}");
    assert_ne!(outcome, HostToolOutcome::Failed);
    let (symbols, outcome) = execute(&mut driver, "codebase_search",
        json!({"project":"fixture","query":"visible_symbol_0","limit":10}), true);
    assert_ne!(outcome, HostToolOutcome::Failed, "{symbols}");
    assert!(symbols.to_string().contains("visible_symbol_0"));
    let (searched, outcome) = execute(&mut driver, "code_search", query, true);
    assert_ne!(outcome, HostToolOutcome::Failed, "{searched}");
    assert!(searched.to_string().contains("visible_symbol_0"));
    assert!(!searched.to_string().contains("code_memory_initialization_required"));
    #[cfg(unix)]
    {
        driver.begin_turn(Default::default(), HostTurnPolicy::default());
        let (reused, outcome) = execute(&mut driver, "code_search",
            json!({"root":root,"query":"visible_symbol_0","mode":"definition"}), true);
        assert_ne!(outcome, HostToolOutcome::Failed, "{reused}");
        fs::write(root.join("file_0.rs"), "fn changed_symbol() {}\n").unwrap();
        let (stale, outcome) = execute(&mut driver, "code_search",
            json!({"root":root,"query":"changed_symbol","mode":"definition"}), true);
        assert_eq!(outcome, HostToolOutcome::Failed);
        assert!(stale.to_string().contains("code_memory_initialization_required"),
            "same-turn writes must invalidate readiness without waiting for another turn");
    }
    let (blocked_other, _) = execute(&mut driver, "code_search",
        json!({"root":other,"query":"visible_symbol_0","mode":"definition"}), true);
    assert!(blocked_other.to_string().contains("code_memory_initialization_required"),
        "one initialized project cannot unlock another scope");
    driver.configured_tool_allowlist = Some(["codebase_index".to_string(), "code_search".to_string()].into_iter().collect());
    let (_, outcome) = execute(&mut driver, "codebase_index",
        json!({"repo_path":other,"project":"fixture_other"}), true);
    assert_ne!(outcome, HostToolOutcome::Failed);
    driver.configured_tool_allowlist = None;
    driver.code_intelligence_provider = None;
    let (unavailable, outcome) = execute(&mut driver, "code_search",
        json!({"root":other,"query":"current search 当前","mode":"text"}), true);
    assert_eq!(outcome, HostToolOutcome::Failed);
    assert!(unavailable.to_string().contains("code_memory_unavailable"));
    assert!(unavailable.to_string().contains("not_started"));
}

#[cfg(test)]
#[test]
fn model_usage_run_accumulator_preserves_missing_fields_across_physical_attempts() {
    let mut total = TokenUsage::default();
    CliAgentHostDriver::accumulate_token_usage(&mut total, &crate::llm::LlmUsage {
        input_tokens: Some(10), output_tokens: Some(20), reasoning_tokens: Some(8),
        total_tokens: Some(30), cached_input_tokens: Some(0), ..Default::default()
    });
    assert_eq!(total.total_tokens, Some(30));
    assert_eq!(total.cached_input_tokens, Some(0));
    assert_eq!(total.budget_token_lower_bound(), 30);
    CliAgentHostDriver::accumulate_token_usage(&mut total, &crate::llm::LlmUsage::default());
    assert_eq!(total.input_tokens, None);
    assert_eq!(total.total_tokens, None);
    assert_eq!(total.cached_input_tokens, None);
    assert_eq!(total.request_count, 2);
    assert_eq!(total.budget_token_lower_bound(), 30);
    let durable = serde_json::to_value(&total).unwrap();
    assert!(durable["total_tokens"].is_null());
    let restored: TokenUsage = serde_json::from_value(durable).unwrap();
    assert_eq!(restored.request_count, 2);
    assert_eq!(restored.budget_token_lower_bound(), 30);
}

#[cfg(test)]
#[test]
fn paused_single_delta_releases_at_its_deadline_without_a_second_callback() {
    for text in [
        "A readable ordinary answer that has a sentence boundary.",
        "Short paused phrase",
    ] {
        let mut gate = ModelTextPresentationGate::for_request(false, true);
        let start = std::time::Instant::now();
        assert!(gate.push_text_at(text.into(), start).is_empty());
        assert!(gate
            .poll_at(start + std::time::Duration::from_millis(99), "")
            .is_empty());
        assert_eq!(
            gate.poll_at(start + std::time::Duration::from_millis(100), "")
                .concat(),
            text
        );
        assert!(gate
            .poll_at(start + std::time::Duration::from_secs(1), "")
            .is_empty());
        assert!(gate.finish_answer().is_empty());
    }
}

#[cfg(test)]
#[test]
fn local_deadline_keeps_actual_tool_and_private_reasoning_hidden_not_xml_examples() {
    let start = std::time::Instant::now();
    let due = start + std::time::Duration::from_secs(1);
    let text = "A private provisional candidate that must not be published.";
    let mut deferred = ModelTextPresentationGate::for_request(true, true);
    assert!(deferred.push_text_at(text.into(), start).is_empty());
    assert!(deferred.poll_at(due, "").is_empty());
    let mut tool = ModelTextPresentationGate::for_request(false, true);
    tool.push_text_at(text.into(), start);
    tool.observe_tool_call();
    assert!(tool.poll_at(due, "").is_empty());
    let mut xml = ModelTextPresentationGate::for_request(false, true);
    xml.push_text_at("<bash command=\"fixture\">".into(), start);
    assert_eq!(xml.poll_at(due, "").concat(), "<bash command=\"fixture\">");
    let mut private_reasoning = ModelTextPresentationGate::for_request(false, true);
    private_reasoning.push_text_at(text.into(), start);
    assert!(private_reasoning
        .poll_at(due, &format!("{text} Private continuation"))
        .is_empty());
}

#[cfg(test)]
#[test]
fn partial_code_markers_stream_as_text_and_attempt_reset_discards_pending_text() {
    let start = std::time::Instant::now();
    let due = start + std::time::Duration::from_secs(1);
    for (prefix, suffix) in [
        ("<ba", "sh command=\"fixture\">"),
        ("Run: ``", "`bash"),
        ("Run: ```s", "hell"),
    ] {
        let mut gate = ModelTextPresentationGate::for_request(false, true);
        assert!(gate.push_text_at(prefix.into(), start).is_empty());
        let mut shown = gate.poll_at(due, "").concat();
        shown.push_str(&gate.push_text_at(suffix.into(), due).concat());
        shown.push_str(&gate.poll_at(due + std::time::Duration::from_secs(1), "").concat());
        shown.push_str(&gate.finish_answer().concat());
        assert_eq!(shown, format!("{prefix}{suffix}"));
        gate.reset_after_transport_retry();
        assert!(gate
            .poll_at(due + std::time::Duration::from_secs(2), "")
            .is_empty());
        assert!(gate
            .push_text_at("New ordinary answer.".into(), due)
            .is_empty());
        assert_eq!(
            gate.poll_at(due + std::time::Duration::from_millis(100), "")
                .concat(),
            "New ordinary answer."
        );
    }
}

#[cfg(test)]
#[test]
fn a_paused_answer_tail_flushes_once_and_attempt_reset_discards_its_deadline() {
    let start = std::time::Instant::now();
    let mut gate = ModelTextPresentationGate::for_request(false, false);
    assert_eq!(
        gate.push_text_at("This first sentence is complete.".into(), start)
            .concat(),
        "This first sentence is complete."
    );
    assert!(gate.push_text_at(" Short tail".into(), start).is_empty());
    assert_eq!(
        gate.poll_at(start + std::time::Duration::from_millis(100), "")
            .concat(),
        " Short tail"
    );
    assert!(gate.finish_answer().is_empty());
    gate.push_text_at(" abandoned tail".into(), start);
    gate.reset_after_transport_retry();
    assert!(gate
        .poll_at(start + std::time::Duration::from_secs(1), "")
        .is_empty());
}

#[cfg(test)]
#[test]
fn slow_tool_capable_model_releases_first_phrase_without_waiting_for_256_characters() {
    let mut gate = ModelTextPresentationGate::for_request(false, true);
    let start = std::time::Instant::now();
    let phrase = "臣亮言：先帝创业未半而中道崩殂，今天下三分，益州疲弊，此诚危急存亡之秋也。";
    let prefix = &phrase[..phrase.len() - '。'.len_utf8()];
    assert!(gate.push_text_at(prefix.to_string(), start).is_empty());
    let chunks = gate.push_text_at(
        "。".to_string(),
        start + std::time::Duration::from_millis(110),
    );
    assert_eq!(chunks.concat(), phrase);
    assert!(phrase.chars().count() < 128);
    assert!(gate.finish_answer().is_empty());
}

#[cfg(test)]
#[test]
fn expired_prefix_wait_does_not_publish_an_explicitly_deferred_tool_round() {
    let mut gate = ModelTextPresentationGate::for_request(true, true);
    let start = std::time::Instant::now();
    assert!(gate
        .push_text_at(
            "I am checking the requested information. ".to_string(),
            start
        )
        .is_empty());
    assert!(gate
        .push_text_at(
            "The next operation will read the source. ".to_string(),
            start + std::time::Duration::from_secs(3),
        )
        .is_empty());
    gate.observe_tool_call();
    assert!(gate.finish_answer().is_empty());
}

#[cfg(test)]
#[test]
fn slow_character_stream_releases_early_and_preserves_complete_answer() {
    let mut gate = ModelTextPresentationGate::for_request(false, true);
    let start = std::time::Instant::now();
    let text =
        "臣亮言：先帝创业未半而中道崩殂，今天下三分，益州疲弊，此诚危急存亡之秋也。".repeat(8);
    let mut chunks = Vec::new();
    let mut first_release_character = None;
    for (index, character) in text.chars().enumerate() {
        let released = gate.push_text_at(
            character.to_string(),
            start + std::time::Duration::from_millis(index as u64 * 10),
        );
        if !released.is_empty() {
            first_release_character.get_or_insert(index + 1);
        }
        chunks.extend(released);
    }
    chunks.extend(gate.finish_answer());
    assert!(first_release_character.expect("must release before completion") < 128);
    assert_eq!(chunks.concat(), text);
    assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 256));
}

#[cfg(test)]
#[test]
fn tool_capable_model_stream_releases_large_answers_as_semantic_phrases() {
    let mut gate = ModelTextPresentationGate::for_request(false, true);
    let text = "第一段说明已经完整，可以稳定显示。第二段继续补充必要信息，并保持 Markdown 内容连续。\n第三段给出最终结论，避免整篇回答突然出现。".repeat(4);
    let chunks = gate.push_text(text.clone());

    assert!(chunks.len() > 1);
    assert_eq!(chunks.concat(), text);
    assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 256));
}

#[cfg(test)]
#[test]
fn web_evidence_is_scoped_to_the_current_user_turn() {
    let collected = vec![
        json!({"role":"user","content":"搜索最新消息","_nola_user_input":true}),
        json!({"role":"assistant","content":"","tool_calls":[]}),
        json!({"role":"tool","name":"web_search","content":"retrieved 5 results"}),
    ];
    assert!(has_web_tool_result_since_latest_user(&collected));

    let stale = vec![
        json!({"role":"user","content":"搜索上一条消息","_nola_user_input":true}),
        json!({"role":"tool","name":"web_search","content":"retrieved 5 results"}),
        json!({"role":"user","content":"搜索另一件事","_nola_user_input":true}),
    ];
    assert!(!has_web_tool_result_since_latest_user(&stale));

    let unrelated = vec![
        json!({"role":"user","content":"搜索最新消息","_nola_user_input":true}),
        json!({"role":"tool","name":"read_file","content":"local file"}),
    ];
    assert!(!has_web_tool_result_since_latest_user(&unrelated));
}

#[cfg(test)]
#[test]
fn patch_failure_key_groups_cosmetic_hunk_variants_by_target() {
    let first = json!({
        "root": "/repo",
        "patch": "--- a/App.swift\n+++ b/App.swift\n@@ -1 +1 @@\n-old\n+new\n"
    });
    let second = json!({
        "root": "/repo",
        "patch": "--- a/App.swift\n+++ b/App.swift\n@@ -40,6 +40,7 @@\n context\n+new\n*** End Patch"
    });
    assert_eq!(
        deterministic_tool_failure_key("apply_patch", first.as_object().unwrap()),
        deterministic_tool_failure_key("apply_patch", second.as_object().unwrap())
    );
}

#[cfg(test)]
mod goal_complete_protocol_tests {
    use super::goal_complete_protocol_payload;

    #[test]
    fn preserves_structured_goal_terminal_fields() {
        let value = goal_complete_protocol_payload(
            "goal_complete: needs_user\ngoal_summary: waiting\ngoal_blockers:\n1. user-confirmation\ngoal_next_action: ask-user",
        )
        .expect("goal payload");
        assert_eq!(value["goal_status"], "needs_user");
        assert_eq!(value["summary"], "waiting");
        assert_eq!(value["blockers"][0], "user-confirmation");
        assert_eq!(value["next_action"], "ask-user");
    }
}

#[cfg(test)]
mod canonical_tool_output_tests {
    use super::*;

    fn outcome(text: Value) -> ToolOutcome {
        ToolOutcome {
            full_output_ref: None,
            data: Some(text.clone()),
            execution: None,
            text: text.to_string(),
            display: "display".to_string(),
            error: false,
            artifacts: Vec::new(),
            activation: None,
        }
    }

    #[test]
    fn explicit_data_is_independent_of_tool_names_and_text_is_inert() {
        let skill = outcome(json!({
            "type": "invoked_skill",
            "skill_id": "documents"
        }));
        assert_eq!(
            canonical_tool_output("skill_invoke", &skill, None)["type"],
            "invoked_skill"
        );

        assert_eq!(
            canonical_tool_output("another_adapter", &skill, None)["type"],
            "invoked_skill"
        );
        let ordinary = canonical_tool_output(
            "read_file",
            &ToolOutcome {
                data: None,
                ..skill.clone()
            },
            None,
        );
        assert!(ordinary.get("type").is_none());
        assert_eq!(ordinary["text"], skill.text);
    }

    #[test]
    fn promotes_skill_runtime_updates_for_core_reduction() {
        let update = outcome(json!({
            "type": "skill_runtime_update",
            "action": "progress",
            "step_id": "validate",
            "status": "completed",
            "summary": "Validated the generated document"
        }));

        let output = canonical_tool_output("skill_update", &update, None);
        assert_eq!(output["type"], "skill_runtime_update");
        assert_eq!(output["action"], "progress");
        assert_eq!(output["step_id"], "validate");
        assert_eq!(output["_nola_display"], "display");

        let ordinary = canonical_tool_output(
            "read_file",
            &ToolOutcome {
                data: None,
                ..update.clone()
            },
            None,
        );
        assert!(ordinary.get("type").is_none());
        assert_eq!(ordinary["text"], update.text);
    }

    #[test]
    fn question_display_text_cannot_be_promoted_into_control_data() {
        let pending = outcome(json!({
            "user_input_status": "unavailable",
            "question": "Which author?"
        }));
        assert_eq!(
            canonical_tool_output("request_user_input", &pending, None)["question"],
            "Which author?"
        );
        assert!(canonical_tool_output(
            "bash",
            &ToolOutcome {
                data: None,
                ..pending
            },
            None
        )
        .get("user_input_status")
        .is_none());
    }

    #[test]
    fn saved_output_reference_is_promoted_to_the_host_envelope() {
        let mut saved = outcome(json!({
            "output_preview": "bounded",
            "truncated": true,
            "full_output_ref": "/tmp/nola-output.json"
        }));
        assert!(
            full_output_ref_from_outcome(&saved).is_none(),
            "business data must not supply Host references"
        );
        saved.full_output_ref = Some("/tmp/nola-output.json".into());

        assert_eq!(
            canonical_tool_output("grep", &saved, None)["full_output_ref"],
            "/tmp/nola-output.json"
        );
    }
}

#[cfg(test)]
mod skill_activation_gate_tests {
    use super::tool_batch_requires_activation_replan;
    use crate::llm::ToolCall;

    fn call(name: &str) -> ToolCall {
        ToolCall {
            id: format!("call-{name}"),
            name: name.to_string(),
            arguments: "{}".to_string(),
        }
    }

    #[test]
    fn preserves_an_ordinary_parallel_batch() {
        let calls = vec![call("read_file"), call("grep")];
        assert!(!tool_batch_requires_activation_replan(&calls));
        assert_eq!(
            calls
                .iter()
                .map(|call| call.name.as_str())
                .collect::<Vec<_>>(),
            ["read_file", "grep"]
        );
    }

    #[test]
    fn preserves_unknown_call_identity_and_multilingual_arguments_for_core_recovery() {
        let original = ToolCall {
            id: "call-unknown-stable".to_string(),
            name: "remembered_or_hallucinated_tool".to_string(),
            arguments: r#"{"query":"继续处理；search 和 current 只是普通文字"}"#.to_string(),
        };

        let calls = vec![original.clone()];
        assert!(!tool_batch_requires_activation_replan(&calls));

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, original.id);
        assert_eq!(calls[0].name, original.name);
        assert_eq!(calls[0].arguments, original.arguments);
    }

    #[test]
    fn rejects_the_entire_mixed_activation_batch_without_dropping_calls() {
        let calls = vec![
            call("bash"),
            call("Skill"),
            call("write_file"),
            call("skill_invoke"),
        ];
        assert!(tool_batch_requires_activation_replan(&calls));
        assert_eq!(calls.len(), 4);
    }

    #[test]
    fn requires_a_standalone_sop_activation_before_work_begins() {
        let calls = vec![call("read_file"), call("SOP"), call("apply_patch")];
        assert!(tool_batch_requires_activation_replan(&calls));
        for name in ["Skill", "skill_invoke", "SOP", "sop_invoke"] {
            assert!(!tool_batch_requires_activation_replan(&[call(name)]));
        }
    }
}

#[cfg(test)]
mod model_tool_projection_contract_tests {
    use super::*;

    fn settings_for(tools: &[Value]) -> Value {
        let projection = AgentModelToolProjection::compile(1, "turn", tools.to_vec())
            .expect("valid fixture model schemas").projection;
        let invocation_policy_digest = "b".repeat(64);
        let stable_prefix_digest = "e".repeat(64);
        let turn_context = serde_json::from_value(json!({
            "stable_prefix_digest": "a".repeat(64)
        }))
        .expect("minimal Turn context");
        let model_prefix_digest = projection.model_prefix_digest_with_invocation_policy(
            &stable_prefix_digest,
            &invocation_policy_digest,
        );
        let request_snapshot_digest =
            projection.request_snapshot_digest(&turn_context, &invocation_policy_digest);
        json!({
            "step_context": {
                "schema_version": 1,
                "step_index": 1,
                "turn_context": turn_context,
                "tool_projection": projection,
                "invocation_policy_digest": invocation_policy_digest,
                "stable_prefix_digest": stable_prefix_digest,
                "model_prefix_digest": model_prefix_digest,
                "request_snapshot_digest": request_snapshot_digest
            }
        })
    }

    #[test]
    fn host_requires_compiled_tool_order_and_rejects_tampering() {
        let alpha = json!({
            "type":"function",
            "function":{"name":"alpha","parameters":{"type":"object"}}
        });
        let beta = json!({
            "type":"function",
            "function":{"name":"beta","parameters":{"type":"object"}}
        });
        let settings = settings_for(&[alpha.clone(), beta.clone()]);

        assert!(validate_model_tool_projection(&settings, &[alpha.clone(), beta.clone()]).is_ok());
        assert!(validate_model_tool_projection(&settings, &[beta.clone(), alpha.clone()]).is_err());
        assert!(validate_model_tool_projection(&settings, std::slice::from_ref(&alpha)).is_err());
        assert!(validate_model_tool_projection(
            &settings,
            &[
                alpha,
                json!({
                    "type":"function",
                    "function":{"name":"beta","parameters":{"type":"object","required":["path"]}}
                }),
            ]
        )
        .is_err());
        assert!(validate_model_tool_projection(&json!({}), &[beta]).is_err());

        let mut invalid_identity = settings_for(&[]);
        invalid_identity["step_context"]["request_snapshot_digest"] = json!("not-a-digest");
        assert!(validate_model_tool_projection(&invalid_identity, &[])
            .expect_err("invalid Step identity must be rejected")
            .contains("request_snapshot_digest"));

        let mut inconsistent_identity = settings_for(&[]);
        inconsistent_identity["step_context"]["model_prefix_digest"] = json!("f".repeat(64));
        assert!(validate_model_tool_projection(&inconsistent_identity, &[])
            .expect_err("inconsistent Step identity must be rejected")
            .contains("model prefix identity"));
    }

    #[test]
    fn builtin_catalog_projects_declarative_loop_progress_contracts() {
        let catalog = build_cli_host_catalog("host");
        let read = &catalog.capabilities["read_file"]
            .input_normalization
            .loop_progress;
        assert_eq!(read.resource_keys, ["path"]);
        assert!(read.selector_keys.contains(&"start_line".to_string()));
        let discovery = &catalog.capabilities["tool_search"]
            .input_normalization
            .loop_progress;
        assert_eq!(
            discovery.constant_resource.as_deref(),
            Some("turn-capability-catalog")
        );
        assert_eq!(discovery.selector_keys, ["query", "max_results"]);

        let write = &catalog.capabilities["write_file"]
            .input_normalization
            .resource_projection;
        assert!(write.resource_keys.contains(&"path".to_string()));
        assert!(write
            .ephemeral_when_true_keys
            .contains(&"dry_run".to_string()));

        let shell = &catalog.capabilities["bash"]
            .input_normalization
            .resource_projection;
        assert_eq!(shell.resource_keys, ["path"]);
        assert_eq!(shell.base_key.as_deref(), Some("cwd"));
        assert!(shell
            .preferred_scope_keys
            .contains(&"validation_targets".to_string()));
    }
}

#[cfg(test)]
mod boundary_fairness_tests {
    use super::*;
    use nola_core::agent::host::AgentBoundarySubmissionSource;

    #[test]
    fn task_notification_is_imported_by_original_store_before_boundary_ack() {
        use nola_runtime_driver::CommittedRuntimeApplicationPort;
        crate::agent_command_port::with_terminal_fixture(|root,receipt| {
            let source=CliBoundarySubmissionSource {cancellation:Default::default(),session_id:Some("parent".into()),run_id:Some("parent-run".into()),
                durable_lease_owner:"boundary-real".into(),durable_state:Default::default()};
            let input=source.take_pending().unwrap().pop().unwrap();
            assert!(matches!(&input,nola_core::agent::host::AgentBoundaryInput::TaskTerminalAvailable {receipt_id,..} if receipt_id==receipt));
            let mut session=crate::session::SessionStore::open(Some("parent"),false,root.to_str().unwrap()).unwrap();
            let notifications=session.assistant_journal_notifications_queue();
            let accepted=session.accept_boundary_input(input.clone()).unwrap();
            assert!(accepted.control.is_some(),"control comes from exact committed recovery");
            assert!(!notifications.lock().unwrap().journal_publications.is_empty(),"actual Task journal publication must precede ACK");
            let checkpoint=session.load_recovery_core_checkpoint().unwrap();
            assert_eq!(checkpoint.state.task_projection.tasks["task"].durability.terminal_receipt_id.as_deref(),Some(receipt));
            // Lost ACK retries the original row; it does not re-execute a Task.
            source.requeue(&input).unwrap();
            let replay=source.take_pending().unwrap().pop().unwrap();
            session.accept_boundary_input(replay.clone()).unwrap();
            source.did_apply(&replay).unwrap();
            assert!(source.take_pending().unwrap().is_empty());
        });
    }

    #[test]
    fn malformed_boundary_envelope_errors_and_releases_its_fifo_lease() {
        crate::agent_command_port::with_terminal_fixture(|_,_| {
            let source=CliBoundarySubmissionSource {cancellation:Default::default(),session_id:Some("corrupt".into()),run_id:Some("corrupt-run".into()),
                durable_lease_owner:"boundary-corrupt".into(),durable_state:Default::default()};
            // Storage-corruption fixture only: the public Application ingress
            // rejects this shape before it can enter the real queue.
            nola_store::NolaStore::open(std::env::var_os("NOLA_DB_PATH").unwrap()).unwrap().enqueue_agent_command(&nola_store::EnqueueAgentCommandInput {
                session_id:"corrupt".into(),command_id:"bad".into(),payload_json:"{}".into(),created_at_ms:1,
            }).unwrap();
            assert!(source.take_pending().unwrap_err().contains("invalid durable Agent envelope"));
            let head=nola_store::NolaStore::open(std::env::var_os("NOLA_DB_PATH").unwrap()).unwrap()
                .peek_agent_command("corrupt").unwrap().unwrap();
            assert!(head.lease_owner.is_none() && head.lease_until_ms.is_none(),
                "invalid input must not strand an owned lease");
            assert!(crate::agent_command_port::claim_next_agent_command("corrupt","other",2,100)
                .unwrap_err().contains("invalid durable Agent envelope"),
                "another consumer also rejects corruption instead of bypassing current ingress validation");
        });
    }

    #[test]
    fn durable_boundary_poll_preserves_fifo_without_draining_the_backlog() {
        let _lock = crate::store::test_env_lock();
        let directory = tempfile::tempdir().unwrap();
        struct Restore(Option<std::ffi::OsString>);
        impl Drop for Restore {
            fn drop(&mut self) {
                match &self.0 {
                    Some(value) => std::env::set_var("NOLA_DB_PATH", value),
                    None => std::env::remove_var("NOLA_DB_PATH"),
                }
            }
        }
        let _restore = Restore(std::env::var_os("NOLA_DB_PATH"));
        std::env::set_var("NOLA_DB_PATH", directory.path().join("boundary.db"));
        let expected = (0..4)
            .map(|index| nola_core::agent::AgentSubmission::RequestTaskCancellation {
                task_id: format!("task-{index}"),
            })
            .collect::<Vec<_>>();
        for (index, submission) in expected.iter().enumerate() {
            crate::agent_command_port::enqueue_agent_command(
                "session",
                &format!("command-{index}"),
                &serde_json::to_string(submission).unwrap(),
                index as i64,
            )
            .unwrap();
        }
        let source = CliBoundarySubmissionSource {
            cancellation: Default::default(),
            session_id: Some("session".into()),
            run_id: Some("session-run".into()),
            durable_lease_owner: "boundary-test".into(),
            durable_state: Default::default(),
        };
        for expected in expected {
            let slice = source.take_pending().unwrap();
            assert_eq!(
                slice.len(),
                1,
                "one command per Host acknowledgement boundary"
            );
            assert_eq!(
                durable_submission_key(&slice[0]),
                durable_submission_key(&nola_core::agent::host::AgentBoundaryInput::Submission {submission:expected})
            );
            source.did_apply(&slice[0]).unwrap();
        }
        assert!(source.take_pending().unwrap().is_empty());
    }
}

#[cfg(test)]
#[test]
fn user_interrupt_is_admitted_at_host_boundary_once_and_requeues_identically() {
    use nola_core::agent::host::AgentBoundarySubmissionSource;
    let cancellation = RuntimeExecutionControl::default();
    let source = CliBoundarySubmissionSource {
        cancellation: cancellation.clone(), session_id: None, run_id: Some("interrupt-run".into()),
        durable_lease_owner: "interrupt-test".into(), durable_state: Default::default(),
    };
    assert!(source.take_pending().unwrap().is_empty());
    cancellation.request();
    let pending = source.take_pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert!(matches!(&pending[0], nola_core::agent::host::AgentBoundaryInput::Submission {
        submission: nola_core::agent::AgentSubmission::Interrupt { .. }
    }));
    assert!(source.take_pending().unwrap().is_empty());
    assert!(!source.durable_state.lock().unwrap().cancellation_acknowledged,
        "dequeue is not an Application ACK");
    source.requeue(&pending[0]).unwrap();
    let retried = source.take_pending().unwrap();
    assert_eq!(durable_submission_key(&retried[0]), durable_submission_key(&pending[0]));
    source.did_apply(&retried[0]).unwrap();
    assert!(source.durable_state.lock().unwrap().cancellation_acknowledged);
    assert!(source.take_pending().unwrap().is_empty());
    let resumed_source = CliBoundarySubmissionSource {
        cancellation, session_id: None, run_id: Some("interrupt-run".into()), durable_lease_owner: "resumed-driver".into(),
        durable_state: std::sync::Arc::clone(&source.durable_state),
    };
    assert!(resumed_source.take_pending().unwrap().is_empty());
}

#[cfg(test)]
#[test]
fn foreground_interrupt_requires_a_run_and_foreign_ack_cannot_retire_it() {
    use nola_core::agent::host::{AgentBoundaryInput, AgentBoundarySubmissionSource};
    let cancellation = RuntimeExecutionControl::default();
    cancellation.request();
    let mut source = CliBoundarySubmissionSource {
        cancellation, session_id: None, run_id: None,
        durable_lease_owner: "exact-interrupt".into(), durable_state: Default::default(),
    };
    assert!(source.take_pending().unwrap_err().contains("exact dispatched Run"));
    assert!(source.durable_state.lock().unwrap().cancellation_id.is_none());
    source.run_id = Some("original-run".into());
    let pending = source.take_pending().unwrap().pop().unwrap();
    source.did_apply(&AgentBoundaryInput::Submission {
        submission: nola_core::agent::AgentSubmission::Interrupt {
            goal_control: None, cancellation_id: "foreign-cancellation".into(), reason: None,
        },
    }).unwrap();
    assert!(!source.durable_state.lock().unwrap().cancellation_acknowledged);
    source.requeue(&pending).unwrap();
    let retried = source.take_pending().unwrap().pop().unwrap();
    assert_eq!(durable_submission_key(&pending), durable_submission_key(&retried));
    source.did_apply(&retried).unwrap();
    assert!(source.durable_state.lock().unwrap().cancellation_acknowledged);
    assert!(source.take_pending().unwrap().is_empty());
}

#[cfg(test)]
#[test]
fn parallel_tool_preference_is_not_inferred_from_capability() {
    for step_allows in [false, true] {
        assert_eq!(
            constrained_parallel_tool_preference(None, step_allows),
            None
        );
        assert_eq!(
            constrained_parallel_tool_preference(Some(false), step_allows),
            Some(false)
        );
        assert_eq!(
            constrained_parallel_tool_preference(Some(true), step_allows),
            Some(step_allows)
        );
    }
}

#[cfg(test)]
mod mcp_tool_semantics_tests {
    use super::{
        code_intelligence_host_effect, effective_mcp_tool_semantics, provider_tool_choice,
    };
    use nola_core::agent::host::{HostCapabilityEffect, ModelResponseMode};
    use nola_core::code_intelligence::{CodeIntelligenceOperation, CODEBASE_MEMORY_PROVIDER_ID};

    #[test]
    fn derived_index_build_is_investigation_while_deletion_is_mutation() {
        assert_eq!(
            code_intelligence_host_effect(CodeIntelligenceOperation::Index),
            HostCapabilityEffect::Investigate
        );
        assert_eq!(
            code_intelligence_host_effect(CodeIntelligenceOperation::Search),
            HostCapabilityEffect::Investigate
        );
        assert_eq!(
            code_intelligence_host_effect(CodeIntelligenceOperation::DeleteIndex),
            HostCapabilityEffect::Mutate
        );
    }

    #[test]
    fn codebase_queries_override_incorrect_destructive_provider_annotations() {
        for tool_name in [
            "index_status",
            "list_projects",
            "get_architecture",
            "search_graph",
            "search_code",
            "trace_path",
            "get_code_snippet",
            "query_graph",
            "get_graph_schema",
            "check_index_coverage",
            "detect_changes",
        ] {
            assert_eq!(
                effective_mcp_tool_semantics(CODEBASE_MEMORY_PROVIDER_ID, tool_name, false, true,),
                (true, false),
                "{tool_name} must project as a read-only Agent Host capability"
            );
        }
    }

    #[test]
    fn codebase_mutations_and_third_party_mcp_keep_concrete_provider_semantics() {
        assert_eq!(
            effective_mcp_tool_semantics(
                CODEBASE_MEMORY_PROVIDER_ID,
                "delete_project",
                false,
                true,
            ),
            (false, true)
        );
        assert_eq!(
            effective_mcp_tool_semantics("third-party.server", "search_graph", false, true),
            (false, true),
            "Nola must not reinterpret an unrelated MCP server by tool name"
        );
        assert_eq!(
            effective_mcp_tool_semantics("third-party.server", "read", true, true),
            (true, false),
            "read-only semantics suppress contradictory destructive hints"
        );
    }

    #[test]
    fn core_required_tool_mode_reaches_the_provider_without_host_routing() {
        assert_eq!(
            provider_tool_choice(true, ModelResponseMode::RequiredTool),
            "required"
        );
        assert_eq!(
            provider_tool_choice(true, ModelResponseMode::Conversational),
            "auto"
        );
        assert_eq!(
            provider_tool_choice(false, ModelResponseMode::RequiredTool),
            "none"
        );
    }
}

#[cfg(test)]
mod model_output_budget_tests {
    use super::{
        model_output_byte_backstop, model_response_provenance,
        model_stream_admission_requires_abort, openrouter_resolved_model, restored_router_model,
        CliAgentHostDriver, ModelOutputStreamBudget, MODEL_OUTPUT_BYTE_LIMIT_REACHED,
    };

    #[test]
    fn openrouter_route_lock_survives_checkpoint_but_not_route_or_credential_change() {
        let mut cfg = retry_test_config();
        cfg.provider = Some("openai-compatible".into());
        cfg.endpoint = "https://openrouter.ai/api/v1/chat/completions".into();
        cfg.model = "openrouter/free".into();
        cfg.fallbacks.clear();
        let physical = "inclusionai/ling-3.0-flash-sante:free";
        assert_eq!(
            openrouter_resolved_model(&cfg, physical).as_deref(),
            Some(physical)
        );
        let continuation = nola_core::agent::protocol::ModelProviderContinuationState {
            provider_namespace: crate::llm::OPENAI_COMPATIBLE_CONTINUATION_NAMESPACE.into(),
            model_id: cfg.model.clone(),
            response_id: Some("response-1".into()),
            state: serde_json::json!({
                "resolved_model_id": physical,
                "route_identity": crate::llm::model_route_identity(&cfg),
            }),
            integrity_kind: "opaque_reference".into(),
            compaction_policy:
                nola_core::agent::protocol::ProviderContinuationCompactionPolicy::ClearOnCompaction,
        };
        let restored: nola_core::agent::protocol::ModelProviderContinuationState =
            serde_json::from_str(&serde_json::to_string(&continuation).unwrap()).unwrap();
        assert_eq!(
            restored_router_model(&cfg, Some(&restored)).as_deref(),
            Some(physical)
        );
        let mut responses_cfg = cfg.clone();
        responses_cfg.provider = Some("openai-responses".into());
        responses_cfg.endpoint = "https://openrouter.ai/api/v1".into();
        let responses_continuation = nola_core::agent::protocol::ModelProviderContinuationState {
            provider_namespace: "openai.responses".into(),
            model_id: responses_cfg.model.clone(),
            response_id: Some("response-2".into()),
            state: serde_json::json!({
                "response_items": [{"type":"message","role":"assistant","content":[]}],
                "resolved_model_id": physical,
                "route_identity": crate::llm::responses_route_identity(&responses_cfg),
            }),
            integrity_kind: "encrypted".into(),
            compaction_policy:
                nola_core::agent::protocol::ProviderContinuationCompactionPolicy::ClearOnCompaction,
        };
        assert_eq!(
            restored_router_model(&responses_cfg, Some(&responses_continuation)).as_deref(),
            Some(physical)
        );
        responses_cfg.api_key = "different-key".into();
        assert_eq!(
            restored_router_model(&responses_cfg, Some(&responses_continuation)),
            None
        );
        cfg.api_key = "different-key".into();
        assert_eq!(restored_router_model(&cfg, Some(&restored)), None);
        cfg.api_key = "must-not-be-presented".into();
        cfg.endpoint = "https://other.example.invalid/api/v1".into();
        assert_eq!(restored_router_model(&cfg, Some(&restored)), None);
    }

    fn retry_test_config() -> crate::llm::LlmConfig {
        crate::llm::LlmConfig {
            siwc: None, native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider: Some("primary-provider".to_string()),
            fallbacks: vec![crate::llm::LlmProviderTarget {
                provider: Some("fallback-provider".to_string()),
                endpoint: "https://fallback.example.invalid/v1".to_string(),
                api_key: Some("must-not-be-presented".to_string()),
                model: Some("fallback-model".to_string()),
            }],
            endpoint: "https://primary.example.invalid/v1".to_string(),
            api_key: "must-not-be-presented".to_string(),
            model: "primary-model".to_string(),
            temperature: 0.7,
            top_p: 0.9,
            max_output_tokens: 2_048,
            max_retries: crate::llm::default_llm_max_retries(),
            thinking_enabled: false, reasoning_summary_supported: None,
            reasoning_selection: None,
            sampling_override: false,
            generation_options: Default::default(),
            cancellation: crate::execution_context::ExecutionContext::default(),
            openai_prompt_cache: false,
            openai_prompt_cache_key: None,
            openai_prompt_cache_key_prefix: None,
            openai_prompt_cache_retention: None,
            openai_prompt_cache_history_breakpoint: false,
            openai_prompt_cache_history_breakpoint_endpoints: None,
            parallel_tool_calls: None,
        }
    }

    #[test]
    fn retry_route_names_failed_and_fallback_models_without_endpoint_or_secret() {
        let llm = retry_test_config();

        let route = CliAgentHostDriver::model_retry_route(&llm, 2, true, "quota exceeded");

        assert_eq!(
            route,
            "primary-provider/primary-model → fallback-provider/fallback-model · quota exceeded"
        );
        assert!(!route.contains("example.invalid"));
        assert!(!route.contains("must-not-be-presented"));
    }

    #[test]
    fn same_target_retry_keeps_attempt_visible_without_claiming_a_fallback() {
        let llm = retry_test_config();

        assert_eq!(
            CliAgentHostDriver::model_retry_route(&llm, 1, false, "temporarily unavailable"),
            "temporarily unavailable · #2"
        );
    }

    #[test]
    fn byte_backstop_bounds_provider_that_ignores_requested_tokens() {
        let limit = model_output_byte_backstop(256);
        let mut budget = ModelOutputStreamBudget::new(256);

        assert!(budget.admit(&"a".repeat(512)));
        assert!(budget.admit(&"b".repeat(limit - 512)));
        assert!(!budget.admit("x"));
        assert_eq!(budget.utf8_bytes(), limit);
    }

    #[test]
    fn byte_backstop_callback_routes_to_incomplete_output_recovery() {
        assert!(!model_stream_admission_requires_abort(
            MODEL_OUTPUT_BYTE_LIMIT_REACHED
        ));
        assert!(model_stream_admission_requires_abort(
            "downstream event sink failed"
        ));
    }

    #[test]
    fn byte_backstop_counts_multilingual_text_and_tool_arguments_as_utf8() {
        let model = "随机模型-λ";
        let tool_arguments = serde_json::json!({"路径": "/tmp/数据", "model": model}).to_string();
        let mut budget = ModelOutputStreamBudget::new(64);

        assert!(budget.admit("你好🌍"));
        assert!(budget.admit(&tool_arguments));
        assert_eq!(budget.utf8_bytes(), "你好🌍".len() + tool_arguments.len());
        assert!(budget.admit(&"é".repeat(64)));
        assert!(!budget.admit(&"é".repeat(model_output_byte_backstop(64))));
    }

    #[test]
    fn long_tool_json_is_not_cut_by_a_character_token_estimate() {
        let mut budget = ModelOutputStreamBudget::new(16_384);
        assert!(budget.admit(&"a".repeat(65_535)));
        assert!(budget.admit(&"b".repeat(65_535)));
        assert_eq!(budget.utf8_bytes(), 131_070);
        budget.start_attempt();
        assert_eq!(budget.utf8_bytes(), 0);
        assert!(budget.admit(&"c".repeat(model_output_byte_backstop(16_384))));
        assert!(!budget.admit("x"));
    }

    #[test]
    fn provenance_records_frozen_budget_and_random_provider_identity() {
        let llm = crate::llm::LlmConfig {
            siwc: None, native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider: Some("openai-compatible".to_string()),
            fallbacks: Vec::new(),
            endpoint: "https://example.invalid/v1".to_string(),
            api_key: String::new(),
            model: "随机/路由-alias".to_string(),
            temperature: 0.7,
            top_p: 0.9,
            max_output_tokens: 2_048,
            max_retries: crate::llm::default_llm_max_retries(),
            thinking_enabled: false, reasoning_summary_supported: None,
            reasoning_selection: None,
            sampling_override: false,
            generation_options: Default::default(),
            cancellation: crate::execution_context::ExecutionContext::default(),
            openai_prompt_cache: false,
            openai_prompt_cache_key: None,
            openai_prompt_cache_key_prefix: None,
            openai_prompt_cache_retention: None,
            openai_prompt_cache_history_breakpoint: false,
            openai_prompt_cache_history_breakpoint_endpoints: None,
            parallel_tool_calls: None,
        };
        let choice = crate::llm::StreamedChoice {
            finish_reason: "length".to_string(),
            response_model: Some("供应商/模型-β".to_string()),
            usage: crate::llm::LlmUsage {
                output_tokens: Some(2_047),
                ..Default::default()
            },
            ..Default::default()
        };

        let provenance = model_response_provenance(&llm, &choice, 3, 8_765);
        assert_eq!(provenance["configured_model"], "随机/路由-alias");
        assert_eq!(provenance["response_model"], "供应商/模型-β");
        assert_eq!(provenance["effective_max_output_tokens"], 2_048);
        assert_eq!(provenance["output_token_parameter"], "max_tokens");
        assert_eq!(provenance["finish_reason"], "length");
        assert_eq!(provenance["provider_usage_output_tokens"], 2_047);
        assert_eq!(provenance["cumulative_utf8_bytes"], 8_765);
        let mut choice = choice;
        choice.usage.output_tokens = None;
        assert!(model_response_provenance(&llm, &choice, 3, 0)["provider_usage_output_tokens"].is_null());
        choice.usage.output_tokens = Some(0);
        assert_eq!(model_response_provenance(&llm, &choice, 3, 0)["provider_usage_output_tokens"], 0);
    }
}

#[cfg(test)]
#[test]
fn ui_publication_rejection_keeps_complete_core_guidance_in_host_tool_result() {
    use effect_delivery_start_barrier_tests::{test_context, test_llm, fenced_request};
    let root = tempfile::tempdir().unwrap();
    let guidance = json!({"error":"ui_revision_conflict","current_revision":7,"expected_revision":6,
        "guidance":"Preserve user edits and use the current revision only after reviewing the interaction",
        "surface":{"surface_id":"original","generation":3}}).to_string();
    let rejected = format!("application_rejected: {guidance}");
    let returned = nola_service_client::ApplicationServiceCommandError::Rejected {
        client_id: "fixture".into(), request_id: "original-command".into(),
        error: nola_protocol::ProtocolFailure {code:"application_rejected".into(), message:guidance,
            recoverable:false, details:Some(json!({"retain":"complete guidance"}))},
    };
    let mut driver = CliAgentHostDriver::new("delivery-host", test_llm("http://127.0.0.1:1".into()), test_context(root.path()))
        .with_ui_publisher(std::sync::Arc::new(move |_| Err(returned.clone())));
    let request = fenced_request(&driver, HostRequest::ExecuteTool {
        capability_id: "render_ui".into(), call_id: "ui-call".into(),
        input: json!({"expected_revision":6,"messages":[{}]}), grant:None,
        permission_instructions:vec![], permission_profile:nola_domain::PermissionProfile::Cautious,
    });
    let mut completed = None;
    driver.dispatch(request, &mut |response| {
        if let HostResponse::ToolCompleted { output, outcome, .. } = response.payload { completed = Some((output,outcome)); }
        Ok(())
    }).unwrap();
    let (output,outcome) = completed.expect("determinate Core rejection must settle the tool");
    assert_eq!(outcome,HostToolOutcome::Failed);
    assert_eq!(output["message"], rejected, "CAS guidance must reach the model without truncation or client reinterpretation");
}

#[cfg(test)]
#[test]
fn ui_publication_unresolved_command_never_emits_terminal_tool_completion() {
    use effect_delivery_start_barrier_tests::{test_context, test_llm, fenced_request};
    use nola_service_client::ApplicationServiceCommandError as Error;
    let errors = [
        Error::Pending {client_id:"source".into(),request_id:"original".into()},
        Error::OutcomeUnknown {client_id:"source".into(),request_id:"original".into(),code:"commit_ack_lost".into(),message:"fixture".into(),reopen_required:true},
        Error::Transport {client_id:"source".into(),request_id:"original".into(),message:"disconnected after send".into(),outcome_unknown:true},
    ];
    for expected in errors {
        let root=tempfile::tempdir().unwrap();
        let returned=expected.clone();
        let mut driver=CliAgentHostDriver::new("delivery-host",test_llm("http://127.0.0.1:1".into()),test_context(root.path()))
            .with_ui_publisher(std::sync::Arc::new(move |_| Err(returned.clone())));
        let request=fenced_request(&driver,HostRequest::ExecuteTool {
            capability_id:"render_ui".into(),call_id:"ui-call".into(),
            input:json!({"expected_revision":0,"messages":[{}]}),grant:None,
            permission_instructions:vec![],permission_profile:nola_domain::PermissionProfile::Cautious,
        });
        let mut terminal=Vec::new();
        let result=driver.dispatch(request,&mut |response| {
            if response.payload.is_terminal() {terminal.push(response.payload);}
            Ok(())
        });
        assert_eq!(result.unwrap_err(),expected.to_string());
        assert!(terminal.is_empty(),"unknown commit must not become model-visible failed/succeeded/cancelled evidence");
    }
}
