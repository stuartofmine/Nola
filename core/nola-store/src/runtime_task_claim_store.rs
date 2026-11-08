// SPDX-License-Identifier: Apache-2.0
//! Durable runtime-task creation, lookup, fair claim, resume, and lease renewal.

use super::*;

struct TaskWriteScope { root:String, repository:Option<String>, git_directory:Option<String>, bounded:bool }
fn task_write_scope(project:&str,metadata:&str)->Result<TaskWriteScope,String> {
    let value:serde_json::Value=serde_json::from_str(metadata).map_err(|e|e.to_string())?;
    let invocation=&value["delegated_invocation"];
    let binding=invocation.get("worktree").filter(|v|!v.is_null())
        .map(|v|serde_json::from_value::<nola_domain::task_workspace::TaskWorktreeBinding>(v.clone()))
        .transpose().map_err(|e|e.to_string())?;
    if let Some(binding)=&binding {binding.validate()?;}
    let repository=invocation.get("project_repository_identity").filter(|v|!v.is_null())
        .map(|v|serde_json::from_value::<nola_domain::task_workspace::TaskRepositoryIdentity>(v.clone()))
        .transpose().map_err(|e|e.to_string())?;
    if let Some(repository)=&repository {repository.validate()?;}
    let bounded=invocation.get("bounded_file_writes").and_then(serde_json::Value::as_bool).unwrap_or(false);
    if bounded && binding.is_none() {return Err("Bounded file writer has no frozen worktree".into());}
    if let (Some(binding),Some(repository))=(&binding,&repository) {
        if binding.repository_identity()!=*repository {return Err("Task repository ownership mismatch".into());}
    }
    Ok(TaskWriteScope {
        root:binding.as_ref().map(|b|b.selection.root.clone()).unwrap_or_else(||project.into()),
        repository:binding.as_ref().map(|b|b.selection.expected_common_directory.clone())
            .or_else(||repository.map(|r|r.common_directory)),
        git_directory:binding.as_ref().map(|b|b.git_directory.clone()),bounded,
    })
}
fn task_writers_conflict(left_project:&str,left_metadata:&str,right_project:&str,right_metadata:&str)->Result<bool,String> {
    let left=task_write_scope(left_project,left_metadata)?;
    let right=task_write_scope(right_project,right_metadata)?;
    let overlap=std::path::Path::new(&left.root).starts_with(&right.root)
        || std::path::Path::new(&right.root).starts_with(&left.root)
        || left.git_directory.as_ref().is_some_and(|directory|Some(directory)==right.git_directory.as_ref());
    if left.bounded && right.bounded {return Ok(overlap);}
    Ok(overlap || left_project==right_project
        || left.repository.as_ref().is_some_and(|repository|Some(repository)==right.repository.as_ref()))
}
pub(super) fn register_workspace_admission(connection:&rusqlite::Connection)->StoreResult<()> {
    connection.create_scalar_function("nola_task_writers_conflict",4,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8|rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
        |context| {
            task_writers_conflict(&context.get::<String>(0)?,&context.get::<String>(1)?,
                &context.get::<String>(2)?,&context.get::<String>(3)?)
                .map_err(|error|rusqlite::Error::UserFunctionError(Box::new(std::io::Error::other(error))))
        })?;
    Ok(())
}

/// Timestamp admission is checked inside the same writer transaction as the
/// owner CAS. Lease deadlines are deliberately not a source of current time.
pub(super) fn runtime_task_time_is_current(
    transaction: &crate::connection::WriteTransaction<'_>,
    task_id: &str,
    now_ms: i64,
) -> StoreResult<bool> {
    let floor: Option<i64> = transaction
        .query_row(
        "SELECT MAX(t.created_at_ms,t.updated_at_ms,COALESCE(t.started_at_ms,0),
                    COALESCE(t.heartbeat_at_ms,0),
                    COALESCE((SELECT MAX(a.claimed_at_ms) FROM runtime_task_attempts a WHERE a.task_id=t.id),0))
         FROM durable_tasks t WHERE t.id=?1",
            params![task_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(floor.is_some_and(|floor| now_ms >= floor))
}

/// Task admission and project queries must use the same filesystem identity.
/// This is grouping, not authorization; callers still check the session owner.
pub(super) fn runtime_task_project_root(root: &str) -> String {
    std::fs::canonicalize(root)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| root.to_owned())
}

pub(super) fn require_fresh_lease_token(
    transaction: &crate::connection::WriteTransaction<'_>,
    task_id: &str,
    token: &str,
) -> StoreResult<()> {
    let reused: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM runtime_task_attempts WHERE task_id=?1 AND lease_token=?2)",
        params![task_id, token],
        |row| row.get(0),
    )?;
    if reused {
        return Err(StoreError::Conflict(
            "runtime task lease token was already used".into(),
        ));
    }
    Ok(())
}

/// One ordering for new claims and permission continuations. Readiness does
/// not grant execution: each caller still atomically validates its own witness.
pub(super) fn fair_candidate(
    transaction: &crate::connection::WriteTransaction<'_>,
    now_ms: i64,
    max_running_project: u32,
) -> StoreResult<Option<String>> {
    transaction.query_row(
        "WITH scored AS (
                   SELECT id, project_root,
                     COALESCE(json_extract(metadata_json,'$.delegated_invocation.worktree.selection.root'),project_root) AS execution_root,
                     CAST(COALESCE(
                       json_extract(metadata_json, '$._nola_scheduler.queued_at_ms'),
                       created_at_ms
                     ) AS INTEGER) AS queued_at_ms,
                     CASE json_extract(metadata_json, '$._nola_scheduler.priority')
                       WHEN 'high' THEN 2 WHEN 'low' THEN 0 ELSE 1 END
                     + MAX(0, (?1 - CAST(COALESCE(
                       json_extract(metadata_json, '$._nola_scheduler.queued_at_ms'),
                       created_at_ms
                     ) AS INTEGER)) / 60000) AS effective_priority
                   FROM durable_tasks
                   WHERE (
                       status IN ('queued','retry_waiting')
                       OR (status='waiting_permission' AND
                           EXISTS (SELECT 1 FROM runtime_task_permission_suspensions s
                               JOIN permission_requests p ON p.request_id=s.permission_request_id AND p.task_id=s.task_id
                               WHERE s.task_id=durable_tasks.id AND s.state='pending'
                                 AND p.status='resolved' AND p.decision IS NOT NULL))
                       OR (status='running' AND lease_until_ms<=?1 AND
                           EXISTS (SELECT 1 FROM runtime_task_permission_suspensions s
                               JOIN permission_requests p ON p.request_id=s.permission_request_id AND p.task_id=s.task_id
                               JOIN runtime_task_attempts original ON original.id=s.attempt_id
                               WHERE s.task_id=durable_tasks.id AND s.state='resumed'
                                 AND p.status='resolved' AND p.decision IS NOT NULL
                                 AND json_extract(s.continuation_json,'$.child_session_id')=durable_tasks.child_session_id
                                 AND json_extract(s.continuation_json,'$.run_id')=(SELECT r.id FROM runs r
                                     WHERE r.conversation_id=durable_tasks.child_session_id AND r.status <> 'sealed'
                                     ORDER BY r.created_at_ms DESC LIMIT 1)
                                 AND NOT EXISTS(SELECT 1 FROM runtime_task_permission_suspensions newer
                                     JOIN runtime_task_attempts na ON na.id=newer.attempt_id
                                     WHERE newer.task_id=s.task_id AND na.attempt_number>original.attempt_number)))
                     )
                     AND cancel_requested=0
                     AND created_at_ms<=?1 AND updated_at_ms<=?1
                     AND COALESCE(started_at_ms,0)<=?1 AND COALESCE(heartbeat_at_ms,0)<=?1
                     AND NOT EXISTS(SELECT 1 FROM runtime_task_attempts a
                         WHERE a.task_id=durable_tasks.id AND a.claimed_at_ms>?1)
                     AND (kind<>'agent' OR EXISTS(SELECT 1 FROM runtime_agent_task_authorities authority
                         WHERE authority.task_id=durable_tasks.id AND authority.phase='admitted'
                           AND json_extract(authority.admission_json,'$.parent_conversation_id')=durable_tasks.owner_session_id
                           AND json_extract(authority.admission_json,'$.child_conversation_id')=durable_tasks.child_session_id
                           AND json_extract(authority.admission_json,'$.parent_tool_call_id')=durable_tasks.parent_tool_call_id
                           AND (json_extract(authority.admission_json,'$.input_watermark') IS NULL
                             OR durable_tasks.input_watermark>=json_extract(authority.admission_json,'$.input_watermark'))))
                     AND (lease_until_ms IS NULL OR status='waiting_permission' OR lease_until_ms<=?1)
                     AND (retry_at_ms IS NULL OR retry_at_ms<=?1)
                 ), project_heads AS (
                   SELECT id, project_root, queued_at_ms, effective_priority,
                     ROW_NUMBER() OVER (
                       PARTITION BY execution_root
                       ORDER BY effective_priority DESC, queued_at_ms ASC, id ASC
                     ) AS project_rank
                   FROM scored
                 ), eligible_heads AS (
                   SELECT head.id, head.queued_at_ms, head.effective_priority
                   FROM project_heads head
                   WHERE head.project_rank=1
                     AND (
                       SELECT COUNT(*) FROM durable_tasks live
                       WHERE live.project_root=head.project_root
                         AND live.status='running'
                         AND live.ownership_token IS NOT NULL
                         AND live.cancel_requested=0
                         AND live.lease_until_ms>?1
                     ) < ?2
                 )
                 SELECT id FROM eligible_heads
                 ORDER BY effective_priority DESC, queued_at_ms ASC, id ASC
                 LIMIT 1",
        params![now_ms, i64::from(max_running_project)],
        |row| row.get::<_, String>(0),
    ).optional().map_err(StoreError::from)
}

impl NolaStore {
    pub(super) fn insert_runtime_task(&mut self, task: &StoredRuntimeTask) -> StoreResult<()> {
        require_id("runtime_task.id", &task.id)?;
        require_id("runtime_task.kind", &task.kind)?;
        require_id("runtime_task.status", &task.status)?;
        require_id("runtime_task.project_root", &task.project_root)?;
        validate_json("runtime_task.metadata_json", &task.metadata_json)?;
        task_write_scope(&task.project_root,&task.metadata_json).map_err(StoreError::Invariant)?;
        let inserted = self.connection.execute(
            "INSERT INTO durable_tasks(
                id, kind, status, title, project_root, owner_session_id, parent_task_id,
                parent_tool_call_id, child_session_id, agent_type, prompt, progress_summary,
                last_activity, result_summary, error, output_path, unread, cancel_requested,
                cancel_requested_at_ms, worker_id, ownership_token, lease_until_ms,
                heartbeat_at_ms, retry_count, retry_at_ms, last_error, input_watermark,
                completion_watermark, created_at_ms, started_at_ms, ended_at_ms,
                updated_at_ms, metadata_json
             ) SELECT
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28,
                ?29, ?30, ?31, ?32, ?33
             WHERE NOT (?2='agent' AND COALESCE(json_extract(?33,'$.can_mutate'),0)=1)
                OR NOT EXISTS (SELECT 1 FROM durable_tasks
                    WHERE kind='agent' AND nola_task_writers_conflict(project_root,metadata_json,?5,?33)
                      AND status NOT IN ('sealed','completed','failed','cancelled')
                      AND json_extract(metadata_json,'$.can_mutate')=1)",
            params![
                task.id,
                task.kind,
                task.status,
                task.title,
                task.project_root,
                task.owner_session_id,
                task.parent_task_id,
                task.parent_tool_call_id,
                task.child_session_id,
                task.agent_type,
                task.prompt,
                task.progress_summary,
                task.last_activity,
                task.result_summary,
                task.error,
                task.output_path,
                task.unread,
                task.cancel_requested,
                task.cancel_requested_at_ms,
                task.worker_id,
                task.ownership_token,
                task.lease_until_ms,
                task.heartbeat_at_ms,
                task.retry_count,
                task.retry_at_ms,
                task.last_error,
                task.input_watermark,
                task.completion_watermark,
                task.created_at_ms,
                task.started_at_ms,
                task.ended_at_ms,
                task.updated_at_ms,
                task.metadata_json,
            ],
        )?;
        if inserted == 0 {
            return Err(StoreError::Conflict("Another write-capable agent task is already active in this project".into()));
        }
        if task.kind=="agent" {
            self.connection.execute("INSERT INTO runtime_agent_task_authorities(task_id,phase,authority_revision,admission_json) VALUES(?1,'prepared',1,NULL)",[&task.id])?;
        }
        Ok(())
    }

    pub fn runtime_task(&self, id: &str) -> StoreResult<Option<StoredRuntimeTask>> {
        require_id("runtime_task.id", id)?;
        self.connection
            .query_row(
                &format!("SELECT {RUNTIME_TASK_COLUMNS} FROM durable_tasks WHERE id = ?1"),
                params![id],
                runtime_task_row,
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn list_runtime_tasks(&self, project_root: &str) -> StoreResult<Vec<StoredRuntimeTask>> {
        require_id("runtime_task.project_root", project_root)?;
        let project_root = runtime_task_project_root(project_root);
        let mut statement = self.connection.prepare(&format!(
            "SELECT {RUNTIME_TASK_COLUMNS} FROM durable_tasks
             WHERE project_root = ?1 ORDER BY updated_at_ms DESC, id ASC"
        ))?;
        let rows = statement.query_map(params![project_root], runtime_task_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Uses the existing session index; unrelated conversations and their task
    /// payloads never enter Application's replay lookup.
    pub fn runtime_task_for_delegation(
        &self, owner_conversation_id: &str, parent_run_id: &str, parent_effect_id: &str,
    ) -> StoreResult<Option<StoredRuntimeTask>> {
        self.connection.query_row(
            &format!("SELECT {RUNTIME_TASK_COLUMNS} FROM durable_tasks
                WHERE owner_session_id=?1 AND kind='agent'
                  AND json_extract(metadata_json,'$.delegation_origin.parent_run_id')=?2
                  AND json_extract(metadata_json,'$.delegation_origin.parent_effect_id')=?3
                ORDER BY updated_at_ms DESC, id ASC LIMIT 1"),
            params![owner_conversation_id, parent_run_id, parent_effect_id], runtime_task_row,
        ).optional().map_err(StoreError::from)
    }

    pub fn list_all_runtime_tasks(&self) -> StoreResult<Vec<StoredRuntimeTask>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {RUNTIME_TASK_COLUMNS} FROM durable_tasks
             ORDER BY updated_at_ms DESC, id ASC"
        ))?;
        let rows = statement.query_map([], runtime_task_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn claim_runtime_task(
        &mut self,
        task_id: &str,
        worker_id: &str,
        worker_generation: &str,
        ownership_token: &str,
        now_ms: i64,
        lease_until_ms: i64,
    ) -> StoreResult<Option<nola_store_port::runtime_task::RuntimeTaskAttempt>> {
        require_id("task_id", task_id)?;
        require_runtime_task_claim_identity("worker_id", worker_id)?;
        require_runtime_task_claim_identity("worker_generation", worker_generation)?;
        require_runtime_task_claim_identity("lease_token", ownership_token)?;
        if lease_until_ms <= now_ms {
            return Err(StoreError::Invariant(
                "runtime task lease must expire after claim time".into(),
            ));
        }
        let transaction = self.write_transaction()?;
        require_fresh_lease_token(&transaction, task_id, ownership_token)?;
        if !runtime_task_time_is_current(&transaction, task_id, now_ms)? {
            transaction.rollback()?;
            return Ok(None);
        }
        if !crate::agent_task_admission_store::permits_execution(&transaction,task_id)? {
            transaction.rollback()?;
            return Ok(None);
        }
        let changed = transaction.execute(
            "UPDATE durable_tasks SET status='running', worker_id=?2, ownership_token=?3,
             heartbeat_at_ms=?4, lease_until_ms=?5, retry_at_ms=NULL,
             started_at_ms=COALESCE(started_at_ms,?4), updated_at_ms=?4
             WHERE id=?1 AND status IN ('queued','retry_waiting')
             AND cancel_requested=0
             AND (lease_until_ms IS NULL OR lease_until_ms<=?4)
             AND (retry_at_ms IS NULL OR retry_at_ms<=?4)",
            params![task_id, worker_id, ownership_token, now_ms, lease_until_ms],
        )? == 1;
        if !changed {
            transaction.commit()?;
            return Ok(None);
        }
        transaction.execute(
            "UPDATE runtime_task_attempts SET state='superseded', revision=revision+1
             WHERE task_id=?1 AND state='leased'",
            params![task_id],
        )?;
        let attempt_number: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(attempt_number),0)+1 FROM runtime_task_attempts WHERE task_id=?1",
            params![task_id],
            |row| row.get(0),
        )?;
        let attempt = nola_store_port::runtime_task::RuntimeTaskAttempt {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: task_id.into(),
            attempt_number,
            worker_id: worker_id.into(),
            worker_generation: worker_generation.into(),
            lease_token: ownership_token.into(),
            lease_expires_at_ms: lease_until_ms,
            state: "leased".into(),
            revision: 1,
            claimed_at_ms: now_ms,
        };
        transaction.execute(
            "INSERT INTO runtime_task_attempts(
                id,task_id,attempt_number,worker_id,worker_generation,lease_token,
                lease_expires_at_ms,state,revision,claimed_at_ms
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                attempt.id,
                attempt.task_id,
                attempt.attempt_number,
                attempt.worker_id,
                attempt.worker_generation,
                attempt.lease_token,
                attempt.lease_expires_at_ms,
                attempt.state,
                attempt.revision,
                attempt.claimed_at_ms
            ],
        )?;
        crate::runtime_task_shell_delivery_store::initialize(&transaction, &attempt)?;
        transaction.commit()?;
        Ok(Some(attempt))
    }

    pub fn runtime_task_current_attempt(
        &self,
        task_id: &str,
    ) -> StoreResult<Option<nola_store_port::runtime_task::RuntimeTaskAttempt>> {
        require_id("runtime_task.id", task_id)?;
        let mut query = self.connection.prepare(
            "SELECT a.id,a.task_id,a.attempt_number,a.worker_id,a.worker_generation,a.lease_token,
                    a.lease_expires_at_ms,a.state,a.revision,a.claimed_at_ms
             FROM durable_tasks t JOIN runtime_task_attempts a ON a.task_id=t.id
             WHERE t.id=?1 AND a.state='leased' AND t.worker_id=a.worker_id
               AND t.ownership_token=a.lease_token AND t.lease_until_ms=a.lease_expires_at_ms",
        )?;
        let mut rows = query.query(params![task_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let attempt = nola_store_port::runtime_task::RuntimeTaskAttempt {
            id: row.get(0)?,
            task_id: row.get(1)?,
            attempt_number: row.get(2)?,
            worker_id: row.get(3)?,
            worker_generation: row.get(4)?,
            lease_token: row.get(5)?,
            lease_expires_at_ms: row.get(6)?,
            state: row.get(7)?,
            revision: row.get(8)?,
            claimed_at_ms: row.get(9)?,
        };
        if rows.next()?.is_some() {
            return Err(StoreError::Invariant(
                "task current ownership matches multiple Attempts".into(),
            ));
        }
        Ok(Some(attempt))
    }

    pub fn runtime_task_attempt(
        &self,
        id: &str,
    ) -> StoreResult<Option<nola_store_port::runtime_task::RuntimeTaskAttempt>> {
        require_id("runtime_task_attempt.id", id)?;
        self.connection
            .query_row(
                "SELECT id,task_id,attempt_number,worker_id,worker_generation,lease_token,
                        lease_expires_at_ms,state,revision,claimed_at_ms
                 FROM runtime_task_attempts WHERE id=?1",
                params![id],
                |row| {
                    Ok(nola_store_port::runtime_task::RuntimeTaskAttempt {
                        id: row.get(0)?,
                        task_id: row.get(1)?,
                        attempt_number: row.get(2)?,
                        worker_id: row.get(3)?,
                        worker_generation: row.get(4)?,
                        lease_token: row.get(5)?,
                        lease_expires_at_ms: row.get(6)?,
                        state: row.get(7)?,
                        revision: row.get(8)?,
                        claimed_at_ms: row.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
    }

    /// Atomically admits one task into the bounded worker pool. Each project
    /// contributes at most its highest-scored runnable head, projects at their
    /// local limit are skipped, and one global head is selected. Waiting time
    /// ages priority every minute so a sustained stream of newer high-priority
    /// work cannot starve an older low-priority project forever.
    #[allow(clippy::too_many_arguments)]
    pub fn claim_runtime_task_fair(
        &mut self,
        task_id: &str,
        worker_id: &str,
        worker_generation: &str,
        ownership_token: &str,
        now_ms: i64,
        lease_until_ms: i64,
        max_running_global: u32,
        max_running_project: u32,
    ) -> StoreResult<Option<nola_store_port::runtime_task::RuntimeTaskAttempt>> {
        require_id("task_id", task_id)?;
        require_runtime_task_claim_identity("worker_id", worker_id)?;
        require_runtime_task_claim_identity("worker_generation", worker_generation)?;
        require_runtime_task_claim_identity("lease_token", ownership_token)?;
        if lease_until_ms <= now_ms {
            return Err(StoreError::Invariant(
                "runtime task lease must expire after claim time".into(),
            ));
        }
        if max_running_global == 0 || max_running_project == 0 {
            return Err(StoreError::Invariant(
                "runtime task concurrency limits must be positive".into(),
            ));
        }
        let transaction = self.write_transaction()?;
        require_fresh_lease_token(&transaction, task_id, ownership_token)?;
        if !runtime_task_time_is_current(&transaction, task_id, now_ms)? {
            transaction.rollback()?;
            return Ok(None);
        }
        if !crate::agent_task_admission_store::permits_execution(&transaction,task_id)? {
            transaction.rollback()?;
            return Ok(None);
        }
        let project_root = transaction
            .query_row(
                "SELECT project_root FROM durable_tasks WHERE id=?1",
                params![task_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(project_root) = project_root else {
            transaction.commit()?;
            return Ok(None);
        };
        let live_global: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM durable_tasks
             WHERE status='running'
               AND ownership_token IS NOT NULL AND cancel_requested=0
               AND lease_until_ms>?1",
            params![now_ms],
            |row| row.get(0),
        )?;
        let live_project: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM durable_tasks
             WHERE project_root=?1 AND status='running'
               AND ownership_token IS NOT NULL AND cancel_requested=0
               AND lease_until_ms>?2",
            params![project_root, now_ms],
            |row| row.get(0),
        )?;
        if live_global >= i64::from(max_running_global)
            || live_project >= i64::from(max_running_project)
        {
            transaction.commit()?;
            return Ok(None);
        }
        let candidate = fair_candidate(&transaction, now_ms, max_running_project)?;
        if candidate.as_deref() != Some(task_id) {
            transaction.commit()?;
            return Ok(None);
        }
        let claimed = transaction.execute(
            "UPDATE durable_tasks SET status='running', worker_id=?2, ownership_token=?3,
             heartbeat_at_ms=?4, lease_until_ms=?5, retry_at_ms=NULL,
             started_at_ms=COALESCE(started_at_ms,?4), updated_at_ms=?4
             WHERE id=?1 AND status IN ('queued','retry_waiting')
             AND cancel_requested=0
             AND (lease_until_ms IS NULL OR lease_until_ms<=?4)
             AND (retry_at_ms IS NULL OR retry_at_ms<=?4)",
            params![task_id, worker_id, ownership_token, now_ms, lease_until_ms],
        )? == 1;
        if !claimed {
            transaction.commit()?;
            return Ok(None);
        }
        transaction.execute(
            "UPDATE runtime_task_attempts SET state='superseded', revision=revision+1
             WHERE task_id=?1 AND state='leased'",
            params![task_id],
        )?;
        let attempt_number: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(attempt_number),0)+1 FROM runtime_task_attempts WHERE task_id=?1",
            params![task_id],
            |row| row.get(0),
        )?;
        let attempt = nola_store_port::runtime_task::RuntimeTaskAttempt {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: task_id.into(),
            attempt_number,
            worker_id: worker_id.into(),
            worker_generation: worker_generation.into(),
            lease_token: ownership_token.into(),
            lease_expires_at_ms: lease_until_ms,
            state: "leased".into(),
            revision: 1,
            claimed_at_ms: now_ms,
        };
        transaction.execute(
            "INSERT INTO runtime_task_attempts(id,task_id,attempt_number,worker_id,
                worker_generation,lease_token,lease_expires_at_ms,state,revision,claimed_at_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                attempt.id,
                attempt.task_id,
                attempt.attempt_number,
                attempt.worker_id,
                attempt.worker_generation,
                attempt.lease_token,
                attempt.lease_expires_at_ms,
                attempt.state,
                attempt.revision,
                attempt.claimed_at_ms
            ],
        )?;
        crate::runtime_task_shell_delivery_store::initialize(&transaction, &attempt)?;
        transaction.commit()?;
        Ok(Some(attempt))
    }

    pub fn renew_runtime_task_lease(
        &mut self,
        renewal: &nola_store_port::runtime_task::RenewRuntimeTaskLease,
    ) -> StoreResult<Option<nola_store_port::runtime_task::RuntimeTaskAttempt>> {
        require_id("runtime_task_renewal.task_id", &renewal.task_id)?;
        require_runtime_task_claim_identity("attempt_id", &renewal.attempt_id)?;
        require_runtime_task_claim_identity("worker_generation", &renewal.worker_generation)?;
        require_runtime_task_claim_identity("lease_token", &renewal.lease_token)?;
        if renewal.expected_attempt_revision < 1
            || renewal.current_expiry_ms <= renewal.now_ms
            || renewal.requested_expiry_ms <= renewal.current_expiry_ms
            || renewal
                .requested_expiry_ms
                .checked_sub(renewal.now_ms)
                .is_none_or(|extension| extension > 10 * 60 * 1_000)
        {
            return Err(StoreError::Invariant(
                "invalid runtime task lease renewal".into(),
            ));
        }
        let transaction = self.write_transaction()?;
        if !runtime_task_time_is_current(&transaction, &renewal.task_id, renewal.now_ms)? {
            transaction.rollback()?;
            return Ok(None);
        }
        if !crate::agent_task_admission_store::permits_execution(&transaction,&renewal.task_id)? {
            transaction.rollback()?;
            return Ok(None);
        }
        let changed = transaction.execute(
            "UPDATE runtime_task_attempts
             SET lease_expires_at_ms=?7, revision=revision+1
             WHERE id=?2 AND task_id=?1 AND revision=?3 AND worker_generation=?4
               AND lease_token=?5 AND lease_expires_at_ms=?6
               AND state='leased' AND lease_expires_at_ms>?8",
            params![
                renewal.task_id,
                renewal.attempt_id,
                renewal.expected_attempt_revision,
                renewal.worker_generation,
                renewal.lease_token,
                renewal.current_expiry_ms,
                renewal.requested_expiry_ms,
                renewal.now_ms
            ],
        )? == 1;
        if !changed {
            transaction.rollback()?;
            return Ok(None);
        }
        let task_changed = transaction.execute(
            "UPDATE durable_tasks SET heartbeat_at_ms=?4, lease_until_ms=?5, updated_at_ms=?4
             WHERE id=?1 AND status IN ('running','waiting_permission') AND ownership_token=?2
               AND lease_until_ms=?3 AND lease_until_ms>?4 AND cancel_requested=0",
            params![
                renewal.task_id,
                renewal.lease_token,
                renewal.current_expiry_ms,
                renewal.now_ms,
                renewal.requested_expiry_ms
            ],
        )? == 1;
        if !task_changed {
            transaction.rollback()?;
            return Ok(None);
        }
        let renewed = transaction.query_row(
            "SELECT id,task_id,attempt_number,worker_id,worker_generation,lease_token,
                    lease_expires_at_ms,state,revision,claimed_at_ms
             FROM runtime_task_attempts WHERE id=?1",
            params![renewal.attempt_id],
            |row| {
                Ok(nola_store_port::runtime_task::RuntimeTaskAttempt {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    attempt_number: row.get(2)?,
                    worker_id: row.get(3)?,
                    worker_generation: row.get(4)?,
                    lease_token: row.get(5)?,
                    lease_expires_at_ms: row.get(6)?,
                    state: row.get(7)?,
                    revision: row.get(8)?,
                    claimed_at_ms: row.get(9)?,
                })
            },
        )?;
        transaction.commit()?;
        Ok(Some(renewed))
    }
}
