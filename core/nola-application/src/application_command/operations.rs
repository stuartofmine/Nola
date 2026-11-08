// SPDX-License-Identifier: Apache-2.0
//! Background operations and workspace ownership command dispatch.

use serde_json::{json, Value};

use super::{encode_application_result, ApplicationCommand};
use crate::repositories::OperationsCommandRepositories;

pub(super) fn execute<R: OperationsCommandRepositories>(
    store: &mut R,
    command: ApplicationCommand,
) -> Result<Value, String> {
    match command {
        ApplicationCommand::SaveWorkspaceCheckpoint { input } => {
            let mut workspace = store.workspace_ownership_repository();
            Ok(json!(
                crate::workspace_ownership::save_workspace_checkpoint(&mut workspace, input,)?
            ))
        }
        ApplicationCommand::ListWorkspaceCheckpoints { root } => {
            let workspace = store.workspace_ownership_repository();
            encode_application_result(crate::workspace_ownership::list_workspace_checkpoints(
                &workspace, &root,
            )?)
        }
        ApplicationCommand::DeleteWorkspaceCheckpoint { id } => {
            let mut workspace = store.workspace_ownership_repository();
            Ok(json!(
                nola_store_port::workspace_ownership::WorkspaceOwnershipRepository::delete_workspace_checkpoint(
                    &mut workspace,
                    &id,
                )
                .map_err(|error| error.to_string())?
            ))
        }
        ApplicationCommand::AcquireSessionRunLease { input } => {
            let mut workspace = store.workspace_ownership_repository();
            Ok(json!(
                crate::workspace_ownership::acquire_session_run_lease(&mut workspace, input,)?
            ))
        }
        ApplicationCommand::GetSessionRunLease { session_id } => {
            let workspace = store.workspace_ownership_repository();
            encode_application_result(crate::workspace_ownership::session_run_lease(
                &workspace,
                &session_id,
            )?)
        }
        ApplicationCommand::ListSessionRunLeases => {
            let workspace = store.workspace_ownership_repository();
            encode_application_result(crate::workspace_ownership::session_run_leases(&workspace)?)
        }
        ApplicationCommand::ReleaseSessionRunLease { session_id, owner } => {
            let mut workspace = store.workspace_ownership_repository();
            Ok(json!(
                nola_store_port::workspace_ownership::WorkspaceOwnershipRepository::release_session_run_lease(
                    &mut workspace,
                    &session_id,
                    &owner,
                )
                .map_err(|error| error.to_string())?
            ))
        }
        ApplicationCommand::Automation { command } => {
            let mut automation = store.automation_repository();
            crate::automation::execute_automation(&mut automation, command)
        }
        command @ (ApplicationCommand::PendingSyncChanges { .. }
        | ApplicationCommand::ResolveSyncChange { .. }
        | ApplicationCommand::GetSyncCursor { .. }
        | ApplicationCommand::SaveSyncCursor { .. }
        | ApplicationCommand::ApplyRemoteSyncPage { .. }) => {
            let mut sync = store.replica_sync_repository();
            crate::replica_sync::execute(&mut sync, command)
        }
        _ => Err("command is outside the operations family".into()),
    }
}
