// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Stable local JSON control plane for user-visible memory management.
// - Boundary: Thin Application Service client; never opens canonical storage.

use std::io::{self, Read};

use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use nola_application::{
    ApplicationCommand, MemoryCandidate, MemoryCommand, MemoryCommitIntent,
    MemoryRecord as ApplicationMemoryRecord,
};
use nola_core::agent::{
    MemoryKind, MemoryNamespace, MemoryProvenance, MemoryProvenanceKind, MemoryRecord,
    MemoryRetention, MemoryScope,
};

use crate::application_port::CliApplicationPort;

#[derive(Debug, Deserialize)]
pub(crate) struct MemoryDraft {
    scope: String,
    kind: String,
    content: String,
    summary: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    confidence: Option<f64>,
    importance: Option<f64>,
    sensitivity: Option<String>,
    expires_at: Option<String>,
    source: Option<String>,
    provenance_kind: Option<String>,
}

pub fn run_if_requested() -> Option<i32> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).map(String::as_str) != Some("memory") {
        return None;
    }
    Some(match run(&args[2..]) {
        Ok(value) => {
            println!("{value}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    })
}

fn run(args: &[String]) -> Result<Value, String> {
    let action = args.first().map(String::as_str).unwrap_or("list");
    let mut application = CliApplicationPort::persistent_default()?;
    match action {
        "list" => Ok(json!({
            "type": "memory_records",
            "records": list_memories(&mut application, 2_000)?,
        })),
        "create" => {
            let draft = read_draft()?;
            save_memory(&mut application, &draft, None)
        }
        "revise" => {
            let id = args
                .get(1)
                .map(String::as_str)
                .ok_or("memory revise requires an id")?;
            let draft = read_draft()?;
            save_memory(&mut application, &draft, Some(id))
        }
        "delete" => {
            let id = args
                .get(1)
                .map(String::as_str)
                .ok_or("memory delete requires an id")?;
            delete_memory(&mut application, id)
        }
        _ => Err("usage: memory [list|create|revise ID|delete ID] --json".to_string()),
    }
}

fn read_draft() -> Result<MemoryDraft, String> {
    let mut payload = String::new();
    io::stdin()
        .take(1_000_001)
        .read_to_string(&mut payload)
        .map_err(|error| format!("read memory request failed: {error}"))?;
    if payload.len() > 1_000_000 {
        return Err("memory request exceeds the 1 MB limit".to_string());
    }
    serde_json::from_str(&payload).map_err(|error| format!("invalid memory request: {error}"))
}

pub(crate) fn save_memory(application: &mut CliApplicationPort, draft: &MemoryDraft, previous_id: Option<&str>) -> Result<Value, String> {
    if let Some(id) = previous_id {
        let existing = get_memory(application, id)?.ok_or("memory revision target does not exist")?;
        if existing.record.content == draft.content.trim() && display_scope(&existing.record) == draft.scope.trim()
            && display_kind(existing.record.kind) == draft.kind.trim() {
            return Err("memory revision must change content, scope, or kind".into());
        }
        let record = commit_draft(application, draft, &[id.to_string()], Some(existing.record.owner))?;
        Ok(json!({"type": "memory_revised", "previous_id": id, "record": memory_value(&record)}))
    } else {
        let record = commit_draft(application, draft, &[], None)?;
        Ok(json!({"type": "memory_created", "record": memory_value(&record)}))
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum MemoryWriteRequest {
    Create {draft: MemoryDraft},
    Revise {id: String, draft: MemoryDraft},
}

pub(crate) fn write_memory(application: &mut CliApplicationPort, request: MemoryWriteRequest) -> Result<Value, String> {
    match request {
        MemoryWriteRequest::Create {draft} => save_memory(application, &draft, None),
        MemoryWriteRequest::Revise {id, draft} => save_memory(application, &draft, Some(&id)),
    }
}

pub(crate) fn delete_memory(application: &mut CliApplicationPort, id: &str) -> Result<Value, String> {
    if id.is_empty() || id.trim() != id || id.len() > 128 || id.chars().any(char::is_control) {
        return Err("invalid memory identity".into());
    }
    let _: bool = application.execute(ApplicationCommand::Memory {
        command: MemoryCommand::Delete {id: id.to_string(), deleted_at_ms: Utc::now().timestamp_millis()},
    })?;
    // A repeated delete is successful only after canonical absence is observed.
    if get_memory(application, id)?.is_some() {
        return Err("memory deletion could not be confirmed".into());
    }
    Ok(json!({"type": "memory_deleted", "id": id}))
}

pub(crate) fn list_memories(application: &mut CliApplicationPort, limit: usize) -> Result<Vec<Value>, String> {
    let records: Vec<ApplicationMemoryRecord> =
        application.execute(ApplicationCommand::Memory {
            command: MemoryCommand::List { limit },
        })?;
    Ok(records.iter().map(memory_value).collect())
}

fn get_memory(
    application: &mut CliApplicationPort,
    id: &str,
) -> Result<Option<ApplicationMemoryRecord>, String> {
    application.execute(ApplicationCommand::Memory {
        command: MemoryCommand::Get { id: id.to_string() },
    })
}

fn commit_draft(
    application: &mut CliApplicationPort,
    draft: &MemoryDraft,
    supersedes: &[String],
    owner: Option<nola_domain::memory_owner::MemoryOwner>,
) -> Result<ApplicationMemoryRecord, String> {
    let source = draft
        .source
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("explicit_user_memory_management");
    let provenance_kind = draft
        .provenance_kind
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("explicit_user");
    let (scope, namespace) = parse_scope(&draft.scope)?;
    let owner = match owner {
        Some(owner) => { owner.validate_scope(&namespace, scope)?; owner }
        None => nola_domain::memory_owner::MemoryExecutionBinding::local_profile().owner_for(&namespace, scope)?,
    };
    let record = MemoryRecord {
        id: format!("mem-{}", Uuid::new_v4()),
        owner,
        namespace,
        scope,
        kind: parse_kind(&draft.kind)?,
        content: draft.content.trim().to_string(),
        summary: draft
            .summary
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        tags: draft.tags.clone(),
        confidence: draft.confidence.unwrap_or(1.0) as f32,
        importance: draft.importance.unwrap_or(0.5) as f32,
        sensitivity: draft
            .sensitivity
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        retention: draft
            .expires_at
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|expires_at| MemoryRetention::ExpiresAt {
                expires_at: expires_at.to_string(),
            })
            .unwrap_or(MemoryRetention::UntilSuperseded),
        supersedes: supersedes.to_vec(),
        provenance: MemoryProvenance {
            kind: parse_provenance_kind(provenance_kind)?,
            source: source.to_string(),
            session_id: None,
            turn_id: None,
            item_ids: Vec::new(),
            observed_at: Some(Utc::now().to_rfc3339()),
            ..MemoryProvenance::default()
        },
    };
    record.validate()?;
    application.execute(ApplicationCommand::Memory {
        command: MemoryCommand::Commit {
            input: MemoryCandidate {
                record,
                quality: None,
                proposed_at_ms: Utc::now().timestamp_millis(),
            },
            intent: MemoryCommitIntent::ExplicitUser,
        },
    })
}


fn parse_scope(value: &str) -> Result<(MemoryScope, MemoryNamespace), String> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "global" | "personal" => Ok((MemoryScope::Global, MemoryNamespace::Personal)),
        "conversation" => Ok((MemoryScope::Conversation, MemoryNamespace::Conversation)),
        "turn" => Ok((MemoryScope::Turn, MemoryNamespace::Conversation)),
        "workspace" => Ok((MemoryScope::Global, MemoryNamespace::Workspace)),
        _ if normalized.starts_with("custom:") => {
            let name = value.trim()["custom:".len()..].trim();
            if name.is_empty() {
                Err("custom memory scope requires a name".to_string())
            } else {
                Ok((
                    MemoryScope::Global,
                    MemoryNamespace::Custom(name.to_string()),
                ))
            }
        }
        _ => Err(
            "memory scope must be global, workspace, conversation, turn, or custom:<name>"
                .to_string(),
        ),
    }
}

fn parse_kind(value: &str) -> Result<MemoryKind, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "preference" => Ok(MemoryKind::Preference),
        "identity" => Ok(MemoryKind::Identity),
        "decision" => Ok(MemoryKind::Decision),
        "project" => Ok(MemoryKind::Project),
        "feedback" => Ok(MemoryKind::Feedback),
        "reference" => Ok(MemoryKind::Reference),
        "other" => Ok(MemoryKind::Other),
        _ => Err("unsupported memory kind".to_string()),
    }
}

fn parse_provenance_kind(value: &str) -> Result<MemoryProvenanceKind, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "explicit_user" => Ok(MemoryProvenanceKind::ExplicitUser),
        "agent_inference" => Ok(MemoryProvenanceKind::AgentInference),
        "tool_result" => Ok(MemoryProvenanceKind::ToolResult),
        "imported" => Ok(MemoryProvenanceKind::Imported),
        "synced" => Ok(MemoryProvenanceKind::Synced),
        "other" => Ok(MemoryProvenanceKind::Other),
        _ => Err("unsupported memory provenance kind".to_string()),
    }
}

fn display_scope(record: &MemoryRecord) -> String {
    match (&record.scope, &record.namespace) {
        (_, MemoryNamespace::Custom(name)) => format!("custom:{name}"),
        (MemoryScope::Turn, _) => "turn".to_string(),
        (MemoryScope::Conversation, _) => "conversation".to_string(),
        (MemoryScope::Global, MemoryNamespace::Workspace) => "workspace".to_string(),
        (MemoryScope::Global, _) => "global".to_string(),
    }
}

fn display_kind(kind: MemoryKind) -> &'static str {
    match kind {
        MemoryKind::Preference => "preference",
        MemoryKind::Identity => "identity",
        MemoryKind::Decision => "decision",
        MemoryKind::Project => "project",
        MemoryKind::Feedback => "feedback",
        MemoryKind::Reference => "reference",
        MemoryKind::Other => "other",
    }
}

fn memory_value(stored: &ApplicationMemoryRecord) -> Value {
    let record = &stored.record;
    json!({
        "id": record.id,
        "owner": record.owner,
        "scope": display_scope(record),
        "kind": display_kind(record.kind),
        "content": record.content,
        "summary": record.summary,
        "tags": record.tags,
        "source": record.provenance.source,
        "confidence": record.confidence,
        "importance": record.importance,
        "sensitivity": record.sensitivity,
        "expires_at": match &record.retention { MemoryRetention::ExpiresAt {expires_at} => Some(expires_at), _ => None },
        "created_at": chrono::DateTime::from_timestamp_millis(stored.created_at_ms).map(|date| date.to_rfc3339()).unwrap_or_default(),
        "updated_at": chrono::DateTime::from_timestamp_millis(stored.updated_at_ms).map(|date| date.to_rfc3339()).unwrap_or_default(),
        "supersedes": record.supersedes,
        "provenance": record.provenance,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_web_memory_write_contract_creates_and_revises_through_shared_service() {
        let directory = tempfile::tempdir().unwrap();
        let mut application = CliApplicationPort::persistent(&directory.path().join("writes.db")).unwrap();
        let request = serde_json::from_value(json!({"operation": "create", "draft": {
            "scope": "global", "kind": "preference", "content": "first", "importance": 0.7, "sensitivity": "private"
        }})).unwrap();
        let created = write_memory(&mut application, request).unwrap();
        assert_eq!(created["type"], "memory_created");
        let id = created["record"]["id"].as_str().unwrap();
        let request = serde_json::from_value(json!({"operation": "revise", "id": id, "draft": {
            "scope": "global", "kind": "preference", "content": "second", "importance": 0.7, "sensitivity": "private"
        }})).unwrap();
        let revised = write_memory(&mut application, request).unwrap();
        assert_eq!(revised["type"], "memory_revised");
        assert_eq!(revised["previous_id"], id);
        assert_eq!(revised["record"]["supersedes"], json!([id]));
        assert_eq!(revised["record"]["sensitivity"], "private");
        assert!(revised["record"]["created_at"].as_str().is_some_and(|value| !value.is_empty()));
        assert_eq!(list_memories(&mut application, 2_000).unwrap().len(), 2);
        assert!(serde_json::from_value::<MemoryWriteRequest>(json!({"operation": "unknown", "draft": {}})).is_err());
    }

    #[test]
    fn shared_memory_list_preserves_revision_lineage_in_isolated_store() {
        let directory = tempfile::tempdir().unwrap();
        let mut application = CliApplicationPort::persistent(&directory.path().join("memory.db")).unwrap();
        let draft: MemoryDraft = serde_json::from_value(json!({"scope": "global", "kind": "preference", "content": "old preference"})).unwrap();
        let original = commit_draft(&mut application, &draft, &[], None).unwrap();
        let revised: MemoryDraft = serde_json::from_value(json!({"scope": "global", "kind": "preference", "content": "new preference"})).unwrap();
        let revision = commit_draft(&mut application, &revised, &[original.record.id.clone()], Some(original.record.owner.clone())).unwrap();
        let records = list_memories(&mut application, 2_000).unwrap();
        assert_eq!(records.len(), 2);
        let value = records.iter().find(|value| value["id"] == revision.record.id).unwrap();
        assert_eq!(value["supersedes"], json!([original.record.id]));
        assert_eq!(value["source"], "explicit_user_memory_management");
        assert_eq!(value["content"], "new preference");
        assert!(value["created_at"].as_str().is_some_and(|text| !text.is_empty()));
        assert!(value["updated_at"].as_str().is_some_and(|text| !text.is_empty()));
        assert!(value["expires_at"].is_null());
        assert!(save_memory(&mut application, &revised, Some(&revision.record.id)).is_err());
        let receipt = delete_memory(&mut application, &revision.record.id).unwrap();
        assert_eq!(receipt, json!({"type": "memory_deleted", "id": revision.record.id}));
        assert_eq!(delete_memory(&mut application, &revision.record.id).unwrap(), receipt);
        let remaining = list_memories(&mut application, 2_000).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0]["id"], original.record.id);
        assert!(delete_memory(&mut application, " ").is_err());
    }
}
