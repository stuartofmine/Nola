// SPDX-License-Identifier: Apache-2.0
//! Replica commands and DTOs, independent of Application and Store.
use serde::{Deserialize, Serialize};
use std::io::Write;

pub const MAX_REPLICA_PAGE_BYTES: usize = 12 * 1024 * 1024;
pub const MAX_REPLICA_PAGE_ITEMS: usize = 500;

struct ReplicaByteBudget(usize);
impl Write for ReplicaByteBudget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "replica page exceeds byte budget; chunked transfer required",
            ));
        }
        self.0 -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingSyncChange {
    pub operation_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub revision: i64,
    pub deleted: bool,
    pub payload_json: String,
    pub created_at_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncCursor {
    pub replica_id: String,
    pub cursor: i64,
    pub updated_at_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteSyncChange {
    pub cursor: i64,
    pub operation_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub revision: i64,
    pub deleted: bool,
    pub payload_json: String,
    pub created_at_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteSyncPage {
    pub replica_id: String,
    pub changes: Vec<RemoteSyncChange>,
    pub next_cursor: i64,
    pub applied_at_ms: i64,
}
impl RemoteSyncPage {
    /// Validate a wire page against the committed cursor without copying bodies.
    /// Application must call this even when the transport is embedded, not IPC.
    pub fn validate(&self, committed_cursor: i64) -> Result<(), String> {
        if committed_cursor < 0 || self.next_cursor < committed_cursor {
            return Err("replica page cursor would regress".into());
        }
        if self.changes.len() > MAX_REPLICA_PAGE_ITEMS {
            return Err("replica page exceeds item budget".into());
        }
        let mut previous = committed_cursor;
        for change in &self.changes {
            if change.cursor <= previous {
                return Err("replica page cursor must advance in strict order".into());
            }
            previous = change.cursor;
        }
        if self.next_cursor != previous {
            return Err("replica page cursor does not match its last change".into());
        }
        serde_json::to_writer(&mut ReplicaByteBudget(MAX_REPLICA_PAGE_BYTES), self)
            .map_err(|error| format!("replica page rejected before submission: {error}"))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteSyncPageResult {
    pub applied: usize,
    pub skipped: usize,
    pub cursor: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReplicaSyncCommand {
    PendingSyncChanges {
        limit: usize,
    },
    ResolveSyncChange {
        operation_id: String,
        status: String,
        remote_cursor: Option<i64>,
        resolved_at_ms: i64,
    },
    GetSyncCursor {
        replica_id: String,
    },
    SaveSyncCursor {
        replica_id: String,
        cursor: i64,
        updated_at_ms: i64,
    },
    ApplyRemoteSyncPage {
        input: RemoteSyncPage,
    },
}
impl crate::CommandMetadata for ReplicaSyncCommand {
    fn wire_command_name(&self) -> &'static str {
        match self {
            Self::PendingSyncChanges { .. } => "pending_sync_changes",
            Self::ResolveSyncChange { .. } => "resolve_sync_change",
            Self::GetSyncCursor { .. } => "get_sync_cursor",
            Self::SaveSyncCursor { .. } => "save_sync_cursor",
            Self::ApplyRemoteSyncPage { .. } => "apply_remote_sync_page",
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommandMetadata;
    #[test]
    fn pages_cannot_acknowledge_unreceived_records_or_regress() {
        let mut page = RemoteSyncPage {
            replica_id: "r".into(),
            changes: vec![],
            next_cursor: 9,
            applied_at_ms: 1,
        };
        assert!(page.validate(0).is_err());
        page.next_cursor = 0;
        assert!(page.validate(0).is_ok());
        assert!(page.validate(1).is_err());
        let change = RemoteSyncChange {
            cursor: 2,
            operation_id: "op".into(),
            entity_type: "conversation".into(),
            entity_id: "c".into(),
            revision: 1,
            deleted: false,
            payload_json: "{}".into(),
            created_at_ms: 1,
        };
        page.changes = vec![change.clone()];
        page.next_cursor = 2;
        assert!(
            page.validate(0).is_ok(),
            "global cursors may have gaps between committed changes"
        );
        page.next_cursor = 3;
        assert!(page.validate(0).is_err());
        page.next_cursor = 2;
        page.changes.push(change);
        assert!(page.validate(0).is_err());
    }
    #[test]
    fn commands_round_trip_with_stable_diagnostic_names() {
        for command in [
            ReplicaSyncCommand::PendingSyncChanges { limit: 10 },
            ReplicaSyncCommand::ResolveSyncChange {
                operation_id: "op".into(),
                status: "acknowledged".into(),
                remote_cursor: Some(1),
                resolved_at_ms: 2,
            },
            ReplicaSyncCommand::GetSyncCursor {
                replica_id: "remote".into(),
            },
            ReplicaSyncCommand::SaveSyncCursor {
                replica_id: "remote".into(),
                cursor: 1,
                updated_at_ms: 2,
            },
            ReplicaSyncCommand::ApplyRemoteSyncPage {
                input: RemoteSyncPage {
                    replica_id: "remote".into(),
                    changes: vec![],
                    next_cursor: 0,
                    applied_at_ms: 2,
                },
            },
        ] {
            let value = serde_json::to_value(&command).unwrap();
            assert_eq!(value["type"], command.wire_command_name());
            assert_eq!(
                serde_json::from_value::<ReplicaSyncCommand>(value).unwrap(),
                command
            );
        }
    }
    #[test]
    fn pending_change_hides_storage_queue_state() {
        let value = serde_json::to_value(PendingSyncChange {
            operation_id: "op".into(),
            entity_type: "conversation_metadata".into(),
            entity_id: "c".into(),
            revision: 2,
            deleted: false,
            payload_json: "{}".into(),
            created_at_ms: 2,
        })
        .unwrap();
        for key in [
            "local_sequence",
            "status",
            "remote_cursor",
            "resolved_at_ms",
        ] {
            assert!(value.get(key).is_none());
        }
    }
}
