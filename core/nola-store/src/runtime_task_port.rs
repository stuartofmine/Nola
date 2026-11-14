// SPDX-License-Identifier: Apache-2.0
//! Same-connection SQLite adapter; existing transaction and scheduler semantics.
use crate::{NolaStore, StoreError};
use rusqlite::{params, OptionalExtension};
use nola_store_port::runtime_task::*;
impl RuntimeTaskRepository for NolaStore {
    fn submit_terminal_input(&mut self, input: &nola_domain::terminal_input::TerminalInputRequest,
        now_ms: i64) -> Result<nola_domain::terminal_input::TerminalInputReceipt, Self::Error> {
        NolaStore::submit_terminal_input(self, input, now_ms)
    }
    fn runtime_task_notification_delivered(&self, delivery: &RuntimeTaskNotificationDelivery) -> Result<bool, Self::Error> {
        NolaStore::runtime_task_notification_delivered(self, delivery)
    }
    fn acknowledge_runtime_task_delivery(&mut self, delivery: &RuntimeTaskNotificationDelivery) -> Result<RuntimeTaskNotificationAcknowledgement, Self::Error> {
        NolaStore::acknowledge_runtime_task_delivery(self, delivery)
    }
    fn prepared_agent_task_ids(&self, owner: &str, after_id: &str, limit: usize) -> Result<Vec<String>, Self::Error> {
        NolaStore::prepared_agent_task_ids(self, owner, after_id, limit)
    }
    fn source_agent_task_admission_location(&self, owner: &str, task_id: &str) -> Result<Option<(String, u64)>, Self::Error> {
        NolaStore::source_agent_task_admission_location(self, owner, task_id)
    }
    fn validated_expert_task_target(&self, task_id:&str, child:&str, require_admitted:bool)
        -> Result<Option<nola_domain::expert_task_target::ExpertTaskTargetV1>,Self::Error> {
        NolaStore::validated_expert_task_target(self,task_id,child,require_admitted)
    }
    fn runtime_agent_task_authority(
        &self,
        t: &str,
    ) -> Result<Option<RuntimeAgentTaskAuthority>, Self::Error> {
        NolaStore::runtime_agent_task_authority(self, t)
    }
    fn admit_runtime_agent_task(
        &mut self,
        a: &AgentTaskAdmission,
    ) -> Result<RuntimeAgentTaskAuthority, Self::Error> {
        NolaStore::admit_runtime_agent_task(self, a)
    }
    fn source_agent_task_admission_evidence(
        &self,
        c: &str,
        r: &str,
        t: &str,
        e: u64,
    ) -> Result<Option<StoredAgentTaskAdmissionEvidence>, Self::Error> {
        NolaStore::source_agent_task_admission_evidence(self, c, r, t, e)
    }
    fn outstanding_runtime_task_guidance_count(&self, t: &str) -> Result<u64, Self::Error> {
        NolaStore::outstanding_runtime_task_guidance_count(self, t)
    }
    fn runtime_task_guidance_records(
        &self,
        t: &str,
    ) -> Result<Vec<RuntimeTaskGuidance>, Self::Error> {
        NolaStore::runtime_task_guidance_records(self, t)
    }
    fn runtime_task_guidance(
        &self,
        t: &str,
        m: &str,
    ) -> Result<Option<RuntimeTaskGuidance>, Self::Error> {
        NolaStore::runtime_task_guidance(self, t, m)
    }
    fn pending_runtime_task_guidance(
        &self,
        t: &str,
        r: &str,
    ) -> Result<Vec<RuntimeTaskGuidance>, Self::Error> {
        NolaStore::pending_runtime_task_guidance(self, t, r)
    }
    fn insert_runtime_task_guidance(
        &mut self,
        g: &RuntimeTaskGuidance,
    ) -> Result<RuntimeTaskGuidance, Self::Error> {
        NolaStore::insert_runtime_task_guidance(self, g)
    }
    fn resolve_runtime_task_guidance(
        &mut self,
        t: &str,
        m: &str,
        s: &str,
        r: Option<&str>,
        x: Option<&str>,
        at: i64,
    ) -> Result<RuntimeTaskGuidance, Self::Error> {
        NolaStore::resolve_runtime_task_guidance(self, t, m, s, r, x, at)
    }
    fn runtime_task_terminal_receipt(
        &self,
        task_id: &str,
    ) -> Result<Option<RuntimeTaskTerminalReceipt>, Self::Error> {
        NolaStore::runtime_task_terminal_receipt(self, task_id)
    }
    type Error = StoreError;
    fn runtime_task_run_binding(
        &self,
        task_id: &str,
    ) -> Result<Option<RuntimeTaskRunBinding>, Self::Error> {
        NolaStore::runtime_task_run_binding(self, task_id)
    }
    fn runtime_task_binding_for_run(&self, child: &str, run: &str) -> Result<Option<RuntimeTaskRunBinding>, Self::Error> {
        let id:Option<String>=self.connection.query_row("SELECT task_id FROM runtime_task_run_bindings WHERE child_session_id=?1 AND run_id=?2",params![child,run],|r|r.get(0)).optional()?;
        id.map(|id|NolaStore::runtime_task_run_binding(self,&id)).transpose().map(Option::flatten)
    }
    fn validate_runtime_task_turn_admission(
        &self,
        lease: &RuntimeTaskLeaseWitness,
        child: &str,
        at: i64,
    ) -> Result<(), Self::Error> {
        NolaStore::validate_runtime_task_turn_admission(self, lease, child, at)
    }
    fn bind_runtime_task_run(
        &mut self,
        lease: &RuntimeTaskLeaseWitness,
        child: &str,
        run: &str,
        at: i64,
    ) -> Result<RuntimeTaskRunBinding, Self::Error> {
        NolaStore::bind_runtime_task_run(self, lease, child, run, at)
    }
    fn is_runtime_task_child_session(&self, child: &str) -> Result<bool, Self::Error> {
        NolaStore::is_runtime_task_child_session(self, child)
    }
    fn reconcile_runtime_task_bound_run_terminal(
        &mut self,
        task: &str,
        run: &str,
        at: i64,
    ) -> Result<bool, Self::Error> {
        NolaStore::reconcile_runtime_task_bound_run_terminal(self, task, run, at)
    }
    fn runtime_task_permission_continuation_grant(
        &self,
        task_id: &str,
        suspension_id: &str,
        selector: &PermissionContinuationGrantSelector,
    ) -> Result<Option<RuntimeTaskPermissionContinuationGrant>, Self::Error> {
        NolaStore::runtime_task_permission_continuation_grant(
            self,
            task_id,
            suspension_id,
            selector,
        )
    }
    fn recover_runtime_task_permission_continuation_fair(
        &mut self,
        input: &RecoverRuntimeTaskPermissionContinuationFair,
    ) -> Result<Option<RuntimeTaskAttempt>, Self::Error> {
        NolaStore::recover_runtime_task_permission_continuation_fair(self, input)
    }
    fn runtime_task_permission_resume(
        &self,
        task_id: &str,
        suspension_id: &str,
        permission_request_id: &str,
        continuation_id: &str,
    ) -> Result<Option<RuntimeTaskPermissionResume>, Self::Error> {
        NolaStore::runtime_task_permission_resume(
            self,
            task_id,
            suspension_id,
            permission_request_id,
            continuation_id,
        )
    }
    fn suspend_runtime_task_for_command_reconciliation(
        &mut self,
        input: &SuspendRuntimeTaskForCommandReconciliation,
    ) -> Result<Option<RuntimeTaskCommandReconciliation>, Self::Error> {
        NolaStore::suspend_runtime_task_for_command_reconciliation(self, input)
    }
    fn pending_runtime_task_command_reconciliation(
        &self,
        task_id: &str,
    ) -> Result<Option<RuntimeTaskCommandReconciliation>, Self::Error> {
        NolaStore::pending_runtime_task_command_reconciliation(self, task_id)
    }
    fn acknowledge_runtime_task_model_delivery(
        &mut self,
        task_id: &str,
    ) -> Result<bool, Self::Error> {
        NolaStore::acknowledge_runtime_task_model_delivery(self, task_id)
    }
    fn acknowledge_runtime_task_result(
        &mut self,
        input: &AcknowledgeRuntimeTaskResult,
    ) -> Result<bool, Self::Error> {
        NolaStore::acknowledge_runtime_task_result(self, input)
    }
    fn append_runtime_task_event(
        &mut self,
        task_id: &str,
        kind: &str,
        status: &str,
        summary: Option<&str>,
        detail_json: &str,
        created_at_ms: i64,
    ) -> Result<i64, Self::Error> {
        NolaStore::append_runtime_task_event(
            self,
            task_id,
            kind,
            status,
            summary,
            detail_json,
            created_at_ms,
        )
    }
    fn claim_runtime_task(
        &mut self,
        task_id: &str,
        worker_id: &str,
        worker_generation: &str,
        ownership_token: &str,
        now_ms: i64,
        lease_until_ms: i64,
    ) -> Result<Option<RuntimeTaskAttempt>, Self::Error> {
        NolaStore::claim_runtime_task(
            self,
            task_id,
            worker_id,
            worker_generation,
            ownership_token,
            now_ms,
            lease_until_ms,
        )
    }
    fn claim_runtime_task_fair(
        &mut self,
        task_id: &str,
        worker_id: &str,
        worker_generation: &str,
        ownership_token: &str,
        now_ms: i64,
        lease_until_ms: i64,
        max_running_global: u32,
        max_running_project: u32,
    ) -> Result<Option<RuntimeTaskAttempt>, Self::Error> {
        NolaStore::claim_runtime_task_fair(
            self,
            task_id,
            worker_id,
            worker_generation,
            ownership_token,
            now_ms,
            lease_until_ms,
            max_running_global,
            max_running_project,
        )
    }
    fn create_runtime_task_with_event(
        &mut self,
        task: &StoredRuntimeTask,
    ) -> Result<(), Self::Error> {
        NolaStore::create_runtime_task_with_event(self, task)
    }
    fn fail_expired_runtime_task(
        &mut self,
        task_id: &str,
        error: &str,
        now_ms: i64,
    ) -> Result<bool, Self::Error> {
        NolaStore::fail_expired_runtime_task(self, task_id, error, now_ms)
    }
    fn fail_expired_runtime_task_with_event(
        &mut self,
        task_id: &str,
        error: &str,
        event_kind: &str,
        event_status: &str,
        event_summary: Option<&str>,
        event_detail_json: &str,
        now_ms: i64,
    ) -> Result<bool, Self::Error> {
        NolaStore::fail_expired_runtime_task_with_event(
            self,
            task_id,
            error,
            event_kind,
            event_status,
            event_summary,
            event_detail_json,
            now_ms,
        )
    }
    fn finish_runtime_task_detailed_with_event(
        &mut self,
        lease: &RuntimeTaskLeaseWitness,
        status: &str,
        completion_watermark: Option<i64>,
        progress_summary: Option<&str>,
        result_summary: Option<&str>,
        error: Option<&str>,
        output_path: Option<&str>,
        unread: bool,
        event_kind: &str,
        event_status: &str,
        event_summary: Option<&str>,
        event_detail_json: &str,
        final_progress: Option<&RuntimeTaskFinalProgress>,
        now_ms: i64,
    ) -> Result<bool, Self::Error> {
        NolaStore::finish_runtime_task_detailed_with_event(
            self,
            lease,
            status,
            completion_watermark,
            progress_summary,
            result_summary,
            error,
            output_path,
            unread,
            event_kind,
            event_status,
            event_summary,
            event_detail_json,
            final_progress,
            now_ms,
        )
    }
    fn renew_runtime_task_lease(
        &mut self,
        renewal: &RenewRuntimeTaskLease,
    ) -> Result<Option<RuntimeTaskAttempt>, Self::Error> {
        NolaStore::renew_runtime_task_lease(self, renewal)
    }
    fn latest_runtime_task_event_id(&self) -> Result<i64, Self::Error> {
        NolaStore::latest_runtime_task_event_id(self)
    }
    fn runtime_task_for_delegation(
        &self, owner: &str, run: &str, effect: &str,
    ) -> Result<Option<StoredRuntimeTask>, Self::Error> {
        NolaStore::runtime_task_for_delegation(self, owner, run, effect)
    }
    fn list_all_runtime_tasks(&self) -> Result<Vec<StoredRuntimeTask>, Self::Error> {
        NolaStore::list_all_runtime_tasks(self)
    }
    fn list_runtime_tasks(
        &self,
        project_root: &str,
    ) -> Result<Vec<StoredRuntimeTask>, Self::Error> {
        NolaStore::list_runtime_tasks(self, project_root)
    }
    fn recover_expired_runtime_task(
        &mut self,
        task_id: &str,
        error: &str,
        retry_at_ms: i64,
        now_ms: i64,
    ) -> Result<bool, Self::Error> {
        NolaStore::recover_expired_runtime_task(self, task_id, error, retry_at_ms, now_ms)
    }
    fn request_runtime_task_cancel(
        &mut self,
        task_id: &str,
        now_ms: i64,
    ) -> Result<bool, Self::Error> {
        NolaStore::request_runtime_task_cancel(self, task_id, now_ms)
    }
    fn observe_runtime_task_permission_budget(
        &mut self,
        mutation: &RuntimeTaskPermissionBudgetMutation,
    ) -> Result<bool, Self::Error> {
        NolaStore::observe_runtime_task_permission_budget(self, mutation)
    }
    fn report_runtime_task_progress(
        &mut self,
        mutation: &RuntimeTaskProgressMutation,
    ) -> Result<Option<RuntimeTaskAttempt>, Self::Error> {
        NolaStore::report_runtime_task_progress(self, mutation)
    }
    fn resume_runtime_task_after_permission_fair(
        &mut self,
        input: &ResumeRuntimeTaskAfterPermissionFair,
    ) -> Result<Option<RuntimeTaskAttempt>, Self::Error> {
        NolaStore::resume_runtime_task_after_permission_fair(self, input)
    }
    fn runtime_task_permission_suspension(
        &self,
        id: &str,
    ) -> Result<Option<RuntimeTaskPermissionSuspension>, Self::Error> {
        NolaStore::runtime_task_permission_suspension(self, id)
    }
    fn pending_runtime_task_permission_suspension(
        &self,
        task_id: &str,
    ) -> Result<Option<RuntimeTaskPermissionSuspension>, Self::Error> {
        NolaStore::pending_runtime_task_permission_suspension(self, task_id)
    }
    fn latest_runtime_task_permission_suspension(
        &self,
        task_id: &str,
    ) -> Result<Option<RuntimeTaskPermissionSuspension>, Self::Error> {
        NolaStore::latest_runtime_task_permission_suspension(self, task_id)
    }
    fn cancel_runtime_task_permission_suspension(
        &mut self,
        input: &CancelRuntimeTaskPermissionSuspension,
    ) -> Result<Option<RuntimeTaskPermissionSuspension>, Self::Error> {
        NolaStore::cancel_runtime_task_permission_suspension(self, input)
    }
    fn suspend_runtime_task_for_permission(
        &mut self,
        input: &SuspendRuntimeTaskForPermission,
    ) -> Result<Option<RuntimeTaskPermissionSuspension>, Self::Error> {
        NolaStore::suspend_runtime_task_for_permission(self, input)
    }
    fn retry_runtime_task(
        &mut self,
        lease: &RuntimeTaskLeaseWitness,
        retry_at_ms: i64,
        error: &str,
        now_ms: i64,
    ) -> Result<bool, Self::Error> {
        NolaStore::retry_runtime_task(self, lease, retry_at_ms, error, now_ms)
    }
    fn runtime_task(&self, id: &str) -> Result<Option<StoredRuntimeTask>, Self::Error> {
        NolaStore::runtime_task(self, id)
    }
    fn runtime_task_attempt(&self, id: &str) -> Result<Option<RuntimeTaskAttempt>, Self::Error> {
        NolaStore::runtime_task_attempt(self, id)
    }
    fn runtime_task_current_attempt(
        &self,
        task_id: &str,
    ) -> Result<Option<RuntimeTaskAttempt>, Self::Error> {
        NolaStore::runtime_task_current_attempt(self, task_id)
    }
    fn runtime_task_attempt_retired_for_user_input(&self, task_id: &str, attempt_id: &str) -> Result<bool, Self::Error> {
        NolaStore::runtime_task_attempt_retired_for_user_input(self, task_id, attempt_id)
    }

    fn runtime_task_events(
        &self,
        task_id: &str,
        limit: usize,
    ) -> Result<Vec<StoredRuntimeTaskEvent>, Self::Error> {
        NolaStore::runtime_task_events(self, task_id, limit)
    }
    fn runtime_task_events_after_session_cursor(
        &self,
        owner_session_id: &str,
        after_cursor: i64,
        limit: usize,
    ) -> Result<Vec<StoredRuntimeTaskEvent>, Self::Error> {
        NolaStore::runtime_task_events_after_session_cursor(
            self,
            owner_session_id,
            after_cursor,
            limit,
        )
    }
}
