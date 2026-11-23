// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Stable local JSON control plane for user-visible memory management.
// - Boundary: Thin Application Service client; never opens canonical storage.

use std::io::{self, IsTerminal, Read};

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
use crate::i18n::{current_language, parse_language_tag, pick_language, set_language, Language};

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
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut json_output = false;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--" {
            break;
        }
        if arg == "--json" {
            json_output = true;
            index += 1;
            continue;
        }
        if arg == "--lang" {
            let Some(value) = args.get(index + 1) else {
                eprintln!("--lang requires a language tag");
                return Some(1);
            };
            let Some(language) = parse_language_tag(value) else {
                eprintln!("--lang unsupported language: {value}");
                return Some(1);
            };
            set_language(language);
            index += 2;
            continue;
        }
        if let Some(value) = arg.strip_prefix("--lang=") {
            let Some(language) = parse_language_tag(value) else {
                eprintln!("--lang unsupported language: {value}");
                return Some(1);
            };
            set_language(language);
            index += 1;
            continue;
        }
        break;
    }
    if args.get(index).map(String::as_str) != Some("memory") {
        return None;
    }
    Some(match run_cli_args(&args[index + 1..], json_output) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    })
}

fn memory_usage() -> &'static str {
    "Usage:\n  nola memory [list] [--json]\n  nola memory add \"<text>\" [--scope global|project] [--json]\n  nola memory search \"<query>\" [--json]\n  nola memory create [--json]   (reads MemoryDraft JSON from stdin)\n  nola memory revise <id> [--json] (reads MemoryDraft JSON from stdin)\n  nola memory delete <id> [--json]"
}

pub(crate) fn run_cli_args(args: &[String], leading_json: bool) -> Result<(), String> {
    let mut json_output = leading_json;
    let mut show_help = false;
    let mut filtered = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--" {
            filtered.extend(args[index + 1..].iter().cloned());
            break;
        }
        if arg == "--help" || arg == "-h" {
            show_help = true;
            index += 1;
        } else if arg == "--json" {
            json_output = true;
            index += 1;
        } else if arg == "--lang" {
            let value = args
                .get(index + 1)
                .ok_or_else(|| "--lang requires a language tag".to_string())?;
            let language = parse_language_tag(value)
                .ok_or_else(|| format!("--lang unsupported language: {value}"))?;
            set_language(language);
            index += 2;
        } else if let Some(value) = arg.strip_prefix("--lang=") {
            let language = parse_language_tag(value)
                .ok_or_else(|| format!("--lang unsupported language: {value}"))?;
            set_language(language);
            index += 1;
        } else {
            filtered.push(arg.clone());
            index += 1;
        }
    }
    if show_help {
        println!("{}", memory_usage());
        return Ok(());
    }

    let mut application = CliApplicationPort::persistent_default()?;
    execute_memory_args(&mut application, &filtered, json_output, io::stdin().is_terminal())
}

fn execute_memory_args(
    application: &mut CliApplicationPort,
    args: &[String],
    json_output: bool,
    stdin_is_terminal: bool,
) -> Result<(), String> {
    let action = args.first().map(String::as_str).unwrap_or("list");
    match action {
        "list" => {
            if args.len() > 1 {
                return Err(memory_usage().to_string());
            }
            let records = list_memories(application, 2_000)?;
            if json_output {
                println!(
                    "{}",
                    json!({
                        "type": "memory_records",
                        "records": records,
                    })
                );
            } else if records.is_empty() {
                println!(
                    "{}",
                    pick_language(
                        current_language(),
                        [
                            "尚未保存记忆。",
                            "尚未儲存記憶。",
                            "No memories stored.",
                            "Keine Erinnerungen gespeichert.",
                            "No hay recuerdos guardados.",
                            "Aucun souvenir enregistré.",
                            "保存されたメモリはありません。",
                            "저장된 메모리가 없습니다.",
                            "Сохранённых воспоминаний нет.",
                        ]
                    )
                );
            } else {
                for record in &records {
                    println!(
                        "[{} / {}] {}: {}",
                        record["scope"].as_str().unwrap_or("global"),
                        record["kind"].as_str().unwrap_or("other"),
                        record["id"].as_str().unwrap_or(""),
                        record["content"].as_str().unwrap_or("")
                    );
                }
            }
            Ok(())
        }
        "add" => {
            let (content, scope) = parse_add_args(&args[1..])?;
            let kind = if scope == "project" || scope == "workspace" {
                "project"
            } else {
                "preference"
            };
            let draft = MemoryDraft {
                scope,
                kind: kind.to_string(),
                content,
                summary: None,
                tags: Vec::new(),
                confidence: None,
                importance: Some(0.6),
                sensitivity: None,
                expires_at: None,
                source: None,
                provenance_kind: None,
            };
            let created = save_memory(application, &draft, None)?;
            if json_output {
                println!("{created}");
            } else {
                let id = created["record"]["id"].as_str().unwrap_or("");
                let stored_scope = created["record"]["scope"].as_str().unwrap_or("global");
                let msg = match current_language() {
                    Language::ZhHans => format!("已添加记忆 {id}（{stored_scope}）。"),
                    Language::ZhHant => format!("已新增記憶 {id}（{stored_scope}）。"),
                    Language::De => format!("Erinnerung {id} ({stored_scope}) hinzugefügt."),
                    Language::Es => format!("Recuerdo {id} ({stored_scope}) añadido."),
                    Language::Fr => format!("Souvenir {id} ({stored_scope}) ajouté."),
                    Language::Ja => format!("メモリ {id}（{stored_scope}）を追加しました。"),
                    Language::Ko => format!("메모리 {id} ({stored_scope}) 추가됨."),
                    Language::Ru => format!("Воспоминание {id} ({stored_scope}) добавлено."),
                    Language::En => format!("Added memory {id} ({stored_scope})."),
                };
                println!("{msg}");
            }
            Ok(())
        }
        "search" => {
            if args.len() < 2 {
                return Err("Usage: nola memory search \"<query>\" [--json]".to_string());
            }
            let query = args[1..].join(" ");
            let matched = search_memories(application, &query)?;
            if json_output {
                println!(
                    "{}",
                    json!({
                        "type": "memory_search_results",
                        "query": query,
                        "records": matched,
                    })
                );
            } else if matched.is_empty() {
                let msg = match current_language() {
                    Language::ZhHans => format!("没有匹配 \"{query}\" 的记忆。"),
                    Language::ZhHant => format!("沒有符合 \"{query}\" 的記憶。"),
                    Language::De => format!("Keine Erinnerungen passend zu „{query}“."),
                    Language::Es => format!("No hay recuerdos que coincidan con \"{query}\"."),
                    Language::Fr => format!("Aucun souvenir ne correspond à « {query} »."),
                    Language::Ja => format!("「{query}」に一致するメモリはありません。"),
                    Language::Ko => format!("\"{query}\"와(과) 일치하는 메모리가 없습니다."),
                    Language::Ru => format!("Нет воспоминаний, соответствующих «{query}»."),
                    Language::En => format!("No memories matched \"{query}\"."),
                };
                println!("{msg}");
            } else {
                for record in &matched {
                    println!(
                        "[{} / {}] {}: {}",
                        record["scope"].as_str().unwrap_or("global"),
                        record["kind"].as_str().unwrap_or("other"),
                        record["id"].as_str().unwrap_or(""),
                        record["content"].as_str().unwrap_or("")
                    );
                }
            }
            Ok(())
        }
        "create" => {
            if stdin_is_terminal {
                return Err("memory create reads a MemoryDraft JSON object from stdin; pipe JSON or use: nola memory add \"<text>\" [--scope global|project]".to_string());
            }
            let draft = read_draft()?;
            let value = save_memory(application, &draft, None)?;
            println!("{value}");
            Ok(())
        }
        "revise" => {
            let id = args
                .get(1)
                .map(String::as_str)
                .ok_or("memory revise requires an id")?;
            if stdin_is_terminal {
                return Err("memory revise <id> reads a MemoryDraft JSON object from stdin".to_string());
            }
            let draft = read_draft()?;
            let value = save_memory(application, &draft, Some(id))?;
            println!("{value}");
            Ok(())
        }
        "delete" => {
            let id = args
                .get(1)
                .map(String::as_str)
                .ok_or("memory delete requires an id")?;
            let value = delete_memory(application, id)?;
            println!("{value}");
            Ok(())
        }
        _ => Err(memory_usage().to_string()),
    }
}

fn parse_add_args(args: &[String]) -> Result<(String, String), String> {
    let mut scope = "global".to_string();
    let mut parts = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--scope" {
            let value = args
                .get(index + 1)
                .ok_or_else(|| "--scope requires global or project".to_string())?;
            scope = validate_cli_add_scope(value)?;
            index += 2;
        } else if let Some(value) = arg.strip_prefix("--scope=") {
            scope = validate_cli_add_scope(value)?;
            index += 1;
        } else if arg.starts_with('-') {
            return Err(format!("unknown option for memory add: {arg}"));
        } else {
            parts.push(arg.clone());
            index += 1;
        }
    }
    let content = parts.join(" ").trim().to_string();
    if content.is_empty() {
        return Err("Usage: nola memory add \"<text>\" [--scope global|project]".to_string());
    }
    Ok((content, scope))
}

fn validate_cli_add_scope(raw: &str) -> Result<String, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "global" | "personal" => Ok("global".to_string()),
        "project" | "workspace" => Ok("project".to_string()),
        _ => Err("memory add --scope must be global or project".to_string()),
    }
}

pub(crate) fn search_memories(
    application: &mut CliApplicationPort,
    query: &str,
) -> Result<Vec<Value>, String> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Err("memory search query must be non-empty".to_string());
    }
    let records = list_memories(application, 2_000)?;
    Ok(records
        .into_iter()
        .filter(|record| {
            let haystack_fields = [
                record["content"].as_str().unwrap_or(""),
                record["summary"].as_str().unwrap_or(""),
                record["scope"].as_str().unwrap_or(""),
                record["kind"].as_str().unwrap_or(""),
            ];
            let tags_match = record["tags"].as_array().is_some_and(|tags| {
                tags.iter()
                    .filter_map(Value::as_str)
                    .any(|tag| tag.to_ascii_lowercase().contains(&needle))
            });
            tags_match
                || haystack_fields
                    .iter()
                    .any(|field| field.to_ascii_lowercase().contains(&needle))
        })
        .collect())
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
        let draft_summary = draft.summary.as_deref().map(str::trim).filter(|value| !value.is_empty());
        let draft_sensitivity = draft.sensitivity.as_deref().map(str::trim).filter(|value| !value.is_empty());
        let existing_expires = match &existing.record.retention {
            MemoryRetention::ExpiresAt { expires_at } => Some(expires_at.as_str()),
            _ => None,
        };
        let draft_expires = draft.expires_at.as_deref().map(str::trim).filter(|value| !value.is_empty());
        let draft_confidence = draft.confidence.unwrap_or(1.0) as f32;
        let draft_importance = draft.importance.unwrap_or(0.5) as f32;
        let identity_changed = existing.record.content != draft.content.trim()
            || display_scope(&existing.record) != draft.scope.trim()
            || display_kind(existing.record.kind) != draft.kind.trim();
        let unchanged = !identity_changed
            && existing.record.summary.as_deref() == draft_summary
            && existing.record.tags == draft.tags
            && (existing.record.confidence - draft_confidence).abs() <= f32::EPSILON
            && (existing.record.importance - draft_importance).abs() <= f32::EPSILON
            && existing.record.sensitivity.as_deref() == draft_sensitivity
            && existing_expires == draft_expires;
        if unchanged {
            return Err("memory revision must change at least one field".into());
        }
        let supersedes = if identity_changed {
            vec![id.to_string()]
        } else {
            existing.record.supersedes.clone()
        };
        let record = commit_draft(
            application,
            draft,
            &supersedes,
            Some(existing.record.owner),
            Some(existing.updated_at_ms),
        )?;
        Ok(json!({"type": "memory_revised", "previous_id": id, "record": memory_value(&record)}))
    } else {
        let record = commit_draft(application, draft, &[], None, None)?;
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

pub(crate) fn workspace_memory_binding(request: &crate::host_invocation_request::HostInvocationRequest) -> Result<nola_domain::memory_owner::MemoryExecutionBinding, String> {
    let mut binding = nola_domain::memory_owner::MemoryExecutionBinding::local_profile();
    binding.workspace_id = Some(request.workspace_identity()?.workspace_id);
    binding.conversation_id = request.session.session_id.clone();
    Ok(binding)
}

pub(crate) fn visible_memories(application: &mut CliApplicationPort, binding: &nola_domain::memory_owner::MemoryExecutionBinding, query: Option<&str>, limit: usize) -> Result<Vec<Value>, String> {
    let records: Vec<ApplicationMemoryRecord> = application.execute(ApplicationCommand::Memory {
        command: MemoryCommand::ListForBinding { binding: binding.clone(), query: query.map(str::to_owned), limit },
    })?;
    Ok(records.iter().map(memory_value).collect())
}

pub(crate) fn add_workspace_memory(application: &mut CliApplicationPort, binding: &nola_domain::memory_owner::MemoryExecutionBinding, draft: &MemoryDraft) -> Result<Value, String> {
    let owner = binding.owner_for(&MemoryNamespace::Workspace, MemoryScope::Global)?;
    Ok(memory_value(&commit_draft(application, draft, &[], Some(owner), None)?))
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
    min_proposed_at_ms: Option<i64>,
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
        Some(owner) => {
            owner.validate_scope(&namespace, scope)?;
            owner
        }
        None => {
            let mut binding = nola_domain::memory_owner::MemoryExecutionBinding::local_profile();
            if matches!(namespace, MemoryNamespace::Workspace) {
                let cwd = std::env::current_dir()
                    .map_err(|err| format!("current_dir failed: {err}"))?;
                let workspace = crate::host_execution_settings::execution_workspace_identity(
                    &cwd, None, None, None,
                )?;
                binding.workspace_id = Some(workspace.workspace_id);
            }
            binding.owner_for(&namespace, scope)?
        }
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
                proposed_at_ms: Utc::now().timestamp_millis().max(min_proposed_at_ms.unwrap_or(0)),
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
        "workspace" | "project" => Ok((MemoryScope::Global, MemoryNamespace::Workspace)),
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
    fn cli_flow_memory_honors_the_selected_project_on_write_list_and_search() {
        let root = tempfile::tempdir().unwrap();
        let mut application = CliApplicationPort::persistent(&root.path().join("cli.sqlite")).unwrap();
        let binding = |name: &str| {
            let mut request = crate::host_invocation_request::HostInvocationRequest::for_test().unwrap();
            request.tool_settings.project_root = root.path().join(name);
            workspace_memory_binding(&request).unwrap()
        };
        let a = binding("a");
        let b = binding("b");
        let draft = |scope, content| serde_json::from_value(json!({
            "scope": scope, "kind": "other", "content": content,
        })).unwrap();
        add_workspace_memory(&mut application, &b, &draft("project", "project 100%")).unwrap();
        for name in ["other 100%", "newer project"] {
            add_workspace_memory(&mut application, &a, &draft("project", name)).unwrap();
        }
        assert_eq!(visible_memories(&mut application, &b, Some("workspace"), 1).unwrap().len(), 1);
        assert!(visible_memories(&mut application, &b, Some("global"), 5).unwrap().is_empty());
        let b_records = visible_memories(&mut application, &b, Some("%"), 1).unwrap();
        assert_eq!(b_records.len(), 1);
        assert_eq!(b_records[0]["content"], "project 100%");
        assert_eq!(b_records[0]["owner"]["workspace_id"], b.workspace_id.unwrap());
        assert_eq!(visible_memories(&mut application, &a, None, 5).unwrap().len(), 2);
        save_memory(&mut application, &draft("global", "shared"), None).unwrap();
        assert_eq!(visible_memories(&mut application, &a, Some("shared"), 1).unwrap().len(), 1);
    }

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
        let revised_id = revised["record"]["id"].as_str().unwrap();
        let revised_meta = write_memory(&mut application, serde_json::from_value(json!({"operation": "revise", "id": revised_id, "draft": {
            "scope": "global", "kind": "preference", "content": "second", "summary": "Short label", "tags": ["style"], "confidence": 0.85, "importance": 0.9, "sensitivity": "private"
        }})).unwrap()).unwrap();
        assert_eq!(revised_meta["record"]["id"], revised_id);
        assert_eq!(revised_meta["record"]["summary"], "Short label");
        assert_eq!(revised_meta["record"]["tags"], json!(["style"]));
        assert_eq!(list_memories(&mut application, 2_000).unwrap().len(), 2);
        assert!(serde_json::from_value::<MemoryWriteRequest>(json!({"operation": "unknown", "draft": {}})).is_err());
    }

    #[test]
    fn shared_memory_list_preserves_revision_lineage_in_isolated_store() {
        let directory = tempfile::tempdir().unwrap();
        let mut application = CliApplicationPort::persistent(&directory.path().join("memory.db")).unwrap();
        let draft: MemoryDraft = serde_json::from_value(json!({"scope": "global", "kind": "preference", "content": "old preference"})).unwrap();
        let original = commit_draft(&mut application, &draft, &[], None, None).unwrap();
        let revised: MemoryDraft = serde_json::from_value(json!({"scope": "global", "kind": "preference", "content": "new preference"})).unwrap();
        let revision = commit_draft(&mut application, &revised, &[original.record.id.clone()], Some(original.record.owner.clone()), None).unwrap();
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

    #[test]
    fn cli_memory_add_search_and_tty_create_guard_work() {
        let directory = tempfile::tempdir().unwrap();
        let mut application =
            CliApplicationPort::persistent(&directory.path().join("cli-memory.db")).unwrap();

        execute_memory_args(
            &mut application,
            &[
                "add".into(),
                "Prefer four-space indentation".into(),
                "--scope".into(),
                "global".into(),
            ],
            false,
            true,
        )
        .unwrap();
        execute_memory_args(
            &mut application,
            &[
                "add".into(),
                "Use NolaStore in CLI tests".into(),
                "--scope=project".into(),
            ],
            true,
            true,
        )
        .unwrap();

        let matches = search_memories(&mut application, "four-space").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["scope"], "global");

        let proj_matches = search_memories(&mut application, "NolaStore").unwrap();
        assert_eq!(proj_matches.len(), 1);
        assert_eq!(proj_matches[0]["scope"], "workspace");

        // TTY stdin create/revise fails fast with usage guidance rather than blocking.
        let tty_err = execute_memory_args(&mut application, &["create".into()], false, true)
            .unwrap_err();
        assert!(tty_err.contains("stdin"));
        assert!(tty_err.contains("memory add"));
    }
}
