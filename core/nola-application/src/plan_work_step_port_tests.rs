// SPDX-License-Identifier: Apache-2.0
//! Real business helpers against an in-memory Port, with no SQLite dependency.
#![allow(unused_variables)]
use super::*;
use crate::work_step_transaction::*;
use nola_store_port::work_step_transaction::*;
#[derive(Default)]
struct MemoryPlan {
    plan: Option<StoredConversationPlan>,
    step: Option<StoredWorkStepTransaction>,
    reject: bool,
    linked: usize,
}
impl ConversationPlanRepository for MemoryPlan {
    type Error = String;
    fn save_conversation_plan(
        &mut self,
        conversation_id: &str,
        sop_id: &str,
        sop_version: u32,
        run_id: &str,
        todos_json: &str,
        updated_at_ms: i64,
    ) -> Result<StoredConversationPlan, Self::Error> {
        let value = StoredConversationPlan {
            conversation_id: conversation_id.into(),
            sop_id: sop_id.into(),
            sop_version,
            run_id: run_id.into(),
            todos_json: todos_json.into(),
            revision: 1,
            frozen_revision: None,
            updated_at_ms,
        };
        self.plan = Some(value.clone());
        Ok(value)
    }
    fn conversation_plan(
        &self,
        conversation_id: &str,
    ) -> Result<Option<StoredConversationPlan>, Self::Error> {
        Ok(self.plan.clone())
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
    ) -> Result<Option<StoredConversationPlan>, Self::Error> {
        panic!("terminal step must not separately write a plan")
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
    ) -> Result<Option<StoredConversationPlan>, Self::Error> {
        panic!("terminal step must not separately write a plan")
    }
    fn conversation_plan_revisions(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<StoredConversationPlanRevision>, Self::Error> {
        Ok(vec![])
    }
}
impl WorkStepTransactionRepository for MemoryPlan {
    fn prepare_work_step_transaction(
        &mut self,
        input: &StoredWorkStepTransaction,
    ) -> Result<(StoredWorkStepTransaction, bool), Self::Error> {
        self.step = Some(input.clone());
        Ok((input.clone(), true))
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
    ) -> Result<Option<StoredWorkStepTransaction>, Self::Error> {
        panic!("linked terminal step must use the atomic Port method")
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
    ) -> Result<Option<StoredWorkStepTransaction>, Self::Error> {
        self.linked += 1;
        if self.reject {
            return Err("injected atomic CAS rejection".into());
        }
        let mut plan = self.plan.clone().unwrap();
        let mut step = self.step.clone().unwrap();
        if plan.revision != plan_expected_revision || step.revision != expected_revision {
            return Ok(None);
        }
        plan.todos_json = todos_json.into();
        plan.revision += 1;
        step.revision += 1;
        step.phase = phase.into();
        step.result_json = result_json.map(str::to_owned);
        self.plan = Some(plan);
        self.step = Some(step.clone());
        Ok(Some(step))
    }
    fn work_step_transaction(
        &self,
        id: &str,
    ) -> Result<Option<StoredWorkStepTransaction>, Self::Error> {
        Ok(self.step.clone())
    }
    fn incomplete_work_step_transactions(
        &self,
        work_id: Option<&str>,
    ) -> Result<Vec<StoredWorkStepTransaction>, Self::Error> {
        Ok(self.step.clone().into_iter().collect())
    }
}

fn steps() -> Value {
    serde_json::json!([{"id":"s","content":"Write","status":"in_progress","priority":"high","dependencyIds":[],"approvalStatus":"not_required","capabilityRefs":[],"expectedArtifactRefs":[],"successCriteria":["exists"],"verificationStatus":"pending"}])
}

#[test]
fn ordinary_plan_save_cannot_drop_execution_references_or_their_entire_step() {
    let mut initial = steps();
    initial[0]["artifactRefs"] = serde_json::json!(["artifact:written"]);
    initial[0]["workStepTransactionRefs"] = serde_json::json!(["transaction:written"]);
    let mut store = MemoryPlan::default();
    let sop = ConversationPlanSopIdentity {
        run_id: "r".into(), sop_id: "test".into(), sop_version: 1,
    };
    execute_conversation_plan_command(&mut store, ConversationPlanCommand::Save {
        conversation_id: "w".into(), expected_revision: None,
        sop: sop.clone(), todos: initial.clone(), updated_at_ms: 1,
    }).unwrap();
    let before = store.plan.clone();
    let mut omitted = steps();
    omitted[0]["id"] = serde_json::json!("replacement");
    let mut removed_artifact = initial.clone();
    removed_artifact[0]["artifactRefs"] = serde_json::json!([]);
    let mut removed_transaction = initial.clone();
    removed_transaction[0]["workStepTransactionRefs"] = serde_json::json!([]);
    for todos in [omitted, removed_artifact, removed_transaction] {
        let error = execute_conversation_plan_command(&mut store, ConversationPlanCommand::Save {
            conversation_id: "w".into(), expected_revision: Some(1),
            sop: sop.clone(), todos, updated_at_ms: 2,
        }).unwrap_err();
        assert!(error.contains("cannot discard persisted"), "{error}");
        assert_eq!(store.plan, before, "rejected plan update changed storage");
    }
    let current = decode_plan_steps(initial.clone()).unwrap();
    assert!(validate_retained_execution_references(&current, &current).is_ok());
    initial[0]["artifactRefs"] = serde_json::json!(["artifact:written", "artifact:additional"]);
    assert!(validate_retained_execution_references(&current, &decode_plan_steps(initial).unwrap()).is_ok());
    let draft = decode_plan_steps(steps()).unwrap();
    assert!(validate_retained_execution_references(&draft, &[]).is_ok());
}

#[test]
fn real_plan_and_terminal_helpers_use_one_atomic_link_and_reject_without_partial_mutation() {
    let mut store = MemoryPlan::default();
    execute_conversation_plan_command(
        &mut store,
        ConversationPlanCommand::Save {
            expected_revision: None,
            conversation_id: "w".into(),
            sop: ConversationPlanSopIdentity {
                run_id: "r".into(),
                sop_id: "test".into(),
                sop_version: 1,
            },
            todos: steps(),
            updated_at_ms: 1,
        },
    )
    .unwrap();
    execute_work_step_transaction_command(
        &mut store,
        WorkStepTransactionCommand::Prepare {
            id: "tx".into(),
            work_id: "w".into(),
            step_id: "s".into(),
            tool_call_id: "call".into(),
            root: "/workspace".into(),
            effect_kind: "workspace_write".into(),
            workspace_checkpoint_id: None,
            request: serde_json::json!({"path":"a"}),
            created_at_ms: 1,
        },
    )
    .unwrap();
    store.step.as_mut().unwrap().phase = "effect_observed".into();
    let before = (store.plan.clone(), store.step.clone());
    let command = || WorkStepTransactionCommand::Advance {
        id: "tx".into(),
        expected_revision: 1,
        phase: WorkStepTransactionPhase::Committed,
        workspace_checkpoint_id: None,
        result: Some(serde_json::json!({"ok":true})),
        result_ref: None,
        error: None,
        updated_at_ms: 2,
    };
    store.reject = true;
    assert!(execute_work_step_transaction_command(&mut store, command())
        .unwrap_err()
        .contains("atomic CAS"));
    assert_eq!((store.plan.clone(), store.step.clone()), before);
    store.reject = false;
    let result = execute_work_step_transaction_command(&mut store, command()).unwrap();
    assert_eq!(result["phase"], "committed");
    assert_eq!(store.linked, 2);
    let plan = conversation_plan(store.plan.unwrap()).unwrap();
    assert_eq!(plan.todos[0].work_step_transaction_refs, vec!["tx"]);
    assert_eq!(plan.revision, 2);
}
#[test]
fn invalid_plan_is_rejected_before_repository_write() {
    let mut store = MemoryPlan::default();
    let mut todos = steps();
    todos[0]["dependencyIds"] = serde_json::json!(["missing"]);
    assert!(execute_conversation_plan_command(
        &mut store,
        ConversationPlanCommand::Save {
            expected_revision: None,
            conversation_id: "w".into(),
            sop: ConversationPlanSopIdentity {
                run_id: "r".into(),
                sop_id: "test".into(),
                sop_version: 1
            },
            todos,
            updated_at_ms: 1
        }
    )
    .is_err());
    assert!(store.plan.is_none());
}
