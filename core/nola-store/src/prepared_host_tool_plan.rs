// SPDX-License-Identifier: Apache-2.0
//! Immutable source-owned preparation; no generic setting or sync projection.
use super::*;
use nola_domain::host_tool_plan::{HostToolPlanReference, HostToolPlanSnapshot};
use nola_store_port::prepared_host_tool_plan::PreparedHostToolPlanRepository;

impl PreparedHostToolPlanRepository for NolaStore {
    type Error = StoreError;

    fn persist_prepared_host_tool_plan(&mut self, snapshot: &HostToolPlanSnapshot, now_ms: u64) -> StoreResult<()> {
        let reference = &snapshot.reference;
        snapshot.validate(&reference.host_id, &reference.client_id, &reference.session_id, Some(now_ms))
            .map_err(StoreError::Invariant)?;
        let raw = serde_json::to_string(snapshot)?;
        if raw.len() > 512 * 1024 { return Err(StoreError::Invariant("Host tool plan exceeds budget".into())); }
        let now = i64::try_from(now_ms).map_err(|_| StoreError::Invariant("Invalid preparation time".into()))?;
        let expiry = i64::try_from(snapshot.expires_at_ms).map_err(|_| StoreError::Invariant("Invalid preparation expiry".into()))?;
        let transaction = self.write_transaction()?;
        // Only expired, generated preparation records are reclaimed. Admitted
        // snapshots remain owned by their Run and are never touched here.
        transaction.execute("DELETE FROM prepared_host_tool_plans WHERE expires_at_ms<=?1", [now])?;
        let count: i64 = transaction.query_row("SELECT COUNT(*) FROM prepared_host_tool_plans", [], |row| row.get(0))?;
        if count >= 256 { return Err(StoreError::Conflict("Host tool plan preparation capacity exhausted".into())); }
        transaction.execute("INSERT INTO prepared_host_tool_plans(manifest_id,host_id,client_id,session_id,expires_at_ms,snapshot_json) VALUES(?1,?2,?3,?4,?5,?6)",
            params![reference.manifest_id, reference.host_id, reference.client_id, reference.session_id, expiry, raw])?;
        transaction.commit()
    }

    fn prepared_host_tool_plan(&self, reference: &HostToolPlanReference, now_ms: u64) -> StoreResult<Option<HostToolPlanSnapshot>> {
        let raw: Option<String> = self.connection.query_row(
            "SELECT snapshot_json FROM prepared_host_tool_plans WHERE manifest_id=?1 AND host_id=?2 AND client_id=?3 AND session_id=?4",
            params![reference.manifest_id, reference.host_id, reference.client_id, reference.session_id], |row| row.get(0)).optional()?;
        raw.map(|raw| {
            let plan: HostToolPlanSnapshot = serde_json::from_str(&raw)?;
            if plan.reference != *reference { return Err(StoreError::Conflict("Host tool plan reference mismatch".into())); }
            plan.validate(&reference.host_id, &reference.client_id, &reference.session_id, Some(now_ms)).map_err(StoreError::Invariant)?;
            Ok(plan)
        }).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(id: &str) -> HostToolPlanSnapshot {
        HostToolPlanSnapshot { schema_version: 1, reference: HostToolPlanReference {
            manifest_id: id.into(), host_id: "host".into(), client_id: "client".into(),
            session_id: "room-context".into(), catalog_revision: "a".repeat(64),
        }, issued_at_ms: 100, expires_at_ms: 300100, tools: vec![] }
    }

    #[test]
    fn prepared_plan_survives_reopen_without_extending_expiry_or_owner() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.sqlite");
        let snapshot = plan("frozen");
        let mut store = NolaStore::open(&path).unwrap();
        store.persist_prepared_host_tool_plan(&snapshot, 100).unwrap();
        drop(store);
        let store = NolaStore::open(&path).unwrap();
        assert_eq!(store.prepared_host_tool_plan(&snapshot.reference, 300099).unwrap(), Some(snapshot.clone()));
        assert!(store.prepared_host_tool_plan(&snapshot.reference, 300100).is_err());
        let mut foreign = snapshot.reference.clone();
        foreign.client_id = "other-client".into();
        assert!(store.prepared_host_tool_plan(&foreign, 101).unwrap().is_none());
        foreign = snapshot.reference.clone();
        foreign.catalog_revision = "b".repeat(64);
        assert!(store.prepared_host_tool_plan(&foreign, 101).is_err());
    }

    #[test]
    fn generic_setting_cannot_forge_or_overwrite_source_plan() {
        let mut store = NolaStore::open(":memory:").unwrap();
        let snapshot = plan("frozen");
        let json = serde_json::to_string(&snapshot).unwrap();
        let mut invalid = snapshot.clone();
        invalid.expires_at_ms = u64::MAX;
        assert!(store.persist_prepared_host_tool_plan(&invalid, 100).is_err());
        invalid = snapshot.clone();
        invalid.schema_version = 2;
        assert!(store.persist_prepared_host_tool_plan(&invalid, 100).is_err());
        store.put_product_setting("host.prepared_tool_plans", &json, 100).unwrap();
        assert!(store.prepared_host_tool_plan(&snapshot.reference, 100).unwrap().is_none());
        store.persist_prepared_host_tool_plan(&snapshot, 100).unwrap();
        let mut forged = snapshot.clone();
        forged.expires_at_ms += 1;
        store.put_product_setting("host.prepared_tool_plans", &serde_json::to_string(&forged).unwrap(), 101).unwrap();
        assert_eq!(store.prepared_host_tool_plan(&snapshot.reference, 101).unwrap(), Some(snapshot));
    }

    #[test]
    fn preparation_is_insert_only_and_capacity_is_transactional() {
        let mut store = NolaStore::open(":memory:").unwrap();
        for index in 0..256 { store.persist_prepared_host_tool_plan(&plan(&index.to_string()), 100).unwrap(); }
        assert!(store.persist_prepared_host_tool_plan(&plan("overflow"), 100).is_err());
        assert!(store.persist_prepared_host_tool_plan(&plan("0"), 100).is_err());
        assert_eq!(store.prepared_host_tool_plan(&plan("0").reference, 101).unwrap(), Some(plan("0")));
        let mut fresh = plan("fresh");
        fresh.issued_at_ms = 300100;
        fresh.expires_at_ms = 600100;
        store.persist_prepared_host_tool_plan(&fresh, 300100).unwrap();
        assert!(store.prepared_host_tool_plan(&plan("0").reference, 300100).unwrap().is_none());
        assert_eq!(store.prepared_host_tool_plan(&fresh.reference, 300100).unwrap(), Some(fresh));
    }
}
