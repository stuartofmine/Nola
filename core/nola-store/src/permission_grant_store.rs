// SPDX-License-Identifier: Apache-2.0
//! Scoped permission grant persistence, revocation, and effective grant queries.

use super::*;

impl NolaStore {
    pub fn upsert_permission_grant(
        &mut self,
        input: &UpsertPermissionGrantInput,
    ) -> StoreResult<StoredPermissionGrant> {
        require_id("permission_grant.id", &input.id)?;
        // This namespace is owned by permission decision transactions. Generic
        // settings edits must neither preempt nor revive request-derived grants.
        if input.id.starts_with("permission-request:") {
            return Err(StoreError::Invariant(
                "request-derived permission grants can only be created by a permission decision".into(),
            ));
        }
        require_id(
            "permission_grant.capability_pattern",
            &input.capability_pattern,
        )?;
        require_id("permission_grant.resource_pattern", &input.resource_pattern)?;
        match input.scope {
            PermissionGrantScope::UserGlobal
                if input.conversation_id.is_some() || input.host_id.is_some() =>
            {
                return Err(StoreError::Invariant(
                    "user_global permission grant cannot target a conversation or Host".into(),
                ));
            }
            PermissionGrantScope::Host
                if input
                    .host_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .is_none()
                    || input.conversation_id.is_some() =>
            {
                return Err(StoreError::Invariant(
                    "Host permission grant requires only host_id".into(),
                ));
            }
            PermissionGrantScope::Conversation
                if input
                    .conversation_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .is_none()
                    || input.host_id.is_some() =>
            {
                return Err(StoreError::Invariant(
                    "conversation permission grant requires only conversation_id".into(),
                ));
            }
            _ => {}
        }
        let transaction = self.write_transaction()?;
        transaction.execute(
            "INSERT INTO permission_grants(
                id, scope, conversation_id, host_id, capability_pattern,
                resource_pattern, decision, created_at_ms, updated_at_ms, revoked_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)
             ON CONFLICT(id) DO UPDATE SET
                scope = excluded.scope,
                conversation_id = excluded.conversation_id,
                host_id = excluded.host_id,
                capability_pattern = excluded.capability_pattern,
                resource_pattern = excluded.resource_pattern,
                decision = excluded.decision,
                updated_at_ms = excluded.updated_at_ms,
                revoked_at_ms = NULL",
            params![
                input.id,
                permission_scope_name(input.scope),
                input.conversation_id,
                input.host_id,
                input.capability_pattern,
                input.resource_pattern,
                permission_decision_name(input.decision),
                input.created_at_ms,
                input.updated_at_ms,
            ],
        )?;
        transaction.commit()?;
        self.permission_grant(&input.id)?
            .ok_or_else(|| StoreError::Invariant("upserted permission grant is unavailable".into()))
    }

    pub fn revoke_permission_grant(&mut self, id: &str, revoked_at_ms: i64) -> StoreResult<bool> {
        require_id("permission_grant.id", id)?;
        Ok(self.connection.execute(
            "UPDATE permission_grants
             SET revoked_at_ms = ?2, updated_at_ms = ?2
             WHERE id = ?1 AND revoked_at_ms IS NULL",
            params![id, revoked_at_ms],
        )? == 1)
    }

    pub fn effective_permission_grants(
        &self,
        conversation_id: Option<&str>,
        host_id: Option<&str>,
    ) -> StoreResult<Vec<StoredPermissionGrant>> {
        let mut statement = self.connection.prepare(
            "SELECT id, scope, conversation_id, host_id, capability_pattern,
                    resource_pattern, decision, created_at_ms, updated_at_ms, revoked_at_ms
             FROM permission_grants
             WHERE revoked_at_ms IS NULL
               AND (scope = 'user_global'
                    OR (scope = 'host' AND host_id = ?2)
                    OR (scope = 'conversation' AND conversation_id = ?1))
             ORDER BY CASE scope
                        WHEN 'user_global' THEN 0
                        WHEN 'host' THEN 1
                        WHEN 'conversation' THEN 2
                      END,
                      updated_at_ms,
                      id",
        )?;
        let rows = statement.query_map(params![conversation_id, host_id], permission_grant_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    fn permission_grant(&self, id: &str) -> StoreResult<Option<StoredPermissionGrant>> {
        self.connection
            .query_row(
                "SELECT id, scope, conversation_id, host_id, capability_pattern,
                        resource_pattern, decision, created_at_ms, updated_at_ms, revoked_at_ms
                 FROM permission_grants WHERE id = ?1",
                params![id],
                permission_grant_row,
            )
            .optional()
            .map_err(StoreError::from)
    }
}
