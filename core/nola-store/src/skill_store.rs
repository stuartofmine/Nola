// SPDX-License-Identifier: Apache-2.0
//! Revisioned skill registry, editor snapshot, enablement, deletion, and sync persistence.

use super::*;

impl NolaStore {
    /// Registers content without changing the selected version or editor state.
    pub fn register_skill_version(&mut self, input: &UpsertSkillInput) -> StoreResult<()> {
        validate_skill_input(input)?;
        let transaction = self.write_transaction()?;
        let (current, _) =
            skill_projection_with_revision(&transaction, &input.id)?.ok_or_else(|| {
                StoreError::Invariant("Skill version requires an existing Skill".into())
            })?;
        if current.source != input.source || current.deleted_at_ms.is_some() {
            return Err(StoreError::Conflict(
                "Skill version ownership mismatch or deleted Skill".into(),
            ));
        }
        register_immutable_skill_version(&transaction, input)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn upsert_skill(&mut self, input: &UpsertSkillInput) -> StoreResult<StoredSkill> {
        validate_skill_input(input)?;
        let transaction = self.write_transaction()?;
        let existing = skill_projection_with_revision(&transaction, &input.id)?;
        if existing
            .as_ref()
            .is_some_and(|(skill, _)| skill.source != input.source)
        {
            return Err(StoreError::Conflict(
                "a Skill cannot change ownership source".to_string(),
            ));
        }

        let changed = existing.as_ref().is_none_or(|(skill, _)| {
            skill.version != input.version
                || skill.display_name != input.display_name
                || skill.summary != input.summary
                || skill.category != input.category
                || skill.instructions_ref != input.instructions_ref
                || skill.instructions_digest != input.instructions_digest
                || skill.instructions_text != input.instructions_text
                || skill.required_tools_json != input.required_tools_json
                || skill.default_mode != input.default_mode
                || skill.deleted_at_ms.is_some()
        });
        let revision = existing.as_ref().map_or(1, |(_, revision)| {
            if changed {
                revision.saturating_add(1)
            } else {
                *revision
            }
        });
        let created_at_ms = existing
            .as_ref()
            .map_or(input.created_at_ms, |(skill, _)| skill.created_at_ms);
        let updated_at_ms = existing
            .as_ref()
            .filter(|_| !changed)
            .map_or(input.updated_at_ms, |(skill, _)| skill.updated_at_ms);
        let enabled = existing
            .as_ref()
            .map_or(input.enabled, |(skill, _)| skill.enabled);

        transaction.execute(
            "INSERT INTO skill_definitions(
                id, source, display_name, summary, category, current_version,
                enabled, created_at_ms, updated_at_ms, deleted_at_ms, revision
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,NULL,?10)
             ON CONFLICT(id) DO UPDATE SET display_name=excluded.display_name,
                summary=excluded.summary,category=excluded.category,
                current_version=excluded.current_version,updated_at_ms=excluded.updated_at_ms,
                deleted_at_ms=NULL,revision=excluded.revision",
            params![
                input.id,
                skill_source_name(input.source),
                input.display_name,
                input.summary,
                input.category,
                input.version,
                enabled,
                created_at_ms,
                updated_at_ms,
                revision
            ],
        )?;
        register_immutable_skill_version(&transaction, input)?;
        let stored = skill_projection_with_revision(&transaction, &input.id)?
            .ok_or_else(|| StoreError::Invariant("upserted Skill is unavailable".to_string()))?;
        if changed {
            let payload = serde_json::to_string(&SkillSyncProjection {
                record: stored.0.clone(),
                revision: stored.1,
            })?;
            enqueue_sync_change(
                &transaction,
                "skill",
                &stored.0.id,
                stored.1,
                false,
                &payload,
                stored.0.updated_at_ms,
            )?;
        }
        transaction.commit()?;
        Ok(stored.0)
    }

    pub fn set_skill_enabled(
        &mut self,
        id: &str,
        enabled: bool,
        updated_at_ms: i64,
    ) -> StoreResult<bool> {
        require_id("skill.id", id)?;
        let transaction = self.write_transaction()?;
        let changed = transaction.execute(
            "UPDATE skill_definitions
             SET enabled=?2,updated_at_ms=?3,revision=revision+1
             WHERE id=?1 AND deleted_at_ms IS NULL AND enabled<>?2",
            params![id, enabled as i64, updated_at_ms],
        )? == 1;
        if changed {
            let stored = skill_projection_with_revision(&transaction, id)?
                .ok_or_else(|| StoreError::Invariant("updated Skill is unavailable".to_string()))?;
            let payload = serde_json::to_string(&SkillSyncProjection {
                record: stored.0.clone(),
                revision: stored.1,
            })?;
            enqueue_sync_change(
                &transaction,
                "skill",
                id,
                stored.1,
                false,
                &payload,
                updated_at_ms,
            )?;
        }
        transaction.commit()?;
        Ok(changed)
    }

    pub fn delete_user_skill(&mut self, id: &str, deleted_at_ms: i64) -> StoreResult<bool> {
        require_id("skill.id", id)?;
        let transaction = self.write_transaction()?;
        let source: Option<String> = transaction
            .query_row(
                "SELECT source FROM skill_definitions WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        if source.as_deref() == Some("built_in") {
            return Err(StoreError::Invariant(
                "built-in Skills can be disabled or reset, not deleted".to_string(),
            ));
        }
        let changed = transaction.execute(
            "UPDATE skill_definitions
             SET enabled=0,deleted_at_ms=?2,updated_at_ms=?2,revision=revision+1
             WHERE id = ?1 AND source = 'user' AND deleted_at_ms IS NULL",
            params![id, deleted_at_ms],
        )? == 1;
        if changed {
            let revision: i64 = transaction.query_row(
                "SELECT revision FROM skill_definitions WHERE id=?1",
                params![id],
                |row| row.get(0),
            )?;
            enqueue_sync_change(
                &transaction,
                "skill",
                id,
                revision,
                true,
                "null",
                deleted_at_ms,
            )?;
        }
        transaction.commit()?;
        Ok(changed)
    }

    pub fn skill(&self, id: &str) -> StoreResult<Option<StoredSkill>> {
        self.connection
            .query_row(SKILL_SELECT_SQL, params![id], skill_row)
            .optional()
            .map_err(StoreError::from)
    }

    pub fn skill_editor_snapshot(&self, id: &str) -> StoreResult<Option<(StoredSkill, i64)>> {
        require_id("skill.id", id)?;
        skill_projection_with_revision(&self.connection, id)
    }

    pub fn list_skills(&self, include_deleted: bool) -> StoreResult<Vec<StoredSkill>> {
        let sql = format!(
            "{} {} ORDER BY d.category, d.display_name, d.id",
            SKILL_SELECT_SQL_BASE,
            if include_deleted {
                ""
            } else {
                "WHERE d.deleted_at_ms IS NULL"
            },
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map([], skill_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn list_skill_editors(&self) -> StoreResult<Vec<(StoredSkill, i64)>> {
        let sql = format!(
            "{} WHERE d.deleted_at_ms IS NULL ORDER BY d.category, d.display_name, d.id",
            SKILL_SELECT_SQL_BASE,
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map([], |row| Ok((skill_row(row)?, row.get(15)?)))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}
