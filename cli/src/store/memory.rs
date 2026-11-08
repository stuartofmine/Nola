use chrono::{SecondsFormat, TimeZone, Utc};
use serde_json::{json, Value};

use nola_application::{
    ApplicationCommand, MemoryCandidate as ApplicationMemoryCandidate, MemoryCommand,
    MemoryCommitIntent, MemoryRecallQuery, MemoryRecord as ApplicationMemoryRecord,
};
use nola_core::agent::{
    MemoryKind, MemoryNamespace, MemoryProvenance, MemoryProvenanceKind, MemoryQualityAssessment,
    MemoryRecord, MemoryRetention, MemoryScope,
};

use super::{stable_id, NolaStore};
use crate::application_port::CliApplicationPort;

/// Commit evidence is retained even if the independent read-back fails.
pub struct MemoryWriteObservation {
    pub record: ApplicationMemoryRecord,
    pub readback_error: Option<String>,
}

impl NolaStore {
    fn canonical_memory_store(&self) -> Result<CliApplicationPort, String> {
        CliApplicationPort::persistent(&self.path)
    }

    pub fn list_memory_records(&self, limit: usize) -> Result<Vec<Value>, String> {
        let mut store = self.canonical_memory_store()?;
        let records: Vec<ApplicationMemoryRecord> = store.execute(ApplicationCommand::Memory {
            command: MemoryCommand::List { limit },
        })?;
        records.iter().map(memory_value).collect()
    }

    pub fn memory_record(&self, id: &str) -> Result<Option<Value>, String> {
        let mut store = self.canonical_memory_store()?;
        let record: Option<ApplicationMemoryRecord> =
            store.execute(ApplicationCommand::Memory {
                command: MemoryCommand::Get { id: id.to_string() },
            })?;
        record.as_ref().map(memory_value).transpose()
    }

    pub fn delete_memory_record(&self, id: &str) -> Result<bool, String> {
        let mut store = self.canonical_memory_store()?;
        store.execute(ApplicationCommand::Memory {
            command: MemoryCommand::Delete {
                id: id.to_string(),
                deleted_at_ms: Utc::now().timestamp_millis(),
            },
        })
    }

    pub fn load_core_memory_records(&self, ids: &[String]) -> Result<Vec<MemoryRecord>, String> {
        let mut store = self.canonical_memory_store()?;
        let records: Vec<ApplicationMemoryRecord> = store.execute(ApplicationCommand::Memory {
            command: MemoryCommand::GetMany { ids: ids.to_vec() },
        })?;
        records.iter().map(core_memory_record).collect()
    }

    pub fn recent_core_memory_records(
        &self,
        scope: &str,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, String> {
        self.search_stored_memories(&nola_domain::memory_owner::MemoryExecutionBinding::local_profile(), scope, None, None, limit)?
            .iter()
            .map(core_memory_record)
            .collect()
    }

    pub fn save_core_memory_record(&self, record: &MemoryRecord) -> Result<(), String> {
        record.validate()?;
        let now_ms = Utc::now().timestamp_millis();
        let mut stored = stored_memory_record(record, now_ms)?;
        stored.quality = Some(MemoryQualityAssessment {
            applicable: true,
            durable: true,
            legible: true,
            rationale: "Accepted by the Core memory worker before durable commit.".to_string(),
        });
        let mut store = self.canonical_memory_store()?;
        let _: ApplicationMemoryRecord = store.execute(ApplicationCommand::Memory {
            command: MemoryCommand::Commit {
                input: stored,
                intent: MemoryCommitIntent::ValidatedAutomatic,
            },
        })?;
        Ok(())
    }

    pub fn save_memory(
        &self,
        scope: &str,
        kind: &str,
        content: &str,
        summary: Option<&str>,
        tags: &[String],
        source: Option<&str>,
    ) -> Result<String, String> {
        self.save_structured_memory(
            &nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            scope,
            kind,
            content,
            summary,
            tags,
            source,
            1.0,
            0.0,
            None,
            None,
            &[],
            &json!({"source": source}),
        ).and_then(|observed| match observed.readback_error {
            Some(error) => Err(error),
            None => Ok(observed.record.record.id),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_structured_memory(
        &self,
        binding: &nola_domain::memory_owner::MemoryExecutionBinding,
        scope: &str,
        kind: &str,
        content: &str,
        summary: Option<&str>,
        tags: &[String],
        source: Option<&str>,
        confidence: f64,
        importance: f64,
        sensitivity: Option<&str>,
        expires_at: Option<&str>,
        supersedes: &[String],
        provenance: &Value,
    ) -> Result<MemoryWriteObservation, String> {
        if content.chars().count() > 100_000 {
            return Err("memory content exceeds the 100000 character limit".to_string());
        }
        if tags.len() > 64 || tags.iter().any(|tag| tag.chars().count() > 128) {
            return Err("memory tags are limited to 64 entries of 128 characters each".to_string());
        }
        let (memory_scope, namespace) = decode_scope(scope)?;
        let provenance = serde_json::from_value::<MemoryProvenance>(provenance.clone())
            .unwrap_or_else(|_| MemoryProvenance {
                kind: MemoryProvenanceKind::Imported,
                source: source.unwrap_or("cli_memory_store").to_string(),
                session_id: None,
                turn_id: None,
                item_ids: Vec::new(),
                observed_at: None,
                ..MemoryProvenance::default()
            });
        let record = MemoryRecord {
            id: stable_id("mem"),
            owner: binding.owner_for(&namespace, memory_scope)?,
            namespace,
            scope: memory_scope,
            kind: decode_kind(kind),
            content: content.to_string(),
            summary: summary.map(str::to_string),
            tags: tags.to_vec(),
            confidence: confidence as f32,
            importance: importance as f32,
            sensitivity: sensitivity.map(str::to_string),
            retention: expires_at
                .map(|value| MemoryRetention::ExpiresAt {
                    expires_at: value.to_string(),
                })
                .unwrap_or(MemoryRetention::UntilSuperseded),
            supersedes: supersedes.to_vec(),
            provenance,
        };
        record.validate()?;
        let now_ms = Utc::now().timestamp_millis();
        let stored = stored_memory_record(&record, now_ms)?;
        let mut store = self.canonical_memory_store()?;
        let stored: ApplicationMemoryRecord = store.execute(ApplicationCommand::Memory {
            command: MemoryCommand::Commit {
                input: stored,
                intent: MemoryCommitIntent::ExplicitUser,
            },
        })?;
        let observed: Result<Option<ApplicationMemoryRecord>, String> = store.execute(ApplicationCommand::Memory {
            command: MemoryCommand::Get { id: stored.record.id.clone() },
        });
        let readback_error = match observed {
            Ok(Some(observed)) if observed == stored && stored.deleted_at_ms.is_none() => None,
            Ok(_) => Some("Memory was committed but its canonical read-back changed; verification is unavailable".into()),
            Err(error) => Some(format!("Memory was committed but read-back failed: {error}")),
        };
        Ok(MemoryWriteObservation { record: stored, readback_error })
    }

    pub fn search_memories(
        &self,
        binding: &nola_domain::memory_owner::MemoryExecutionBinding,
        scope: &str,
        query: Option<&str>,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Value>, String> {
        self.search_stored_memories(binding, scope, query, kind, limit)?
            .iter()
            .map(memory_value)
            .collect()
    }

    fn search_stored_memories(
        &self,
        binding: &nola_domain::memory_owner::MemoryExecutionBinding,
        scope: &str,
        query: Option<&str>,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ApplicationMemoryRecord>, String> {
        let (scope, namespace) = decode_scope(scope)?;
        let mut store = self.canonical_memory_store()?;
        store.execute(ApplicationCommand::Memory {
            command: MemoryCommand::Recall {
                input: MemoryRecallQuery {
                    binding: binding.clone(),
                    namespace,
                    scope,
                    query: query.map(str::to_string),
                    kind: kind.map(decode_kind),
                    now_iso: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
                    limit,
                },
            },
        })
    }
}

fn stored_memory_record(
    record: &MemoryRecord,
    now_ms: i64,
) -> Result<ApplicationMemoryCandidate, String> {
    record.validate()?;
    Ok(ApplicationMemoryCandidate {
        record: record.clone(),
        quality: None,
        proposed_at_ms: now_ms,
    })
}

fn core_memory_record(stored: &ApplicationMemoryRecord) -> Result<MemoryRecord, String> {
    let record = stored.record.clone();
    record.validate()?;
    Ok(record)
}

fn memory_value(record: &ApplicationMemoryRecord) -> Result<Value, String> {
    let core = core_memory_record(record)?;
    Ok(json!({
        "id": core.id,
        "owner": core.owner,
        "scope": display_scope(&core.scope, &core.namespace),
        "kind": encode_kind(core.kind),
        "content": core.content,
        "summary": core.summary,
        "tags": core.tags,
        "source": core.provenance.source,
        "confidence": core.confidence,
        "importance": core.importance,
        "sensitivity": core.sensitivity,
        "expires_at": match &core.retention { MemoryRetention::ExpiresAt { expires_at } => Some(expires_at), _ => None },
        "supersedes": core.supersedes,
        "provenance": core.provenance,
        "created_at": timestamp_iso(record.created_at_ms),
        "updated_at": timestamp_iso(record.updated_at_ms),
    }))
}

fn timestamp_iso(value: i64) -> String {
    Utc.timestamp_millis_opt(value)
        .single()
        .unwrap_or_else(Utc::now)
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn decode_scope(value: &str) -> Result<(MemoryScope, MemoryNamespace), String> {
    let selection = match value {
        "global" => (MemoryScope::Global, MemoryNamespace::Personal),
        "workspace" => (MemoryScope::Global, MemoryNamespace::Workspace),
        "conversation" => (MemoryScope::Conversation, MemoryNamespace::Conversation),
        "turn" => (MemoryScope::Turn, MemoryNamespace::Conversation),
        custom if custom.starts_with("custom:") && !custom[7..].trim().is_empty() => (
            MemoryScope::Global, MemoryNamespace::Custom(custom[7..].to_string()),
        ),
        _ => return Err("memory scope must be global, workspace, conversation, turn, or custom:<name>".into()),
    };
    Ok(selection)
}

fn display_scope(scope: &MemoryScope, namespace: &MemoryNamespace) -> String {
    match (scope, namespace) {
        (MemoryScope::Global, MemoryNamespace::Personal) => "global".into(),
        (MemoryScope::Global, MemoryNamespace::Workspace) => "workspace".into(),
        (MemoryScope::Global, MemoryNamespace::Custom(name)) => format!("custom:{name}"),
        (MemoryScope::Global, MemoryNamespace::Conversation) | (MemoryScope::Conversation, _) => {
            "conversation".into()
        }
        (MemoryScope::Turn, _) => "turn".into(),
    }
}

fn encode_scope(scope: MemoryScope) -> &'static str {
    match scope {
        MemoryScope::Turn => "turn",
        MemoryScope::Conversation => "conversation",
        MemoryScope::Global => "global",
    }
}

fn decode_kind(value: &str) -> MemoryKind {
    match value {
        "preference" => MemoryKind::Preference,
        "identity" | "user" => MemoryKind::Identity,
        "decision" => MemoryKind::Decision,
        "project" => MemoryKind::Project,
        "feedback" => MemoryKind::Feedback,
        "reference" => MemoryKind::Reference,
        _ => MemoryKind::Other,
    }
}

fn encode_kind(kind: MemoryKind) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::{ApplicationCommand, ApplicationMemoryRecord, MemoryCommand};
    use crate::store::NolaStore;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn memory_adapter_round_trips_through_service_and_survives_cold_reopen() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nola-memory-service-e2e-{nonce}"));
        std::fs::create_dir_all(&root).unwrap();
        let database = root.join("nola.db");

        let id = {
            let store = NolaStore::open(&database).unwrap();
            store
                .save_memory(
                    "global",
                    "preference",
                    "Use concise architectural status reports.",
                    Some("Concise reports"),
                    &["architecture".to_string()],
                    Some("memory-service-e2e"),
                )
                .unwrap()
        };

        let reopened = NolaStore::open(&database).unwrap();
        let loaded = reopened.memory_record(&id).unwrap().unwrap();
        assert_eq!(
            loaded["content"],
            "Use concise architectural status reports."
        );
        assert_eq!(loaded["tags"][0], "architecture");

        let mut canonical = super::CliApplicationPort::persistent(&database).unwrap();
        let durable: Option<ApplicationMemoryRecord> = canonical
            .execute(ApplicationCommand::Memory {
                command: MemoryCommand::Get { id: id.clone() },
            })
            .unwrap();
        let durable = durable.unwrap();
        assert_eq!(
            durable.record.content,
            "Use concise architectural status reports."
        );
    }
}

#[cfg(test)]
mod owner_execution_tests {
    use super::*;
    use nola_domain::memory_owner::MemoryExecutionBinding;

    #[test]
    fn concurrent_conversation_bindings_survive_service_round_trip_without_shared_owner_state() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("concurrent-memory.db");
        let handles = ["conversation-a", "conversation-b"].map(|conversation| {
            let database = database.clone();
            std::thread::spawn(move || {
                let binding = MemoryExecutionBinding {
                    conversation_id: Some(conversation.into()),
                    ..MemoryExecutionBinding::local_profile()
                };
                let store = NolaStore::open(&database).unwrap();
                let saved = store.save_structured_memory(
                    &binding, "conversation", "preference", "same private marker", None, &[],
                    Some("fixture"), 1.0, 0.8, None, None, &[], &json!({"source":"fixture"}),
                ).unwrap();
                let records = store.search_memories(&binding, "conversation", None, None, 1).unwrap();
                assert_eq!(records.len(), 1);
                assert_eq!(records[0]["id"], saved.record.record.id);
                (binding, saved.record.record.id)
            })
        });
        let [a, b] = handles.map(|handle| handle.join().unwrap());
        assert_ne!(a.1, b.1, "content deduplication must include concrete ownership");
        let store = NolaStore::open(&database).unwrap();
        for (binding, id) in [a, b] {
            let recalled = store.search_memories(&binding, "conversation", None, None, 1).unwrap();
            assert_eq!(recalled[0]["id"], id);
        }
        assert!(store.search_memories(&MemoryExecutionBinding::local_profile(), "conversation", None, None, 1).is_err());
        assert!(store.search_memories(&MemoryExecutionBinding::local_profile(), "conversaton", None, None, 1).is_err());
    }
}
