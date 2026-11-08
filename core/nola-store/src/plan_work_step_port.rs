// SPDX-License-Identifier: Apache-2.0
//! Same-connection plan/step adapters preserve the existing receipt savepoint.
use crate::{NolaStore, StoreError, StoreResult};
use nola_store_port::conversation_plan::*;
use nola_store_port::work_step_transaction::*;
impl ConversationPlanRepository for NolaStore {
    type Error = StoreError;
    fn save_conversation_plan(
        &mut self,
        conversation_id: &str,
        sop_id: &str,
        sop_version: u32,
        run_id: &str,
        todos_json: &str,
        updated_at_ms: i64,
    ) -> StoreResult<StoredConversationPlan> {
        NolaStore::save_conversation_plan(
            self,
            conversation_id,
            sop_id,
            sop_version,
            run_id,
            todos_json,
            updated_at_ms,
        )
    }
    fn conversation_plan(
        &self,
        conversation_id: &str,
    ) -> StoreResult<Option<StoredConversationPlan>> {
        NolaStore::conversation_plan(self, conversation_id)
    }
    fn save_conversation_plan_if_revision(
        &mut self,
        conversation_id: &str,
        sop_id: &str,
        sop_version: u32,
        run_id: &str,
        todos_json: &str,
        expected_revision: i64,
        updated_at_ms: i64,
    ) -> StoreResult<Option<StoredConversationPlan>> {
        NolaStore::save_conversation_plan_if_revision(
            self,
            conversation_id,
            sop_id,
            sop_version,
            run_id,
            todos_json,
            expected_revision,
            updated_at_ms,
        )
    }
    fn save_conversation_plan_revision_if_revision(
        &mut self,
        conversation_id: &str,
        sop_id: &str,
        sop_version: u32,
        run_id: &str,
        todos_json: &str,
        expected_revision: i64,
        frozen_revision: Option<Option<i64>>,
        change_kind: &str,
        change_summary: &str,
        updated_at_ms: i64,
    ) -> StoreResult<Option<StoredConversationPlan>> {
        NolaStore::save_conversation_plan_revision_if_revision(
            self,
            conversation_id,
            sop_id,
            sop_version,
            run_id,
            todos_json,
            expected_revision,
            frozen_revision,
            change_kind,
            change_summary,
            updated_at_ms,
        )
    }
    fn conversation_plan_revisions(
        &self,
        conversation_id: &str,
    ) -> StoreResult<Vec<StoredConversationPlanRevision>> {
        NolaStore::conversation_plan_revisions(self, conversation_id)
    }
}
impl WorkStepTransactionRepository for NolaStore {
    fn prepare_work_step_transaction(
        &mut self,
        input: &StoredWorkStepTransaction,
    ) -> StoreResult<(StoredWorkStepTransaction, bool)> {
        NolaStore::prepare_work_step_transaction(self, input)
    }
    fn advance_work_step_transaction(
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
        NolaStore::advance_work_step_transaction(
            self,
            id,
            expected_revision,
            phase,
            workspace_checkpoint_id,
            result_json,
            result_ref,
            error,
            updated_at_ms,
        )
    }
    fn advance_work_step_transaction_and_link_plan(
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
        NolaStore::advance_work_step_transaction_and_link_plan(
            self,
            id,
            expected_revision,
            phase,
            workspace_checkpoint_id,
            result_json,
            result_ref,
            error,
            updated_at_ms,
            conversation_id,
            plan_expected_revision,
            sop_id,
            sop_version,
            run_id,
            todos_json,
        )
    }
    fn work_step_transaction(&self, id: &str) -> StoreResult<Option<StoredWorkStepTransaction>> {
        NolaStore::work_step_transaction(self, id)
    }
    fn incomplete_work_step_transactions(
        &self,
        work_id: Option<&str>,
    ) -> StoreResult<Vec<StoredWorkStepTransaction>> {
        NolaStore::incomplete_work_step_transactions(self, work_id)
    }
}
