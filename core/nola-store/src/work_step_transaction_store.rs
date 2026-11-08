// SPDX-License-Identifier: Apache-2.0
//! Durable work-step transaction preparation, CAS advancement, and recovery queries.

use super::*;

impl NolaStore {
    pub fn prepare_work_step_transaction(
        &mut self,
        input: &StoredWorkStepTransaction,
    ) -> StoreResult<(StoredWorkStepTransaction, bool)> {
        require_id("work_step_transaction.id", &input.id)?;
        require_id("work_step_transaction.work_id", &input.work_id)?;
        require_id("work_step_transaction.step_id", &input.step_id)?;
        require_id("work_step_transaction.tool_call_id", &input.tool_call_id)?;
        require_id("work_step_transaction.root", &input.root)?;
        require_id("work_step_transaction.effect_kind", &input.effect_kind)?;
        validate_json("work_step_transaction.request_json", &input.request_json)?;
        if input.phase != "prepared" || input.revision != 1 {
            return Err(StoreError::Invariant(
                "new work step transaction must start prepared at revision 1".to_string(),
            ));
        }
        let transaction = self.write_transaction()?;
        let existing = transaction
            .query_row(
                "SELECT id,work_id,step_id,tool_call_id,root,effect_kind,phase,
                        workspace_checkpoint_id,request_json,result_json,result_ref,error,
                        revision,created_at_ms,updated_at_ms
                 FROM work_step_transactions WHERE tool_call_id=?1",
                params![input.tool_call_id],
                work_step_transaction_row,
            )
            .optional()?;
        if let Some(existing) = existing {
            if existing.id == input.id
                && existing.work_id == input.work_id
                && existing.step_id == input.step_id
                && existing.root == input.root
                && existing.effect_kind == input.effect_kind
                && existing.request_json == input.request_json
            {
                return Ok((existing, false));
            }
            return Err(StoreError::Invariant(format!(
                "tool call {} is already bound to a different work step transaction",
                input.tool_call_id
            )));
        }
        transaction.execute(
            "INSERT INTO work_step_transactions(
                id,work_id,step_id,tool_call_id,root,effect_kind,phase,
                workspace_checkpoint_id,request_json,result_json,result_ref,error,
                revision,created_at_ms,updated_at_ms
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,NULL,NULL,NULL,1,?10,?11)",
            params![
                input.id,
                input.work_id,
                input.step_id,
                input.tool_call_id,
                input.root,
                input.effect_kind,
                input.phase,
                input.workspace_checkpoint_id,
                input.request_json,
                input.created_at_ms,
                input.updated_at_ms,
            ],
        )?;
        transaction.commit()?;
        Ok((input.clone(), true))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn advance_work_step_transaction(
        &mut self,
        id: &str,
        expected_revision: i64,
        phase: &str,
        workspace_checkpoint_id: Option<&str>,
        result_json: Option<&str>,
        result_ref: Option<&str>,
        error: Option<&str>,
        updated_at_ms: i64,
    ) -> StoreResult<Option<StoredWorkStepTransaction>> {
        self.advance_work_step_transaction_internal(
            id,
            expected_revision,
            phase,
            workspace_checkpoint_id,
            result_json,
            result_ref,
            error,
            updated_at_ms,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn advance_work_step_transaction_and_link_plan(
        &mut self,
        id: &str,
        expected_revision: i64,
        phase: &str,
        workspace_checkpoint_id: Option<&str>,
        result_json: Option<&str>,
        result_ref: Option<&str>,
        error: Option<&str>,
        updated_at_ms: i64,
        conversation_id: &str,
        plan_expected_revision: i64,
        sop_id: &str,
        sop_version: u32,
        run_id: &str,
        todos_json: &str,
    ) -> StoreResult<Option<StoredWorkStepTransaction>> {
        require_id("conversation_plan.conversation_id", conversation_id)?;
        require_id("conversation_plan.sop_id", sop_id)?;
        require_id("conversation_plan.run_id", run_id)?;
        let todos: serde_json::Value = serde_json::from_str(todos_json)?;
        if !todos.is_array() || plan_expected_revision <= 0 || sop_version == 0 {
            return Err(StoreError::Invariant(
                "work step plan link requires typed todos, a positive revision, and SOP identity"
                    .to_string(),
            ));
        }
        self.advance_work_step_transaction_internal(
            id,
            expected_revision,
            phase,
            workspace_checkpoint_id,
            result_json,
            result_ref,
            error,
            updated_at_ms,
            Some((
                conversation_id,
                plan_expected_revision,
                sop_id,
                sop_version,
                run_id,
                todos_json,
            )),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn advance_work_step_transaction_internal(
        &mut self,
        id: &str,
        expected_revision: i64,
        phase: &str,
        workspace_checkpoint_id: Option<&str>,
        result_json: Option<&str>,
        result_ref: Option<&str>,
        error: Option<&str>,
        updated_at_ms: i64,
        plan_link: Option<(&str, i64, &str, u32, &str, &str)>,
    ) -> StoreResult<Option<StoredWorkStepTransaction>> {
        require_id("work_step_transaction.id", id)?;
        if let Some(value) = result_json {
            validate_json("work_step_transaction.result_json", value)?;
        }
        if !matches!(
            phase,
            "checkpointed" | "effect_observed" | "committed" | "failed"
        ) {
            return Err(StoreError::Invariant(format!(
                "invalid work step transaction phase: {phase}"
            )));
        }
        let transaction = self.write_transaction()?;
        let current = transaction
            .query_row(
                "SELECT id,work_id,step_id,tool_call_id,root,effect_kind,phase,
                        workspace_checkpoint_id,request_json,result_json,result_ref,error,
                        revision,created_at_ms,updated_at_ms
                 FROM work_step_transactions WHERE id=?1",
                params![id],
                work_step_transaction_row,
            )
            .optional()?
            .ok_or_else(|| {
                StoreError::Invariant(format!("work step transaction not found: {id}"))
            })?;
        if current.revision != expected_revision {
            return Ok(None);
        }
        let transition_allowed = matches!(
            (current.phase.as_str(), phase),
            ("prepared", "checkpointed")
                | ("prepared", "failed")
                | ("checkpointed", "effect_observed")
                | ("checkpointed", "failed")
                | ("effect_observed", "committed")
                | ("effect_observed", "failed")
        );
        if !transition_allowed {
            return Err(StoreError::Invariant(format!(
                "invalid work step transaction transition: {} -> {phase}",
                current.phase
            )));
        }
        let revision = expected_revision + 1;
        let changed = transaction.execute(
            "UPDATE work_step_transactions SET
                phase=?2,
                workspace_checkpoint_id=COALESCE(?3,workspace_checkpoint_id),
                result_json=COALESCE(?4,result_json),
                result_ref=COALESCE(?5,result_ref),
                error=COALESCE(?6,error),
                revision=?7,
                updated_at_ms=?8
             WHERE id=?1 AND revision=?9",
            params![
                id,
                phase,
                workspace_checkpoint_id,
                result_json,
                result_ref,
                error,
                revision,
                updated_at_ms,
                expected_revision,
            ],
        )?;
        if changed != 1 {
            return Ok(None);
        }
        if let Some((
            conversation_id,
            plan_expected_revision,
            sop_id,
            sop_version,
            run_id,
            todos_json,
        )) = plan_link
        {
            if current.work_id != conversation_id {
                return Err(StoreError::Invariant(
                    "work step transaction and plan conversation identities differ".to_string(),
                ));
            }
            let plan_changed = transaction.execute(
                "UPDATE conversation_plans
                 SET todos_json=?2,revision=revision+1,updated_at_ms=?3
                 WHERE conversation_id=?1 AND revision=?4 AND sop_id=?5 AND sop_version=?6 AND run_id=?7",
                params![
                    conversation_id,
                    todos_json,
                    updated_at_ms,
                    plan_expected_revision,
                    sop_id,
                    sop_version,
                    run_id,
                ],
            )?;
            if plan_changed != 1 {
                return Ok(None);
            }
            transaction.execute(
                "INSERT INTO conversation_plan_revisions(
                    conversation_id,revision,sop_id,sop_version,run_id,todos_json,change_kind,change_summary,updated_at_ms
                 ) VALUES(?1,?2,?3,?4,?5,?6,'execution',?7,?8)",
                params![
                    conversation_id,
                    plan_expected_revision + 1,
                    sop_id,
                    sop_version,
                    run_id,
                    todos_json,
                    format!("linked work step transaction {id}"),
                    updated_at_ms,
                ],
            )?;
        }
        let saved = transaction.query_row(
            "SELECT id,work_id,step_id,tool_call_id,root,effect_kind,phase,
                    workspace_checkpoint_id,request_json,result_json,result_ref,error,
                    revision,created_at_ms,updated_at_ms
             FROM work_step_transactions WHERE id=?1",
            params![id],
            work_step_transaction_row,
        )?;
        transaction.commit()?;
        Ok(Some(saved))
    }

    pub fn work_step_transaction(
        &self,
        id: &str,
    ) -> StoreResult<Option<StoredWorkStepTransaction>> {
        require_id("work_step_transaction.id", id)?;
        self.connection
            .query_row(
                "SELECT id,work_id,step_id,tool_call_id,root,effect_kind,phase,
                        workspace_checkpoint_id,request_json,result_json,result_ref,error,
                        revision,created_at_ms,updated_at_ms
                 FROM work_step_transactions WHERE id=?1",
                params![id],
                work_step_transaction_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn incomplete_work_step_transactions(
        &self,
        work_id: Option<&str>,
    ) -> StoreResult<Vec<StoredWorkStepTransaction>> {
        if let Some(work_id) = work_id {
            require_id("work_step_transaction.work_id", work_id)?;
            let mut statement = self.connection.prepare(
                "SELECT id,work_id,step_id,tool_call_id,root,effect_kind,phase,
                        workspace_checkpoint_id,request_json,result_json,result_ref,error,
                        revision,created_at_ms,updated_at_ms
                 FROM work_step_transactions
                 WHERE phase IN ('prepared','checkpointed','effect_observed') AND work_id=?1
                 ORDER BY created_at_ms,id",
            )?;
            let rows = statement.query_map(params![work_id], work_step_transaction_row)?;
            return rows.collect::<Result<Vec<_>, _>>().map_err(Into::into);
        }
        let mut statement = self.connection.prepare(
            "SELECT id,work_id,step_id,tool_call_id,root,effect_kind,phase,
                    workspace_checkpoint_id,request_json,result_json,result_ref,error,
                    revision,created_at_ms,updated_at_ms
             FROM work_step_transactions
             WHERE phase IN ('prepared','checkpointed','effect_observed')
             ORDER BY created_at_ms,id",
        )?;
        let rows = statement.query_map([], work_step_transaction_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}
