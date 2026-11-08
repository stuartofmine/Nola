// SPDX-License-Identifier: Apache-2.0
//! Bounded diagnostic Part events, never canonical content or replay input.
use nola_agent_engine::agent::parts::{AgentPart, PartUpdate};
use nola_agent_engine::agent::AgentEvent;
use serde::Serialize;
use serde_json::{Map, Value};
use std::cell::Cell;
use std::io::Write;

mod bounded_serializer;
use bounded_serializer::BoundedValue;

pub(super) const MAX_PART_EVENT_BYTES: usize = 12_000;
const FIELD_BYTES: usize = 256;
const UPDATE_BYTES: usize = 2_048;

struct PrefixWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for PrefixWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let count = bytes.len().min(self.limit - self.bytes.len());
        self.bytes.extend_from_slice(&bytes[..count]);
        if count != bytes.len() {
            return Err(std::io::Error::other("diagnostic byte budget exhausted"));
        }
        Ok(count)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Stop visiting source-sized fields at the byte bound. The returned JSON,
/// including its escaping and omission marker, fits the same bound.
fn value_preview(value: &impl Serialize, limit: usize) -> Value {
    let truncated = Cell::new(false);
    serialized_preview(
        &BoundedValue::new(value, &truncated),
        limit,
        Some(&truncated),
    )
}

fn serialized_preview(
    value: &impl Serialize,
    limit: usize,
    truncated: Option<&Cell<bool>>,
) -> Value {
    let mut writer = PrefixWriter {
        bytes: Vec::with_capacity(limit),
        limit,
    };
    if serde_json::to_writer(&mut writer, value).is_ok() && !truncated.is_some_and(Cell::get) {
        return serde_json::from_slice(&writer.bytes).expect("serialized bounded JSON");
    }
    // A JSON string can cost six encoded bytes per source byte. Account for
    // its second escaping layer and the diagnostic envelope, not just UTF-8.
    writer.bytes.truncate(limit.saturating_sub(64) / 6);
    while std::str::from_utf8(&writer.bytes).is_err() {
        writer.bytes.pop();
    }
    let prefix = String::from_utf8(writer.bytes).expect("UTF-8 diagnostic prefix");
    serde_json::json!({"input_preview": prefix, "truncated": true})
}

fn json_preview(value: &Value, limit: usize) -> Value {
    value_preview(value, limit)
}

#[cfg(test)]
pub(super) fn bounded_event_json(value: &Value) -> Value {
    json_preview(value, 4_096)
}

pub(super) fn bounded_event_text(value: &str, maximum_characters: usize) -> String {
    if value.chars().nth(maximum_characters).is_none() {
        return value.into();
    }
    let side = maximum_characters / 2;
    let prefix = value.chars().take(side).collect::<String>();
    let suffix = value
        .chars()
        .rev()
        .take(side)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{prefix}\n...[event snapshot omitted]...\n{suffix}")
}

fn text_preview(value: &str, limit: usize) -> Value {
    if value.len() > limit {
        return Value::String(bounded_event_text(value, limit.saturating_sub(64) / 6));
    }
    let mut writer = PrefixWriter {
        bytes: Vec::with_capacity(limit),
        limit,
    };
    if serde_json::to_writer(&mut writer, value).is_ok() {
        return Value::String(value.into());
    }
    Value::String(bounded_event_text(value, limit.saturating_sub(64) / 6))
}

fn field(fields: &mut Map<String, Value>, name: &str, value: &impl Serialize) {
    fields.insert(name.into(), value_preview(value, FIELD_BYTES));
}

fn optional_text(fields: &mut Map<String, Value>, name: &str, value: Option<&str>) {
    fields.insert(
        name.into(),
        value.map_or(Value::Null, |text| text_preview(text, 512)),
    );
}

fn part_preview(part: &AgentPart) -> Value {
    let mut fields = Map::new();
    field(&mut fields, "id", &part.id());
    field(&mut fields, "message_id", &part.message_id());
    field(&mut fields, "turn_id", &part.turn_id());
    let kind = match part {
        AgentPart::Text(text) => {
            fields.insert("text".into(), text_preview(&text.text, 2_048));
            field(&mut fields, "is_complete", &text.is_complete);
            if let Some(review) = text.review {
                field(&mut fields, "review", &review);
            }
            "text"
        }
        AgentPart::Reasoning(reasoning) => {
            fields.insert("title".into(), text_preview(&reasoning.title, 512));
            fields.insert("detail".into(), text_preview(&reasoning.detail, 2_048));
            field(&mut fields, "is_complete", &reasoning.is_complete);
            "reasoning"
        }
        AgentPart::Tool(tool) => {
            field(&mut fields, "call_id", &tool.call_id);
            field(&mut fields, "capability_id", &tool.capability_id);
            fields.insert("title".into(), text_preview(&tool.title, 512));
            optional_text(&mut fields, "summary", tool.summary.as_deref());
            field(&mut fields, "origin", &tool.origin);
            field(&mut fields, "state", &tool.state);
            fields.insert("input".into(), json_preview(&tool.input, 512));
            field(&mut fields, "permission", &tool.permission);
            optional_text(
                &mut fields,
                "output_preview",
                tool.output_preview.as_deref(),
            );
            if let Some(output) = &tool.structured_output {
                fields.insert("structured_output".into(), json_preview(output, 512));
            }
            // This is private diagnostic history, not a public locator/grant.
            field(&mut fields, "full_output_ref", &tool.full_output_ref);
            field(&mut fields, "artifacts", &tool.artifacts);
            field(&mut fields, "started_at_ms", &tool.started_at_ms);
            field(&mut fields, "finished_at_ms", &tool.finished_at_ms);
            "tool"
        }
        AgentPart::Source(source) => {
            field(&mut fields, "source_id", &source.source_id);
            fields.insert("title".into(), text_preview(&source.title, 512));
            field(&mut fields, "uri", &source.uri);
            field(&mut fields, "mime_type", &source.mime_type);
            field(&mut fields, "evidence", &source.evidence);
            "source"
        }
        AgentPart::Citation(citation) => {
            field(&mut fields, "citation_id", &citation.citation_id);
            field(&mut fields, "source_id", &citation.source_id);
            fields.insert("title".into(), text_preview(&citation.title, 512));
            field(&mut fields, "uri", &citation.uri);
            field(&mut fields, "mime_type", &citation.mime_type);
            field(&mut fields, "location", &citation.location);
            field(&mut fields, "range", &citation.range);
            field(
                &mut fields,
                "origin_tool_call_id",
                &citation.origin_tool_call_id,
            );
            "citation"
        }
        AgentPart::Artifact(artifact) => {
            field(&mut fields, "artifact", &artifact.artifact);
            "artifact"
        }
        AgentPart::Step(step) => {
            fields.insert("title".into(), text_preview(&step.title, 2_048));
            field(&mut fields, "status", &step.status);
            "step"
        }
        AgentPart::Compaction(compaction) => {
            fields.insert("summary".into(), text_preview(&compaction.summary, 2_048));
            field(
                &mut fields,
                "retained_item_ids",
                &compaction.retained_item_ids,
            );
            field(
                &mut fields,
                "source_fingerprint",
                &compaction.source_fingerprint,
            );
            field(
                &mut fields,
                "original_token_count",
                &compaction.original_token_count,
            );
            field(
                &mut fields,
                "compacted_token_count",
                &compaction.compacted_token_count,
            );
            "compaction"
        }
    };
    fields.insert("kind".into(), kind.into());
    Value::Object(fields)
}

pub(super) fn event_payload_for_persistence(
    event: &AgentEvent,
) -> Result<Value, serde_json::Error> {
    let AgentEvent::PartUpdated {
        turn_id,
        projection_revision,
        update,
        part,
    } = event
    else {
        // Other event contracts are unchanged by the Part diagnostics migration.
        return serde_json::to_value(event);
    };
    let part = part_preview(part);
    let update = match update {
        // A creation has one snapshot, not two copies of the same fields.
        PartUpdate::Created { .. } => serde_json::json!({
            "type": "created", "part_id": part["id"], "snapshot_in": "part",
        }),
        PartUpdate::TextReplaced {
            part_id,
            is_complete,
            ..
        } => serde_json::json!({
            "type": "text_replaced", "part_id": value_preview(part_id, FIELD_BYTES),
            "text": part["text"], "is_complete": is_complete,
        }),
        PartUpdate::ReasoningReplaced {
            part_id,
            is_complete,
            ..
        } => serde_json::json!({
            "type": "reasoning_replaced", "part_id": value_preview(part_id, FIELD_BYTES),
            "title": part["title"], "detail": part["detail"], "is_complete": is_complete,
        }),
        PartUpdate::ToolInputReplaced { part_id, state, .. } => serde_json::json!({
            "type": "tool_input_replaced", "part_id": value_preview(part_id, FIELD_BYTES),
            "input": part["input"], "raw_input": "", "state": state,
        }),
        PartUpdate::ToolInputResolved { part_id, .. } => serde_json::json!({
            "type": "tool_input_resolved", "part_id": value_preview(part_id, FIELD_BYTES),
            "input": part["input"],
        }),
        PartUpdate::ToolState {
            part_id,
            state,
            timestamp_ms,
            ..
        } => serde_json::json!({
            "type": "tool_state", "part_id": value_preview(part_id, FIELD_BYTES),
            "state": state, "timestamp_ms": timestamp_ms, "snapshot_in": "part",
        }),
        _ => value_preview(update, UPDATE_BYTES),
    };
    let payload = serde_json::json!({
        "type": "part_updated", "diagnostic_schema": "nola.part_event.v1",
        "turn_id": value_preview(turn_id, FIELD_BYTES),
        "projection_revision": projection_revision, "update": update, "part": part,
    });
    // The owner of the complete frame enforces actual encoded bytes; field
    // budgets alone are never the final admission predicate.
    let mut writer = PrefixWriter {
        bytes: Vec::with_capacity(MAX_PART_EVENT_BYTES),
        limit: MAX_PART_EVENT_BYTES - 1,
    };
    serde_json::to_writer(&mut writer, &payload)?;
    Ok(payload)
}
