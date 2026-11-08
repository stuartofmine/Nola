// SPDX-License-Identifier: Apache-2.0
//! Permission repository on the owning Store's existing connection and UoW.
use crate::{NolaStore, StoreError};
use nola_store_port::permission::{
    PermissionRepository, StoredPermissionGrant, StoredPermissionRequest,
    UpsertPermissionGrantInput, StoredPermissionResolutionOutcome,
};
impl PermissionRepository for NolaStore {
    type Error = StoreError;
    fn ensure_core_permission_request(
        &mut self,
        request: &StoredPermissionRequest,
    ) -> Result<StoredPermissionRequest, Self::Error> {
        crate::permission_transport::ensure(self, request)
    }
    fn enqueue_permission_request(
        &mut self,
        request: &StoredPermissionRequest,
    ) -> Result<(), Self::Error> {
        NolaStore::enqueue_permission_request(self, request)
    }
    fn permission_request(
        &self,
        request_id: &str,
    ) -> Result<Option<StoredPermissionRequest>, Self::Error> {
        NolaStore::permission_request(self, request_id)
    }
    fn pending_permission_for_task(
        &self,
        task_id: &str,
    ) -> Result<Option<StoredPermissionRequest>, Self::Error> {
        NolaStore::pending_permission_for_task(self, task_id)
    }
    fn pending_permissions_for_owner(
        &self,
        owner_session_id: &str,
    ) -> Result<Vec<StoredPermissionRequest>, Self::Error> {
        NolaStore::pending_permissions_for_owner(self, owner_session_id)
    }
    fn resolve_permission_request(
        &mut self,
        request_id: &str,
        decision: &str,
        custom_response: Option<&str>,
        resolved_at_ms: i64,
        grant: Option<&UpsertPermissionGrantInput>,
    ) -> Result<StoredPermissionResolutionOutcome, Self::Error> {
        NolaStore::resolve_permission_request(
            self,
            request_id,
            decision,
            custom_response,
            resolved_at_ms,
            grant,
        )
    }
    fn upsert_permission_grant(
        &mut self,
        input: &UpsertPermissionGrantInput,
    ) -> Result<StoredPermissionGrant, Self::Error> {
        NolaStore::upsert_permission_grant(self, input)
    }
    fn revoke_permission_grant(
        &mut self,
        id: &str,
        revoked_at_ms: i64,
    ) -> Result<bool, Self::Error> {
        NolaStore::revoke_permission_grant(self, id, revoked_at_ms)
    }
    fn effective_permission_grants(
        &self,
        conversation_id: Option<&str>,
        host_id: Option<&str>,
    ) -> Result<Vec<StoredPermissionGrant>, Self::Error> {
        NolaStore::effective_permission_grants(self, conversation_id, host_id)
    }
}
