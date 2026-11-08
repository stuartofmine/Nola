// SPDX-License-Identifier: Apache-2.0
//! Application-owned conversions; transport contracts live in Protocol.
use nola_protocol::{
    PendingSyncChange, RemoteSyncPage, RemoteSyncPageResult, ReplicaSyncCommand, SyncCursor,
};

pub(super) fn execute<R: nola_store_port::replica_sync::ReplicaSyncRepository + ?Sized>(
    store: &mut R,
    command: super::ApplicationCommand,
) -> Result<serde_json::Value, String> {
    use super::ApplicationCommand;
    use serde_json::json;
    fn encode<T: serde::Serialize>(value: T) -> Result<serde_json::Value, String> {
        serde_json::to_value(value).map_err(|error| error.to_string())
    }
    match command {
        ApplicationCommand::PendingSyncChanges { limit } => {
            let changes = store
                .pending_sync_changes(limit)
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(pending_change)
                .collect::<Vec<_>>();
            encode(changes)
        }
        ApplicationCommand::ResolveSyncChange {
            operation_id,
            status,
            remote_cursor,
            resolved_at_ms,
        } => Ok(json!(store
            .resolve_sync_change(&operation_id, &status, remote_cursor, resolved_at_ms,)
            .map_err(|error| error.to_string())?)),
        ApplicationCommand::GetSyncCursor { replica_id } => {
            let cursor = store
                .sync_cursor(&replica_id)
                .map_err(|error| error.to_string())?
                .map(to_wire_cursor);
            encode(cursor)
        }
        ApplicationCommand::SaveSyncCursor {
            replica_id,
            cursor,
            updated_at_ms,
        } => {
            let cursor = store
                .save_sync_cursor(&replica_id, cursor, updated_at_ms)
                .map_err(|error| error.to_string())?;
            encode(to_wire_cursor(cursor))
        }
        ApplicationCommand::ApplyRemoteSyncPage { input } => {
            let committed_cursor = store
                .sync_cursor(&input.replica_id)
                .map_err(|error| error.to_string())?
                .map(|cursor| cursor.cursor)
                .unwrap_or(0);
            input.validate(committed_cursor)?;
            let store_input = remote_page(input);
            let result = store
                .apply_remote_sync_page(&store_input)
                .map_err(|error| error.to_string())?;
            encode(page_result(result))
        }
        _ => Err("expected a replica synchronization command".into()),
    }
}

pub(super) fn pending_change(
    change: nola_store_port::replica_sync::StoredSyncChange,
) -> PendingSyncChange {
    PendingSyncChange {
        operation_id: change.operation_id,
        entity_type: change.entity_type,
        entity_id: change.entity_id,
        revision: change.revision,
        deleted: change.deleted,
        payload_json: change.payload_json,
        created_at_ms: change.created_at_ms,
    }
}
pub(super) fn to_wire_cursor(
    cursor: nola_store_port::replica_sync::StoredSyncCursor,
) -> SyncCursor {
    SyncCursor {
        replica_id: cursor.replica_id,
        cursor: cursor.cursor,
        updated_at_ms: cursor.updated_at_ms,
    }
}
pub(super) fn remote_page(
    page: RemoteSyncPage,
) -> nola_store_port::replica_sync::ApplyRemoteSyncPageInput {
    nola_store_port::replica_sync::ApplyRemoteSyncPageInput {
        replica_id: page.replica_id,
        next_cursor: page.next_cursor,
        applied_at_ms: page.applied_at_ms,
        changes: page
            .changes
            .into_iter()
            .map(
                |change| nola_store_port::replica_sync::RemoteSyncChangeInput {
                    cursor: change.cursor,
                    operation_id: change.operation_id,
                    entity_type: change.entity_type,
                    entity_id: change.entity_id,
                    revision: change.revision,
                    deleted: change.deleted,
                    payload_json: change.payload_json,
                    created_at_ms: change.created_at_ms,
                },
            )
            .collect(),
    }
}
pub(super) fn page_result(
    result: nola_store_port::replica_sync::ApplyRemoteSyncPageResult,
) -> RemoteSyncPageResult {
    RemoteSyncPageResult {
        applied: result.applied,
        skipped: result.skipped,
        cursor: result.cursor,
    }
}
impl From<ReplicaSyncCommand> for super::ApplicationCommand {
    fn from(command: ReplicaSyncCommand) -> Self {
        match command {
            ReplicaSyncCommand::PendingSyncChanges { limit } => Self::PendingSyncChanges { limit },
            ReplicaSyncCommand::ResolveSyncChange {
                operation_id,
                status,
                remote_cursor,
                resolved_at_ms,
            } => Self::ResolveSyncChange {
                operation_id,
                status,
                remote_cursor,
                resolved_at_ms,
            },
            ReplicaSyncCommand::GetSyncCursor { replica_id } => Self::GetSyncCursor { replica_id },
            ReplicaSyncCommand::SaveSyncCursor {
                replica_id,
                cursor,
                updated_at_ms,
            } => Self::SaveSyncCursor {
                replica_id,
                cursor,
                updated_at_ms,
            },
            ReplicaSyncCommand::ApplyRemoteSyncPage { input } => {
                Self::ApplyRemoteSyncPage { input }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use nola_store_port::replica_sync::*;

    struct RejectingReplica {
        applied_calls: usize,
    }
    impl ReplicaSyncRepository for RejectingReplica {
        type Error = &'static str;
        fn pending_sync_changes(&self, _: usize) -> Result<Vec<StoredSyncChange>, Self::Error> {
            unreachable!()
        }
        fn resolve_sync_change(
            &mut self,
            _: &str,
            _: &str,
            _: Option<i64>,
            _: i64,
        ) -> Result<bool, Self::Error> {
            unreachable!()
        }
        fn sync_cursor(&self, replica: &str) -> Result<Option<StoredSyncCursor>, Self::Error> {
            Ok(Some(StoredSyncCursor {
                replica_id: replica.into(),
                cursor: 4,
                updated_at_ms: 1,
            }))
        }
        fn save_sync_cursor(
            &mut self,
            _: &str,
            _: i64,
            _: i64,
        ) -> Result<StoredSyncCursor, Self::Error> {
            panic!(
                "Application must not acknowledge a page outside the atomic repository operation"
            )
        }
        fn apply_remote_sync_page(
            &mut self,
            input: &ApplyRemoteSyncPageInput,
        ) -> Result<ApplyRemoteSyncPageResult, Self::Error> {
            self.applied_calls += 1;
            assert_eq!(input.changes[0].payload_json, "original fact");
            Err("missing sealed body")
        }
    }

    #[test]
    fn non_sqlite_port_keeps_page_validation_and_failed_acknowledgement_at_the_application_boundary(
    ) {
        let mut repository = RejectingReplica { applied_calls: 0 };
        let mut page = RemoteSyncPage {
            replica_id: "r".into(),
            next_cursor: 6,
            applied_at_ms: 2,
            changes: vec![nola_protocol::RemoteSyncChange {
                cursor: 5,
                operation_id: "op".into(),
                entity_type: "conversation_append".into(),
                entity_id: "c".into(),
                revision: 2,
                deleted: false,
                payload_json: "original fact".into(),
                created_at_ms: 1,
            }],
        };
        assert!(execute(
            &mut repository,
            ReplicaSyncCommand::ApplyRemoteSyncPage {
                input: page.clone()
            }
            .into()
        )
        .is_err());
        assert_eq!(
            repository.applied_calls, 0,
            "invalid cursor never reaches persistence"
        );
        page.next_cursor = 5;
        assert_eq!(
            execute(
                &mut repository,
                ReplicaSyncCommand::ApplyRemoteSyncPage { input: page }.into()
            )
            .unwrap_err(),
            "missing sealed body"
        );
        assert_eq!(repository.applied_calls, 1);
        assert_eq!(repository.sync_cursor("r").unwrap().unwrap().cursor, 4);
    }

    #[test]
    fn conversion_moves_payload_without_copying_transcript_bytes() {
        let payload = "x".repeat(1_000_000);
        let pointer = payload.as_ptr();
        let page = RemoteSyncPage {
            replica_id: "r".into(),
            next_cursor: 1,
            applied_at_ms: 2,
            changes: vec![nola_protocol::RemoteSyncChange {
                cursor: 1,
                operation_id: "op".into(),
                entity_type: "conversation_append".into(),
                entity_id: "c".into(),
                revision: 2,
                deleted: false,
                payload_json: payload,
                created_at_ms: 1,
            }],
        };
        let command: super::super::ApplicationCommand =
            ReplicaSyncCommand::ApplyRemoteSyncPage { input: page }.into();
        let super::super::ApplicationCommand::ApplyRemoteSyncPage { input } = command else {
            panic!("wrong command")
        };
        assert_eq!(remote_page(input).changes[0].payload_json.as_ptr(), pointer);
    }
}
