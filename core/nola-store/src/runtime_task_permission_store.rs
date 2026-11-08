// SPDX-License-Identifier: Apache-2.0
//! Durable permission requests associated with runtime-task execution.

use super::*;

impl NolaStore {
    pub fn enqueue_permission_request(
        &mut self,
        request: &StoredPermissionRequest,
    ) -> StoreResult<()> {
        validate_permission_request(request)?;
        let transaction = self.write_transaction()?;
        permission_transport::require_admission(&transaction, &request.task_id)?;
        transaction.execute(
            "INSERT INTO permission_requests(
                request_id,task_id,child_session_id,capability,input_json,scope_rule,status,
                decision,custom_response,created_at_ms,resolved_at_ms,core_permission_request_id,permission_instructions_json
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                request.request_id,
                request.task_id,
                request.child_session_id,
                request.capability,
                request.input_json,
                request.scope_rule,
                request.status,
                request.decision,
                request.custom_response,
                request.created_at_ms,
                request.resolved_at_ms,
                request.core_permission_request_id,
                request.permission_instructions_json,
            ],
        )?;
        permission_source::retain_bound_source(&transaction, request)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn permission_request(
        &self,
        request_id: &str,
    ) -> StoreResult<Option<StoredPermissionRequest>> {
        require_id("permission_request.request_id", request_id)?;
        self.connection
            .query_row(
                &format!(
                    "SELECT {PERMISSION_REQUEST_COLUMNS} FROM permission_requests
                     WHERE request_id=?1"
                ),
                params![request_id],
                permission_request_row,
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn pending_permission_for_task(
        &self,
        task_id: &str,
    ) -> StoreResult<Option<StoredPermissionRequest>> {
        require_id("permission_request.task_id", task_id)?;
        self.connection
            .query_row(
                &format!(
                    "SELECT {PERMISSION_REQUEST_COLUMNS} FROM permission_requests
                     WHERE task_id=?1 AND status='pending'
                     ORDER BY created_at_ms,request_id LIMIT 1"
                ),
                params![task_id],
                permission_request_row,
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn pending_permissions_for_owner(
        &self,
        owner_session_id: &str,
    ) -> StoreResult<Vec<StoredPermissionRequest>> {
        require_id("permission_request.owner_session_id", owner_session_id)?;
        let mut statement = self.connection.prepare(&format!(
            "SELECT {} FROM permission_requests p
             JOIN durable_tasks t ON t.id=p.task_id
             WHERE t.owner_session_id=?1 AND p.status='pending'
             ORDER BY p.created_at_ms,p.request_id",
            PERMISSION_REQUEST_QUALIFIED_COLUMNS
        ))?;
        let rows = statement.query_map(params![owner_session_id], permission_request_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn resolve_permission_request(
        &mut self,
        request_id: &str,
        decision: &str,
        custom_response: Option<&str>,
        resolved_at_ms: i64,
        grant: Option<&UpsertPermissionGrantInput>,
    ) -> StoreResult<nola_store_port::permission::StoredPermissionResolutionOutcome> {
        permission_transport::resolve(self, request_id, decision, custom_response, resolved_at_ms, grant)
    }

}
