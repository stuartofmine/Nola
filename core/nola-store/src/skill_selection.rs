// SPDX-License-Identifier: Apache-2.0
//! Durable package/override selection; instruction bodies remain immutable versions.
use super::*;
pub use nola_store_port::skill::SkillInstructionSelection;

impl NolaStore {
    pub fn skill_instruction_selection(
        &self,
        id: &str,
    ) -> StoreResult<Option<SkillInstructionSelection>> {
        Ok(self.connection.query_row(
            "SELECT package_version,override_version FROM skill_instruction_selections WHERE skill_id=?1",
            params![id], |row| Ok(SkillInstructionSelection {
                package_version: row.get(0)?, override_version: row.get(1)?,
            }),
        ).optional()?)
    }

    pub fn set_skill_instruction_selection(
        &mut self,
        id: &str,
        selection: &SkillInstructionSelection,
        updated_at_ms: i64,
    ) -> StoreResult<()> {
        if !self.has_write_unit_of_work() {
            return Err(StoreError::Invariant(
                "Skill selection requires command admission".into(),
            ));
        }
        let old = self.skill_instruction_selection(id)?;
        if old.as_ref() == Some(selection) {
            return Ok(());
        }
        let transaction = self.write_transaction()?;
        let (current, revision) =
            skill_projection_with_revision(&transaction, id)?.ok_or_else(|| {
                StoreError::Invariant("Skill selection requires an existing Skill".into())
            })?;
        let active = selection
            .override_version
            .as_ref()
            .unwrap_or(&selection.package_version);
        if current.source != SkillSourceKind::BuiltIn
            || current.deleted_at_ms.is_some()
            || &current.version != active
        {
            return Err(StoreError::Conflict(
                "Skill selection does not match the active built-in version".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO skill_instruction_selections(skill_id,package_version,override_version)
             VALUES (?1,?2,?3) ON CONFLICT(skill_id) DO UPDATE SET
             package_version=excluded.package_version,override_version=excluded.override_version",
            params![id, selection.package_version, selection.override_version],
        )?;
        // A package update changes what a future reset would select, even if
        // the active override text is unchanged. Invalidate stale editors.
        if old.is_some() {
            let revision = revision.saturating_add(1);
            transaction.execute(
                "UPDATE skill_definitions SET revision=?2,updated_at_ms=MAX(updated_at_ms,?3) WHERE id=?1",
                params![id, revision, updated_at_ms],
            )?;
            let (record, _) = skill_projection_with_revision(&transaction, id)?.unwrap();
            let payload = serde_json::to_string(&SkillSyncProjection {
                record: record.clone(),
                revision,
            })?;
            enqueue_sync_change(
                &transaction,
                "skill",
                id,
                revision,
                false,
                &payload,
                record.updated_at_ms,
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
}
