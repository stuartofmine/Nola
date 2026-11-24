// SPDX-License-Identifier: Apache-2.0
//! Chat/Anthropic protocol normalization and provider-specific contracts.

#[cfg(test)]
use super::stream_state::merge_usage_from_envelope;
#[cfg(test)]
use super::contracts::LlmUsage;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::env;

use super::contracts::{
    effective_max_output_tokens, LlmConfig, LlmEvent, StreamedChoice, ToolSlot,
};
use super::diagnostics::log_local_debug_payload;
use super::stream_state::append_choice_text;

pub(super) fn normalize_openai_compatible_messages(messages: &[Value]) -> Vec<Value> {
    // Core owns instruction order. A late recovery instruction refers to the
    // preceding partial answer; hoisting it into the initial system message
    // changes that relationship and invalidates the otherwise stable cache
    // prefix. Preserve roles, structured content, and message boundaries.
    let mut normalized = messages.iter().map(openai_request_message).collect::<Vec<_>>();
    normalize_trailing_assistant_messages(&mut normalized);
    repair_openai_tool_message_sequence(&mut normalized);
    strip_ephemeral_message_metadata(&mut normalized);
    strip_excess_media_items(&mut normalized, 80);
    normalized
}

fn openai_request_message(message: &Value) -> Value {
    let mut message = message.clone();
    if let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) {
        for block in blocks {
            if block.get("type").and_then(Value::as_str) == Some("input_file") {
                *block = json!({"type":"file", "file":{
                    "filename":block["filename"], "file_data":block["file_data"]}});
            }
        }
    }
    message
}

fn strip_ephemeral_message_metadata(messages: &mut [Value]) {
    for message in messages {
        if let Some(map) = message.as_object_mut() {
            map.remove("_nola_ephemeral");
            // Core's canonical-user provenance is consumed by Host turn
            // helpers only. It must not reach a provider or become part of
            // physical snapshot/source-alignment equality.
            map.remove("_nola_user_input");
            map.remove("_nola_cache_tail");
            // Host consumes artifact metadata before wire projection (including
            // optional image injection). Archive projection must not leak this
            // private field or differ from the physical cached tool result.
            map.remove("_nola_artifacts");
        }
    }
}

fn strip_excess_media_items(messages: &mut [Value], limit: usize) {
    fn is_media(value: &Value) -> bool {
        matches!(
            value.get("type").and_then(Value::as_str),
            Some(
                "image"
                    | "image_url"
                    | "input_image"
                    | "audio"
                    | "input_audio"
                    | "video"
                    | "input_video"
                    | "file"
                    | "input_file"
            )
        )
    }

    let total = messages
        .iter()
        .filter_map(|message| message.get("content").and_then(Value::as_array))
        .flatten()
        .filter(|item| is_media(item))
        .count();
    let mut remaining_to_drop = total.saturating_sub(limit);
    if remaining_to_drop == 0 {
        return;
    }
    for message in messages {
        let Some(content) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        content.retain(|item| {
            if remaining_to_drop > 0 && is_media(item) {
                remaining_to_drop -= 1;
                false
            } else {
                true
            }
        });
        if remaining_to_drop == 0 {
            break;
        }
    }
}

#[derive(Debug, Default)]
struct ToolProtocolRepairStats {
    inserted_results: usize,
    reordered_results: usize,
    dropped_orphan_results: usize,
    dropped_invalid_calls: usize,
    repaired_ids: usize,
    repaired_arguments: usize,
    repaired_call_shapes: usize,
}

impl ToolProtocolRepairStats {
    fn changed(&self) -> bool {
        self.inserted_results > 0
            || self.reordered_results > 0
            || self.dropped_orphan_results > 0
            || self.dropped_invalid_calls > 0
            || self.repaired_ids > 0
            || self.repaired_arguments > 0
            || self.repaired_call_shapes > 0
    }
}

#[allow(dead_code)]
fn assistant_tool_calls(message: &Value) -> Vec<(String, String)> {
    message
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|call| {
            let id = call.get("id").and_then(Value::as_str)?.trim();
            if id.is_empty() {
                return None;
            }
            let name = call
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .trim();
            Some((id.to_string(), name.to_string()))
        })
        .collect()
}

fn repair_openai_tool_message_sequence(messages: &mut Vec<Value>) {
    use std::collections::HashSet;

    let original = std::mem::take(messages);
    let mut repaired = Vec::with_capacity(original.len());
    let mut stats = ToolProtocolRepairStats::default();
    let mut used_ids = HashSet::<String>::new();
    let mut index = 0usize;
    while index < original.len() {
        let role = original[index]
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let has_tool_calls = role == "assistant"
            && original[index]
                .get("tool_calls")
                .and_then(Value::as_array)
                .is_some_and(|calls| !calls.is_empty());
        if !has_tool_calls {
            if role == "tool" {
                stats.dropped_orphan_results += 1;
            } else {
                repaired.push(original[index].clone());
            }
            index += 1;
            continue;
        }

        let mut assistant = original[index].clone();
        let mut expected = Vec::<(String, Option<String>, String)>::new();
        if let Some(calls) = assistant
            .get_mut("tool_calls")
            .and_then(Value::as_array_mut)
        {
            let original_calls = std::mem::take(calls);
            let mut valid_calls = Vec::with_capacity(original_calls.len());
            for (call_index, mut call) in original_calls.into_iter().enumerate() {
                let raw_id = call
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or_default()
                    .to_string();
                let name = call
                    .get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if !call.is_object() || name.is_empty() {
                    stats.dropped_invalid_calls = stats.dropped_invalid_calls.saturating_add(1);
                    expected.push((raw_id, None, name));
                    continue;
                }
                let arguments = call
                    .get("function")
                    .and_then(|function| function.get("arguments"))
                    .cloned();
                let canonical_arguments = match arguments {
                    None | Some(Value::Null) => {
                        stats.repaired_arguments = stats.repaired_arguments.saturating_add(1);
                        "{}".to_string()
                    }
                    Some(Value::String(raw)) if raw.trim().is_empty() => {
                        stats.repaired_arguments = stats.repaired_arguments.saturating_add(1);
                        "{}".to_string()
                    }
                    Some(Value::String(raw)) => {
                        let Ok(parsed) = serde_json::from_str::<Value>(&raw) else {
                            stats.dropped_invalid_calls =
                                stats.dropped_invalid_calls.saturating_add(1);
                            expected.push((raw_id, None, name));
                            continue;
                        };
                        if !parsed.is_object() {
                            stats.dropped_invalid_calls =
                                stats.dropped_invalid_calls.saturating_add(1);
                            expected.push((raw_id, None, name));
                            continue;
                        }
                        raw
                    }
                    Some(Value::Object(object)) => {
                        stats.repaired_arguments = stats.repaired_arguments.saturating_add(1);
                        serde_json::to_string(&Value::Object(object))
                            .unwrap_or_else(|_| "{}".to_string())
                    }
                    Some(_) => {
                        stats.dropped_invalid_calls = stats.dropped_invalid_calls.saturating_add(1);
                        expected.push((raw_id, None, name));
                        continue;
                    }
                };
                let canonical_id = if !raw_id.is_empty() && used_ids.insert(raw_id.clone()) {
                    raw_id.clone()
                } else {
                    stats.repaired_ids = stats.repaired_ids.saturating_add(1);
                    let mut suffix = 0usize;
                    loop {
                        let candidate = format!("call_nola_repaired_{index}_{call_index}_{suffix}");
                        if used_ids.insert(candidate.clone()) {
                            break candidate;
                        }
                        suffix = suffix.saturating_add(1);
                    }
                };
                if let Some(object) = call.as_object_mut() {
                    if object.get("type").and_then(Value::as_str) != Some("function") {
                        object.insert("type".to_string(), Value::String("function".to_string()));
                        stats.repaired_call_shapes = stats.repaired_call_shapes.saturating_add(1);
                    }
                    object.insert("id".to_string(), Value::String(canonical_id.clone()));
                    if let Some(function) =
                        object.get_mut("function").and_then(Value::as_object_mut)
                    {
                        if function.get("name").and_then(Value::as_str) != Some(name.as_str()) {
                            stats.repaired_call_shapes =
                                stats.repaired_call_shapes.saturating_add(1);
                        }
                        function.insert("name".to_string(), Value::String(name.clone()));
                        function
                            .insert("arguments".to_string(), Value::String(canonical_arguments));
                    } else {
                        stats.dropped_invalid_calls = stats.dropped_invalid_calls.saturating_add(1);
                        expected.push((raw_id, None, name));
                        continue;
                    }
                }
                expected.push((raw_id, Some(canonical_id), name));
                valid_calls.push(call);
            }
            *calls = valid_calls;
        }
        if assistant
            .get("tool_calls")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            if let Some(object) = assistant.as_object_mut() {
                object.remove("tool_calls");
            }
        }
        repaired.push(assistant);
        index += 1;

        let mut results = Vec::<Value>::new();
        let mut deferred_ephemeral = Vec::<Value>::new();
        while index < original.len() {
            let next_role = original[index]
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if next_role == "tool" {
                results.push(original[index].clone());
                index += 1;
                continue;
            }
            let is_ephemeral =
                next_role == "user" && original[index].get("_nola_ephemeral").is_some();
            if is_ephemeral {
                deferred_ephemeral.push(original[index].clone());
                index += 1;
                continue;
            }
            break;
        }

        let mut result_used = vec![false; results.len()];
        for (expected_index, (raw_id, canonical_id, name)) in expected.into_iter().enumerate() {
            let matched = results
                .iter()
                .enumerate()
                .position(|(result_index, result)| {
                    if result_used[result_index] {
                        return false;
                    }
                    let result_id = result
                        .get("tool_call_id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .unwrap_or_default();
                    result_id == raw_id
                        || canonical_id
                            .as_deref()
                            .is_some_and(|canonical_id| result_id == canonical_id)
                });
            if let Some(result_index) = matched {
                result_used[result_index] = true;
                let Some(canonical_id) = canonical_id else {
                    continue;
                };
                if result_index != expected_index {
                    stats.reordered_results = stats.reordered_results.saturating_add(1);
                }
                let mut result = results[result_index].clone();
                if let Some(object) = result.as_object_mut() {
                    object.insert(
                        "tool_call_id".to_string(),
                        Value::String(canonical_id.clone()),
                    );
                    if !object.contains_key("name") {
                        object.insert("name".to_string(), Value::String(name));
                    }
                }
                repaired.push(result);
            } else if let Some(canonical_id) = canonical_id {
                stats.inserted_results += 1;
                repaired.push(json!({
                    "role": "tool",
                    "tool_call_id": canonical_id,
                    "name": name,
                    "content": "[tool result unavailable: the prior execution record was interrupted or incomplete]",
                }));
            }
        }
        stats.dropped_orphan_results = stats
            .dropped_orphan_results
            .saturating_add(result_used.iter().filter(|used| !**used).count());
        repaired.extend(deferred_ephemeral);
    }

    if stats.changed() {
        log_local_debug_payload(
            "tool_protocol_repair",
            &json!({
                "type": "tool_protocol_repair",
                "inserted_results": stats.inserted_results,
                "reordered_results": stats.reordered_results,
                "dropped_orphan_results": stats.dropped_orphan_results,
                "dropped_invalid_calls": stats.dropped_invalid_calls,
                "repaired_ids": stats.repaired_ids,
                "repaired_arguments": stats.repaired_arguments,
                "repaired_call_shapes": stats.repaired_call_shapes,
            }),
        );
    }
    *messages = repaired;
}

#[cfg(test)]
mod canonical_tool_protocol_tests {
    use super::*;

    #[test]
    fn multimodal_browser_evidence_survives_openai_and_anthropic_wire_conversion() {
        let message = json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "Inspect this browser layout."},
                {
                    "type": "image_url",
                    "image_url": {"url": "data:image/png;base64,ZmFrZS1wbmc="}
                }
            ]
        });

        let openai = super::openai_request_message(&message);
        assert_eq!(
            openai
                .pointer("/content/1/type")
                .and_then(serde_json::Value::as_str),
            Some("image_url")
        );
        assert_eq!(
            openai
                .pointer("/content/1/image_url/url")
                .and_then(serde_json::Value::as_str),
            Some("data:image/png;base64,ZmFrZS1wbmc=")
        );

        let blocks = super::anthropic_user_content_blocks(&message);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[1]["type"], "image");
        assert_eq!(blocks[1]["source"]["type"], "base64");
        assert_eq!(blocks[1]["source"]["media_type"], "image/png");
        assert_eq!(blocks[1]["source"]["data"], "ZmFrZS1wbmc=");
    }

    #[test]
    fn anthropic_wire_conversion_preserves_pdf_as_document_block() {
        let message = json!({
            "role": "user",
            "content": [{
                "type": "input_file",
                "filename": "report.pdf",
                "file_data": "data:application/pdf;base64,ZmFrZS1wZGY="
            }]
        });
        let blocks = super::anthropic_user_content_blocks(&message);
        let chat = super::normalize_openai_compatible_messages(&[message.clone()]);
        assert_eq!(chat[0]["content"][0]["type"], "file");
        assert_eq!(chat[0]["content"][0]["file"]["filename"], "report.pdf");
        assert_eq!(
            blocks,
            vec![json!({
                "type": "document",
                "source": {
                    "type": "base64",
                    "media_type": "application/pdf",
                    "data": "ZmFrZS1wZGY="
                }
            })]
        );
    }

    fn parallel_tool_messages(order: [usize; 3], ephemeral_gap: usize) -> Vec<Value> {
        let ids = ["call_1", "call_2", "call_3"];
        let mut messages = vec![json!({
            "role":"assistant",
            "tool_calls": ids.iter().map(|id| json!({
                "id": id,
                "type": "function",
                "function": {"name":"read_file", "arguments":"{}"}
            })).collect::<Vec<_>>()
        })];
        for position in 0..=order.len() {
            if position == ephemeral_gap {
                messages.push(json!({
                    "role":"user",
                    "content":"reflect",
                    "_nola_ephemeral":"agent_reflection_hint"
                }));
            }
            if let Some(result) = order.get(position).copied() {
                messages.push(json!({
                    "role":"tool",
                    "tool_call_id":ids[result],
                    "content":format!("result-{result}")
                }));
            }
        }
        messages
    }

    #[test]
    fn canonical_repair_assigns_unique_ids_and_pairs_duplicate_results_in_order() {
        let mut messages = vec![
            json!({
                "role":"assistant",
                "tool_calls":[
                    {"id":"","function":{"name":"read_file","arguments":"{}"}},
                    {"id":"duplicate","function":{"name":"bash","arguments":"{}"}},
                    {"id":"duplicate","function":{"name":"write_file","arguments":"{}"}}
                ]
            }),
            json!({"role":"tool","tool_call_id":"","content":"first"}),
            json!({"role":"tool","tool_call_id":"duplicate","content":"second"}),
            json!({"role":"tool","tool_call_id":"duplicate","content":"third"}),
        ];

        repair_openai_tool_message_sequence(&mut messages);

        let ids = messages[0]["tool_calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|call| call["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert!(ids.iter().all(|id| !id.is_empty()));
        assert_eq!(
            ids.iter().collect::<std::collections::HashSet<_>>().len(),
            3
        );
        let result_ids = messages[1..]
            .iter()
            .map(|message| message["tool_call_id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(result_ids, ids);
    }

    #[test]
    fn canonical_repair_never_moves_tool_results_across_a_real_user_message() {
        let mut messages = vec![
            json!({"role":"assistant","tool_calls":[{"id":"call_1","function":{"name":"bash","arguments":"{}"}}]}),
            json!({"role":"user","content":"new request"}),
            json!({"role":"tool","tool_call_id":"call_1","content":"late result"}),
        ];

        repair_openai_tool_message_sequence(&mut messages);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"], "new request");
        assert!(messages[1]["content"]
            .as_str()
            .unwrap()
            .contains("unavailable"));
    }

    #[test]
    fn canonical_repair_defers_ephemeral_reflection_until_all_results_are_paired() {
        let mut messages = vec![
            json!({"role":"assistant","tool_calls":[
                {"id":"call_1","function":{"name":"read_file","arguments":"{}"}},
                {"id":"call_2","function":{"name":"read_file","arguments":"{}"}}
            ]}),
            json!({"role":"tool","tool_call_id":"call_1","content":"one"}),
            json!({"role":"user","content":"reflect","_nola_ephemeral":"agent_reflection_hint"}),
            json!({"role":"tool","tool_call_id":"call_2","content":"two"}),
        ];

        repair_openai_tool_message_sequence(&mut messages);

        assert_eq!(
            messages
                .iter()
                .map(|message| message["role"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["assistant", "tool", "tool", "user"]
        );
        assert_eq!(messages[1]["tool_call_id"], "call_1");
        assert_eq!(messages[2]["tool_call_id"], "call_2");
        assert_eq!(messages[3]["_nola_ephemeral"], "agent_reflection_hint");
    }

    #[test]
    fn anthropic_serialization_uses_the_same_canonical_tool_ids() {
        let mut messages = vec![
            json!({"role":"assistant","tool_calls":[
                {"id":"","function":{"name":"read_file","arguments":"{}"}},
                {"id":"duplicate","function":{"name":"bash","arguments":"{}"}},
                {"id":"duplicate","function":{"name":"write_file","arguments":"{}"}}
            ]}),
            json!({"role":"tool","tool_call_id":"","content":"one"}),
            json!({"role":"tool","tool_call_id":"duplicate","content":"two"}),
            json!({"role":"tool","tool_call_id":"duplicate","content":"three"}),
        ];
        repair_openai_tool_message_sequence(&mut messages);
        strip_ephemeral_message_metadata(&mut messages);
        let (_, anthropic) = convert_messages_to_anthropic(&messages);
        let tool_use_ids = anthropic
            .iter()
            .flat_map(|message| message["content"].as_array().into_iter().flatten())
            .filter(|block| block["type"] == "tool_use")
            .map(|block| block["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        let tool_result_ids = anthropic
            .iter()
            .flat_map(|message| message["content"].as_array().into_iter().flatten())
            .filter(|block| block["type"] == "tool_result")
            .map(|block| block["tool_use_id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();

        assert_eq!(tool_result_ids, tool_use_ids);
        assert!(tool_use_ids.iter().all(|id| !id.is_empty()));
        assert_eq!(
            tool_use_ids
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            tool_use_ids.len(),
        );
    }

    #[test]
    fn canonical_repair_handles_every_parallel_result_order_and_ephemeral_gap() {
        let permutations = [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ];
        for order in permutations {
            for gap in 0..=3 {
                let normalized =
                    normalize_openai_compatible_messages(&parallel_tool_messages(order, gap));
                assert_eq!(
                    normalized
                        .iter()
                        .map(|message| message["role"].as_str().unwrap())
                        .collect::<Vec<_>>(),
                    vec!["assistant", "tool", "tool", "tool", "user"]
                );
                assert_eq!(normalized[1]["tool_call_id"], "call_1");
                assert_eq!(normalized[2]["tool_call_id"], "call_2");
                assert_eq!(normalized[3]["tool_call_id"], "call_3");
                assert!(normalized[4].get("_nola_ephemeral").is_none());

                let (_, anthropic) = convert_messages_to_anthropic(&normalized);
                assert!(anthropic
                    .windows(2)
                    .all(|window| window[0]["role"] != window[1]["role"]));
                assert_eq!(anthropic.len(), 2);
                assert_eq!(anthropic[0]["role"], "assistant");
                assert_eq!(anthropic[1]["role"], "user");
                let blocks = anthropic[1]["content"].as_array().unwrap();
                assert_eq!(blocks.len(), 4);
                assert_eq!(blocks[0]["tool_use_id"], "call_1");
                assert_eq!(blocks[1]["tool_use_id"], "call_2");
                assert_eq!(blocks[2]["tool_use_id"], "call_3");
                assert_eq!(blocks[3]["type"], "text");
            }
        }
    }

    #[test]
    fn canonical_repair_drops_irreparable_calls_and_is_idempotent() {
        let messages = vec![
            json!({"role":"assistant","tool_calls":[
                {"id":"good","function":{"name":"read_file","arguments":"{\"path\":\"a\"}"}},
                {"id":"missing_args","function":{"name":"bash"}},
                {"id":"missing_function"},
                {"id":"missing_name","function":{"name":"","arguments":"{}"}},
                {"id":"invalid_json","function":{"name":"bash","arguments":"not-json"}},
                {"id":"non_object","function":{"name":"bash","arguments":"[]"}}
            ]}),
            json!({"role":"tool","tool_call_id":"good","content":"good"}),
            json!({"role":"tool","tool_call_id":"missing_args","content":"missing args"}),
            json!({"role":"tool","tool_call_id":"missing_function","content":"bad"}),
            json!({"role":"tool","tool_call_id":"missing_name","content":"bad"}),
            json!({"role":"tool","tool_call_id":"invalid_json","content":"bad"}),
            json!({"role":"tool","tool_call_id":"non_object","content":"bad"}),
        ];

        let once = normalize_openai_compatible_messages(&messages);
        let twice = normalize_openai_compatible_messages(&once);

        assert_eq!(twice, once);
        let calls = once[0]["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["id"], "good");
        assert_eq!(calls[1]["id"], "missing_args");
        assert_eq!(calls[0]["type"], "function");
        assert_eq!(calls[1]["function"]["arguments"], "{}");
        assert_eq!(once.len(), 3);
        assert_eq!(once[1]["tool_call_id"], "good");
        assert_eq!(once[2]["tool_call_id"], "missing_args");
    }

    #[test]
    fn consecutive_assistant_tool_fragments_merge_before_result_repair() {
        let messages = vec![
            json!({"role":"user","content":"read"}),
            json!({"role":"assistant","tool_calls":[{
                "id":"call_1","function":{"name":"read_file","arguments":"{\"path\":\""}
            }]}),
            json!({"role":"assistant","tool_calls":[{
                "id":"call_1","function":{"name":"read_file","arguments":"Cargo.toml\"}"}
            }]}),
            json!({"role":"tool","tool_call_id":"call_1","content":"actual result"}),
        ];

        let normalized = normalize_openai_compatible_messages(&messages);

        assert_eq!(normalized.len(), 3);
        let arguments = normalized[1]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(arguments).unwrap()["path"],
            "Cargo.toml"
        );
        assert_eq!(normalized[2]["content"], "actual result");
    }
}

fn normalize_trailing_assistant_messages(messages: &mut Vec<Value>) {
    let original = std::mem::take(messages);
    let mut normalized = Vec::<Value>::with_capacity(original.len());
    for message in original {
        let is_assistant = message.get("role").and_then(Value::as_str) == Some("assistant");
        let previous_is_assistant = normalized
            .last()
            .and_then(|previous| previous.get("role"))
            .and_then(Value::as_str)
            == Some("assistant");
        if is_assistant && previous_is_assistant {
            if let Some(previous) = normalized.last_mut() {
                merge_assistant_message_fragment(previous, &message);
            }
        } else {
            normalized.push(message);
        }
    }
    *messages = normalized;
}

fn merge_assistant_message_fragment(target: &mut Value, fragment: &Value) {
    merge_assistant_text_field(target, fragment, "content");
    merge_assistant_text_field(target, fragment, "reasoning_content");
    for array_field in ["nola_response_items", "thinking_blocks"] {
        if let Some(incoming_items) = fragment.get(array_field).and_then(Value::as_array) {
            if !incoming_items.is_empty() {
                if let Some(target_object) = target.as_object_mut() {
                    let target_items = target_object
                        .entry(array_field.to_string())
                        .or_insert_with(|| Value::Array(Vec::new()));
                    if let Some(target_array) = target_items.as_array_mut() {
                        target_array.extend(incoming_items.iter().cloned());
                    }
                }
            }
        }
    }

    let incoming_calls = fragment
        .get("tool_calls")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if incoming_calls.is_empty() {
        return;
    }
    let Some(target_object) = target.as_object_mut() else {
        return;
    };
    let target_calls = target_object
        .entry("tool_calls".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(target_calls) = target_calls.as_array_mut() else {
        return;
    };
    for incoming in incoming_calls {
        let incoming_id = incoming
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        let matched = (!incoming_id.is_empty())
            .then(|| {
                target_calls.iter().position(|existing| {
                    existing.get("id").and_then(Value::as_str).map(str::trim) == Some(incoming_id)
                })
            })
            .flatten();
        if let Some(index) = matched {
            merge_tool_call_fragment(&mut target_calls[index], &incoming);
        } else {
            target_calls.push(incoming);
        }
    }
}

fn merge_assistant_text_field(target: &mut Value, fragment: &Value, field: &str) {
    let Some(incoming) = fragment.get(field) else {
        return;
    };
    let incoming = if let Some(text) = incoming.as_str() {
        text.to_string()
    } else if incoming.is_null() {
        String::new()
    } else {
        incoming.to_string()
    };
    if incoming.trim().is_empty() {
        return;
    }
    let current = target
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default();
    let merged = if current.trim().is_empty() {
        incoming
    } else {
        format!("{current}\n\n{incoming}")
    };
    if let Some(object) = target.as_object_mut() {
        object.insert(field.to_string(), Value::String(merged));
    }
}

fn merge_tool_call_fragment(target: &mut Value, fragment: &Value) {
    let target_name = target
        .get("function")
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let incoming_name = fragment
        .get("function")
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let target_arguments = target
        .get("function")
        .and_then(|function| function.get("arguments"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let incoming_arguments = fragment
        .get("function")
        .and_then(|function| function.get("arguments"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let Some(function) = target.get_mut("function").and_then(Value::as_object_mut) else {
        *target = fragment.clone();
        return;
    };
    if target_name.trim().is_empty() && !incoming_name.trim().is_empty() {
        function.insert("name".to_string(), Value::String(incoming_name));
    }
    let arguments = merge_tool_argument_fragments(&target_arguments, &incoming_arguments);
    function.insert("arguments".to_string(), Value::String(arguments));
}

fn merge_tool_argument_fragments(existing: &str, incoming: &str) -> String {
    if incoming.is_empty() || incoming == existing {
        return existing.to_string();
    }
    if existing.is_empty() {
        return incoming.to_string();
    }
    let concatenated = format!("{existing}{incoming}");
    if serde_json::from_str::<Value>(&concatenated).is_ok_and(|value| value.is_object()) {
        return concatenated;
    }
    if serde_json::from_str::<Value>(incoming).is_ok_and(|value| value.is_object()) {
        return incoming.to_string();
    }
    if serde_json::from_str::<Value>(existing).is_ok_and(|value| value.is_object()) {
        return existing.to_string();
    }
    concatenated
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChatProvider {
    OpenAiCompatible(OpenAiFlavor),
    Anthropic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OpenAiFlavor {
    OpenAi,
    OpenRouter,
    DeepSeek,
    DashScope,
    LlamaCpp,
    Vllm,
    Generic,
}

pub(super) fn detect_chat_provider(
    provider_id: Option<&str>,
    endpoint: &str,
    model: &str,
) -> ChatProvider {
    if super::contracts::resolved_model_protocol(provider_id, endpoint)
        == nola_domain::generation::ModelProtocol::Anthropic
    {
        return ChatProvider::Anthropic;
    }
    if let Some(provider) = provider_id
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
        .map(str::to_ascii_lowercase)
    {
        match provider.as_str() {
            "openrouter" => return ChatProvider::OpenAiCompatible(OpenAiFlavor::OpenRouter),
            "anthropic" | "claude" => return ChatProvider::Anthropic,
            "deepseek" => return ChatProvider::OpenAiCompatible(OpenAiFlavor::DeepSeek),
            "dashscope" | "qwen" => return ChatProvider::OpenAiCompatible(OpenAiFlavor::DashScope),
            "llamacpp" | "llama.cpp" | "llama-cpp" => {
                return ChatProvider::OpenAiCompatible(OpenAiFlavor::LlamaCpp)
            }
            "vllm" => return ChatProvider::OpenAiCompatible(OpenAiFlavor::Vllm),
            _ => {}
        }
    }

    let endpoint_host = url::Url::parse(endpoint.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_default();
    let model_lc = model.to_ascii_lowercase();
    if endpoint_host == "openrouter.ai" {
        ChatProvider::OpenAiCompatible(OpenAiFlavor::OpenRouter)
    } else if endpoint_host == "api.openai.com" {
        ChatProvider::OpenAiCompatible(OpenAiFlavor::OpenAi)
    } else if endpoint_host == "api.deepseek.com"
        || model_lc.contains("deepseek-v4")
        || model_lc.contains("deepseek-r1")
    {
        ChatProvider::OpenAiCompatible(OpenAiFlavor::DeepSeek)
    } else if endpoint_host == "aliyuncs.com" || endpoint_host.ends_with(".aliyuncs.com") {
        ChatProvider::OpenAiCompatible(OpenAiFlavor::DashScope)
    } else if endpoint_host == "vllm" {
        ChatProvider::OpenAiCompatible(OpenAiFlavor::Vllm)
    } else if model_lc.starts_with("gpt-")
        || model_lc.starts_with("codex-")
        || matches!(model_lc.as_str(), "o1" | "o3" | "o4-mini")
        || model_lc.starts_with("o1-")
        || model_lc.starts_with("o3-")
        || model_lc.starts_with("o4-")
    {
        ChatProvider::OpenAiCompatible(OpenAiFlavor::OpenAi)
    } else if model_lc.contains("qwen")
        && nola_core::model_privacy::is_private_or_local_endpoint(endpoint)
    {
        ChatProvider::OpenAiCompatible(OpenAiFlavor::LlamaCpp)
    } else {
        ChatProvider::OpenAiCompatible(OpenAiFlavor::Generic)
    }
}

pub(super) fn anthropic_request_body(
    cfg: &LlmConfig,
    messages: &[Value],
    tools: &[Value],
    tool_choice: &str,
) -> Result<Value, String> {
    cfg.generation_options.validate()?;
    if cfg.generation_options.presence_penalty.is_some()
        || cfg.generation_options.frequency_penalty.is_some()
        || cfg.generation_options.repetition_penalty.is_some()
        || cfg.generation_options.seed.is_some()
    {
        return Err("Anthropic does not support these generation parameters".into());
    }
    let mut canonical_messages = messages.to_vec();
    normalize_trailing_assistant_messages(&mut canonical_messages);
    repair_openai_tool_message_sequence(&mut canonical_messages);
    // Locate the ephemeral tail boundary BEFORE stripping `_nola_cache_tail`
    // so the Anthropic message cache breakpoint lands on the last durable
    // content block and never after per-step runtime facts.
    let cache_tail_index = canonical_messages
        .iter()
        .position(|message| message.get("_nola_cache_tail").and_then(Value::as_bool) == Some(true));
    strip_ephemeral_message_metadata(&mut canonical_messages);
    strip_excess_media_items(&mut canonical_messages, 80);
    let cache_control = cfg
        .openai_prompt_cache
        .then(|| anthropic_cache_control(cfg.openai_prompt_cache_retention.as_deref()));
    let (mut system, mut anthropic_messages) = convert_messages_to_anthropic_with_cache_boundary(
        &canonical_messages,
        cache_tail_index,
        cache_control.as_ref(),
    );
    if let Some(cache_control) = cache_control.as_ref() {
        // Only the leading instruction segment is a reusable policy prefix.
        // Late system/developer instructions remain trusted top-level blocks:
        // This route has no negotiated inline-system capability. Never weaken
        // them to user messages merely to improve cache reuse. Dynamic instructions still
        // invalidate the subsequent history cache, but not this policy prefix.
        let leading_instruction_count = canonical_messages
            .iter()
            .take_while(|message| matches!(message["role"].as_str(), Some("system" | "developer")))
            .filter(|message| {
                message_content_as_text(message).is_some_and(|text| !text.trim().is_empty())
            })
            .count();
        if let Some(block) = leading_instruction_count
            .checked_sub(1)
            .and_then(|index| system.get_mut(index))
        {
            block["cache_control"] = cache_control.clone();
        }
        // Only fall back to the last message when no `_nola_cache_tail` marker
        // was present; when a tail boundary exists, the breakpoint was already
        // attached to the last durable content block before the tail.
        if cache_tail_index.is_none() {
            attach_anthropic_cache_breakpoint(
                &mut anthropic_messages,
                false,
                cfg.openai_prompt_cache_retention.as_deref(),
            );
        }
    }
    let max_tokens = effective_max_output_tokens(cfg)?;
    let mut body = json!({
        "model": cfg.model,
        "messages": anthropic_messages,
        "stream": true,
        "max_tokens": max_tokens,
    });
    if !system.is_empty() {
        body["system"] = Value::Array(system);
    }
    if let Some(top_k) = cfg.generation_options.top_k {
        body["top_k"] = json!(top_k);
    }
    if let Some(stop) = &cfg.generation_options.stop {
        body["stop_sequences"] = json!(stop);
    }
    if cfg.sampling_override {
        body["temperature"] = json!(cfg.temperature);
        body["top_p"] = json!(cfg.top_p);
    }
    let mut anthropic_tools = convert_tools_to_anthropic(tools);
    if !anthropic_tools.is_empty() {
        if let Some(cache_control) = cache_control.as_ref() {
            if let Some(last_tool) = anthropic_tools.last_mut().and_then(Value::as_object_mut) {
                last_tool.insert("cache_control".to_string(), cache_control.clone());
            }
        }
        body["tools"] = Value::Array(anthropic_tools);
        body["tool_choice"] = json!({
            "type": match tool_choice { "required" => "any", "none" => "none", _ => "auto" }
        });
        if tool_choice != "none" {
            if let Some(parallel) = cfg.parallel_tool_calls {
                body["tool_choice"]["disable_parallel_tool_use"] = Value::Bool(!parallel);
            }
        }
    }
    if let Some(selection) = &cfg.reasoning_selection {
        selection.validate(max_tokens)?;
        match selection {
            nola_core::agent::reasoning::ReasoningSelection::Budget { tokens } => {
                body["thinking"] = json!({"type":"enabled", "budget_tokens":tokens});
            }
            nola_core::agent::reasoning::ReasoningSelection::Effort { value } if value == "none" => {
                // Off is a distinct, explicitly admitted control, never a token
                // budget or Anthropic's provider-default effort.
                body["thinking"] = json!({"type":"disabled"});
            }
            nola_core::agent::reasoning::ReasoningSelection::Effort { value } => {
                // Native effort does not prove support for adaptive thinking.
                // Leave the thinking mode at the provider default.
                body["output_config"] = json!({"effort":value});
            }
        }
    }
    Ok(body)
}

fn anthropic_cache_control(retention: Option<&str>) -> Value {
    if retention.is_some_and(|value| value.trim().eq_ignore_ascii_case("1h")) {
        json!({"type":"ephemeral","ttl":"1h"})
    } else {
        json!({"type":"ephemeral"})
    }
}

pub(super) fn attach_anthropic_cache_breakpoint(
    messages: &mut Vec<Value>,
    skip_cache_write: bool,
    retention: Option<&str>,
) {
    if messages.is_empty() {
        return;
    }

    let marker_index = if skip_cache_write {
        let Some(index) = messages.len().checked_sub(2) else {
            return;
        };
        index
    } else {
        messages.len() - 1
    };

    let cache_control = anthropic_cache_control(retention);
    // Cache editing is a separate, unimplemented route capability. Do not add
    // cache_reference to old tool results when the marker advances: those
    // results must retain the same payload, and we never send cache_edits.
    add_cache_control_marker_to_message(&mut messages[marker_index], &cache_control);
}

fn add_cache_control_marker_to_message(message: &mut Value, cache_control: &Value) -> bool {
    let Some(content) = message.get_mut("content").and_then(Value::as_array_mut) else {
        return false;
    };

    // Cache through the final supported block, not just the first text in a
    // merged user message. In particular all results of a parallel tool batch
    // must be inside the cached prefix. No fabricated empty text is necessary.
    for block in content.iter_mut().rev() {
        if !matches!(
            block["type"].as_str(),
            Some("text" | "image" | "document" | "tool_use" | "tool_result")
        ) {
            continue;
        }
        if let Some(object) = block.as_object_mut() {
            if !object.contains_key("cache_control") {
                object.insert("cache_control".to_string(), cache_control.clone());
            }
            return true;
        }
    }

    false
}

fn convert_messages_to_anthropic(messages: &[Value]) -> (Vec<Value>, Vec<Value>) {
    convert_messages_to_anthropic_with_cache_boundary(messages, None, None)
}

fn anthropic_thinking_blocks_for_assistant(message: &Value) -> Vec<Value> {
    let mut blocks = Vec::new();
    let sources = [
        message.get("nola_response_items").and_then(Value::as_array),
        message.get("thinking_blocks").and_then(Value::as_array),
        message
            .get("_nola_provider_continuation")
            .and_then(|continuation| continuation.pointer("/state/response_items"))
            .and_then(Value::as_array),
        message.get("content").and_then(Value::as_array),
    ];
    for items in sources.into_iter().flatten() {
        for item in items {
            match item.get("type").and_then(Value::as_str) {
                Some("thinking") => {
                    let signature = item
                        .get("signature")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if signature.trim().is_empty() {
                        continue;
                    }
                    let thinking = item
                        .get("thinking")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    blocks.push(json!({
                        "type": "thinking",
                        "thinking": thinking,
                        "signature": signature,
                    }));
                }
                Some("redacted_thinking") => {
                    let data = item.get("data").and_then(Value::as_str).unwrap_or_default();
                    if data.trim().is_empty() {
                        continue;
                    }
                    blocks.push(json!({
                        "type": "redacted_thinking",
                        "data": data,
                    }));
                }
                _ => {}
            }
        }
        if !blocks.is_empty() {
            break;
        }
    }
    blocks
}

fn convert_messages_to_anthropic_with_cache_boundary(
    messages: &[Value],
    cache_tail_index: Option<usize>,
    cache_control: Option<&Value>,
) -> (Vec<Value>, Vec<Value>) {
    let mut system_parts = Vec::new();
    let mut out = Vec::new();

    for (index, message) in messages.iter().enumerate() {
        if cache_tail_index == Some(index) {
            if let Some(cache_control) = cache_control {
                for durable_message in out.iter_mut().rev() {
                    if add_cache_control_marker_to_message(durable_message, cache_control) {
                        break;
                    }
                }
            }
        }
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match role {
            "system" | "developer" => {
                if let Some(content) = message_content_as_text(message) {
                    if !content.trim().is_empty() {
                        system_parts.push(json!({"type":"text", "text":content}));
                    }
                }
            }
            "assistant" => {
                let mut content = anthropic_thinking_blocks_for_assistant(message);
                if let Some(text) = message_content_as_text(message) {
                    if !text.is_empty() {
                        content.push(json!({"type": "text", "text": text}));
                    }
                }
                for tool_call in message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let Some(function) = tool_call.get("function") else {
                        continue;
                    };
                    let Some(name) = function.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    let id = tool_call
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("toolu_nola");
                    let input = function
                        .get("arguments")
                        .and_then(Value::as_str)
                        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                        .unwrap_or_else(|| json!({}));
                    content.push(json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": input,
                    }));
                }
                if !content.is_empty() {
                    push_anthropic_message(&mut out, "assistant", content);
                }
            }
            "tool" => {
                let id = message
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let content = message_content_as_text(message).unwrap_or_default();
                push_anthropic_message(
                    &mut out,
                    "user",
                    vec![json!({
                        "type": "tool_result",
                        "tool_use_id": id,
                        "content": content,
                    })],
                );
            }
            "user" => {
                push_anthropic_message(&mut out, "user", anthropic_user_content_blocks(message));
            }
            _ => {
                let content = message_content_as_text(message).unwrap_or_default();
                push_anthropic_message(
                    &mut out,
                    "user",
                    vec![json!({"type": "text", "text": content})],
                );
            }
        }
    }

    (system_parts, out)
}

fn anthropic_user_content_blocks(message: &Value) -> Vec<Value> {
    let Some(items) = message.get("content").and_then(Value::as_array) else {
        return vec![json!({
            "type": "text",
            "text": message_content_as_text(message).unwrap_or_default()
        })];
    };
    let mut blocks = Vec::new();
    for item in items {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => blocks.push(json!({
                "type": "text",
                "text": item.get("text").and_then(Value::as_str).unwrap_or_default()
            })),
            Some("image_url") => {
                let data_url = item.pointer("/image_url/url").and_then(Value::as_str);
                let parsed = data_url.and_then(|url| {
                    let (header, data) = url.split_once(',')?;
                    let media_type = header
                        .strip_prefix("data:")
                        .and_then(|value| value.strip_suffix(";base64"))?;
                    Some((media_type, data))
                });
                if let Some((media_type, data)) = parsed {
                    blocks.push(json!({
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": media_type,
                            "data": data
                        }
                    }));
                } else {
                    blocks.push(json!({
                        "type": "text",
                        "text": "[Unsupported image attachment omitted]"
                    }));
                }
            }
            Some("input_file") => {
                let filename = item
                    .get("filename")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let data_url = item.get("file_data").and_then(Value::as_str);
                let parsed = data_url.and_then(|url| {
                    let (header, data) = url.split_once(',')?;
                    let media_type = header
                        .strip_prefix("data:")
                        .and_then(|value| value.strip_suffix(";base64"))?;
                    Some((media_type, data))
                });
                if let Some((media_type, data)) = parsed {
                    if media_type.eq_ignore_ascii_case("application/pdf") {
                        blocks.push(json!({
                            "type": "document",
                            "source": {
                                "type": "base64",
                                "media_type": media_type,
                                "data": data
                            }
                        }));
                        continue;
                    }
                }
                let placeholder = match filename {
                    Some(name) => format!("[Unsupported file attachment omitted: {name}]"),
                    None => "[Unsupported file attachment omitted]".to_string(),
                };
                blocks.push(json!({
                    "type": "text",
                    "text": placeholder
                }));
            }
            _ => {}
        }
    }
    if blocks.is_empty() {
        blocks.push(json!({
            "type": "text",
            "text": "[Attachment omitted]"
        }));
    }
    blocks
}

fn push_anthropic_message(out: &mut Vec<Value>, role: &str, content: Vec<Value>) {
    if content.is_empty() {
        return;
    }
    if let Some(previous_content) = out
        .last_mut()
        .filter(|previous| previous.get("role").and_then(Value::as_str) == Some(role))
        .and_then(|previous| previous.get_mut("content"))
        .and_then(Value::as_array_mut)
    {
        previous_content.extend(content);
        return;
    }
    out.push(json!({"role": role, "content": content}));
}

fn message_content_as_text(message: &Value) -> Option<String> {
    match message.get("content") {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Array(parts)) => {
            let text = parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("");
            Some(text)
        }
        Some(value) if !value.is_null() => Some(value.to_string()),
        _ => None,
    }
}

fn convert_tools_to_anthropic(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter_map(|tool| {
            let function = tool.get("function")?;
            let name = function.get("name").and_then(Value::as_str)?;
            let description = function
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let input_schema = function
                .get("parameters")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
            Some(json!({
                "name": name,
                "description": description,
                "input_schema": input_schema,
            }))
        })
        .collect()
}

fn upsert_anthropic_thinking_block(
    response_items: &mut Vec<Value>,
    index: usize,
    thinking_delta: &str,
    signature_delta: &str,
) {
    let target_index = index as u64;
    if let Some(existing) = response_items.iter_mut().rev().find(|item| {
        item.get("type").and_then(Value::as_str) == Some("thinking")
            && item.get("_index").and_then(Value::as_u64) == Some(target_index)
    }) {
        if let Some(object) = existing.as_object_mut() {
            if !thinking_delta.is_empty() {
                let current = object
                    .get("thinking")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let mut updated = String::with_capacity(current.len() + thinking_delta.len());
                updated.push_str(current);
                updated.push_str(thinking_delta);
                object.insert("thinking".to_string(), Value::String(updated));
            }
            if !signature_delta.is_empty() {
                let current = object
                    .get("signature")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let mut updated = String::with_capacity(current.len() + signature_delta.len());
                updated.push_str(current);
                updated.push_str(signature_delta);
                object.insert("signature".to_string(), Value::String(updated));
            }
        }
        return;
    }
    response_items.push(json!({
        "type": "thinking",
        "thinking": thinking_delta,
        "signature": signature_delta,
        "_index": target_index,
    }));
}

fn finalize_anthropic_thinking_block(response_items: &mut [Value], index: usize) {
    let target_index = index as u64;
    for item in response_items.iter_mut().rev() {
        if item.get("type").and_then(Value::as_str) == Some("thinking")
            && item.get("_index").and_then(Value::as_u64) == Some(target_index)
        {
            if let Some(object) = item.as_object_mut() {
                object.remove("_index");
            }
            break;
        }
    }
}

pub(super) fn finalize_anthropic_response_items(response_items: &mut Vec<Value>) {
    response_items.retain_mut(|item| {
        if let Some(object) = item.as_object_mut() {
            object.remove("_index");
        }
        match item.get("type").and_then(Value::as_str) {
            Some("thinking") => item
                .get("signature")
                .and_then(Value::as_str)
                .is_some_and(|sig| !sig.trim().is_empty()),
            Some("redacted_thinking") => item
                .get("data")
                .and_then(Value::as_str)
                .is_some_and(|data| !data.trim().is_empty()),
            _ => true,
        }
    });
}

pub(super) fn parse_anthropic_stream_event<F>(
    parsed: &Value,
    choice: &mut StreamedChoice,
    slots: &mut BTreeMap<usize, ToolSlot>,
    on_event: &mut F,
) -> Result<(), String>
where
    F: FnMut(LlmEvent) -> Result<(), String>,
{
    match parsed
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "error" => {
            // Anthropic can report a terminal provider failure inside a
            // successful HTTP SSE response. Preserve the complete typed
            // envelope so the shared transport policy can distinguish
            // context overflow from transient failures without inspecting
            // localized message text.
            return Err(serde_json::to_string(parsed)
                .unwrap_or_else(|_| "{\"type\":\"error\"}".to_string()));
        }
        "content_block_start" => {
            let index = parsed.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let Some(block) = parsed.get("content_block") else {
                return Ok(());
            };
            match block
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
            {
                "text" => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        if !text.is_empty() {
                            append_choice_text(choice, text, on_event)?;
                        }
                    }
                }
                "thinking" => {
                    let initial_thinking = block
                        .get("thinking")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let initial_signature = block
                        .get("signature")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    upsert_anthropic_thinking_block(
                        &mut choice.response_items,
                        index,
                        initial_thinking,
                        initial_signature,
                    );
                    if !initial_thinking.is_empty() {
                        choice.reasoning.push_str(initial_thinking);
                        on_event(LlmEvent::ReasoningDelta(initial_thinking.to_string()))?;
                    }
                }
                "redacted_thinking" => {
                    if let Some(data) = block
                        .get("data")
                        .and_then(Value::as_str)
                        .filter(|data| !data.is_empty())
                    {
                        choice.response_items.push(json!({
                            "type": "redacted_thinking",
                            "data": data,
                        }));
                    }
                }
                "tool_use" => {
                    nola_core::agent::model_tool_batch::validate_model_tool_batch_size(
                        slots.len() + usize::from(!slots.contains_key(&index)),
                    )
                    .map_err(str::to_string)?;
                    let slot = slots.entry(index).or_default();
                    if let Some(id) = block.get("id").and_then(Value::as_str) {
                        slot.id = id.to_string();
                    }
                    if let Some(name) = block.get("name").and_then(Value::as_str) {
                        slot.name = name.to_string();
                    }
                    if let Some(input) = block.get("input") {
                        if input.is_object() && !input.as_object().is_some_and(|obj| obj.is_empty())
                        {
                            slot.arguments = input.to_string();
                        }
                    }
                    if !slot.id.is_empty() && !slot.name.is_empty() {
                        on_event(LlmEvent::ToolCallDelta {
                            call_id: slot.id.clone(),
                            name: slot.name.clone(),
                            arguments_delta: slot.arguments.clone(),
                        })?;
                    }
                }
                _ => {}
            }
        }
        "content_block_delta" => {
            let index = parsed.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let Some(delta) = parsed.get("delta") else {
                return Ok(());
            };
            match delta
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
            {
                "text_delta" => {
                    if let Some(text) = delta.get("text").and_then(Value::as_str) {
                        if !text.is_empty() {
                            append_choice_text(choice, text, on_event)?;
                        }
                    }
                }
                "thinking_delta" => {
                    if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                        if !text.is_empty() {
                            upsert_anthropic_thinking_block(
                                &mut choice.response_items,
                                index,
                                text,
                                "",
                            );
                            choice.reasoning.push_str(text);
                            on_event(LlmEvent::ReasoningDelta(text.to_string()))?;
                        }
                    }
                }
                "signature_delta" => {
                    if let Some(signature) = delta.get("signature").and_then(Value::as_str) {
                        if !signature.is_empty() {
                            upsert_anthropic_thinking_block(
                                &mut choice.response_items,
                                index,
                                "",
                                signature,
                            );
                        }
                    }
                }
                "input_json_delta" => {
                    if let Some(partial) = delta.get("partial_json").and_then(Value::as_str) {
                        nola_core::agent::model_tool_batch::validate_model_tool_batch_size(
                            slots.len() + usize::from(!slots.contains_key(&index)),
                        )
                        .map_err(str::to_string)?;
                        let slot = slots.entry(index).or_default();
                        slot.arguments.push_str(partial);
                        if !slot.id.is_empty() && !slot.name.is_empty() {
                            on_event(LlmEvent::ToolCallDelta {
                                call_id: slot.id.clone(),
                                name: slot.name.clone(),
                                arguments_delta: partial.to_string(),
                            })?;
                        }
                    }
                }
                _ => {}
            }
        }
        "content_block_stop" => {
            if let Some(index) = parsed.get("index").and_then(Value::as_u64) {
                finalize_anthropic_thinking_block(&mut choice.response_items, index as usize);
            }
        }
        "message_delta" => {
            if let Some(reason) = parsed
                .get("delta")
                .and_then(|delta| delta.get("stop_reason"))
                .and_then(Value::as_str)
            {
                choice.finish_reason = match reason {
                    "end_turn" => "stop",
                    "tool_use" => "tool_calls",
                    other => other,
                }
                .to_string();
            }
        }
        _ => {}
    }
    Ok(())
}

/// OpenAI-compatible open-weights serving engines (Strata, llama.cpp, vLLM,
/// DashScope/Qwen, DeepSeek, etc.) render Jinja chat templates that hoist or
/// rewrite the leading `<|im_start|>system` header whenever a `role: "system"`
/// message appears mid-conversation, invalidating the entire KV prefix cache
/// back to token 0. Preserve all leading system messages byte-for-byte, and
/// convert any *non-leading* system message in-place to `role: "user"` wrapped
/// in `<system_checkpoint>` so message order and prefix cache stay intact.
pub(super) fn normalize_mid_conversation_system_messages_for_provider(
    cfg: &LlmConfig,
    body: &mut Value,
) {
    if matches!(
        detect_chat_provider(cfg.provider.as_deref(), &cfg.endpoint, &cfg.model),
        ChatProvider::OpenAiCompatible(OpenAiFlavor::OpenAi)
    ) {
        return;
    }
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    let mut seen_non_system = false;
    for message in messages.iter_mut() {
        let Some(obj) = message.as_object_mut() else {
            continue;
        };
        let is_system = obj.get("role").and_then(Value::as_str) == Some("system");
        if !is_system {
            seen_non_system = true;
            continue;
        }
        if !seen_non_system {
            continue;
        }
        obj.insert("role".to_string(), Value::String("user".to_string()));
        match obj.get_mut("content") {
            Some(Value::String(text)) => {
                *text = format!("<system_checkpoint>\n{text}\n</system_checkpoint>");
            }
            Some(Value::Array(parts)) => {
                parts.insert(
                    0,
                    json!({"type": "text", "text": "<system_checkpoint>"}),
                );
                parts.push(json!({"type": "text", "text": "</system_checkpoint>"}));
            }
            _ => {}
        }
    }
}

pub(super) fn apply_thinking_policy(cfg: &LlmConfig, body: &mut Value) {
    let Some(obj) = body.as_object_mut() else {
        return;
    };

    let provider = detect_chat_provider(cfg.provider.as_deref(), &cfg.endpoint, &cfg.model);
    let history_uses_reasoning_content =
        obj.get("messages")
            .and_then(Value::as_array)
            .is_some_and(|messages| {
                messages.iter().any(|message| {
                    message.get("role").and_then(Value::as_str) == Some("assistant")
                        && message.get("reasoning_content").is_some()
                })
            });
    let explicitly_disabled = matches!(
        cfg.reasoning_selection,
        Some(nola_core::agent::reasoning::ReasoningSelection::Effort { ref value }) if value == "none"
    ) || (cfg.compaction_output_budget.is_some()
        && !cfg.thinking_enabled && cfg.reasoning_selection.is_none());
    if provider == ChatProvider::OpenAiCompatible(OpenAiFlavor::OpenRouter) {
        if explicitly_disabled {
            obj.insert("reasoning".into(), json!({"enabled": false}));
        }
        return;
    }
    let provider_requires_reasoning_content = match provider {
        ChatProvider::OpenAiCompatible(OpenAiFlavor::DeepSeek) => !explicitly_disabled,
        ChatProvider::OpenAiCompatible(OpenAiFlavor::DashScope) => cfg.thinking_enabled,
        _ => false,
    };
    if provider_requires_reasoning_content || history_uses_reasoning_content {
        if let Some(messages) = obj.get_mut("messages").and_then(Value::as_array_mut) {
            for message in messages {
                let is_assistant = message.get("role").and_then(Value::as_str) == Some("assistant");
                let has_tool_calls = message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .is_some_and(|calls| !calls.is_empty());
                if is_assistant && has_tool_calls && message.get("reasoning_content").is_none() {
                    message["reasoning_content"] = Value::String(String::new());
                }
            }
        }
    }
    if matches!(
        provider,
        ChatProvider::OpenAiCompatible(OpenAiFlavor::DeepSeek)
    ) {
        // Capability describes what the model can do, not a user choice to
        // enable/disable thinking. An automatic selection must leave both
        // vendor controls absent so the provider's own default applies.
        if explicitly_disabled {
            obj.insert("thinking".to_string(), json!({"type": "disabled"}));
        } else if cfg.reasoning_selection.is_some() {
            obj.insert("thinking".to_string(), json!({"type": "enabled"}));
        }
        return;
    }

    if matches!(
        provider,
        ChatProvider::OpenAiCompatible(
            OpenAiFlavor::DashScope | OpenAiFlavor::LlamaCpp | OpenAiFlavor::Vllm
        )
    ) {
        // Match DeepSeek / OpenRouter semantics: an automatic (unset) selection
        // must leave vendor thinking controls absent so the provider/model's
        // native server default applies instead of silently disabling thinking.
        if explicitly_disabled {
            obj.insert("enable_thinking".to_string(), Value::Bool(false));
            obj.insert(
                "chat_template_kwargs".to_string(),
                json!({"enable_thinking": false}),
            );
        } else if cfg.reasoning_selection.is_some() || cfg.thinking_enabled {
            obj.insert(
                "enable_thinking".to_string(),
                Value::Bool(cfg.thinking_enabled),
            );
            obj.insert(
                "chat_template_kwargs".to_string(),
                json!({"enable_thinking": cfg.thinking_enabled}),
            );
        }
    }
}

#[cfg(test)]
mod thinking_http_contract_tests {
    use super::super::streaming::stream_openai_compatible_chat;
    use super::LlmConfig;
    use serde_json::{json, Value};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn real_http_body_keeps_empty_reasoning_for_later_tool_rounds() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock LLM");
        let address = listener.local_addr().expect("mock LLM address");
        listener
            .set_nonblocking(true)
            .expect("set mock LLM nonblocking");
        let server = thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "LLM request did not reach the HTTP boundary within five seconds"
                        );
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("accept LLM request: {error}"),
                }
            };
            stream
                .set_nonblocking(false)
                .expect("set mock LLM stream blocking");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set read timeout");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            let (header_end, content_length) = loop {
                let read = stream.read(&mut buffer).expect("read LLM request");
                assert!(read > 0, "request ended before headers");
                request.extend_from_slice(&buffer[..read]);
                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .expect("content-length header");
                break (header_end + 4, content_length);
            };
            while request.len() < header_end + content_length {
                let read = stream.read(&mut buffer).expect("read LLM body");
                assert!(read > 0, "request ended before body");
                request.extend_from_slice(&buffer[..read]);
            }
            let body: Value =
                serde_json::from_slice(&request[header_end..header_end + content_length])
                    .expect("valid request JSON");

            let events = concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n"
            );
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                events.len(),
                events
            )
            .expect("write mock response");
            body
        });

        let cfg = LlmConfig {
            siwc: None, native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider: None,
            fallbacks: Vec::new(),
            endpoint: format!("http://{address}/v1"),
            api_key: "test-key".to_string(),
            model: "deepseek-v4-flash".to_string(),
            temperature: 0.2,
            top_p: 0.9,
            max_output_tokens: 256,
            max_retries: crate::llm::default_llm_max_retries(),
            thinking_enabled: true, reasoning_summary_supported: None,
            reasoning_selection: None,
            sampling_override: false,
            generation_options: Default::default(),
            cancellation: Default::default(),
            openai_prompt_cache: false,
            openai_prompt_cache_key: None,
            openai_prompt_cache_key_prefix: None,
            openai_prompt_cache_retention: None,
            openai_prompt_cache_history_breakpoint: false,
            openai_prompt_cache_history_breakpoint_endpoints: None,
            parallel_tool_calls: None,
        };
        let messages = vec![
            json!({"role":"user", "content":"inspect"}),
            json!({
                "role":"assistant",
                "content":"",
                "reasoning_content":"first thought",
                "tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{}"}}]
            }),
            json!({"role":"tool","tool_call_id":"call-1","content":"first result"}),
            json!({
                "role":"assistant",
                "content":"",
                "tool_calls":[{"id":"call-2","type":"function","function":{"name":"read_file","arguments":"{}"}}]
            }),
            json!({"role":"tool","tool_call_id":"call-2","content":"second result"}),
            json!({"role":"user", "content":"continue"}),
        ];

        let response = stream_openai_compatible_chat(&cfg, &messages, &[], "auto", |_| Ok(()))
            .expect("mock request succeeds");
        assert_eq!(response.content, "ok");
        let body = server.join().expect("mock server completes");
        assert!(body.get("temperature").is_none());
        assert!(body.get("top_p").is_none());
        assert_eq!(body["messages"][1]["reasoning_content"], "first thought");
        assert_eq!(body["messages"][3]["reasoning_content"], "");
    }
}

pub(super) fn non_stream_required_tools_enabled(cfg: &LlmConfig) -> bool {
    if let Ok(value) = env::var("NOLA_LLM_STREAM_REQUIRED_TOOLS") {
        return !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        );
    }

    !matches!(
        detect_chat_provider(cfg.provider.as_deref(), &cfg.endpoint, &cfg.model),
        ChatProvider::OpenAiCompatible(OpenAiFlavor::LlamaCpp | OpenAiFlavor::Vllm)
    )
}


fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

pub(super) fn normalize_chat_endpoint(endpoint: &str) -> Result<String, String> {
    super::transport::normalized_model_endpoint(
        endpoint,
        nola_domain::generation::ModelProtocol::ChatCompletions,
    )
    .map(|url| url.to_string())
}

pub(super) fn normalize_anthropic_endpoint(endpoint: &str) -> Result<String, String> {
    super::transport::normalized_model_endpoint(
        endpoint,
        nola_domain::generation::ModelProtocol::Anthropic,
    )
    .map(|url| url.to_string())
}

#[cfg(test)]
mod provider_usage_presence_tests {
    use super::*;

    #[test]
    fn model_usage_absent_partial_zero_and_split_frames_remain_distinct() {
        let mut usage = LlmUsage::default();
        merge_usage_from_envelope(&mut usage, &json!({"choices":[]}));
        assert_eq!(usage.input_tokens, None);
        assert_eq!(usage.total_tokens, None);
        assert_eq!(usage.cache_usage(false, false).input_tokens, None);
        merge_usage_from_envelope(&mut usage, &json!({"message":{"usage":{"input_tokens":0,"cache_read_input_tokens":0}}}));
        assert_eq!(usage.input_tokens, Some(0));
        assert_eq!(usage.output_tokens, None);
        assert_eq!(usage.total_tokens, None);
        merge_usage_from_envelope(&mut usage, &json!({"usage":{"output_tokens":10,"output_tokens_details":{"reasoning_tokens":7}}}));
        assert_eq!(usage.input_tokens, Some(0));
        assert_eq!(usage.output_tokens, Some(10));
        assert_eq!(usage.reasoning_tokens, Some(7));
        assert_eq!(usage.total_tokens, None);
        merge_usage_from_envelope(&mut usage, &json!({"usage":{"total_tokens":10}}));
        assert_eq!(usage.total_tokens, Some(10));
        merge_usage_from_envelope(&mut usage, &json!({"usage":{"total_tokens":null,"input_tokens":-1}}));
        assert_eq!(usage.total_tokens, Some(10));
        assert_eq!(usage.input_tokens, Some(0));
    }

    #[test]
    fn model_usage_cache_only_does_not_invent_input_or_provider_total() {
        let mut usage = LlmUsage::default();
        merge_usage_from_envelope(&mut usage, &json!({"usage":{"input_tokens_details":{"cached_tokens":12,"cache_write_tokens":0}}}));
        assert_eq!(usage.cached_input_tokens, Some(12));
        assert_eq!(usage.cache_write_input_tokens, Some(0));
        assert_eq!(usage.cache_usage(false, false).input_tokens, None);
        assert_eq!(usage.cache_usage(true, false).input_tokens, None);
        assert_eq!(usage.total_tokens, None);
        let durable = serde_json::to_value(&usage).unwrap();
        assert!(durable["input_tokens"].is_null());
        assert_eq!(durable["cache_write_input_tokens"], 0);
        assert_eq!(serde_json::from_value::<LlmUsage>(durable).unwrap(), usage);
        assert!(serde_json::from_value::<LlmUsage>(json!({"input_tokens":"0"})).is_err());
    }

    #[test]
    fn anthropic_request_caps_media_at_80_and_preserves_unsupported_attachment_turns() {
        let cfg = LlmConfig {
            siwc: None,
            native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider: Some("anthropic".to_string()),
            fallbacks: Vec::new(),
            endpoint: "https://api.anthropic.com".to_string(),
            api_key: "test-key".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            temperature: 0.2,
            top_p: 0.9,
            max_output_tokens: 256,
            max_retries: 1,
            thinking_enabled: false,
            reasoning_summary_supported: None,
            reasoning_selection: None,
            sampling_override: false,
            generation_options: Default::default(),
            cancellation: Default::default(),
            openai_prompt_cache: false,
            openai_prompt_cache_key: None,
            openai_prompt_cache_key_prefix: None,
            openai_prompt_cache_retention: None,
            openai_prompt_cache_history_breakpoint: false,
            openai_prompt_cache_history_breakpoint_endpoints: None,
            parallel_tool_calls: None,
        };
        let mut content = Vec::new();
        for _ in 0..85 {
            content.push(json!({
                "type": "image_url",
                "image_url": { "url": "data:image/png;base64,AAAA" }
            }));
        }
        let messages = vec![
            json!({ "role": "user", "content": content }),
            json!({ "role": "assistant", "content": "noted" }),
            json!({
                "role": "user",
                "content": [
                    { "type": "image_url", "image_url": { "url": "https://example.com/photo.png" } },
                    { "type": "input_file", "filename": "report.docx", "file_data": "data:application/vnd.openxmlformats-officedocument.wordprocessingml.document;base64,AAAA" }
                ]
            }),
        ];
        let body = anthropic_request_body(&cfg, &messages, &[], "auto").expect("anthropic body");
        let anthropic_messages = body["messages"].as_array().expect("messages array");
        assert_eq!(anthropic_messages.len(), 3);
        let image_count = anthropic_messages[0]["content"]
            .as_array()
            .expect("first user blocks")
            .iter()
            .filter(|block| block["type"] == "image")
            .count();
        // 85 images in message 0 + 2 media items in message 2 = 87 total;
        // capping to 80 drops the 7 oldest items from message 0, leaving 78.
        assert_eq!(image_count, 78);
        let third_blocks = anthropic_messages[2]["content"]
            .as_array()
            .expect("unsupported attachment turn preserved");
        assert_eq!(third_blocks.len(), 2);
        assert_eq!(third_blocks[0]["type"], "text");
        assert!(third_blocks[0]["text"].as_str().unwrap().contains("image"));
        assert_eq!(third_blocks[1]["type"], "text");
        assert!(third_blocks[1]["text"].as_str().unwrap().contains("report.docx"));
    }

    #[test]
    fn anthropic_prompt_caching_marks_tools_system_and_durable_prefix_before_cache_tail() {
        let cfg = LlmConfig {
            siwc: None,
            native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider: Some("anthropic".to_string()),
            fallbacks: Vec::new(),
            endpoint: "https://api.anthropic.com".to_string(),
            api_key: "test-key".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            temperature: 0.2,
            top_p: 0.9,
            max_output_tokens: 1024,
            max_retries: 1,
            thinking_enabled: false,
            reasoning_summary_supported: None,
            reasoning_selection: None,
            sampling_override: false,
            generation_options: Default::default(),
            cancellation: Default::default(),
            openai_prompt_cache: true,
            openai_prompt_cache_key: None,
            openai_prompt_cache_key_prefix: None,
            openai_prompt_cache_retention: None,
            openai_prompt_cache_history_breakpoint: false,
            openai_prompt_cache_history_breakpoint_endpoints: None,
            parallel_tool_calls: None,
        };
        let tools = vec![
            json!({"type": "function", "function": {"name": "read_file", "description": "Read", "parameters": {"type": "object"}}}),
            json!({"type": "function", "function": {"name": "write_file", "description": "Write", "parameters": {"type": "object"}}}),
        ];
        let messages = vec![
            json!({"role": "system", "content": "Static policy."}),
            json!({"role": "user", "content": "Inspect the repo"}),
            json!({"role": "assistant", "content": "", "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "read_file", "arguments": "{\"path\":\"Cargo.toml\"}"}}
            ]}),
            json!({"role": "tool", "tool_call_id": "call_1", "content": "durable tool output"}),
            json!({"role": "user", "content": "ephemeral runtime facts step 1", "_nola_cache_tail": true}),
        ];

        let body = anthropic_request_body(&cfg, &messages, &tools, "auto").expect("anthropic body");
        // (a) Last tool carries ephemeral cache_control; earlier tools do not.
        assert!(body["tools"][0].get("cache_control").is_none());
        assert_eq!(body["tools"][1]["cache_control"], json!({"type": "ephemeral"}));
        // (b) Static leading system block carries cache_control.
        assert_eq!(body["system"][0]["cache_control"], json!({"type": "ephemeral"}));
        // (c) Durable tool_result block BEFORE `_nola_cache_tail` carries cache_control,
        // NOT the merged ephemeral runtime-facts text block after it.
        let last_user_blocks = body["messages"][2]["content"].as_array().expect("merged user content");
        assert_eq!(last_user_blocks.len(), 2);
        assert_eq!(last_user_blocks[0]["type"], "tool_result");
        assert_eq!(last_user_blocks[0]["cache_control"], json!({"type": "ephemeral"}));
        assert_eq!(last_user_blocks[1]["type"], "text");
        assert!(last_user_blocks[1].get("cache_control").is_none());
        assert!(!body.to_string().contains("_nola_cache_tail"));
    }

    #[test]
    fn detect_chat_provider_does_not_classify_public_https_qwen_endpoint_as_llamacpp() {
        assert_eq!(
            detect_chat_provider(None, "https://gateway.example.com/v1", "qwen3-coder-480b"),
            ChatProvider::OpenAiCompatible(OpenAiFlavor::Generic)
        );
        assert_eq!(
            detect_chat_provider(None, "https://api.together.xyz/v1", "Qwen/Qwen2.5-Coder-32B"),
            ChatProvider::OpenAiCompatible(OpenAiFlavor::Generic)
        );
        assert_eq!(
            detect_chat_provider(None, "http://127.0.0.1:8080/v1", "qwen3.6-35b-a3b"),
            ChatProvider::OpenAiCompatible(OpenAiFlavor::LlamaCpp)
        );
    }

    #[test]
    fn anthropic_extended_thinking_signature_and_redacted_blocks_roundtrip_before_tool_use() {
        let mut choice = StreamedChoice::default();
        let mut slots = BTreeMap::<usize, ToolSlot>::new();
        let events = vec![
            json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": {"type": "thinking", "thinking": "", "signature": ""}
            }),
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "thinking_delta", "thinking": "Step 1: check files. "}
            }),
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "thinking_delta", "thinking": "Step 2: call tool."}
            }),
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "signature_delta", "signature": "sig_part_1_"}
            }),
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "signature_delta", "signature": "sig_part_2"}
            }),
            json!({"type": "content_block_stop", "index": 0}),
            json!({
                "type": "content_block_start",
                "index": 1,
                "content_block": {"type": "redacted_thinking", "data": "opaque_redacted_blob"}
            }),
            json!({
                "type": "content_block_start",
                "index": 2,
                "content_block": {"type": "tool_use", "id": "toolu_1", "name": "read_file", "input": {}}
            }),
            json!({
                "type": "content_block_delta",
                "index": 2,
                "delta": {"type": "input_json_delta", "partial_json": "{\"path\":\"src/lib.rs\"}"}
            }),
        ];
        for event in &events {
            parse_anthropic_stream_event(event, &mut choice, &mut slots, &mut |_| Ok(())).unwrap();
        }
        finalize_anthropic_response_items(&mut choice.response_items);

        assert_eq!(choice.reasoning, "Step 1: check files. Step 2: call tool.");
        assert_eq!(choice.response_items.len(), 2);
        assert_eq!(
            choice.response_items[0],
            json!({
                "type": "thinking",
                "thinking": "Step 1: check files. Step 2: call tool.",
                "signature": "sig_part_1_sig_part_2"
            })
        );
        assert_eq!(
            choice.response_items[1],
            json!({
                "type": "redacted_thinking",
                "data": "opaque_redacted_blob"
            })
        );

        let history = vec![
            json!({"role": "user", "content": "Inspect lib.rs"}),
            json!({
                "role": "assistant",
                "content": "",
                "nola_response_items": choice.response_items,
                "tool_calls": [{
                    "id": "toolu_1",
                    "type": "function",
                    "function": {"name": "read_file", "arguments": "{\"path\":\"src/lib.rs\"}"}
                }]
            }),
            json!({"role": "tool", "tool_call_id": "toolu_1", "content": "pub fn ok() {}"}),
        ];
        let (_, anthropic) = convert_messages_to_anthropic(&history);
        let assistant_blocks = anthropic[1]["content"].as_array().expect("assistant blocks");
        assert_eq!(assistant_blocks.len(), 3);
        assert_eq!(assistant_blocks[0]["type"], "thinking");
        assert_eq!(assistant_blocks[0]["signature"], "sig_part_1_sig_part_2");
        assert_eq!(assistant_blocks[1]["type"], "redacted_thinking");
        assert_eq!(assistant_blocks[1]["data"], "opaque_redacted_blob");
        assert_eq!(assistant_blocks[2]["type"], "tool_use");
        assert_eq!(assistant_blocks[2]["id"], "toolu_1");
    }

    #[test]
    fn mid_conversation_system_messages_rewritten_for_open_weights_engines_but_not_openai() {
        let base_messages = json!([
            {"role": "system", "content": "Leading system prompt 0"},
            {"role": "system", "content": "Leading system prompt 1"},
            {"role": "user", "content": "Run task"},
            {"role": "assistant", "content": "", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "bash", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": "c1", "content": "exit=1"},
            {"role": "system", "content": "Engineering-loop checkpoint: previous tool failed."}
        ]);

        let make_cfg = |endpoint: &str, model: &str| LlmConfig {
            siwc: None,
            native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider: None,
            fallbacks: Vec::new(),
            endpoint: endpoint.to_string(),
            api_key: "test-key".to_string(),
            model: model.to_string(),
            temperature: 0.2,
            top_p: 0.9,
            max_output_tokens: 256,
            max_retries: crate::llm::default_llm_max_retries(),
            thinking_enabled: true,
            reasoning_summary_supported: None,
            reasoning_selection: None,
            sampling_override: false,
            generation_options: Default::default(),
            cancellation: Default::default(),
            openai_prompt_cache: false,
            openai_prompt_cache_key: None,
            openai_prompt_cache_key_prefix: None,
            openai_prompt_cache_retention: None,
            openai_prompt_cache_history_breakpoint: false,
            openai_prompt_cache_history_breakpoint_endpoints: None,
            parallel_tool_calls: None,
        };

        // Open-weights / local serving endpoint (e.g. Strata / llama.cpp / vLLM / DashScope)
        let mut local_body = json!({"messages": base_messages.clone()});
        let local_cfg = make_cfg("http://192.168.31.18:18081/v1", "qwen3.8-flash-next-iq3_s");
        normalize_mid_conversation_system_messages_for_provider(&local_cfg, &mut local_body);
        let msgs = local_body["messages"].as_array().expect("messages");
        // Leading system prefix stays byte-for-byte unchanged
        assert_eq!(msgs[0], base_messages[0]);
        assert_eq!(msgs[1], base_messages[1]);
        assert_eq!(msgs[2], base_messages[2]);
        // Mid-conversation system message is converted in-place to user checkpoint
        assert_eq!(msgs[5]["role"], "user");
        assert_eq!(
            msgs[5]["content"],
            "<system_checkpoint>\nEngineering-loop checkpoint: previous tool failed.\n</system_checkpoint>"
        );

        // Official OpenAI API keeps mid-conversation system role intact
        let mut openai_body = json!({"messages": base_messages.clone()});
        let openai_cfg = make_cfg("https://api.openai.com/v1", "gpt-5");
        normalize_mid_conversation_system_messages_for_provider(&openai_cfg, &mut openai_body);
        assert_eq!(openai_body["messages"], base_messages);
    }
}
