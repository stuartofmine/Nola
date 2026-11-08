// SPDX-License-Identifier: Apache-2.0
//! Read-only local and remote presentation, permission, and context usage input projections.

use super::*;

impl NolaStore {
    fn validate_checkpoint_read_source(
        &self,
        id: &str,
        current: Option<&StoredCheckpoint>,
    ) -> StoreResult<()> {
        use nola_store_port::run_effect::RunEffectRepository;
        if let Some(current) = current {
            let exact: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM core_checkpoints WHERE conversation_id=?1 AND revision=?2 AND event_cursor=?3)",
                params![id,current.revision,current.event_cursor], |row| row.get(0),
            )?;
            if current.conversation_id != id || !exact {
                return Err(StoreError::Conflict(
                    "recovered presentation checkpoint changed during read".into(),
                ));
            }
        } else if self.checkpoint_replay_pending(id)? {
            return Err(StoreError::Conflict(
                "presentation requires the recovered checkpoint frontier".into(),
            ));
        }
        Ok(())
    }
    /// Reads only usage inputs from the authoritative local checkpoint. A
    /// newer remote mirror deliberately falls through to the remote read path.
    /// Resolve ownership from record metadata before loading either large JSON
    /// payload. Equal timestamps intentionally prefer the local checkpoint.

    pub fn load_local_presentation_inputs(
        &self,
        conversation_id: &str,
        message_ids_json: &str,
        current: Option<&StoredCheckpoint>,
    ) -> StoreResult<Option<String>> {
        let _snapshot = read_snapshot::pin(&self.connection)?;
        if self.has_remote_presentation_owner(conversation_id)? {
            return Ok(None);
        }
        self.validate_checkpoint_read_source(conversation_id, current)?;
        // Admission happens in SQLite before building the selected aggregate.
        // This bounds bytes crossing into Rust; json_each may still scan the
        // original checkpoint and SQLite work is not constant in its size.
        // Materialize the small order relation once: a correlated json_each
        // join otherwise reparses the full checkpoint for every projected Part.
        let selected: Option<Option<String>> = self.connection.query_row(
            "WITH selected_messages AS (SELECT value AS id FROM json_each(?2)),
             checkpoint AS (SELECT conversation_id, COALESCE(?3,checkpoint_json) AS checkpoint_json, event_cursor FROM core_checkpoints JOIN core_checkpoint_bodies USING(conversation_id) WHERE conversation_id = ?1),
             selected_runs AS (
                SELECT DISTINCT m.run_id AS id FROM messages m JOIN selected_messages s ON s.id=m.id
                WHERE m.conversation_id=?1 AND m.role='assistant' AND m.run_id IS NOT NULL
             ),
             read_memberships AS (
                SELECT m.id AS message_id,m.run_id FROM messages m JOIN selected_runs selected ON selected.id=m.run_id
                JOIN runs r ON r.id=m.run_id
                WHERE m.conversation_id=?1 AND m.role='assistant' AND r.conversation_id=?1
                    AND r.kind='conversation' AND NOT EXISTS(SELECT 1 FROM remote_run_sources source WHERE source.run_id=r.id)
                UNION
                SELECT m.id AS message_id,historical.run_id FROM selected_messages selected
                JOIN messages m ON m.id=selected.id
                JOIN canonical_message_run_owners current
                    ON current.conversation_id=m.conversation_id AND current.message_id=m.id AND current.run_id=m.run_id
                JOIN canonical_message_run_owners historical
                    ON historical.conversation_id=current.conversation_id AND historical.message_id=current.message_id
                JOIN runs r ON r.id=historical.run_id JOIN runs live ON live.id=m.run_id
                WHERE m.conversation_id=?1 AND m.role='assistant'
                    AND r.conversation_id=?1 AND r.kind='conversation'
                    AND live.conversation_id=?1 AND live.kind='conversation'
                    AND NOT EXISTS(SELECT 1 FROM remote_run_sources source WHERE source.run_id=r.id OR source.run_id=live.id)
             ),
             related_runs AS (SELECT DISTINCT run_id AS id FROM read_memberships),
             admission_owners AS (
                SELECT r.id AS run_id,member.message_id FROM related_runs selected
                JOIN runs r ON r.id=selected.id JOIN read_memberships member
                    ON member.run_id=r.id AND member.message_id=r.assistant_message_id
                WHERE r.conversation_id=?1
             ),
             projection_order AS MATERIALIZED (
                SELECT o.key,o.value FROM checkpoint k,
                    json_each(k.checkpoint_json, '$.state.conversation.projection.order') o
             ),
             canonical_parts AS (
                SELECT p.key AS id,p.value,CAST(o.key AS INTEGER) AS ordinal,m.id AS message_id,m.run_id FROM checkpoint k,
                    json_each(k.checkpoint_json, '$.state.conversation.projection.parts') p
                JOIN projection_order o ON o.value = p.key
                JOIN message_parts mp ON mp.id=p.key JOIN messages m ON m.id=mp.message_id
                WHERE m.conversation_id=?1 AND m.role='assistant' AND m.run_id IN (SELECT id FROM related_runs)
             ),
             selected_parts AS (
                SELECT id,value FROM canonical_parts WHERE message_id IN (SELECT id FROM selected_messages) ORDER BY ordinal
             ),
             turn_owner_facts AS (
                SELECT b.turn_id,b.run_id,b.message_id,b.admission
                FROM assistant_turn_message_bindings b
                JOIN read_memberships member ON member.run_id=b.run_id AND member.message_id=b.message_id
                WHERE b.conversation_id=?1 AND NOT EXISTS(SELECT 1 FROM assistant_message_segments segment
                    WHERE segment.run_id=b.run_id AND segment.message_id=b.message_id)
             ),
             turn_owners AS (
                SELECT turn_id,run_id,message_id FROM turn_owner_facts WHERE admission=1
                UNION
                SELECT b.turn_id,b.run_id,b.message_id FROM turn_owner_facts b WHERE b.admission=0
                    AND NOT EXISTS (SELECT 1 FROM turn_owner_facts a
                        WHERE a.run_id=b.run_id AND a.turn_id=b.turn_id AND a.admission=1)
             ),
             activity_owner_facts AS (
                SELECT b.activity_kind,b.activity_id,b.turn_id,b.run_id,b.message_id FROM assistant_activity_message_bindings b
                JOIN read_memberships member ON member.run_id=b.run_id AND member.message_id=b.message_id
                WHERE b.conversation_id=?1
             ),
             selected_tasks AS (
                SELECT t.key AS id, t.value FROM checkpoint k,
                    json_each(k.checkpoint_json, '$.state.task_projection.tasks') t
                JOIN json_each(k.checkpoint_json, '$.state.task_projection.order') o ON o.value = t.key
                WHERE EXISTS (SELECT 1 FROM activity_owner_facts owner JOIN selected_messages s ON s.id=owner.message_id
                    WHERE owner.activity_kind='task' AND owner.activity_id=t.key AND owner.turn_id=json_extract(t.value,'$.owner_turn_id'))
                ORDER BY CAST(o.key AS INTEGER)
             ),
             selected_skills AS (
                SELECT s.key AS id, s.value FROM checkpoint k,
                    json_each(k.checkpoint_json, '$.state.skill_runtime.runs') s
                WHERE EXISTS (SELECT 1 FROM activity_owner_facts owner JOIN selected_messages selected ON selected.id=owner.message_id
                    WHERE owner.activity_kind='skill' AND owner.activity_id=s.key AND owner.turn_id=json_extract(s.value,'$.owner.parent_turn_id'))
             ),
             selected_attempts AS (
                SELECT a.id, a.record_json AS value FROM run_attempts a
                WHERE EXISTS (SELECT 1 FROM read_memberships member JOIN selected_messages selected ON selected.id=member.message_id
                    WHERE member.run_id=a.run_id)
                ORDER BY a.run_id, json_extract(a.record_json, '$.created_at_ms'), a.id
             ),
             part_owner_facts AS (
                SELECT mp.id AS part_id,b.turn_id,member.run_id,member.message_id
                FROM read_memberships member JOIN message_parts mp ON mp.message_id=member.message_id
                JOIN messages current ON current.id=member.message_id
                LEFT JOIN assistant_turn_part_bindings b
                    ON b.run_id=member.run_id AND b.part_id=mp.id AND b.message_id=member.message_id AND b.conversation_id=?1
                WHERE (b.part_id IS NOT NULL AND EXISTS(
                    SELECT 1 FROM assistant_turn_message_bindings t
                    WHERE t.conversation_id=?1 AND t.run_id=b.run_id
                    AND t.turn_id=b.turn_id AND t.message_id=b.message_id))
                    OR (member.run_id=current.run_id AND NOT EXISTS(
                        SELECT 1 FROM assistant_turn_part_bindings bound WHERE bound.part_id=mp.id))
             )
             SELECT CASE WHEN
                COALESCE((SELECT SUM(length(CAST(value AS BLOB)) + 2*length(CAST(json_quote(id) AS BLOB)) + 8) FROM selected_parts),0)
                + COALESCE((SELECT SUM(length(CAST(value AS BLOB)) + 2*length(CAST(json_quote(id) AS BLOB)) + 8) FROM selected_tasks),0)
                + COALESCE((SELECT SUM(length(CAST(value AS BLOB)) + length(CAST(json_quote(id) AS BLOB)) + 4) FROM selected_skills),0)
                + COALESCE((SELECT SUM(length(CAST(value AS BLOB)) + 2) FROM selected_attempts),0)
                + COALESCE((SELECT SUM(6*(length(CAST(part_id AS BLOB))+length(CAST(COALESCE(turn_id,'') AS BLOB))+length(CAST(run_id AS BLOB))+length(CAST(message_id AS BLOB))+length(CAST(?1 AS BLOB)))+160) FROM part_owner_facts),0)
                + COALESCE((SELECT SUM(6*(length(CAST(run_id AS BLOB))+length(CAST(message_id AS BLOB))+length(CAST(?1 AS BLOB)))+96) FROM admission_owners),0)
                + COALESCE((SELECT SUM(6*(length(CAST(turn_id AS BLOB))+length(CAST(run_id AS BLOB))+length(CAST(message_id AS BLOB))+length(CAST(?1 AS BLOB)))+160) FROM turn_owner_facts),0)
                + COALESCE(length(CAST(json_extract(checkpoint_json, '$.state.conversation.projection.active_message_segments') AS BLOB)),0)
                + COALESCE(length(CAST(json_extract(checkpoint_json, '$.state.conversation.projection.activity_message_owners') AS BLOB)),0)
                + COALESCE((SELECT SUM(6*(length(CAST(activity_id AS BLOB))+length(CAST(turn_id AS BLOB))+length(CAST(run_id AS BLOB))+length(CAST(message_id AS BLOB)))+256) FROM activity_owner_facts),0)
                + 6*(COALESCE(length(CAST(json_extract(checkpoint_json, '$.session_id') AS BLOB)), 0) +
                    COALESCE(length(CAST(json_extract(checkpoint_json, '$.state.session_id') AS BLOB)), 0) +
                    COALESCE(length(CAST(json_extract(checkpoint_json, '$.last_event_cursor.sequence') AS BLOB)), 0) +
                    COALESCE(length(CAST(json_extract(checkpoint_json, '$.state.conversation.projection.revision') AS BLOB)), 0) +
                    COALESCE(length(CAST(json_extract(checkpoint_json, '$.state.task_projection.revision') AS BLOB)), 0) +
                    COALESCE(length(CAST(json_extract(checkpoint_json, '$.state.skill_runtime.revision') AS BLOB)), 0) +
                    COALESCE(length(CAST(json_extract(checkpoint_json, '$.state.skill_runtime.catalog.revision') AS BLOB)), 0)) + 4096 <= 2097152
             THEN json_object(
                'conversation_id', conversation_id,
                'session_id', json_extract(checkpoint_json, '$.session_id'),
                'state_session_id', json_extract(checkpoint_json, '$.state.session_id'),
                'cursor', json_extract(checkpoint_json, '$.last_event_cursor.sequence'),
                'cursor_type', json_type(checkpoint_json, '$.last_event_cursor.sequence'),
                'event_cursor', event_cursor,
                'projection', json_object(
                    'revision', json_extract(checkpoint_json, '$.state.conversation.projection.revision'),
                    'active_message_segments', json_extract(checkpoint_json, '$.state.conversation.projection.active_message_segments'),
                    'activity_message_owners', json_extract(checkpoint_json, '$.state.conversation.projection.activity_message_owners'),
                    'order', json((SELECT json_group_array(id) FROM selected_parts)),
                    'parts', json((SELECT json_group_object(id, json(value)) FROM selected_parts))
                ),
                'tasks', json_object(
                    'revision', json_extract(checkpoint_json, '$.state.task_projection.revision'),
                    'order', json((SELECT json_group_array(id) FROM selected_tasks)),
                    'tasks', json((SELECT json_group_object(id, json(value)) FROM selected_tasks))
                ),
                'skill_revision', json_extract(checkpoint_json, '$.state.skill_runtime.revision'),
                'catalog_revision', json_extract(checkpoint_json, '$.state.skill_runtime.catalog.revision'),
                'skill_runs', json((SELECT json_group_object(id, json(value)) FROM selected_skills)),
                'model_attempts', json((SELECT json_group_array(json(value)) FROM selected_attempts)),
                'ownership',json_object(
                    'activities',json((SELECT json_group_array(json_object('kind',activity_kind,'id',activity_id,'turn_id',turn_id,'owner',json_object('conversation_id',?1,'run_id',run_id,'assistant_message_id',message_id))) FROM activity_owner_facts)),
                    'admissions',json((SELECT json_group_array(json_object('conversation_id',?1,'run_id',run_id,'assistant_message_id',message_id)) FROM admission_owners)),
                    'parts',json((SELECT json_group_array(json_object('part_id',part_id,'turn_id',turn_id,'owner',json_object('conversation_id',?1,'run_id',run_id,'assistant_message_id',message_id))) FROM part_owner_facts)),
                    'turns',json((SELECT json_group_array(json_object('turn_id',turn_id,'admission',json(CASE WHEN admission=1 THEN 'true' ELSE 'false' END),'owner',json_object('conversation_id',?1,'run_id',run_id,'assistant_message_id',message_id))) FROM turn_owner_facts))
                )
             ) ELSE NULL END FROM checkpoint",
            params![conversation_id, message_ids_json, current.map(|checkpoint| checkpoint.checkpoint_json.as_str())],
            |row| row.get(0),
        ).optional()?;
        match selected {
            None => Ok(None),
            Some(Some(json)) => Ok(Some(json)),
            Some(None) => Err(StoreError::Invariant(
                "local presentation inputs exceed bounded read budget".into(),
            )),
        }
    }

    pub fn load_session_permission_inputs(
        &self,
        conversation_id: &str,
        current: Option<&nola_store_port::conversation_query::CurrentPermissionState>,
    ) -> StoreResult<Option<String>> {
        let _snapshot = read_snapshot::pin(&self.connection)?;
        if self.has_remote_presentation_owner(conversation_id)? {
            return Ok(None);
        }
        if let Some(current) = current {
            let exact: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM core_checkpoints WHERE conversation_id=?1 AND revision=?2 AND event_cursor=?3)",
                params![conversation_id, current.revision, current.event_cursor], |row| row.get(0),
            )?;
            if current.conversation_id != conversation_id || !exact {
                return Err(StoreError::Conflict("permission read frontier changed".into()));
            }
        } else {
            self.validate_checkpoint_read_source(conversation_id, None)?;
        }
        let payload = current.map(serde_json::to_string).transpose()
            .map_err(|e| StoreError::Invariant(e.to_string()))?;
        self.connection.query_row(
            "WITH candidates AS (
                SELECT conversation_id, COALESCE(?2,(SELECT json_object(
                    'session_id', json_extract(checkpoint_json,'$.session_id'),
                    'state_session_id', json_extract(checkpoint_json,'$.state.session_id'),
                    'event_cursor', json_extract(checkpoint_json,'$.last_event_cursor.sequence'),
                    'cursor_type', json_type(checkpoint_json,'$.last_event_cursor.sequence'),
                    'active_turn_id', json_extract(checkpoint_json,'$.state.execution.active_turn.turn_id'),
                    'active_permission_policy', json_extract(checkpoint_json,'$.state.execution.active_permission_policy'),
                    'permissions', json_extract(checkpoint_json,'$.state.execution.permissions'))
                    FROM core_checkpoint_bodies WHERE conversation_id=?1)) AS payload, event_cursor, revision, updated_at_ms
                FROM core_checkpoints WHERE conversation_id = ?1
                  AND NOT EXISTS(SELECT 1 FROM runs active JOIN remote_run_sources source ON source.run_id=active.id
                      WHERE active.conversation_id=?1 AND active.kind='conversation'
                      AND active.status NOT IN ('sealed'))
             ), selected AS (
                SELECT candidates.*, (SELECT r.id FROM runs r JOIN messages m ON m.id=r.assistant_message_id
                    AND m.run_id=r.id AND m.conversation_id=r.conversation_id AND m.role='assistant'
                    WHERE r.conversation_id=candidates.conversation_id AND r.kind='conversation'
                    ORDER BY m.ordinal DESC,r.rowid DESC LIMIT 1) AS run_id
                FROM candidates LIMIT 1
             ) SELECT json_object(
                'conversation_id', conversation_id,
                'session_id', json_extract(payload, '$.session_id'),
                'state_session_id', json_extract(payload, '$.state_session_id'),
                'cursor', json_extract(payload, '$.event_cursor'),
                'cursor_type', CASE WHEN ?2 IS NOT NULL THEN json_type(payload, '$.event_cursor') ELSE json_extract(payload, '$.cursor_type') END,
                'stored_cursor', event_cursor,
                'run_id', run_id,
                'admission', json((SELECT CASE WHEN length(CAST(e.payload_json AS BLOB))<=16384
                    AND json_valid(e.payload_json) THEN e.payload_json ELSE NULL END
                    FROM run_events e WHERE e.run_id=selected.run_id AND e.event_kind='turn_started'
                    ORDER BY e.sequence ASC LIMIT 1)),
                'active_turn_id', json_extract(payload, '$.active_turn_id'),
                'permissions', json_object(
                    'active_policy', json_extract(payload, '$.active_permission_policy'),
                    'revision', json_extract(payload, '$.permissions.revision'),
                    'pending', json_extract(payload, '$.permissions.pending')
                )
             ) FROM selected",
            params![conversation_id,payload], |row| row.get(0),
        ).optional().map_err(Into::into)
    }

    /// Current canonical conversation Run ownership, never mirror timestamps.
    pub fn has_remote_presentation_owner(&self, conversation_id: &str) -> StoreResult<bool> {
        self.connection.query_row(
            "SELECT COALESCE((SELECT EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id)
             FROM runs r WHERE r.conversation_id=?1 AND r.kind='conversation'
             ORDER BY r.created_at_ms DESC,r.rowid DESC LIMIT 1),0)",
            [conversation_id],|r|r.get(0)).map_err(Into::into)
    }


    /// Reads only the optimistic-concurrency token for a canonical checkpoint.
    /// Hot streaming paths must not copy the complete checkpoint JSON merely
    /// to verify that their in-memory Core still belongs to the current state.
    pub fn checkpoint_revision(&self, conversation_id: &str) -> StoreResult<Option<i64>> {
        Ok(self
            .connection
            .query_row(
                "SELECT revision FROM core_checkpoints WHERE conversation_id = ?1",
                params![conversation_id],
                |row| row.get(0),
            )
            .optional()?)
    }
}
