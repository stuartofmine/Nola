// SPDX-License-Identifier: Apache-2.0

#[cfg(test)]
mod command_semantic_tests {
    use super::{command_semantic, ShellExitEvidence};

    #[test]
    fn search_no_matches_does_not_hide_a_downstream_failure() {
        let evidence = ShellExitEvidence { exit_code: 1, pipeline_exit_codes: vec![0, 1] };
        assert!(command_semantic("grep pattern file | consumer", 1, Some(&evidence)).0);
        let evidence = ShellExitEvidence { exit_code: 1, pipeline_exit_codes: vec![1, 0] };
        let (failed, note) = command_semantic("grep pattern file | head -40", 1, Some(&evidence));
        assert!(!failed);
        assert_eq!(note, "No matches found");
    }


    #[test]
    fn sigpipe_from_head_is_informational() {
        let evidence = ShellExitEvidence { exit_code: 141, pipeline_exit_codes: vec![141, 0] };
        let (failed, note) = command_semantic("git diff -- src | head -160", 141, Some(&evidence));
        assert!(!failed);
        assert!(note.contains("bounded pipeline consumer"));
    }

    #[test]
    fn unrelated_sigpipe_remains_a_failure() {
        let (failed, _) = command_semantic("custom-producer | custom-consumer", 141, None);
        assert!(failed);
    }

    #[test]
    fn sigpipe_requires_actual_pipeline_evidence_not_head_in_source() {
        let evidence = ShellExitEvidence { exit_code: 141, pipeline_exit_codes: vec![141, 0] };
        for command in ["echo 'x | head -20'", "exit 141; echo hi | head -20", "producer | head -20 | false", "producer | head missing-file", "producer || head -20"] {
            assert!(command_semantic(command, 141, Some(&evidence)).0, "{command}");
        }
        assert!(command_semantic("producer | head -20", 141, None).0);
        for codes in [vec![2, 141, 0], vec![0, 141], vec![141], vec![141, 1]] {
            let e = ShellExitEvidence { exit_code: 141, pipeline_exit_codes: codes };
            assert!(command_semantic("producer | head -20", 141, Some(&e)).0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn bash_exit_evidence_preserves_pipeline_status_and_shell_exit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("exit-evidence");
        for (command, code, success) in [("yes | head -20", 141, true), ("exit 141", 141, false), ("false", 1, false), ("printf ok", 0, true)] {
            let output = std::process::Command::new("bash")
                .args(["-o", "pipefail", "-c", &super::shell_evidence_script(command)])
                .env("NOLA_SHELL_EVIDENCE_PATH", &path).output().unwrap();
            assert_eq!(output.status.code(), Some(code));
            let evidence = super::read_shell_exit_evidence(&path, code).unwrap();
            assert_eq!(!command_semantic(command, code, Some(&evidence)).0, success);
            if command.contains("head") { assert_eq!(evidence.pipeline_exit_codes, vec![141, 0]); }
            assert!(super::read_shell_exit_evidence(&path, code + 1).is_none());
        }
        assert!(command_semantic("grep pattern file", -1, None).0);
    }
}

fn ok(text: String, display: String) -> ToolOutcome {
    ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text,
        display,
        error: false,
        artifacts: Vec::new(),
        activation: None,
    }
}

fn ok_data(data: Value, display: String) -> ToolOutcome {
    ok(data.to_string(), display).with_data(data)
}

fn error_data(data: Value) -> ToolOutcome {
    let mut outcome = error(data.to_string()).with_data(data.clone());
    if data.get("tool_execution_skipped").and_then(Value::as_bool) == Some(true) {
        outcome.execution = Some(nola_host_protocol::tool_result::ExecutionReceipt {
            state:nola_host_protocol::tool_result::ExecutionState::NotStarted,
            ..Default::default()
        });
    }
    outcome
}

fn error_data_with_display(data: Value, display: String) -> ToolOutcome {
    let mut outcome = error_data(data);
    outcome.display = display;
    outcome
}

fn system_time(_args: &Map<String, Value>) -> ToolOutcome {
    let local = Local::now();
    let utc = Utc::now();
    let offset_seconds = local.offset().local_minus_utc();
    let sign = if offset_seconds < 0 { '-' } else { '+' };
    let absolute_offset = offset_seconds.unsigned_abs();
    let offset = format!(
        "{sign}{:02}:{:02}",
        absolute_offset / 3_600,
        (absolute_offset % 3_600) / 60
    );
    let payload = json!({
        "local_time": local.to_rfc3339(),
        "utc_time": utc.to_rfc3339(),
        "utc_offset": offset,
        "unix_milliseconds": utc.timestamp_millis(),
    });
    ok_data(payload.clone(),
        format!("{} ({offset})", local.to_rfc3339()),
    )
}

fn error(text: String) -> ToolOutcome {
    let display = localize_tool_error_display(&text);
    ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        display: truncate(&display.replace('\n', " "), 160),
        text,
        error: true,
        artifacts: Vec::new(),
        activation: None,
    }
}

fn error_with_display(text: String, display: String) -> ToolOutcome {
    ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        display: truncate(&display.replace('\n', " "), 160),
        text,
        error: true,
        artifacts: Vec::new(),
        activation: None,
    }
}

#[cfg(test)]
fn clamp_outcome(outcome: ToolOutcome) -> ToolOutcome {
    clamp_outcome_with_limit(outcome, MAX_TOOL_RESULT_CHARS)
}

#[cfg(test)]
fn clamp_outcome_with_limit(outcome: ToolOutcome, max_chars: usize) -> ToolOutcome {
    clamp_outcome_with_writer(outcome, max_chars, persist_large_tool_output)
}

fn tool_output_navigation_hint() -> String {
    // The path already lives in full_output_ref. Repeating it and the full
    // tool schema here can consume the entire bounded preview budget.
    "Partial evidence. Search full_output_ref with tool_output_search; read needed ranges with tool_output_read from preview_end_byte_offset, then next_offset. Exhaust pages only for exhaustive tasks; report gaps.".into()
}

/// Select execution diagnostics before any serialized-prefix truncation. The
/// saved payload is still exact JSON; selected fields are not a byte range of
/// that file, so they must never advertise a continuation byte offset.
fn project_shell_diagnostics(envelope: &mut Value, data: Option<&Value>, max_chars: usize) -> bool {
    let Some(data) = data.filter(|data| data["type"] == "shell_execution") else { return false; };
    let Some(object) = envelope.as_object_mut() else { return false; };
    object.remove("output_preview");
    object.remove("preview_end_byte_offset");
    object.remove("_nola_control");
    object.insert("preview_format".into(), json!("selected_shell_fields"));
    object.insert("continuation_hint".into(), json!("Selected diagnostics, not a contiguous file slice. Search full_output_ref with tool_output_search, or read JSON from offset 0 with tool_output_read. Do not rerun the command merely to recover omitted output."));
    for key in ["type", "state", "exit_code", "signal", "exit_evidence",
        "interruption_reason", "process_tree_stop", "stop_requested", "output_complete",
        "stdout_read_error", "stderr_read_error",
        "verification_status", "verification_error", "verification_message"] {
        if let Some(value) = data.get(key) { object.insert(key.into(), value.clone()); }
    }
    let stdout_unavailable = data["stdout"].is_null()
        && data.get("stdout_read_error").is_some_and(|v| !v.is_null());
    let stderr_unavailable = data["stderr"].is_null()
        && data.get("stderr_read_error").is_some_and(|v| !v.is_null());
    object.insert("stdout".into(), if stdout_unavailable { Value::Null } else { json!("") });
    object.insert("stderr".into(), if stderr_unavailable { Value::Null } else { json!("") });
    let overhead = envelope.to_string().chars().count();
    let available = max_chars.saturating_sub(overhead);
    let stdout = data["stdout"].as_str().unwrap_or_default();
    let stderr = data["stderr"].as_str().unwrap_or_default();
    fn encoded_len(text: &str) -> usize {
        serde_json::to_string(text).expect("JSON string").chars().count().saturating_sub(2)
    }
    fn bounded(text: &str, budget: usize) -> String {
        if encoded_len(text) <= budget { return text.into(); }
        let marker = "\n… omitted …\n";
        let available = budget.saturating_sub(encoded_len(marker));
        if budget < encoded_len(marker) { return String::new(); }
        let take = |reverse: bool, limit: usize| {
            let chars: Box<dyn Iterator<Item=char>> = if reverse { Box::new(text.chars().rev()) } else { Box::new(text.chars()) };
            let mut used = 0;
            let mut selected: Vec<char> = chars.take_while(|ch| {
                used += encoded_len(&ch.to_string()); used <= limit
            }).collect();
            if reverse { selected.reverse(); }
            selected.into_iter().collect::<String>()
        };
        format!("{}{marker}{}", take(false, available / 2), take(true, available - available / 2))
    }
    let stderr_budget = encoded_len(stderr).min(available / 2);
    let stdout_preview = bounded(stdout, available - stderr_budget);
    let stderr_preview = bounded(stderr, available.saturating_sub(encoded_len(&stdout_preview)));
    envelope["stdout"] = if stdout_unavailable {
        Value::Null
    } else { json!(stdout_preview) };
    envelope["stderr"] = if stderr_unavailable {
        Value::Null
    } else { json!(stderr_preview) };
    true
}

fn clamp_outcome_with_writer(
    mut outcome: ToolOutcome,
    max_chars: usize,
    persist: impl FnOnce(&str) -> std::io::Result<PathBuf>,
) -> ToolOutcome {
    // A short display string must not hide an oversized typed result. Persist
    // the actual payload before Core bounds it, so its reference remains useful.
    let ui_control = outcome.data.as_ref().and_then(|data| bounded_structured_output_preview(&data.to_string()))
        .or_else(|| bounded_structured_output_preview(&outcome.text));
    let browser_evidence = outcome.data.as_ref().and_then(|data| bounded_browser_evidence(data, max_chars / 4));
    outcome.text = nola_domain::tool_evidence::ModelToolEvidence::text(&outcome.text);
    let oversized_data = outcome.data.as_ref().map(|data|
        nola_domain::tool_evidence::ModelToolEvidence::from_host_output(data).into_value().to_string())
        .filter(|encoded| encoded.chars().count() > max_chars);
    let replace_oversized_data = oversized_data.is_some();
    if outcome.text.chars().count() > max_chars || oversized_data.is_some() {
        let original = oversized_data.unwrap_or_else(|| outcome.text.clone());
        let structured_preview = ui_control;
        let mut envelope = match persist(&original) {
            Ok(path) => {
                let path = path.to_string_lossy().into_owned();
                outcome.full_output_ref = Some(path.clone());
                let preview = original
                    .chars()
                    .take(max_chars.saturating_sub(500))
                    .collect::<String>();
                let preview_end_byte_offset = preview.len();
                json!({
                    "output_preview": preview,
                    "preview_end_byte_offset": preview_end_byte_offset,
                    "truncated": true,
                    "original_chars": original.chars().count(),
                    "original_bytes": original.len(),
                    "full_output_ref": path,
                    "_nola_control": structured_preview,
                    "continuation_hint": tool_output_navigation_hint(),
                })
            }
            Err(err) => {
                outcome.error = true;
                outcome.display = "Tool output preservation failed; execution may already have taken effect".into();
                json!({
                    "output_preview": original.chars().take(max_chars.saturating_sub(500)).collect::<String>(),
                    "truncated": true,
                    "full_output_available": false,
                    "original_chars": original.chars().count(),
                    "_nola_control": structured_preview,
                    "output_preservation_error": err.to_string(),
                    "recovery_hint": "The operation may already have taken effect. Inspect current state before retrying a mutation. Missing output is not evidence of no changes; do not claim complete coverage.",
                })
            }
        };
        if let Some(evidence) = browser_evidence { envelope["structured_preview"] = evidence; }
        // Include JSON escaping and navigation metadata in the initial preview
        // budget. Never mutate this projection on a subsequent model turn.
        if !project_shell_diagnostics(&mut envelope, outcome.data.as_ref(), max_chars) {
            let preview = envelope["output_preview"].as_str().unwrap_or_default().to_string();
            envelope["output_preview"] = json!("");
            let available = max_chars.saturating_sub(envelope.to_string().chars().count());
            let mut used = 0usize;
            let preview: String = preview.chars().take_while(|ch| {
                used = used.saturating_add(
                    serde_json::to_string(&ch.to_string()).expect("JSON character")
                        .chars().count().saturating_sub(2));
                used <= available
            }).collect();
            if envelope.get("preview_end_byte_offset").is_some() {
                envelope["preview_end_byte_offset"] = json!(preview.len());
            }
            envelope["output_preview"] = json!(preview);
        }
        // This envelope is Host-produced protocol data, not model/tool prose.
        // Keep it typed so Core sees output_preview/truncated directly instead
        // of {text:"{...}"}. Never decode arbitrary presentation text here.
        outcome.text = envelope.to_string();
        if outcome.data.is_none() || replace_oversized_data {
            outcome.data = Some(envelope);
        }
    }
    if full_output_ref_from_outcome(&outcome).is_some() {
        let navigation_capability_ids = [
            "tool_output_list",
            "tool_output_read",
            "tool_output_search",
            "tool_output_find",
        ];
        if let Some(activation) = outcome.activation.as_mut() {
            for capability_id in navigation_capability_ids {
                if !activation
                    .capability_ids
                    .iter()
                    .any(|existing| existing == capability_id)
                {
                    activation.capability_ids.push(capability_id.to_string());
                }
            }
        } else {
            outcome.activation = Some(nola_core::agent::host::HostCapabilityActivation {
                source: "full_output_ref".to_string(),
                capability_ids: navigation_capability_ids
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                capability_descriptors: Vec::new(),
            });
        }
    }
    outcome
}

#[cfg(test)]
#[test]
fn short_display_does_not_hide_large_typed_output_from_preservation() {
    let payload = json!({"text":"matches", "session_id":"fixture-session", "running":true,
        "rows":vec!["detail"; 2000]});
    let result = clamp_outcome_with_writer(ok_data(payload.clone(), "matches".into()), 4000, |text| {
        assert_eq!(serde_json::from_str::<Value>(text).unwrap(), payload);
        Ok(PathBuf::from("/fixture/saved-output.txt"))
    });
    assert_eq!(result.full_output_ref.as_deref(), Some("/fixture/saved-output.txt"));
    assert!(result.activation.is_some());
    let envelope: Value = serde_json::from_str(&result.text).unwrap();
    assert_eq!(envelope["truncated"], true);
    assert_eq!(result.data.as_ref(), Some(&envelope));
    assert!(result.data.as_ref().unwrap().get("rows").is_none());
    assert_eq!(envelope["_nola_control"]["session_id"], "fixture-session");
    assert_eq!(envelope["_nola_control"]["running"], true);
    assert!(result.text.chars().count() <= 4000);
}

#[cfg(test)]
#[test]
fn large_presentation_preserves_small_typed_control_data() {
    let data = json!({"session_id":"fixture-session", "running":true});
    let mut outcome = ok_data(data.clone(), "running".into());
    outcome.text = "large presentation".repeat(500);
    let result = clamp_outcome_with_writer(outcome, 1000, |_| {
        Ok(PathBuf::from("/fixture/saved-output.txt"))
    });
    assert_eq!(result.data, Some(data));
    assert_eq!(result.full_output_ref.as_deref(), Some("/fixture/saved-output.txt"));
}

#[cfg(test)]
#[test]
fn output_preservation_failure_cannot_masquerade_as_complete_success() {
    let result = clamp_outcome_with_writer(ok("x".repeat(3_000), "executed".into()), 1_000, |_| {
        Err(std::io::Error::other("fixture disk failure"))
    });
    assert!(result.error);
    assert!(full_output_ref_from_outcome(&result).is_none());
    assert!(result.activation.is_none());
    let output: Value = serde_json::from_str(&result.text).unwrap();
    assert_eq!(output["full_output_available"], false);
    assert_eq!(output["truncated"], true);
    assert!(output["recovery_hint"].as_str().unwrap().contains("before retrying"));
    assert_eq!(result.data, Some(output));
}

#[cfg(test)]
#[test]
fn large_text_output_is_a_typed_canonical_preview_without_double_json() {
    let original = "工作区修改\n".repeat(500);
    let outcome = clamp_outcome_with_writer(ok(original.clone(), "Workspace status".into()), 700, |text| {
        assert_eq!(text, original);
        Ok(PathBuf::from("/fixture/tool-output.txt"))
    });
    let envelope = outcome.data.as_ref().expect("typed preservation envelope");
    assert_eq!(envelope["truncated"], true);
    assert!(envelope.get("text").is_none());
    let preview = envelope["output_preview"].as_str().unwrap();
    assert!(!preview.is_empty(), "navigation metadata must leave room for evidence");
    assert!(outcome.text.chars().count() <= 700);
    let offset = envelope["preview_end_byte_offset"].as_u64().unwrap() as usize;
    assert_eq!(preview.as_bytes(), &original.as_bytes()[..offset]);
    assert_eq!(outcome.full_output_ref.as_deref(), Some("/fixture/tool-output.txt"));
    let core = nola_host_protocol::tool_result::ToolResult {
        contract: nola_host_protocol::tool_result::ToolResultContract::V1,
        presentation: nola_host_protocol::tool_result::ToolPresentation { text: outcome.text.clone(), summary: outcome.display.clone() },
        data: outcome.data.clone(),
        execution: Default::default(),
        full_output_ref: outcome.full_output_ref.clone(),
        memory_usage: None,
    }.into_core_output();
    assert_eq!(core["output_preview"], envelope["output_preview"]);
    assert_eq!(core["full_output_ref"], "/fixture/tool-output.txt");
    assert!(core.get("text").is_none());
}

#[cfg(test)]
#[test]
fn clamping_large_discovery_output_preserves_discovered_capabilities() {
    let outcome = ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text: "x".repeat(2_000),
        display: "large discovery".to_string(),
        error: false,
        artifacts: Vec::new(),
        activation: Some(nola_core::agent::host::HostCapabilityActivation {
            source: "tool_search".to_string(),
            capability_ids: vec!["codebase_search".to_string()],
            capability_descriptors: Vec::new(),
        }),
    };

    let bounded = clamp_outcome_with_limit(outcome, 1_000);
    let activation = bounded.activation.expect("typed activation");

    assert_eq!(activation.source, "tool_search");
    assert!(activation
        .capability_ids
        .iter()
        .any(|capability_id| capability_id == "codebase_search"));
    assert!(activation
        .capability_ids
        .iter()
        .any(|capability_id| capability_id == "tool_output_read"));
}

/// Keeps small control-plane fields available when a large structured tool
/// result is moved out of line. UI clients must not have to parse the saved
/// full payload merely to recover a browser address or session identity.
fn bounded_structured_output_preview(text: &str) -> Option<Value> {
    let Value::Object(object) = serde_json::from_str::<Value>(text).ok()? else {
        return None;
    };
    let mut preview = Map::new();
    for key in [
        "target",
        "session_id",
        "running",
        "persistent",
        "headless",
        "preview_url",
        "url",
        "final_url",
        "title",
        "active_index",
        "page_count",
        "revision",
    ] {
        if let Some(value) = object.get(key).filter(|value| {
            matches!(value, Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_))
        }) {
            preview.insert(key.to_string(), value.clone());
        }
    }
    if let Some(Value::Array(pages)) = object.get("pages") {
        let pages = pages
            .iter()
            .take(20)
            .filter_map(|page| {
                let Value::Object(page) = page else { return None };
                let mut bounded = Map::new();
                for key in ["index", "title", "url", "active"] {
                    if let Some(value) = page.get(key).filter(|value| {
                        matches!(value, Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_))
                    }) {
                        bounded.insert(key.to_string(), value.clone());
                    }
                }
                (!bounded.is_empty()).then_some(Value::Object(bounded))
            })
            .collect::<Vec<_>>();
        if !pages.is_empty() {
            preview.insert("pages".to_string(), Value::Array(pages));
        }
    }
    (!preview.is_empty()).then_some(Value::Object(preview))
}

/// A small actionable region, not another rendering of the full document.
/// The full output reference remains the source for unreturned regions.
fn bounded_browser_evidence(output: &Value, budget: usize) -> Option<Value> {
    let fields = output.as_object()?;
    let elements_key = if fields.contains_key("elements") { "elements" }
        else if fields.contains_key("interactive_elements") { "interactive_elements" }
        else { return None };
    let elements = fields.get(elements_key)?.as_array()?;
    let mut result = json!({"type":"browser_observation_region","elements":[],"elements_total":elements.len(),
        "elements_json_pointer":format!("/{elements_key}")});
    for key in ["target", "session_id", "generation", "revision", "url", "final_url", "title"] {
        if let Some(value) = fields.get(key) {
            let value = match value { Value::String(text) if key == "title" => json!(text.chars().take(256).collect::<String>()),
                Value::String(_) => value.clone(),
                Value::Number(_) | Value::Bool(_) | Value::Null => value.clone(), _ => continue };
            let mut candidate = result.clone(); candidate[key] = value;
            if candidate.to_string().chars().count() <= budget { result = candidate; }
        }
    }
    for element in elements {
        let Some(element) = element.as_object() else { continue };
        let mut entry = Map::new();
        for key in ["ref", "selector", "role", "name", "text", "disabled", "checked"] {
            if let Some(value) = element.get(key) {
                let value = match value { Value::String(text) if matches!(key, "name" | "text") => json!(text.chars().take(160).collect::<String>()),
                    Value::String(_) => value.clone(),
                    Value::Bool(_) | Value::Number(_) => value.clone(), _ => continue };
                entry.insert(key.into(), value);
            }
        }
        let mut candidate = result.clone();
        candidate["elements"].as_array_mut()?.push(Value::Object(entry));
        if candidate.to_string().chars().count() > budget { break; }
        result = candidate;
    }
    (result.to_string().chars().count() <= budget).then_some(result)
}

pub(crate) fn full_output_ref_from_outcome(outcome: &ToolOutcome) -> Option<String> {
    outcome.full_output_ref.clone()
}

fn bounded_code_intelligence_provenance(output: &Value) -> Option<Value> {
    let provenance = output.get("_nola_code_intelligence_provenance")?;
    let encoded = serde_json::to_string(provenance).ok()?;
    if encoded.chars().count() <= 2_000 {
        return Some(provenance.clone());
    }
    let object = provenance.as_object()?;
    let mut bounded = Map::new();
    for key in [
        "operation",
        "repository_root",
        "current_repository_revision",
        "provider_repository_revision",
        "freshness",
        "requires_current_state_verification",
        "reasons",
    ] {
        if let Some(value) = object.get(key) {
            bounded.insert(key.to_string(), value.clone());
        }
    }
    bounded.insert(
        "fact_count".to_string(),
        Value::from(
            object
                .get("facts")
                .and_then(Value::as_array)
                .map_or(0, Vec::len) as u64,
        ),
    );
    bounded.insert("facts_available_in_full_output".to_string(), Value::Bool(true));
    Some(Value::Object(bounded))
}

fn clamp_external_output_preview(envelope: &mut Value) {
    loop {
        if envelope.get("preview_end_byte_offset").is_some() {
            envelope["preview_end_byte_offset"] = json!(envelope["output_preview"]
                .as_str().map_or(0, str::len));
        }
        let encoded_chars = serde_json::to_string(envelope)
            .map_or(0, |value| value.chars().count());
        if encoded_chars <= MAX_TOOL_RESULT_CHARS {
            return;
        }
        let Some(preview) = envelope
            .get("output_preview")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return;
        };
        let preview_chars = preview.chars().count();
        if preview_chars == 0 {
            return;
        }
        let keep = preview_chars.saturating_sub(encoded_chars - MAX_TOOL_RESULT_CHARS);
        envelope["output_preview"] = Value::String(preview.chars().take(keep).collect());
    }
}

pub(crate) fn bound_external_tool_output_in_store(output: Value, store: &ToolOutputStore) -> (Value, Option<String>) {
    let code_intelligence_provenance = bounded_code_intelligence_provenance(&output);
    let evidence = nola_domain::tool_evidence::ModelToolEvidence::from_host_output(&output).into_value();
    let serialized = serde_json::to_string_pretty(&evidence).unwrap_or_else(|_| evidence.to_string());
    let character_count = serialized.chars().count();
    if character_count <= MAX_TOOL_RESULT_CHARS {
        return (output, None);
    }
    match store.persist(&serialized) {
        Ok(path) => {
            let path = path.to_string_lossy().into_owned();
            let provenance_chars = code_intelligence_provenance
                .as_ref()
                .and_then(|value| serde_json::to_string(value).ok())
                .map_or(0, |value| value.chars().count());
            let preview = serialized
                .chars()
                .take(MAX_TOOL_RESULT_CHARS.saturating_sub(500 + provenance_chars))
                .collect::<String>();
            let mut bounded = json!({
                "output_preview": preview,
                "preview_end_byte_offset": preview.len(),
                "truncated": true,
                "original_chars": character_count,
                "original_bytes": serialized.len(),
                "full_output_ref": path,
                "continuation_hint": tool_output_navigation_hint(),
            });
            if let Some(region) = bounded_browser_evidence(&evidence, MAX_TOOL_RESULT_CHARS / 4) {
                bounded["structured_preview"] = region;
            }
            if let (Some(provenance), Some(object)) =
                (code_intelligence_provenance, bounded.as_object_mut())
            {
                object.insert("_nola_code_intelligence_provenance".to_string(), provenance);
            }
            if !project_shell_diagnostics(&mut bounded, Some(&output), MAX_TOOL_RESULT_CHARS) {
                clamp_external_output_preview(&mut bounded);
            }
            (bounded, Some(path))
        }
        Err(_) => {
            let provenance_chars = code_intelligence_provenance
                .as_ref()
                .and_then(|value| serde_json::to_string(value).ok())
                .map_or(0, |value| value.chars().count());
            let mut bounded = json!({
                "output_preview": serialized
                    .chars()
                    .take(MAX_TOOL_RESULT_CHARS.saturating_sub(500 + provenance_chars))
                    .collect::<String>(),
                "truncated": true,
                "original_chars": character_count,
                "original_bytes": serialized.len(),
                "full_output_available": false,
                "output_preservation_error": "tool_output_preservation_failed",
                "recovery_hint": "The operation may already have taken effect. Inspect current state before retrying a mutation. Missing output is not evidence of no changes; do not claim complete coverage.",
            });
            if let (Some(mut provenance), Some(object)) =
                (code_intelligence_provenance, bounded.as_object_mut())
            {
                if let Some(provenance) = provenance.as_object_mut() {
                    provenance.insert(
                        "facts_available_in_full_output".to_string(),
                        Value::Bool(false),
                    );
                }
                object.insert("_nola_code_intelligence_provenance".to_string(), provenance);
            }
            if !project_shell_diagnostics(&mut bounded, Some(&output), MAX_TOOL_RESULT_CHARS) {
                clamp_external_output_preview(&mut bounded);
            }
            (bounded, None)
        }
    }
}

#[cfg(test)]
#[test]
fn shell_unknown_snapshot_preserves_read_failure_and_receipt_through_bounding() {
    let data = json!({"type":"shell_execution", "state":"unknown",
        "interruption_reason":"timeout", "process_tree_stop":"unknown",
        "stop_requested":true, "output_complete":false,
        "stdout":"中😀\n".repeat(4_000), "stderr":null,
        "stderr_read_error":"capture unavailable"});
    let mut input = error_with_display("timed out".into(), "timed out".into());
    input.data = Some(data.clone());
    input.execution = Some(nola_host_protocol::tool_result::ExecutionReceipt {
        state:nola_host_protocol::tool_result::ExecutionState::Unknown,
        ..Default::default()
    });
    let result = clamp_outcome_with_writer(input, 2000, |saved| {
        assert_eq!(serde_json::from_str::<Value>(saved).unwrap(), data);
        Ok(PathBuf::from("/fixture/interrupted-command.json"))
    });
    assert!(result.error);
    assert!(result.text.chars().count() <= 2000);
    assert_eq!(result.execution.unwrap().state, nola_host_protocol::tool_result::ExecutionState::Unknown);
    let output = result.data.unwrap();
    assert_eq!(output["state"], "unknown");
    assert_eq!(output["process_tree_stop"], "unknown");
    assert_eq!(output["output_complete"], false);
    assert!(output["stderr"].is_null());
    assert_eq!(output["stderr_read_error"], "capture unavailable");
    assert_eq!(output["full_output_ref"], "/fixture/interrupted-command.json");
    assert!(output.get("preview_end_byte_offset").is_none());
}

#[cfg(test)]
#[test]
fn shell_projection_preserves_diagnostics_before_host_and_core_bounding() {
    let data = json!({"type":"shell_execution", "state":"completed", "exit_code":1,
        "exit_evidence":{"exit_code":1,"pipeline_exit_codes":[0,1]},
        "command":"irrelevant command source".repeat(2000),
        "stdout":format!("output-start\n{}\noutput-end", "中间内容\\\"\n".repeat(2000)),
        "stderr":"actual failure diagnostic", "signal":null});
    let result = clamp_outcome_with_writer(ok_data(data.clone(), "command failed".into()), 2000, |saved| {
        assert_eq!(serde_json::from_str::<Value>(saved).unwrap(), data);
        Ok(PathBuf::from("/fixture/exact-command.json"))
    });
    let output = result.data.as_ref().unwrap();
    assert!(result.text.chars().count() <= 2000);
    assert_eq!(output["exit_code"], 1);
    assert_eq!(output["exit_evidence"]["pipeline_exit_codes"], json!([0,1]));
    assert_eq!(output["stderr"], "actual failure diagnostic");
    let stdout = output["stdout"].as_str().unwrap();
    assert!(stdout.starts_with("output-start"));
    assert!(stdout.ends_with("output-end"));
    assert!(output.get("command").is_none());
    assert!(output.get("preview_end_byte_offset").is_none());
    let core = nola_host_protocol::tool_result::ToolResult {
        contract: nola_host_protocol::tool_result::ToolResultContract::V1,
        presentation: nola_host_protocol::tool_result::ToolPresentation {text:result.text, summary:result.display},
        data:result.data, execution:Default::default(), full_output_ref:result.full_output_ref, memory_usage:None,
    }.into_core_output();
    assert_eq!(core["stderr"], "actual failure diagnostic");
    assert_eq!(core["full_output_ref"], "/fixture/exact-command.json");
}

#[cfg(test)]
#[test]
fn external_shell_projection_and_preservation_failure_keep_diagnostics() {
    let root = tempfile::tempdir().unwrap();
    let data = json!({"type":"shell_execution","exit_code":0,"state":"completed",
        "command":"large script".repeat(2000),"stdout":"constraint still fails", "stderr":""});
    let store = ToolOutputStore::new(root.path().join("output"));
    let (bounded, reference) = bound_external_tool_output_in_store(data.clone(), &store);
    assert!(reference.is_some());
    assert_eq!(bounded["stdout"], "constraint still fails");
    assert!(bounded.get("preview_end_byte_offset").is_none());
    assert!(bounded.to_string().chars().count() <= MAX_TOOL_RESULT_CHARS);
    let failed = clamp_outcome_with_writer(ok_data(data, "executed".into()), 2000, |_| Err(std::io::Error::other("fixture")));
    assert!(failed.error);
    assert_eq!(failed.data.as_ref().unwrap()["stdout"], "constraint still fails");
    assert_eq!(failed.data.as_ref().unwrap()["full_output_available"], false);
}

#[cfg(test)]
#[test]
fn external_output_preservation_failure_is_not_evidence_of_no_effect() {
    // A file cannot be the store directory; this is deterministic even when
    // tests run as a privileged user and does not alter the real output store.
    let unavailable_root = tempfile::NamedTempFile::new().unwrap();
    let store = ToolOutputStore::new(unavailable_root.path().to_path_buf());
    let (bounded, reference) = bound_external_tool_output_in_store(json!({
        "content": "x".repeat(MAX_TOOL_RESULT_CHARS + 1),
        "_nola_code_intelligence_provenance": {
            "freshness": "requires_verification",
            "requires_current_state_verification": true,
            "facts_available_in_full_output": true
        }
    }), &store);

    assert!(reference.is_none());
    assert!(bounded.get("full_output_ref").is_none());
    assert!(bounded.get("preview_end_byte_offset").is_none());
    assert_eq!(bounded["truncated"], true);
    assert_eq!(bounded["full_output_available"], false);
    assert_eq!(bounded["output_preservation_error"], "tool_output_preservation_failed");
    assert_eq!(bounded["_nola_code_intelligence_provenance"]["facts_available_in_full_output"], false);
    assert_eq!(bounded["_nola_code_intelligence_provenance"]["requires_current_state_verification"], true);
    assert!(bounded["recovery_hint"].as_str().unwrap().contains("before retrying a mutation"));
    assert!(!bounded["output_preview"].as_str().unwrap().is_empty());
    assert!(bounded.to_string().chars().count() <= MAX_TOOL_RESULT_CHARS);
}

#[cfg(test)]
#[test]
fn bounded_code_intelligence_output_preserves_a_bounded_freshness_contract() {
    let facts = (0..64)
        .map(|index| json!({
            "repository_root": "/workspace",
            "path": format!("src/very/long/module/{index}/implementation.rs"),
            "symbol": format!("crate::module::{index}::implementation_symbol"),
            "path_exists": true,
            "current_content_sha256": "a".repeat(64),
            "provider_content_sha256": "a".repeat(64),
            "evidence_matches_current_source": true,
            "symbol_matches_current_source": true,
        }))
        .collect::<Vec<_>>();
    let (bounded, full_output_ref) = bound_external_tool_output(json!({
        "matches": [{"path": "src/lib.rs", "snippet": "x".repeat(MAX_TOOL_RESULT_CHARS + 1)}],
        "_nola_code_intelligence_provenance": {
            "operation": "search",
            "repository_root": "/workspace",
            "current_repository_revision": "current",
            "provider_repository_revision": "provider",
            "facts": facts,
            "freshness": "requires_verification",
            "requires_current_state_verification": true,
            "reasons": ["provider_snapshot_is_not_verifiably_current"]
        },
    }));

    let provenance = &bounded["_nola_code_intelligence_provenance"];
    assert_eq!(provenance["freshness"], "requires_verification");
    assert_eq!(provenance["requires_current_state_verification"], true);
    assert_eq!(provenance["fact_count"], 64);
    assert_eq!(provenance["facts_available_in_full_output"], true);
    assert!(provenance.get("facts").is_none());
    assert_eq!(bounded["truncated"], true);
    assert!(full_output_ref.is_some());
    assert!(serde_json::to_string(&bounded).unwrap().chars().count() <= MAX_TOOL_RESULT_CHARS);
}

#[cfg(test)]
#[test]
fn web_search_result_limit_preserves_a_model_ready_report() {
    let report = ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text: "e".repeat(6_000),
        display: "search".to_string(),
        error: false,
        artifacts: Vec::new(),
        activation: None,
    };
    let web = clamp_outcome_with_limit(report.clone(), MAX_WEB_SEARCH_RESULT_CHARS);
    let generic = clamp_outcome(report);
    assert_eq!(web.text.chars().count(), 6_000);
    assert!(web.full_output_ref.is_none());
    assert!(generic.full_output_ref.is_some());
    assert_eq!(generic.data.as_ref().unwrap()["truncated"], true);
    assert_eq!(generic.data.as_ref().unwrap()["full_output_ref"].as_str(), generic.full_output_ref.as_deref());
}

#[cfg(test)]
#[test]
fn web_fetch_uses_the_inline_web_evidence_budget() {
    let fetched_page = ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text: "e".repeat(6_000),
        display: "fetch".to_string(),
        error: false,
        artifacts: Vec::new(),
        activation: None,
    };
    let web = clamp_outcome_with_limit(fetched_page.clone(), MAX_WEB_SEARCH_RESULT_CHARS);
    let generic = clamp_outcome(fetched_page);
    assert_eq!(web.text.chars().count(), 6_000);
    assert!(web.full_output_ref.is_none());
    assert!(generic.full_output_ref.is_some());
    assert_eq!(generic.data.as_ref().unwrap()["truncated"], true);
    assert_eq!(generic.data.as_ref().unwrap()["full_output_ref"].as_str(), generic.full_output_ref.as_deref());
}

#[cfg(test)]
#[test]
fn large_external_tool_output_has_a_typed_full_output_reference() {
    let original = json!({"content": "中文\n\"".repeat(MAX_TOOL_RESULT_CHARS)});
    let serialized = serde_json::to_string_pretty(&original).unwrap();
    let (bounded, full_output_ref) = bound_external_tool_output(original);

    assert_eq!(bounded["truncated"], true);
    assert_eq!(
        bounded["full_output_ref"].as_str(),
        full_output_ref.as_deref()
    );
    assert!(full_output_ref.is_some());
    let preview = bounded["output_preview"].as_str().unwrap();
    let offset = bounded["preview_end_byte_offset"].as_u64().unwrap() as usize;
    assert_eq!(preview.as_bytes(), &serialized.as_bytes()[..offset]);
    assert_eq!(bounded["original_bytes"], serialized.len());
    assert!(bounded.to_string().chars().count() <= MAX_TOOL_RESULT_CHARS);
}

#[cfg(test)]
#[test]
fn external_preview_offset_tracks_post_escape_budget_trimming() {
    let original = "中文\"\n".repeat(MAX_TOOL_RESULT_CHARS);
    let mut envelope = json!({"output_preview":original,
        "preview_end_byte_offset":original.len(), "truncated":true});
    clamp_external_output_preview(&mut envelope);
    let preview = envelope["output_preview"].as_str().unwrap();
    assert!(preview.len() < original.len());
    let offset = envelope["preview_end_byte_offset"].as_u64().unwrap() as usize;
    assert_eq!(preview.as_bytes(), &original.as_bytes()[..offset]);
    assert!(envelope.to_string().chars().count() <= MAX_TOOL_RESULT_CHARS);
}

#[cfg(test)]
#[test]
fn clamped_browser_output_preserves_preview_control_plane_fields() {
    let outcome = clamp_outcome(ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text: json!({
            "accessibility": "x".repeat(MAX_TOOL_RESULT_CHARS + 1_000),
            "target": "managed",
            "session_id": "nola-conversation-1",
            "preview_url": "http://127.0.0.1:4321/view?token=test",
            "final_url": "https://example.com/current",
            "title": "Current page",
            "pages": [{
                "index": 0,
                "title": "Current page",
                "url": "https://example.com/current",
                "active": true
            }]
        })
        .to_string(),
        display: "browser_managed_navigate: Current page".to_string(),
        error: false,
        artifacts: Vec::new(),
        activation: None,
    });
    let value: Value = serde_json::from_str(&outcome.text).expect("bounded JSON");

    assert_eq!(value["truncated"], true);
    assert!(
        outcome
            .text
            .chars()
            .take(1_000)
            .collect::<String>()
            .contains("http://127.0.0.1:4321/view?token=test"),
        "control-plane metadata must survive a second bounded Core preview"
    );
    assert_eq!(
        value["_nola_control"]["preview_url"],
        "http://127.0.0.1:4321/view?token=test"
    );
    assert_eq!(
        value["_nola_control"]["final_url"],
        "https://example.com/current"
    );
    assert_eq!(value["_nola_control"]["pages"][0]["active"], true);
}

#[cfg(test)]
#[test]
fn clamped_local_output_exposes_the_saved_path_structurally() {
    let outcome = clamp_outcome(ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text: "x".repeat(MAX_TOOL_RESULT_CHARS + 1_000),
        display: "large".to_string(),
        error: false,
        artifacts: Vec::new(),
        activation: None,
    });

    let full_output_ref = full_output_ref_from_outcome(&outcome).expect("saved output path");
    assert_eq!(serde_json::from_str::<Value>(&outcome.text).unwrap()["full_output_ref"], full_output_ref);
    let activation = outcome.activation.expect("output continuation activation");
    assert_eq!(activation.source, "full_output_ref");
    assert!(activation
        .capability_ids
        .contains(&"tool_output_search".to_string()));
}

#[cfg(test)]
#[test]
fn capability_activation_is_structured_and_not_embedded_in_tool_text() {
    let outcome = ok(
        "model-visible tool description".to_string(),
        "1 match".to_string(),
    )
    .with_capability_activation(vec!["workspace_status".to_string()]);
    assert!(!outcome.text.contains("nola_capability_activation:"));
    let activation = outcome.activation.expect("typed capability activation");
    assert_eq!(activation.source, "tool_search");
    assert_eq!(activation.capability_ids, vec!["workspace_status"]);
}

#[cfg(test)]
fn persist_large_tool_output(text: &str) -> std::io::Result<PathBuf> {
    output_store::ToolOutputStore::new(tool_output_root()).persist(text)
}

#[cfg(test)]
fn bound_external_tool_output(output: Value) -> (Value, Option<String>) {
    bound_external_tool_output_in_store(output, &ToolOutputStore::for_test())
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out = text
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    out.push('…');
    out
}

#[cfg(test)]
mod permission_transport_input_tests {
    use super::*;
    use nola_host_protocol::PermissionTransportInterruptionReason::{Disconnected, Unavailable};

    #[test]
    fn bounded_native_permission_reader_preserves_explicit_deny_and_reports_disconnect() {
        assert_eq!(read_native_permission_response(io::Cursor::new(b"")), Err(Disconnected));
        assert_eq!(read_native_permission_response(io::Cursor::new(b"broken json\n")), Err(Unavailable));
        let denied = read_native_permission_response(io::Cursor::new(
            b"{\"resolution\":{\"type\":\"authorization\",\"decision\":\"deny\"}}\n"
        )).unwrap();
        assert_eq!(denied.pointer("/resolution/decision").and_then(Value::as_str), Some("deny"));
        let mut reader = io::Cursor::new(vec![b'x'; 1_000_000]);
        assert_eq!(read_native_permission_response(&mut reader), Err(Unavailable));
        assert_eq!(reader.position(), 32 * 1024 + 1);
        struct BrokenReader;
        impl io::Read for BrokenReader {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "isolated broken transport"))
            }
        }
        assert_eq!(read_native_permission_response(io::BufReader::new(BrokenReader)), Err(Disconnected));
    }

    #[test]
    fn resolver_transport_interruption_reaches_core_and_legacy_error_is_not_denial() {
        let request = ToolRequest { id: "pending-request".into(), name: "write_file".into(), args: Default::default() };
        let resolver: HostPermissionResolver = std::sync::Arc::new(|_, _, _| HostPermissionResolution::TransportInterrupted(Disconnected));
        assert_eq!(present_agent_core_permission_with_resolver(&request, Some(resolver), &[], true),
            HostPermissionResolution::TransportInterrupted(Disconnected));
        let resolver_guard = enter_host_permission_resolver(
            std::sync::Arc::new(|_, _, _| HostPermissionResolution::TransportInterrupted(Disconnected))
        );
        let direct = confirm_tool(&request);
        drop(resolver_guard);
        assert_eq!(direct, ToolPermissionDecision::TransportInterrupted(Disconnected));
        let outcome = permission_transport_interrupted_outcome(Disconnected);
        assert!(outcome.error);
        assert!(outcome.text.contains("transport interrupted"));
        assert!(!outcome.text.contains("denied by user"));
    }
}

#[cfg(test)]
mod controlled_walk_regressions {
    use super::*;
    #[test]
    fn cancellation_stops_before_visiting_and_generated_directories_are_skipped() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("target")).unwrap();
        fs::write(root.path().join("target/generated.rs"), "x").unwrap();
        fs::write(root.path().join("source.rs"), "x").unwrap();
        let mut visited = 0;
        walk_files_controlled(root.path(), 10, &mut |_| { visited += 1; true }, &|| true);
        assert_eq!(visited, 0);
        walk_files(root.path(), 10, &mut |_| { visited += 1; true });
        assert_eq!(visited, 1);
    }
    #[test]
    fn cancelled_code_search_returns_a_failure_not_partial_success() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("source.rs"), "fn source() {}").unwrap();
        let control = crate::runtime_control::RuntimeExecutionControl::default();
        control.request();
        assert!(code_search_matches(root.path(), "source", "text", &glob_regex("**").unwrap(), "", false, false, 0, "", 10, &control).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn directory_symlink_cycle_does_not_escape_or_repeat() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("source.rs"), "x").unwrap();
        std::os::unix::fs::symlink(root.path(), root.path().join("cycle")).unwrap();
        let mut visited = 0;
        walk_files(root.path(), 10, &mut |_| { visited += 1; true });
        assert_eq!(visited, 1);
    }

    #[test]
    fn file_mutation_permission_prompts_show_readable_bounded_diffs_and_mask_secrets() {
        let mut edit_args = Map::new();
        edit_args.insert("path".into(), Value::String("src/lib.rs".into()));
        edit_args.insert("old_text".into(), Value::String("let a = 1;\nlet token = \"sk-ant-api03-secretvalue1234567890\";".into()));
        edit_args.insert("new_text".into(), Value::String("let a = 2;\nlet b = 3;".into()));
        let edit_req = ToolRequest {
            id: "edit-1".into(),
            name: "edit_file".into(),
            args: edit_args,
        };
        let prompt = tool_permission_prompt(&edit_req);
        assert!(prompt.rows.iter().any(|(_, v)| v == "src/lib.rs"));
        let diff = prompt.rows.iter().find(|(_, v)| v.contains("- let a = 1;")).expect("diff row");
        assert!(diff.1.contains("+ let a = 2;"));
        assert!(!diff.1.contains("sk-ant-api03-secretvalue1234567890"));
        assert!(builtin_permission_review_text(&edit_req).is_none());

        let long_body = (1..=60)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut write_args = Map::new();
        write_args.insert("path".into(), Value::String("long.txt".into()));
        write_args.insert("content".into(), Value::String(long_body));
        let write_req = ToolRequest {
            id: "write-1".into(),
            name: "write_file".into(),
            args: write_args,
        };
        let write_prompt = tool_permission_prompt(&write_req);
        let preview = write_prompt
            .rows
            .iter()
            .find(|(_, v)| v.contains("+ line 1"))
            .expect("preview row");
        assert!(preview.1.contains("20"));
        assert!(preview.1.lines().count() <= FILE_DIFF_PREVIEW_MAX_LINES + 1);
    }

    #[test]
    fn should_redact_approval_arg_keeps_credentials_and_allows_file_diffs() {
        assert!(!should_redact_approval_arg("write_file", "content"));
        assert!(!should_redact_approval_arg("write_file", "json_content"));
        assert!(!should_redact_approval_arg("append_file", "content"));
        assert!(!should_redact_approval_arg("edit_file", "old_text"));
        assert!(!should_redact_approval_arg("edit_file", "new_text"));
        assert!(!should_redact_approval_arg("edit_file_lines", "new_text"));
        assert!(!should_redact_approval_arg("replace_in_files", "old_text"));
        assert!(!should_redact_approval_arg("replace_in_files", "new_text"));
        assert!(!should_redact_approval_arg("apply_patch", "patch"));
        assert!(should_redact_approval_arg("http_request", "api_key"));
        assert!(should_redact_approval_arg("http_request", "authorization"));
        assert!(should_redact_approval_arg("http_request", "bearer_token"));
        assert!(should_redact_approval_arg("http_request", "client_secret"));
        assert!(should_redact_approval_arg("http_request", "password"));
        assert!(should_redact_approval_arg("chrome_native_type_text", "text"));
    }
}
