// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Shared prompt and structured tool-call parsing for text-only models.
// - Security: Only catalog-listed tools are accepted; model permission claims are discarded.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr;
use uuid::Uuid;

const TOOL_CALL_OPEN: &str = "<nola_tool_call>";
const TOOL_CALL_CLOSE: &str = "</nola_tool_call>";
const DSML_TOOL_CALLS_OPEN: &str = "<｜｜DSML｜｜tool_calls>";
const CANONICAL_TOOL_CALLS_OPEN: &str = "<tool_calls>";
const MAX_MODEL_OUTPUT_BYTES: usize = 2_000_000;
const MAX_TOOL_ARGUMENT_BYTES: usize = 64_000;
const MAX_TOOL_CALLS: usize = 16;

/// Provider-neutral DTO for native or canonical tagged tool calls returned to
/// a platform model adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentModelToolCall {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub permission: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentModelParseInput {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub reasoning: String,
    #[serde(default)]
    pub native_tool_calls: Vec<AgentModelToolCall>,
    #[serde(default)]
    pub allowed_tool_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentModelParseResult {
    pub content: String,
    pub reasoning: String,
    pub tool_calls: Vec<AgentModelToolCall>,
    pub finish_reason: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentModelContinuationInput {
    #[serde(default)]
    pub accumulated: String,
    #[serde(default)]
    pub continuation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentModelContinuationResult {
    pub append: String,
}

/// Returns only the suffix that a Host should append to an already presented
/// model response. The policy is shared by every platform so reconnects do not
/// duplicate prose or reopen an unfinished Markdown fence differently.
pub fn stitch_agent_model_continuation(accumulated: &str, continuation: &str) -> String {
    if accumulated.is_empty() || continuation.is_empty() {
        return continuation.to_string();
    }

    let (candidate, stripped_reopened_fence) =
        if let Some((marker, minimum_count)) = unclosed_markdown_fence(accumulated) {
            strip_reopened_markdown_fence(continuation, marker, minimum_count)
                .map(|value| (value, true))
                .unwrap_or((continuation, false))
        } else {
            (continuation, false)
        };
    let overlap = longest_suffix_prefix_overlap(accumulated, candidate, 4_096);
    if overlap >= 16 {
        return candidate.chars().skip(overlap).collect();
    }
    if stripped_reopened_fence
        && !accumulated.chars().last().is_some_and(char::is_whitespace)
        && !candidate.chars().next().is_some_and(char::is_whitespace)
    {
        return format!("\n{candidate}");
    }
    candidate.to_string()
}

fn unclosed_markdown_fence(content: &str) -> Option<(char, usize)> {
    let mut open_fence: Option<(char, usize)> = None;
    for line in content.lines() {
        let trimmed = line.trim_start();
        let Some(marker) = trimmed.chars().next() else {
            continue;
        };
        if marker != '`' && marker != '~' {
            continue;
        }
        let marker_count = trimmed
            .chars()
            .take_while(|character| *character == marker)
            .count();
        if marker_count < 3 {
            continue;
        }
        let suffix = trimmed.chars().skip(marker_count).collect::<String>();
        match open_fence {
            None => open_fence = Some((marker, marker_count)),
            Some((open_marker, open_count))
                if marker == open_marker
                    && marker_count >= open_count
                    && suffix.trim().is_empty() =>
            {
                open_fence = None;
            }
            _ => {}
        }
    }
    open_fence
}

fn strip_reopened_markdown_fence(
    continuation: &str,
    marker: char,
    minimum_count: usize,
) -> Option<&str> {
    let without_blank_prefix =
        continuation.trim_start_matches(|value| value == '\r' || value == '\n');
    let line_end = without_blank_prefix
        .find('\n')
        .unwrap_or(without_blank_prefix.len());
    let first_line = &without_blank_prefix[..line_end];
    let indentation = first_line.chars().take_while(|value| *value == ' ').count();
    if indentation > 3 {
        return None;
    }
    let content = &first_line[indentation..];
    let marker_count = content.chars().take_while(|value| *value == marker).count();
    if marker_count < minimum_count {
        return None;
    }
    let rest_start = if line_end < without_blank_prefix.len() {
        line_end + 1
    } else {
        line_end
    };
    Some(&without_blank_prefix[rest_start..])
}

fn longest_suffix_prefix_overlap(left: &str, right: &str, maximum_chars: usize) -> usize {
    let left_chars = left.chars().collect::<Vec<_>>();
    let right_chars = right.chars().collect::<Vec<_>>();
    let maximum = left_chars.len().min(right_chars.len()).min(maximum_chars);
    (1..=maximum)
        .rev()
        .find(|length| left_chars[left_chars.len() - length..] == right_chars[..*length])
        .unwrap_or(0)
}

pub fn parse_agent_model_output(input: AgentModelParseInput) -> AgentModelParseResult {
    let allowed = input
        .allowed_tool_names
        .iter()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect::<HashSet<_>>();
    let mut warnings = Vec::new();

    if input.content.len() > MAX_MODEL_OUTPUT_BYTES {
        warnings.push("model output exceeded the mobile size limit".to_string());
        return AgentModelParseResult {
            content: input.content.chars().take(MAX_MODEL_OUTPUT_BYTES).collect(),
            reasoning: input.reasoning,
            tool_calls: Vec::new(),
            finish_reason: "invalid_tool_output".to_string(),
            warnings,
        };
    }

    let (content, candidates) = if input.native_tool_calls.is_empty() {
        text_protocol_tool_calls(&input.content, &mut warnings).unwrap_or_else(|| {
            let tagged = tagged_tool_calls(&input.content, &mut warnings);
            if tagged.1.is_empty() {
                bare_json_tool_calls(&input.content, &allowed).unwrap_or(tagged)
            } else {
                tagged
            }
        })
    } else {
        (input.content, input.native_tool_calls)
    };
    let mut tool_calls = Vec::new();
    for mut call in candidates.into_iter().take(MAX_TOOL_CALLS) {
        let name = call.name.trim();
        if name.is_empty() {
            warnings.push("ignored a tool call with no name".to_string());
            continue;
        }
        if !allowed.contains(name) {
            warnings.push(format!("ignored unavailable tool '{name}'"));
            continue;
        }
        if !call.arguments.is_object() {
            warnings.push(format!(
                "ignored tool '{name}' because arguments were not an object"
            ));
            continue;
        }
        if serde_json::to_vec(&call.arguments)
            .map(|bytes| bytes.len() > MAX_TOOL_ARGUMENT_BYTES)
            .unwrap_or(true)
        {
            warnings.push(format!(
                "ignored tool '{name}' because arguments were too large"
            ));
            continue;
        }
        if call.id.trim().is_empty() {
            call.id = Uuid::new_v4().to_string();
        }
        call.name = name.to_string();
        call.permission = None;
        tool_calls.push(call);
    }
    if tool_calls.len() == MAX_TOOL_CALLS {
        warnings.push("tool call limit reached".to_string());
    }

    AgentModelParseResult {
        content: content.trim().to_string(),
        reasoning: input.reasoning,
        finish_reason: if tool_calls.is_empty() {
            "stop".to_string()
        } else {
            "tool_calls".to_string()
        },
        tool_calls,
        warnings,
    }
}

fn text_protocol_tool_calls(
    content: &str,
    warnings: &mut Vec<String>,
) -> Option<(String, Vec<AgentModelToolCall>)> {
    let markers = [DSML_TOOL_CALLS_OPEN, CANONICAL_TOOL_CALLS_OPEN];
    let position = markers
        .iter()
        .filter_map(|marker| content.find(marker))
        .min()?;
    let normalized = content[position..].replace("｜｜DSML｜｜", "");
    let calls = parse_canonical_text_tool_calls(normalized.trim());
    match calls {
        Some(calls) => Some((content[..position].trim_end().to_string(), calls)),
        None => {
            warnings.push("ignored malformed text tool-call protocol".to_string());
            None
        }
    }
}

fn parse_canonical_text_tool_calls(envelope: &str) -> Option<Vec<AgentModelToolCall>> {
    let inner = envelope
        .strip_prefix(CANONICAL_TOOL_CALLS_OPEN)?
        .strip_suffix("</tool_calls>")?;
    let mut rest = inner;
    let mut calls = Vec::new();
    while !rest.trim().is_empty() {
        rest = rest.trim_start();
        let (call_open, call_close) = if rest.starts_with("<invoke") {
            ("<invoke", "</invoke>")
        } else if rest.starts_with("<tool") && !rest.starts_with("<tool_calls") {
            ("<tool", "</tool>")
        } else {
            return None;
        };
        rest = &rest[rest.find(call_open)?..];
        let tag_end = rest.find('>')?;
        let name = xml_attribute(&rest[..=tag_end], "name")?;
        if name.trim().is_empty() {
            return None;
        }
        let after_tag = &rest[tag_end + 1..];
        let call_end = after_tag.find(call_close)?;
        let mut parameter_rest = &after_tag[..call_end];
        let mut arguments = serde_json::Map::new();
        while !parameter_rest.trim().is_empty() {
            parameter_rest = parameter_rest.trim_start();
            let parameter_start = parameter_rest.find("<parameter")?;
            if !parameter_rest[..parameter_start].trim().is_empty() {
                return None;
            }
            parameter_rest = &parameter_rest[parameter_start..];
            let parameter_tag_end = parameter_rest.find('>')?;
            let parameter_tag = &parameter_rest[..=parameter_tag_end];
            let parameter_name = xml_attribute(parameter_tag, "name")?;
            let value_start = &parameter_rest[parameter_tag_end + 1..];
            let parameter_end = value_start.find("</parameter>")?;
            let raw_value = xml_unescape(value_start[..parameter_end].trim());
            let force_string = xml_attribute(parameter_tag, "string")
                .map(|value| value.eq_ignore_ascii_case("true"))
                .unwrap_or(true);
            let value = if force_string {
                Value::String(raw_value)
            } else {
                nola_kernel::strict_json::decode(raw_value.as_bytes()).ok()?
            };
            if arguments.insert(parameter_name, value).is_some() { return None; }
            parameter_rest = &value_start[parameter_end + "</parameter>".len()..];
        }
        calls.push(AgentModelToolCall {
            id: format!("call_text_{}", calls.len()),
            name,
            arguments: Value::Object(arguments),
            permission: None,
        });
        rest = &after_tag[call_end + call_close.len()..];
    }
    (!calls.is_empty()).then_some(calls)
}

fn xml_attribute(tag: &str, name: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let prefix = format!("{name}={quote}");
        if let Some(attribute_start) = tag.find(&prefix) {
            let start = attribute_start + prefix.len();
            let end = tag[start..].find(quote)? + start;
            return Some(xml_unescape(&tag[start..end]));
        }
    }
    None
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn tagged_tool_calls(
    content: &str,
    warnings: &mut Vec<String>,
) -> (String, Vec<AgentModelToolCall>) {
    let mut visible = String::new();
    let mut calls = Vec::new();
    let mut cursor = 0;
    while let Some(relative_start) = content[cursor..].find(TOOL_CALL_OPEN) {
        let start = cursor + relative_start;
        visible.push_str(&content[cursor..start]);
        let payload_start = start + TOOL_CALL_OPEN.len();
        let Some(relative_end) = content[payload_start..].find(TOOL_CALL_CLOSE) else {
            warnings.push("ignored an unterminated tool-call block".to_string());
            visible.push_str(&content[payload_start..]);
            cursor = content.len();
            break;
        };
        let end = payload_start + relative_end;
        match nola_kernel::strict_json::decode(content[payload_start..end].trim().as_bytes()) {
            Ok(value) => append_payload_calls(value, &mut calls, warnings),
            Err(_) => warnings.push("ignored malformed tool-call JSON".to_string()),
        }
        cursor = end + TOOL_CALL_CLOSE.len();
    }
    if cursor < content.len() {
        visible.push_str(&content[cursor..]);
    }
    (visible, calls)
}

/// Recovers the small set of provider responses that contain nothing but a
/// JSON array of `{name, parameters|arguments}` objects instead of using the
/// native tool-call channel. This is deliberately stricter than the tagged
/// protocol: mixed prose, unknown capability names, nested provider envelopes,
/// and malformed objects remain visible text and are never executed.
fn bare_json_tool_calls(
    content: &str,
    allowed: &HashSet<String>,
) -> Option<(String, Vec<AgentModelToolCall>)> {
    let trimmed = content.trim();
    if trimmed.is_empty() || !(trimmed.starts_with('[') || trimmed.starts_with('{')) {
        return None;
    }
    let value = nola_kernel::strict_json::decode(trimmed.as_bytes()).ok()?;
    let values = match value {
        Value::Array(values) => values,
        Value::Object(object) => vec![Value::Object(object)],
        _ => return None,
    };
    if values.is_empty() {
        return None;
    }
    let mut calls = Vec::with_capacity(values.len());
    for value in values {
        let object = value.as_object()?;
        let name = object.get("name").and_then(Value::as_str)?.trim();
        if name.is_empty() || !allowed.contains(name) {
            return None;
        }
        let arguments = object
            .get("parameters")
            .or_else(|| object.get("arguments"))
            .filter(|value| value.is_object())
            .cloned()?;
        calls.push(AgentModelToolCall {
            id: format!("call_bare_json_{}", calls.len()),
            name: name.to_string(),
            arguments,
            permission: None,
        });
    }
    Some((String::new(), calls))
}

fn append_payload_calls(
    value: Value,
    calls: &mut Vec<AgentModelToolCall>,
    warnings: &mut Vec<String>,
) {
    let values = match value {
        Value::Array(values) => values,
        Value::Object(mut object) => match object.remove("tool_calls") {
            Some(Value::Array(values)) => values,
            Some(_) => {
                warnings.push("ignored a tool_calls value that was not an array".to_string());
                return;
            }
            None => vec![Value::Object(object)],
        },
        _ => {
            warnings.push("ignored a tool-call payload that was not an object".to_string());
            return;
        }
    };
    for value in values {
        match serde_json::from_value::<AgentModelToolCall>(value) {
            Ok(call) => calls.push(call),
            Err(_) => warnings.push("ignored an invalid tool-call object".to_string()),
        }
    }
}

fn json_c_string<T: Serialize>(value: &T) -> *mut c_char {
    serde_json::to_string(value)
        .ok()
        .and_then(|value| CString::new(value).ok())
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}

fn input_bytes<'a>(input_ptr: *const u8, input_len: usize) -> Option<&'a [u8]> {
    if input_ptr.is_null() || input_len == 0 {
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(input_ptr, input_len) })
}

#[no_mangle]
pub extern "C" fn nola_core_parse_agent_model_output(
    input_ptr: *const u8,
    input_len: usize,
) -> *mut c_char {
    let bytes = match input_bytes(input_ptr, input_len) {
        Some(bytes) => bytes,
        None => return ptr::null_mut(),
    };
    let input = match serde_json::from_slice::<AgentModelParseInput>(bytes) {
        Ok(input) => input,
        Err(_) => return ptr::null_mut(),
    };
    json_c_string(&parse_agent_model_output(input))
}

#[no_mangle]
pub extern "C" fn nola_core_stitch_agent_model_continuation(
    input_ptr: *const u8,
    input_len: usize,
) -> *mut c_char {
    let bytes = match input_bytes(input_ptr, input_len) {
        Some(bytes) => bytes,
        None => return ptr::null_mut(),
    };
    let input = match serde_json::from_slice::<AgentModelContinuationInput>(bytes) {
        Ok(input) => input,
        Err(_) => return ptr::null_mut(),
    };
    json_c_string(&AgentModelContinuationResult {
        append: stitch_agent_model_continuation(&input.accumulated, &input.continuation),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ambiguous_ui_tool_json_never_becomes_an_executable_call() {
        for payload in [
            r#"{"name":"render_ui","name":"render_ui","arguments":{}}"#,
            r#"{"name":"render_ui","arguments":{"messages":[{"version":"0.9.1","version":"0.9.1"}]}}"#,
            r#"{"name":"render_ui","arguments":{"data":{"name":1,"na\u006de":2}}}"#,
        ] {
            for content in [payload.to_owned(), format!("<nola_tool_call>{payload}</nola_tool_call>")] {
                let result = parse_agent_model_output(AgentModelParseInput {
                    content, reasoning: String::new(), native_tool_calls: Vec::new(),
                    allowed_tool_names: vec!["render_ui".into()],
                });
                assert!(result.tool_calls.is_empty());
            }
        }
    }

    #[test]
    fn structured_text_protocol_rejects_duplicate_parameters_and_invalid_json_values() {
        for parameters in [
            r#"<parameter name="value" string="true">first</parameter><parameter name="value" string="true">second</parameter>"#,
            r#"<parameter name="value" string="false">{"x":1,"x":2}</parameter>"#,
            r#"<parameter name="value" string="false">not-json</parameter>"#,
        ] {
            let envelope = format!("<tool_calls><invoke name=\"render_ui\">{parameters}</invoke></tool_calls>");
            assert!(parse_canonical_text_tool_calls(&envelope).is_none());
        }
        let calls = parse_canonical_text_tool_calls(
            r#"<tool_calls><invoke name="render_ui"><parameter name="value" string="false">{"x":null,"items":[true,2]}</parameter></invoke></tool_calls>"#
        ).unwrap();
        assert_eq!(calls[0].arguments["value"], json!({"x":null,"items":[true,2]}));
    }

    #[test]
    fn parses_catalog_listed_tagged_tool_call() {
        let result = parse_agent_model_output(AgentModelParseInput {
            content: "Checking. <nola_tool_call>{\"name\":\"device_read_file\",\"arguments\":{}}</nola_tool_call>".to_string(),
            reasoning: String::new(),
            native_tool_calls: Vec::new(),
            allowed_tool_names: vec!["device_read_file".to_string()],
        });
        assert_eq!(result.content, "Checking.");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.finish_reason, "tool_calls");
        assert!(result.tool_calls[0].permission.is_none());
    }

    #[test]
    fn parses_dsml_tool_call_and_withholds_protocol_from_visible_text() {
        let result = parse_agent_model_output(AgentModelParseInput {
            content: "I'll load it.\n<｜｜DSML｜｜tool_calls>\n<｜｜DSML｜｜invoke name=\"skill_invoke\">\n<｜｜DSML｜｜parameter name=\"skill\" string=\"true\">Real lifecycle Skill</｜｜DSML｜｜parameter>\n</｜｜DSML｜｜invoke>\n</｜｜DSML｜｜tool_calls>".to_string(),
            reasoning: String::new(),
            native_tool_calls: Vec::new(),
            allowed_tool_names: vec!["skill_invoke".to_string()],
        });
        assert_eq!(result.content, "I'll load it.");
        assert_eq!(result.finish_reason, "tool_calls");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "skill_invoke");
        assert_eq!(
            result.tool_calls[0].arguments["skill"],
            "Real lifecycle Skill"
        );
    }

    #[test]
    fn parses_deepseek_dsml_tool_tag_and_withholds_protocol() {
        let result = parse_agent_model_output(AgentModelParseInput {
            content: "<｜｜DSML｜｜tool_calls><｜｜DSML｜｜tool name=\"write_file\"><｜｜DSML｜｜parameter name=\"path\" string=\"true\">report.json</｜｜DSML｜｜parameter></｜｜DSML｜｜tool></｜｜DSML｜｜tool_calls>".to_string(),
            reasoning: String::new(),
            native_tool_calls: Vec::new(),
            allowed_tool_names: vec!["write_file".to_string()],
        });
        assert!(result.content.is_empty());
        assert_eq!(result.finish_reason, "tool_calls");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "write_file");
        assert_eq!(result.tool_calls[0].arguments["path"], "report.json");
    }

    #[test]
    fn rejects_unknown_and_non_object_arguments() {
        let result = parse_agent_model_output(AgentModelParseInput {
            content: String::new(),
            reasoning: String::new(),
            native_tool_calls: vec![AgentModelToolCall {
                id: String::new(),
                name: "host_shell_exec".to_string(),
                arguments: json!(["pwd"]),
                permission: None,
            }],
            allowed_tool_names: vec!["device_read_file".to_string()],
        });
        assert!(result.tool_calls.is_empty());
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn recovers_bare_json_tool_call_array_only_for_allowed_tools() {
        let result = parse_agent_model_output(AgentModelParseInput {
            content: r#"[{"name":"write_file","parameters":{"path":"out.txt","content":"ok"}}]"#
                .to_string(),
            reasoning: String::new(),
            native_tool_calls: Vec::new(),
            allowed_tool_names: vec!["write_file".to_string()],
        });
        assert!(result.content.is_empty());
        assert_eq!(result.finish_reason, "tool_calls");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].arguments["path"], "out.txt");
    }

    #[test]
    fn leaves_mixed_or_unknown_bare_json_as_visible_text() {
        let result = parse_agent_model_output(AgentModelParseInput {
            content: r#"Here is JSON: [{"name":"write_file","parameters":{}}]"#.to_string(),
            reasoning: String::new(),
            native_tool_calls: Vec::new(),
            allowed_tool_names: vec!["write_file".to_string()],
        });
        assert!(result.tool_calls.is_empty());
        assert_eq!(
            result.content,
            "Here is JSON: [{\"name\":\"write_file\",\"parameters\":{}}]"
        );
    }

    #[test]
    fn stitches_repeated_continuation_prefix_once() {
        let accumulated = "A sufficiently long repeated paragraph prefix.";
        let continuation = "long repeated paragraph prefix. Then continue.";
        assert_eq!(
            stitch_agent_model_continuation(accumulated, continuation),
            " Then continue."
        );
    }

    #[test]
    fn removes_reopened_markdown_fence_from_continuation() {
        assert_eq!(
            stitch_agent_model_continuation(
                "Example:\n```rust\nlet value =",
                "```rust\n 42;\n```\nDone."
            ),
            " 42;\n```\nDone."
        );
    }
}
