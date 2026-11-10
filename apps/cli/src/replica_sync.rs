// SPDX-License-Identifier: Apache-2.0
//! Authenticated HTTP transport for canonical replica synchronization.
//!
//! This module deliberately contains no product merge policy. Core owns the
//! outbox, revision validation, conflict handling, and atomic remote-page
//! application. The Host only maps typed Core facts to and from HTTP.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(test)]
use nola_application::ApplicationCommand;
use nola_protocol::{
    PendingSyncChange, RemoteSyncChange, RemoteSyncPage, RemoteSyncPageResult, ReplicaSyncCommand,
    SyncCursor,
};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::application_port::CliApplicationPort;

const MAX_REPLICA_RESPONSE_BYTES: usize = nola_protocol::MAX_REPLICA_PAGE_BYTES;
const MAX_REPLICA_ERROR_BYTES: usize = 4096;
// Worker upload budget, smaller than the local Application IPC frame.
const MAX_REPLICA_PUSH_BYTES: usize = 4 * 1024 * 1024;

struct PushByteCounter {
    used: usize,
    maximum: usize,
}
impl Write for PushByteCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.maximum.saturating_sub(self.used) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "replica HTTP byte budget exhausted",
            ));
        }
        self.used += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn push_prefix(
    pending: &[PendingSyncChange],
    origin_device_id: &str,
) -> Result<Vec<PushChange>, String> {
    let mut counter = PushByteCounter {
        used: 0,
        maximum: MAX_REPLICA_PUSH_BYTES,
    };
    serde_json::to_writer(&mut counter, &PushRequest { changes: &[] })
        .map_err(|error| error.to_string())?;
    let mut used = counter.used;
    let mut changes = Vec::new();
    for entry in pending {
        let change = PushChange {
            operation_id: entry.operation_id.clone(),
            entity_type: entry.entity_type.clone(),
            entity_id: entry.entity_id.clone(),
            revision: entry.revision,
            deleted: entry.deleted,
            payload: serde_json::from_str(&entry.payload_json).map_err(|error| {
                format!("decode sync payload {} failed: {error}", entry.operation_id)
            })?,
            origin_device_id: origin_device_id.into(),
        };
        let separator = usize::from(!changes.is_empty());
        let mut counter = PushByteCounter {
            used: 0,
            maximum: MAX_REPLICA_PUSH_BYTES.saturating_sub(used + separator),
        };
        if serde_json::to_writer(&mut counter, &change).is_err() {
            if changes.is_empty() {
                return Err("replica record exceeds HTTP byte budget; chunked transfer required; entry remains pending".into());
            }
            break;
        }
        used += counter.used + separator;
        changes.push(change);
        // Keep the seal acknowledgement in a completed HTTP exchange before
        // offering any message reference in a subsequent push.
        if entry.entity_type == "canonical_text_seal" {
            break;
        }
    }
    Ok(changes)
}

fn decode_response<T: serde::de::DeserializeOwned>(
    response: reqwest::blocking::Response,
) -> Result<T, String> {
    let status = response.status();
    let maximum = if status.is_success() {
        MAX_REPLICA_RESPONSE_BYTES
    } else {
        MAX_REPLICA_ERROR_BYTES
    };
    let mut bytes = Vec::new();
    response
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read replica response failed: {error}"))?;
    if bytes.len() > maximum {
        return Err(format!(
            "replica response ({status}) exceeds {maximum} bytes"
        ));
    }
    if !status.is_success() {
        return Err(format!(
            "replica request failed ({status}): {}",
            String::from_utf8_lossy(&bytes)
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("decode replica response failed: {error}"))
}

#[derive(Debug, Clone)]
pub struct ReplicaSyncConfiguration {
    pub base_url: String,
    pub access_token: String,
    pub replica_id: String,
    pub origin_device_id: String,
    pub page_size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaSyncReport {
    pub pushed: usize,
    pub applied: usize,
    pub skipped: usize,
    pub cursor: i64,
}

pub struct ReplicaSyncHost {
    client: Client,
    configuration: ReplicaSyncConfiguration,
}

impl ReplicaSyncHost {
    pub fn new(configuration: ReplicaSyncConfiguration) -> Result<Self, String> {
        if configuration.base_url.trim().is_empty()
            || configuration.access_token.trim().is_empty()
            || configuration.replica_id.trim().is_empty()
            || configuration.origin_device_id.trim().is_empty()
        {
            return Err("replica sync configuration is incomplete".to_string());
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| format!("create replica HTTP client failed: {error}"))?;
        Ok(Self {
            client,
            configuration,
        })
    }

    pub fn synchronize(
        &self,
        application: &mut CliApplicationPort,
    ) -> Result<ReplicaSyncReport, String> {
        let pushed = self.push_pending(application)?;
        let (applied, skipped, cursor) = self.pull_all(application)?;
        Ok(ReplicaSyncReport {
            pushed,
            applied,
            skipped,
            cursor,
        })
    }

    fn push_pending(&self, application: &mut CliApplicationPort) -> Result<usize, String> {
        let limit = self.configuration.page_size.clamp(1, 100);
        let pending: Vec<PendingSyncChange> =
            application.execute(ReplicaSyncCommand::PendingSyncChanges { limit })?;
        if pending.is_empty() {
            return Ok(0);
        }
        let changes = push_prefix(&pending, &self.configuration.origin_device_id)?;
        let pending = &pending[..changes.len()];
        let response = self
            .client
            .post(self.endpoint())
            .bearer_auth(&self.configuration.access_token)
            .json(&PushRequest { changes: &changes })
            .send()
            .map_err(|error| format!("push replica changes failed: {error}"))?;
        let response: PushResponse = decode_response(response)?;
        if response.results.len() != pending.len() {
            return Err("replica push result count does not match request".to_string());
        }
        let expected: HashMap<_, _> = pending
            .iter()
            .map(|entry| (entry.operation_id.as_str(), entry.revision))
            .collect();
        let mut seen = HashSet::new();
        for result in &response.results {
            if !seen.insert(result.operation_id.as_str())
                || expected.get(result.operation_id.as_str()) != Some(&result.current_revision)
                || result.cursor < 0
            {
                return Err("replica push acknowledgement identity/revision mismatch; queue remains pending".into());
            }
            if !matches!(
                result.status.as_str(),
                "accepted" | "idempotent" | "unchanged"
            ) {
                return Err(format!(
                    "replica conflict for {}: {} (remote revision {})",
                    result.operation_id, result.status, result.current_revision
                ));
            }
        }
        let resolved_at_ms = now_ms()?;
        for result in response.results {
            let core_status = if result.status == "unchanged" {
                "unchanged"
            } else {
                "acknowledged"
            };
            let _: bool = application.execute(ReplicaSyncCommand::ResolveSyncChange {
                operation_id: result.operation_id,
                status: core_status.to_string(),
                remote_cursor: Some(result.cursor),
                resolved_at_ms,
            })?;
        }
        Ok(pending.len())
    }

    fn pull_all(
        &self,
        application: &mut CliApplicationPort,
    ) -> Result<(usize, usize, i64), String> {
        let stored: Option<SyncCursor> =
            application.execute(ReplicaSyncCommand::GetSyncCursor {
                replica_id: self.configuration.replica_id.clone(),
            })?;
        let mut cursor = stored.map(|value| value.cursor).unwrap_or(0);
        let mut applied = 0;
        let mut skipped = 0;
        loop {
            let response = self
                .client
                .get(self.endpoint())
                .bearer_auth(&self.configuration.access_token)
                .query(&[
                    ("cursor", cursor.to_string()),
                    (
                        "limit",
                        self.configuration.page_size.clamp(1, 500).to_string(),
                    ),
                ])
                .send()
                .map_err(|error| format!("pull replica changes failed: {error}"))?;
            let page: PullResponse = decode_response(response)?;
            if page.changes.len() > self.configuration.page_size.clamp(1, 500) {
                return Err("replica pull page exceeds requested count".into());
            }
            if page.has_more && page.next_cursor <= cursor {
                return Err("replica pull page makes no progress".into());
            }
            let changes = page
                .changes
                .into_iter()
                .map(|change| {
                    Ok(RemoteSyncChange {
                        cursor: change.cursor,
                        operation_id: change.operation_id,
                        entity_type: change.entity_type,
                        entity_id: change.entity_id,
                        revision: change.revision,
                        deleted: change.deleted,
                        payload_json: change.payload.to_string(),
                        created_at_ms: parse_server_time_ms(&change.committed_at)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let input = RemoteSyncPage {
                replica_id: self.configuration.replica_id.clone(),
                changes,
                next_cursor: page.next_cursor,
                applied_at_ms: now_ms()?,
            };
            input.validate(cursor)?;
            let result: RemoteSyncPageResult =
                application.execute(ReplicaSyncCommand::ApplyRemoteSyncPage { input })?;
            applied += result.applied;
            skipped += result.skipped;
            cursor = result.cursor;
            if !page.has_more {
                return Ok((applied, skipped, cursor));
            }
        }
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/v1/sync/replicas",
            self.configuration.base_url.trim_end_matches('/')
        )
    }
}

#[derive(Serialize)]
struct PushRequest<'a> {
    changes: &'a [PushChange],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PushChange {
    operation_id: String,
    entity_type: String,
    entity_id: String,
    revision: i64,
    deleted: bool,
    payload: Value,
    origin_device_id: String,
}

#[derive(Deserialize)]
struct PushResponse {
    results: Vec<PushResult>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PushResult {
    operation_id: String,
    status: String,
    current_revision: i64,
    cursor: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullResponse {
    changes: Vec<PullChange>,
    next_cursor: i64,
    has_more: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullChange {
    cursor: i64,
    operation_id: String,
    entity_type: String,
    entity_id: String,
    revision: i64,
    deleted: bool,
    payload: Value,
    committed_at: String,
}

fn now_ms() -> Result<i64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))
}

fn parse_server_time_ms(value: &str) -> Result<i64, String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|date| date.timestamp_millis())
        .map_err(|error| format!("invalid replica committedAt: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use tiny_http::{Header, Response, Server};

    fn assert_worker_part_metadata(
        application: &mut CliApplicationPort,
        conversation: &str,
        ordinal: i64,
        digest: &str,
        created: i64,
        updated: i64,
    ) {
        let mut page: Option<nola_protocol::ExportPage> = application
            .execute(nola_protocol::ExportCommand::LoadConversationExportPage {
                conversation_id: conversation.into(),
                after_ordinal: None,
                expected_revision: None,
            })
            .unwrap();
        let revision = page.as_ref().unwrap().conversation.revision;
        if ordinal > 1 {
            page = application
                .execute(nola_protocol::ExportCommand::LoadConversationExportPage {
                    conversation_id: conversation.into(),
                    after_ordinal: Some(ordinal - 1),
                    expected_revision: Some(revision),
                })
                .unwrap();
        }
        let page = page.unwrap();
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].ordinal, ordinal);
        assert_eq!(page.messages[0].parts.len(), 1);
        let part = &page.messages[0].parts[0];
        assert_eq!(part.content_digest.as_deref(), Some(digest));
        assert_eq!((part.created_at_ms, part.updated_at_ms), (created, updated));
    }

    #[test]
    #[ignore = "Requires scripts/test_worker_replica_e2e.py --with-cli"]
    fn isolated_worker_round_trip_uses_real_cli_transport() {
        let address = std::env::var("NOLA_REPLICA_E2E_URL").expect("isolated Worker URL required");
        let url = url::Url::parse(&address).unwrap();
        assert_eq!(url.scheme(), "http");
        assert_eq!(
            url.host_str(),
            Some("127.0.0.1"),
            "never run this fixture against a remote account"
        );
        let access_token =
            std::env::var("NOLA_REPLICA_E2E_TOKEN").expect("isolated Worker token required");
        let configuration = ReplicaSyncConfiguration {
            base_url: address,
            access_token,
            replica_id: "isolated-worker".into(),
            origin_device_id: "cli-sender".into(),
            page_size: 100,
        };
        let sender_host = ReplicaSyncHost::new(configuration.clone()).unwrap();
        let receiver_host = ReplicaSyncHost::new(ReplicaSyncConfiguration {
            origin_device_id: "cli-receiver".into(),
            ..configuration
        })
        .unwrap();
        let mut sender = CliApplicationPort::ephemeral().unwrap();
        let mut receiver = CliApplicationPort::ephemeral().unwrap();
        let payload = serde_json::to_string(&"x".repeat(700_000)).unwrap();
        for index in 0..7 {
            let _: Value = sender
                .execute(ApplicationCommand::PutProductSetting {
                    key: format!("real-worker-{index}"),
                    value_json: payload.clone(),
                    updated_at_ms: 1,
                })
                .unwrap();
        }
        let first = sender_host.synchronize(&mut sender).unwrap();
        assert!(first.pushed > 0 && first.pushed < 7);
        let second = sender_host.synchronize(&mut sender).unwrap();
        assert_eq!(first.pushed + second.pushed, 7);
        assert_eq!(sender_host.synchronize(&mut sender).unwrap().pushed, 0);
        let received = receiver_host.synchronize(&mut receiver).unwrap();
        assert_eq!(received.applied, 7);
        assert_eq!(received.pushed, 0);
        for index in 0..7 {
            let source: Value = sender
                .execute(ApplicationCommand::GetProductSetting {
                    key: format!("real-worker-{index}"),
                })
                .unwrap();
            let destination: Value = receiver
                .execute(ApplicationCommand::GetProductSetting {
                    key: format!("real-worker-{index}"),
                })
                .unwrap();
            assert!(!source.is_null());
            assert_eq!(destination, source);
        }
        let repeated = receiver_host.synchronize(&mut receiver).unwrap();
        assert_eq!(
            repeated.pushed, 0,
            "received facts must not create echo publications"
        );
        assert_eq!(repeated.applied, 0);
        assert_eq!(repeated.cursor, received.cursor);

        let _: Value = sender
            .execute(ApplicationCommand::CreateConversation {
                input: nola_application::CreateConversation {
                    id: "real-conversation".into(),
                    title: "Original".into(),
                    settings_json: Some(r#"{"memoryEnabledOverride":true}"#.into()),
                    created_at_ms: 1,
                    updated_at_ms: 1,
                },
            })
            .unwrap();
        let body = "界".repeat(230_000);
        for ordinal in 1..=7_i64 {
            let _: Value = sender
                .execute(
                    nola_protocol::TranscriptCommand::ImportConversationTranscript {
                        input: nola_protocol::ImportConversationTranscript {
                            import_id: format!("real-import-{ordinal}"),
                            source:
                                nola_protocol::ConversationTranscriptImportSource::ExternalArchive,
                            conversation_id: "real-conversation".into(),
                            expected_message_count: ordinal as usize - 1,
                            imported_at_ms: ordinal + 1,
                            messages: vec![nola_protocol::TranscriptMessageInput {
                                id: format!("real-message-{ordinal}"),
                                run_id: None,
                                role: "user".into(),
                                status: "completed".into(),
                                ordinal,
                                created_at_ms: 1,
                                completed_at_ms: Some(1),
                                parts: vec![nola_protocol::TranscriptPartInput {
                                    id: format!("real-part-{ordinal}"),
                                    kind: "text".into(),
                                    ordinal: 1,
                                    state: "completed".into(),
                                    content_text: format!("message-{ordinal}|{body}"),
                                    content_json: None,
                                    content_digest: Some(format!(
                                        "worker-original-digest-{ordinal}"
                                    )),
                                    created_at_ms: 100 + ordinal,
                                    updated_at_ms: 200 + ordinal,
                                }],
                            }],
                        },
                    },
                )
                .unwrap();
        }
        let _: Value = sender
            .execute(
                nola_application::ApplicationCommand::RenameConversation {
                        conversation_id: "real-conversation".into(),
                        title: "Synced profile".into(),
                        title_source: None,
                        updated_at_ms: 10,
                },
            )
            .unwrap();
        let first = sender_host.synchronize(&mut sender).unwrap();
        assert!(first.pushed > 0 && first.pushed < 9);
        let second = sender_host.synchronize(&mut sender).unwrap();
        assert_eq!(first.pushed + second.pushed, 9);
        let received = receiver_host.synchronize(&mut receiver).unwrap();
        assert_eq!(received.applied, 9);
        for ordinal in 1..=7 {
            assert_worker_part_metadata(
                &mut receiver,
                "real-conversation",
                ordinal,
                &format!("worker-original-digest-{ordinal}"),
                100 + ordinal,
                200 + ordinal,
            );
            let expected = format!("message-{ordinal}|{body}");
            let mut offset = 0_usize;
            while offset < expected.len() {
                let range: nola_protocol::ConversationTextRange = receiver
                    .execute(
                        nola_protocol::ConversationRecallCommand::ReadConversationTextRange {
                            conversation_id: "real-conversation".into(),
                            message_id: format!("real-message-{ordinal}"),
                            part_id: format!("real-part-{ordinal}"),
                            offset_bytes: offset as u64,
                            limit_bytes: 32_768,
                        },
                    )
                    .unwrap();
                let text = range.text.as_str();
                assert!(!text.is_empty() && text.len() <= 32_768);
                assert_eq!(range.offset_bytes, offset as u64);
                assert_eq!(range.total_utf8_bytes, expected.len() as u64);
                assert_eq!(range.next_offset_bytes, (offset + text.len()) as u64);
                assert_eq!(text, &expected[offset..offset + text.len()]);
                offset += text.len();
            }
        }
        let window = |application: &mut CliApplicationPort| -> Value {
            application
                .execute(ApplicationCommand::LoadConversationProjectionWindowV1 {
                    conversation_id: "real-conversation".into(),
                    before_ordinal: None,
                    limit: 10,
                    content_byte_limit: 65_536,
                })
                .unwrap()
        };
        let restored = window(&mut receiver);
        assert_eq!(restored["projection"], window(&mut sender)["projection"]);
        assert!(restored.to_string().len() < 100_000);
        let settings: Option<String> = receiver
            .execute(ApplicationCommand::LoadConversationSettings {
                conversation_id: "real-conversation".into(),
            })
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&settings.unwrap()).unwrap()["memoryEnabledOverride"],
            true
        );
        assert_eq!(receiver_host.synchronize(&mut receiver).unwrap().pushed, 0);
    }

    #[test]
    #[ignore = "Requires scripts/test_worker_replica_e2e.py --with-cli"]
    fn isolated_worker_chunked_body_survives_http_and_database_restarts() {
        use nola_store_port::transcript_publication::{
            PreparedTranscriptPublication, TranscriptPublicationRepository,
        };
        let address = std::env::var("NOLA_REPLICA_E2E_URL").expect("isolated Worker URL required");
        let url = url::Url::parse(&address).unwrap();
        assert_eq!(url.scheme(), "http");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        let configuration = ReplicaSyncConfiguration {
            base_url: address,
            access_token: std::env::var("NOLA_REPLICA_E2E_TOKEN").unwrap(),
            replica_id: "chunked-worker".into(),
            origin_device_id: "chunk-sender".into(),
            page_size: 100,
        };
        let sender_host = ReplicaSyncHost::new(configuration.clone()).unwrap();
        let receiver_host = ReplicaSyncHost::new(ReplicaSyncConfiguration {
            origin_device_id: "chunk-receiver".into(),
            ..configuration
        })
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let sender_path = root.path().join("sender.db");
        let receiver_path = root.path().join("receiver.db");
        let body = "界\\\"\n".repeat(2_500_000);
        assert_eq!(body.chars().count(), 10_000_000);
        let chunk_bytes = 30_000;
        let chunk_count = body.len().div_ceil(chunk_bytes);
        let reference = serde_json::from_value(json!({"content_id":"chunk-body","conversation_id":"chunk-conversation",
            "message_id":"chunk-message","part_id":"chunk-part","field":"content_text","total_utf8_bytes":body.len(),"chunk_count":chunk_count})).unwrap();
        // Explicit storage fixture: this is not the Mac restore entry point or
        // an externally enabled publication RPC. Transport and readers below
        // use the actual CLI Application and Worker HTTP production paths.
        {
            let mut store = nola_store::NolaStore::open(&sender_path).unwrap();
            store
                .create_conversation(&nola_store::CreateConversationInput {
                    id: "chunk-conversation".into(),
                    title: "Chunked".into(),
                    revision: 1,
                    settings_json: None,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                })
                .unwrap();
            let mut chunks =
                body.as_bytes()
                    .chunks(chunk_bytes)
                    .enumerate()
                    .map(|(index, bytes)| {
                        Ok((
                            (index * chunk_bytes) as u64,
                            std::str::from_utf8(bytes).unwrap().to_string(),
                        ))
                    });
            store
                .prepare_transcript_content(&reference, &mut chunks, 256 * 1024 * 1024)
                .unwrap();
            let append = nola_store::AppendMessagesInput {
                conversation_id: "chunk-conversation".into(),
                expected_message_count: 0,
                updated_at_ms: 2,
                messages: vec![nola_store::MessageInput {
                    id: "chunk-message".into(),
                    run_id: None,
                    role: "user".into(),
                    status: "completed".into(),
                    ordinal: 1,
                    created_at_ms: 1,
                    completed_at_ms: Some(2),
                    parts: vec![nola_store::MessagePartInput {
                        id: "chunk-part".into(),
                        kind: "text".into(),
                        ordinal: 1,
                        state: "completed".into(),
                        content_text: String::new(),
                        content_json: None,
                        content_digest: Some("worker-chunk-digest".into()),
                        created_at_ms: 101,
                        updated_at_ms: 202,
                    }],
                }],
            };
            let manifest_json = serde_json::to_string(&json!({"expected_conversation_revision":1,"transcript":{
                "import_id":"chunk-import","conversation_id":"chunk-conversation","expected_message_count":0,
                "imported_at_ms":2,"messages":append.messages},"uploads":[{"upload_id":"chunk-body","message_id":"chunk-message",
                "part_id":"chunk-part","expected_utf8_bytes":body.len()}]})).unwrap();
            store.begin_write_unit_of_work().unwrap();
            store
                .publish_transcript(&PreparedTranscriptPublication {
                    import_id: "chunk-import".into(),
                    manifest_json,
                    expected_conversation_revision: 1,
                    append,
                    references: vec![reference],
                })
                .unwrap();
            store.commit_write_unit_of_work().unwrap();
        }
        let mut total_pushed = 0;
        let mut total_applied = 0;
        let mut saw_seal = false;
        let mut rounds = 0;
        loop {
            rounds += 1;
            assert!(rounds < 20);
            let mut sender = CliApplicationPort::persistent(&sender_path).unwrap();
            let pending: Vec<PendingSyncChange> = sender
                .execute(ReplicaSyncCommand::PendingSyncChanges { limit: 100 })
                .unwrap();
            if pending.is_empty() {
                break;
            }
            let sends_append = pending
                .iter()
                .any(|entry| entry.entity_type == "conversation_append");
            if !saw_seal {
                assert!(
                    !sends_append,
                    "HTTP reference cannot precede durable content acknowledgement"
                );
            }
            assert!(pending
                .iter()
                .all(|entry| entry.payload_json.len() < 200_000));
            let pushed = sender_host.push_pending(&mut sender).unwrap();
            assert!(pushed > 0);
            total_pushed += pushed;
            saw_seal |= pending[..pushed]
                .iter()
                .any(|entry| entry.entity_type == "canonical_text_seal");
            drop(sender);
            let mut receiver = CliApplicationPort::persistent(&receiver_path).unwrap();
            let received = receiver_host.synchronize(&mut receiver).unwrap();
            assert_eq!(received.pushed, 0);
            total_applied += received.applied;
            drop(receiver);
            let database = rusqlite::Connection::open_with_flags(
                &receiver_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            let visible: i64 = database
                .query_row(
                    "SELECT COUNT(*) FROM message_parts WHERE id='chunk-part'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(visible, i64::from(sends_append));
        }
        assert!(rounds > 3 && saw_seal);
        assert_eq!(total_pushed, chunk_count + 3); // conversation, chunks, seal, append
        assert_eq!(total_applied, total_pushed);
        let mut receiver = CliApplicationPort::persistent(&receiver_path).unwrap();
        assert_worker_part_metadata(
            &mut receiver,
            "chunk-conversation",
            1,
            "worker-chunk-digest",
            101,
            202,
        );
        let mut offset = 0;
        while offset < body.len() {
            let range: nola_protocol::ConversationTextRange = receiver
                .execute(
                    nola_protocol::ConversationRecallCommand::ReadConversationTextRange {
                        conversation_id: "chunk-conversation".into(),
                        message_id: "chunk-message".into(),
                        part_id: "chunk-part".into(),
                        offset_bytes: offset as u64,
                        limit_bytes: 32768,
                    },
                )
                .unwrap();
            assert_eq!(range.total_utf8_bytes, body.len() as u64);
            assert!(!range.text.is_empty() && range.text.len() <= 32768);
            assert_eq!(range.text, body[offset..offset + range.text.len()]);
            offset += range.text.len();
            assert_eq!(range.next_offset_bytes, offset as u64);
        }
        let window: Value = receiver
            .execute(ApplicationCommand::LoadConversationProjectionWindowV1 {
                conversation_id: "chunk-conversation".into(),
                before_ordinal: None,
                limit: 10,
                content_byte_limit: 65_536,
            })
            .unwrap();
        assert!(window.to_string().len() < 100_000);
        let repeated = receiver_host.synchronize(&mut receiver).unwrap();
        assert_eq!(repeated.pushed, 0);
        assert_eq!(repeated.applied, 0);
        drop(receiver);
        let mut sender = CliApplicationPort::persistent(&sender_path).unwrap();
        let branch_intent = json!({"operation_id":"chunk-branch-operation","source_conversation_id":"chunk-conversation",
            "expected_source_revision":2,"through_message_id":"chunk-message","conversation":{"id":"chunk-branch",
            "title":"Branch","settings_json":null,"created_at_ms":3,"updated_at_ms":3},"committed_at_ms":3});
        let _: Value = sender
            .execute(ApplicationCommand::CreateConversationBranch {
                input: serde_json::from_value(branch_intent.clone()).unwrap(),
            })
            .unwrap();
        drop(sender);
        let mut branch_pushed = 0;
        let mut branch_applied = 0;
        let mut branch_rounds = 0;
        loop {
            branch_rounds += 1;
            assert!(branch_rounds < 20);
            let mut sender = CliApplicationPort::persistent(&sender_path).unwrap();
            let pending: Vec<PendingSyncChange> = sender
                .execute(ReplicaSyncCommand::PendingSyncChanges { limit: 100 })
                .unwrap();
            if pending.is_empty() {
                break;
            }
            branch_pushed += sender_host.push_pending(&mut sender).unwrap();
            drop(sender);
            let mut receiver = CliApplicationPort::persistent(&receiver_path).unwrap();
            let report = receiver_host.synchronize(&mut receiver).unwrap();
            assert_eq!(report.pushed, 0);
            branch_applied += report.applied;
        }
        assert!(branch_rounds > 3);
        assert_eq!(branch_pushed, branch_applied);
        let store = nola_store::NolaStore::open(&receiver_path).unwrap();
        let lineage = store
            .conversation_branch_lineage("chunk-branch")
            .unwrap()
            .unwrap();
        assert_eq!(serde_json::to_value(lineage).unwrap(), branch_intent);
        drop(store);
        let database = rusqlite::Connection::open_with_flags(
            &receiver_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let (message_id,part_id): (String,String) = database.query_row(
            "SELECT m.id,p.id FROM messages m JOIN message_parts p ON p.message_id=m.id WHERE m.conversation_id='chunk-branch'",[],
            |row|Ok((row.get(0)?,row.get(1)?))).unwrap();
        drop(database);
        let store = nola_store::NolaStore::open(&receiver_path).unwrap();
        let owner = serde_json::from_value(json!({
            "conversation_id": "chunk-branch", "message_id": message_id, "part_id": part_id,
        }))
        .unwrap();
        let stored = store.part_derivation_record(&owner).unwrap().unwrap();
        assert_eq!(
            stored.provenance,
            nola_store_port::part_derivation::PartDerivationProvenance::Replica
        );
        let derivation = stored.derivation;
        assert_eq!(derivation.operation_id, "chunk-branch-operation");
        assert_eq!(derivation.target.owner, owner);
        assert_eq!(
            derivation.immediate_source.owner.conversation_id(),
            "chunk-conversation"
        );
        assert_eq!(
            derivation.immediate_source.owner.message_id(),
            "chunk-message"
        );
        assert_eq!(derivation.immediate_source.owner.part_id(), "chunk-part");
        assert_eq!(
            derivation.root_source.as_ref(),
            Some(&derivation.immediate_source)
        );
        assert_eq!(derivation.target.fields, derivation.immediate_source.fields);
        assert_eq!(
            derivation.target.fields[0]
                .as_ref()
                .unwrap()
                .total_utf8_bytes,
            body.len() as u64
        );
        drop(store);
        let mut receiver = CliApplicationPort::persistent(&receiver_path).unwrap();
        assert_worker_part_metadata(
            &mut receiver,
            "chunk-branch",
            1,
            "worker-chunk-digest",
            101,
            202,
        );
        let tail_offset = body.len() - 6000;
        let tail: nola_protocol::ConversationTextRange = receiver
            .execute(
                nola_protocol::ConversationRecallCommand::ReadConversationTextRange {
                    conversation_id: "chunk-branch".into(),
                    message_id,
                    part_id,
                    offset_bytes: tail_offset as u64,
                    limit_bytes: 32768,
                },
            )
            .unwrap();
        assert_eq!(tail.text, body[tail_offset..]);
        assert_eq!(tail.total_utf8_bytes, body.len() as u64);
        assert_eq!(receiver_host.synchronize(&mut receiver).unwrap().pushed, 0);
        drop(receiver);
        // A third phase verifies archive-wide visibility through the real
        // Worker. Only staging preparation is a Store fixture in this test.
        let mut archive_claim = derivation.clone();
        archive_claim.operation_id = "archive-origin-claim".into();
        archive_claim.target = derivation.immediate_source.clone();
        archive_claim.immediate_source = derivation.target.clone();
        archive_claim.root_source = Some(derivation.target.clone());
        {
            use nola_store_port::archive_restore::{
                ArchiveConversationProfile, ArchiveMessage, ArchiveRestoreEntry,
                ArchiveRestoreRepository, BeginArchiveRestore, CommitArchiveRestore,
            };
            let mut source = nola_store::NolaStore::open(&sender_path).unwrap();
            let directory = source.archive_directory_revision().unwrap();
            source
                .begin_archive_restore(&BeginArchiveRestore {
                    restore_id: "http-archive".into(),
                    expected_directory_revision: directory,
                    created_at_ms: 4,
                })
                .unwrap();
            source
                .stage_archive_entry(
                    "http-archive",
                    0,
                    &ArchiveRestoreEntry::Profile {
                        profile: ArchiveConversationProfile {
                            id: "chunk-conversation".into(),
                            title: "Atomic restore".into(),
                            settings_json: None,
                            created_at_ms: 1,
                            updated_at_ms: 4,
                        },
                    },
                )
                .unwrap();
            let reference=serde_json::from_value(json!({"content_id":"http-archive-body","conversation_id":"chunk-conversation",
                "message_id":"chunk-message","part_id":"chunk-part","field":"content_text","total_utf8_bytes":body.len(),"chunk_count":chunk_count})).unwrap();
            let mut chunks =
                body.as_bytes()
                    .chunks(chunk_bytes)
                    .enumerate()
                    .map(|(index, bytes)| {
                        Ok((
                            (index * chunk_bytes) as u64,
                            std::str::from_utf8(bytes).unwrap().to_string(),
                        ))
                    });
            source
                .prepare_archive_content("http-archive", &reference, &mut chunks, 256 * 1024 * 1024)
                .unwrap();
            source
                .stage_archive_entry(
                    "http-archive",
                    1,
                    &ArchiveRestoreEntry::Message {
                        conversation_id: "chunk-conversation".into(),
                        message: ArchiveMessage {
                            id: "chunk-message".into(),
                            run_id: None,
                            role: "user".into(),
                            status: "completed".into(),
                            ordinal: 1,
                            created_at_ms: 1,
                            completed_at_ms: Some(4),
                            parts: vec![nola_store::MessagePartInput {
                                id: "chunk-part".into(),
                                kind: "text".into(),
                                ordinal: 1,
                                state: "completed".into(),
                                content_text: String::new(),
                                content_json: None,
                                content_digest: Some("worker-archive-digest".into()),
                                created_at_ms: 303,
                                updated_at_ms: 404,
                            }],
                        },
                        references: vec![reference],
                    },
                )
                .unwrap();
            let progress = source
                .stage_archive_entry(
                    "http-archive",
                    2,
                    &ArchiveRestoreEntry::PartDerivation {
                        record: archive_claim.clone(),
                    },
                )
                .unwrap();
            source.begin_write_unit_of_work().unwrap();
            source
                .commit_archive_restore(&CommitArchiveRestore {
                    restore_id: "http-archive".into(),
                    expected_directory_revision: directory,
                    expected_entry_count: progress.next_sequence,
                    manifest_digest: progress.manifest_digest,
                    committed_at_ms: 4,
                })
                .unwrap();
            source.commit_write_unit_of_work().unwrap();
        }
        let mut archive_facts = 0;
        let mut archive_rounds = 0;
        let mut archive_committed = false;
        loop {
            archive_rounds += 1;
            assert!(archive_rounds < 20);
            let mut sender = CliApplicationPort::persistent(&sender_path).unwrap();
            let pending: Vec<PendingSyncChange> = sender
                .execute(ReplicaSyncCommand::PendingSyncChanges { limit: 100 })
                .unwrap();
            if pending.is_empty() {
                break;
            }
            let final_commit = pending
                .iter()
                .any(|entry| entry.entity_type == "archive_restore_commit");
            if final_commit {
                assert_eq!(
                    pending.len(),
                    1,
                    "archive final must wait for stage and body ACKs"
                );
            }
            let pushed = sender_host.push_pending(&mut sender).unwrap();
            assert!(pushed > 0);
            archive_facts += pushed;
            drop(sender);
            let mut receiver = CliApplicationPort::persistent(&receiver_path).unwrap();
            let received = receiver_host.synchronize(&mut receiver).unwrap();
            assert_eq!(received.pushed, 0);
            assert_eq!(received.applied, pushed);
            drop(receiver);
            let database = rusqlite::Connection::open_with_flags(
                &receiver_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            let (title,content,branch_exists):(String,String,bool)=database.query_row(
                "SELECT c.title,json_extract(r.reference_json,'$.content_id'),EXISTS(SELECT 1 FROM conversations WHERE id='chunk-branch')
                 FROM conversations c JOIN messages m ON m.conversation_id=c.id JOIN message_parts p ON p.message_id=m.id
                 JOIN message_part_body_references r ON r.part_id=p.id AND r.field='content_text' WHERE c.id='chunk-conversation'",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
            if final_commit {
                assert_eq!(title, "Atomic restore");
                assert_eq!(content, "http-archive-body");
                assert!(!branch_exists);
            } else {
                assert_eq!(title, "Chunked");
                assert_eq!(content, "chunk-body");
                assert!(branch_exists);
            }
            let has_claim: bool = database.query_row(
                "SELECT EXISTS(SELECT 1 FROM message_part_derivations WHERE target_part_id='chunk-part' AND provenance='archive')",
                [], |row| row.get(0)).unwrap();
            assert_eq!(
                has_claim, final_commit,
                "archive provenance must publish atomically with its target"
            );
            archive_committed |= final_commit;
        }
        assert!(archive_committed && archive_rounds > 3);
        let mut receiver = CliApplicationPort::persistent(&receiver_path).unwrap();
        assert_worker_part_metadata(
            &mut receiver,
            "chunk-conversation",
            1,
            "worker-archive-digest",
            303,
            404,
        );
        let exported: nola_protocol::ExportPage = receiver
            .execute::<Option<nola_protocol::ExportPage>>(
                nola_protocol::ExportCommand::LoadConversationExportPage {
                    conversation_id: "chunk-conversation".into(),
                    after_ordinal: None,
                    expected_revision: None,
                },
            )
            .unwrap()
            .unwrap();
        assert!(exported.messages[0].parts[0].has_derivation);
        let restored: nola_protocol::StoredPartDerivation = receiver
            .execute::<Option<nola_protocol::StoredPartDerivation>>(
                nola_protocol::ExportCommand::ReadConversationPartDerivation {
                    owner: archive_claim.target.owner.clone(),
                    expected_conversation_revision: exported.conversation.revision,
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(restored.derivation, archive_claim);
        assert_eq!(
            restored.provenance,
            nola_protocol::PartDerivationProvenance::Archive
        );
        let mut offset = 0;
        while offset < body.len() {
            let range: nola_protocol::ConversationTextRange = receiver
                .execute(
                    nola_protocol::ConversationRecallCommand::ReadConversationTextRange {
                        conversation_id: "chunk-conversation".into(),
                        message_id: "chunk-message".into(),
                        part_id: "chunk-part".into(),
                        offset_bytes: offset as u64,
                        limit_bytes: 32768,
                    },
                )
                .unwrap();
            assert!(!range.text.is_empty());
            assert_eq!(range.text, body[offset..offset + range.text.len()]);
            offset += range.text.len();
        }
        assert_eq!(receiver_host.synchronize(&mut receiver).unwrap().pushed, 0);
        println!("chunked_worker_acceptance characters=10000000 bytes={} chunks={} rounds={} exact_original=true no_echo=true branch_facts={} branch_lineage=true branch_derivation=true replica_provenance=true branch_tail=true archive_facts={} archive_rounds={} archive_atomic=true archive_derivation=true archive_provenance=true",
            body.len(),chunk_count,rounds,branch_pushed,archive_facts,archive_rounds);
    }

    #[test]
    fn http_batches_acknowledge_only_the_sent_prefix_and_resume_remaining_queue() {
        let server = Server::http("127.0.0.1:0").unwrap();
        let host = host_at(format!("http://{}", server.server_addr()));
        let worker = thread::spawn(move || {
            let mut cursor = 0;
            for _ in 0..2 {
                let mut request = server
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap()
                    .unwrap();
                let mut raw = Vec::new();
                request.as_reader().read_to_end(&mut raw).unwrap();
                assert!(raw.len() <= MAX_REPLICA_PUSH_BYTES);
                let decoded: Value = serde_json::from_slice(&raw).unwrap();
                let results: Vec<_> = decoded["changes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|change| {
                        cursor += 1;
                        json!({"operationId":change["operationId"],"status":"accepted",
                        "currentRevision":change["revision"],"cursor":cursor})
                    })
                    .collect();
                request
                    .respond(Response::from_string(
                        json!({"results":results}).to_string(),
                    ))
                    .unwrap();
            }
            assert_eq!(cursor, 7);
        });
        let mut application = CliApplicationPort::ephemeral().unwrap();
        let payload = serde_json::to_string(&"x".repeat(700_000)).unwrap();
        for index in 0..7 {
            let _: Value = application
                .execute(ApplicationCommand::PutProductSetting {
                    key: format!("batch-{index}"),
                    value_json: payload.clone(),
                    updated_at_ms: 1,
                })
                .unwrap();
        }
        let before: Vec<PendingSyncChange> = application
            .execute(ReplicaSyncCommand::PendingSyncChanges { limit: 100 })
            .unwrap();
        assert_eq!(before.len(), 7);
        let first = host.push_pending(&mut application).unwrap();
        assert!(first > 0 && first < 7);
        let remaining: Vec<PendingSyncChange> = application
            .execute(ReplicaSyncCommand::PendingSyncChanges { limit: 100 })
            .unwrap();
        assert_eq!(remaining, before[first..]);
        assert_eq!(host.push_pending(&mut application).unwrap(), 7 - first);
        assert_eq!(host.push_pending(&mut application).unwrap(), 0);
        worker.join().unwrap();
    }

    #[test]
    fn immutable_seal_ends_http_batch_before_reference_publication() {
        let entries: Vec<_> = [
            "canonical_text_chunk",
            "canonical_text_seal",
            "conversation_append",
        ]
        .into_iter()
        .enumerate()
        .map(|(index, kind)| PendingSyncChange {
            operation_id: format!("op-{index}"),
            entity_type: kind.into(),
            entity_id: "body".into(),
            revision: 1,
            deleted: false,
            payload_json: "{}".into(),
            created_at_ms: 1,
        })
        .collect();
        let prefix = push_prefix(&entries, "device").unwrap();
        assert_eq!(prefix.len(), 2);
        assert_eq!(prefix.last().unwrap().entity_type, "canonical_text_seal");
    }

    #[test]
    fn a_single_http_oversized_record_is_not_skipped_to_send_later_records() {
        let mut entry = PendingSyncChange {
            operation_id: "large".into(),
            entity_type: "record".into(),
            entity_id: "r".into(),
            revision: 1,
            deleted: false,
            created_at_ms: 1,
            payload_json: serde_json::to_string(&"x".repeat(MAX_REPLICA_PUSH_BYTES)).unwrap(),
        };
        let large = entry.clone();
        entry.operation_id = "later".into();
        entry.payload_json = "{}".into();
        match push_prefix(&[large, entry], "device") {
            Err(error) => assert!(error.contains("entry remains pending")),
            Ok(_) => panic!("oversized head must not be skipped"),
        }
    }

    fn host_at(address: String) -> ReplicaSyncHost {
        ReplicaSyncHost::new(ReplicaSyncConfiguration {
            base_url: address,
            access_token: "isolated-test-token".into(),
            replica_id: "account".into(),
            origin_device_id: "test-device".into(),
            page_size: 100,
        })
        .unwrap()
    }

    #[test]
    fn duplicate_or_unknown_acknowledgements_cannot_settle_pending_operations() {
        for unknown in [false, true] {
            let server = Server::http("127.0.0.1:0").unwrap();
            let host = host_at(format!("http://{}", server.server_addr()));
            let worker = thread::spawn(move || {
                let mut request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                let mut raw = String::new();
                request.as_reader().read_to_string(&mut raw).unwrap();
                let changes = serde_json::from_str::<Value>(&raw).unwrap();
                let first = &changes["changes"][0];
                let id = if unknown {
                    "not-requested"
                } else {
                    first["operationId"].as_str().unwrap()
                };
                let result = json!({"operationId":id,"status":"accepted","currentRevision":first["revision"],"cursor":1});
                request
                    .respond(Response::from_string(
                        json!({"results":[result.clone(),result]}).to_string(),
                    ))
                    .unwrap();
            });
            let mut application = CliApplicationPort::ephemeral().unwrap();
            for key in ["first", "second"] {
                let _: Value = application
                    .execute(ApplicationCommand::PutProductSetting {
                        key: key.into(),
                        value_json: "true".into(),
                        updated_at_ms: 1,
                    })
                    .unwrap();
            }
            assert!(host
                .push_pending(&mut application)
                .unwrap_err()
                .contains("identity/revision mismatch"));
            let pending: Vec<PendingSyncChange> = application
                .execute(ReplicaSyncCommand::PendingSyncChanges { limit: 10 })
                .unwrap();
            assert_eq!(pending.len(), 2);
            worker.join().unwrap();
        }
    }

    #[test]
    fn pull_without_progress_fails_without_committing_a_cursor() {
        let server = Server::http("127.0.0.1:0").unwrap();
        let host = host_at(format!("http://{}", server.server_addr()));
        let worker = thread::spawn(move || {
            let request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            request
                .respond(Response::from_string(
                    r#"{"changes":[],"nextCursor":0,"hasMore":true}"#,
                ))
                .unwrap();
        });
        let mut application = CliApplicationPort::ephemeral().unwrap();
        assert!(host
            .pull_all(&mut application)
            .unwrap_err()
            .contains("no progress"));
        let cursor: Option<SyncCursor> = application
            .execute(ReplicaSyncCommand::GetSyncCursor {
                replica_id: "account".into(),
            })
            .unwrap();
        assert!(cursor.is_none());
        worker.join().unwrap();
    }

    #[test]
    fn http_success_and_error_bodies_have_independent_bounds() {
        for (status, size) in [
            (200, MAX_REPLICA_RESPONSE_BYTES + 1),
            (500, MAX_REPLICA_ERROR_BYTES + 1),
        ] {
            let server = Server::http("127.0.0.1:0").unwrap();
            let host = host_at(format!("http://{}", server.server_addr()));
            let worker = thread::spawn(move || {
                let request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                let _ = request
                    .respond(Response::from_string("x".repeat(size)).with_status_code(status));
            });
            let mut application = CliApplicationPort::ephemeral().unwrap();
            let error = host.pull_all(&mut application).unwrap_err();
            assert!(error.contains("exceeds"));
            assert!(error.len() < 256);
            let cursor: Option<SyncCursor> = application
                .execute(ReplicaSyncCommand::GetSyncCursor {
                    replica_id: "account".into(),
                })
                .unwrap();
            assert!(cursor.is_none());
            worker.join().unwrap();
        }
    }

    #[test]
    fn ipc_budget_accounts_for_nested_json_escaping() {
        let command = ReplicaSyncCommand::ApplyRemoteSyncPage {
            input: RemoteSyncPage {
                replica_id: "account".into(),
                next_cursor: 1,
                applied_at_ms: 1,
                changes: vec![RemoteSyncChange {
                    cursor: 1,
                    operation_id: "op".into(),
                    entity_type: "record".into(),
                    entity_id: "r".into(),
                    revision: 1,
                    deleted: false,
                    created_at_ms: 1,
                    payload_json: serde_json::to_string(&"\n".repeat(4 * 1024 * 1024)).unwrap(),
                }],
            },
        };
        let ReplicaSyncCommand::ApplyRemoteSyncPage { input } = command else {
            panic!("wrong fixture")
        };
        assert!(input.validate(0).unwrap_err().contains("byte budget"));
    }

    #[test]
    fn authenticated_http_round_trip_uses_core_outbox_and_atomic_apply() {
        let server = Server::http("127.0.0.1:0").unwrap();
        let address = format!("http://{}", server.server_addr());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let worker = thread::spawn(move || {
            for index in 0..2 {
                let mut request = server.recv().unwrap();
                captured.lock().unwrap().push((
                    request.method().as_str().to_string(),
                    request.url().to_string(),
                    request
                        .headers()
                        .iter()
                        .find(|header| header.field.equiv("authorization"))
                        .map(|header| header.value.as_str().to_string()),
                ));
                let body = if index == 0 {
                    let mut raw = String::new();
                    request.as_reader().read_to_string(&mut raw).unwrap();
                    let operation_id = serde_json::from_str::<Value>(&raw).unwrap()["changes"][0]
                        ["operationId"]
                        .as_str()
                        .unwrap()
                        .to_string();
                    json!({"results":[{"operationId":operation_id,"status":"accepted","currentRevision":1,"cursor":1}]})
                } else {
                    json!({
                        "changes":[{
                            "cursor":1,
                            "operationId":"remote-op",
                            "entityType":"conversation",
                            "entityId":"remote-conversation",
                            "revision":1,
                            "deleted":false,
                            "payload":{
                                "id":"remote-conversation","title":"From another device","revision":1,"configuration_revision":1,
                                "created_at_ms":2,"updated_at_ms":2,"settings_json":null,
                                "messages":[{
                                    "id":"remote-user","run_id":null,"role":"user","status":"completed",
                                    "ordinal":1,"created_at_ms":2,"completed_at_ms":2,
                                    "parts":[{"id":"remote-part","kind":"text","ordinal":1,"state":"completed","content_text":"Remote complete message","content_json":null,"content_json_deferred":false,"content_digest":"original-part-digest","created_at_ms":1,"updated_at_ms":2}]
                                }]
                            },
                            "originDeviceId":"other-device",
                            "committedAt":"2026-08-20T00:00:00Z"
                        }],
                        "nextCursor":1,
                        "hasMore":false
                    })
                };
                let response = Response::from_string(body.to_string())
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
                request.respond(response).unwrap();
            }
        });

        let mut application = CliApplicationPort::ephemeral().unwrap();
        let _: Value = application
            .execute(ApplicationCommand::PutProductSetting {
                key: "local.theme".to_string(),
                value_json: "\"sand\"".to_string(),
                updated_at_ms: 1,
            })
            .unwrap();
        let host = ReplicaSyncHost::new(ReplicaSyncConfiguration {
            base_url: address,
            access_token: "test-token".to_string(),
            replica_id: "account".to_string(),
            origin_device_id: "device-a".to_string(),
            page_size: 100,
        })
        .unwrap();
        let report = host.synchronize(&mut application).unwrap();
        assert_eq!(
            report,
            ReplicaSyncReport {
                pushed: 1,
                applied: 1,
                skipped: 0,
                cursor: 1
            }
        );
        let remote: Option<nola_application::Conversation> = application
            .execute(ApplicationCommand::LoadConversation {
                conversation_id: "remote-conversation".to_string(),
            })
            .unwrap();
        let remote = remote.unwrap();
        let part = &remote.messages[0].parts[0];
        assert_eq!(part.content_text, "Remote complete message");
        assert!(!part.content_json_deferred);
        assert_eq!(part.content_digest.as_deref(), Some("original-part-digest"));
        assert_eq!(part.created_at_ms, 1);
        assert_eq!(part.updated_at_ms, 2);
        let pending: Vec<PendingSyncChange> = application
            .execute(ReplicaSyncCommand::PendingSyncChanges { limit: 10 })
            .unwrap();
        assert!(
            pending.is_empty(),
            "remote apply must not create an echo outbox entry"
        );
        worker.join().unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].2.as_deref(), Some("Bearer test-token"));
        assert!(requests[1].1.contains("cursor=0"));
    }
}
