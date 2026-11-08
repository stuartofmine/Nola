use nola_store::{NolaStore, StoredAutomationRun, STORE_SCHEMA_VERSION};
use tempfile::tempdir;
use nola_domain::automation_execution::{AutomationExecutionLease, AutomationWorkerObservation};

fn queued_run(id: &str, occurrence: &str) -> StoredAutomationRun {
    StoredAutomationRun {
        definition_revision: 1,
        id: id.into(),
        automation_id: "morning-brief".into(),
        occurrence_key: occurrence.into(),
        trigger: "schedule".into(),
        status: "queued".into(),
        attempt: 0,
        worker_id: None,
        lease_token: None,
        lease_until_ms: None,
        scheduled_for_ms: 1_000,
        started_at_ms: None,
        finished_at_ms: None,
        next_retry_at_ms: None,
        result_json: None,
        error_json: None,
        created_at_ms: 900,
        updated_at_ms: 900,
    }
}

fn seed_agent_definition(store: &mut NolaStore) {
    store.upsert_automation_definition(&nola_store::StoredAutomationDefinition {
        id: "morning-brief".into(), name: "Brief".into(), schedule_format: "manual".into(),
        enabled: true, revision: 0, metadata_json: "{}".into(), created_at_ms: 1,
        updated_at_ms: 1, deleted_at_ms: None,
        execution_spec: serde_json::from_value(serde_json::json!({"kind":"agent","spec":{
            "prompt":"Brief once","execution_scope":{"kind":"local_workspace","root":"/fixture"},"permission_profile":"cautious",
            "model_selection":"use_current_default_at_admission","context_refs":[],"skill_refs":[]
        }})).unwrap(),
    }).unwrap();
}

#[test]
fn worker_preparation_and_admission_request_are_durable_and_forbid_uncertain_retry() {
    use nola_domain::automation_execution::AutomationExecutionPhase;
    let root = tempdir().unwrap();
    let path = root.path().join("preparation.db");
    let mut store = NolaStore::open(&path).unwrap();
    seed_agent_definition(&mut store);
    store.enqueue_automation_run(&queued_run("run", "slot")).unwrap();
    assert!(store.claim_automation_run("run", "worker", "lease", 1_000, 2_000).unwrap());
    let lease = AutomationExecutionLease { occurrence_id: "run".into(), definition_revision: 1,
        worker_id: "worker".into(), lease_token: "lease".into() };
    assert_eq!(store.observe_automation_execution("run").unwrap().phase, AutomationExecutionPhase::Claimed);
    assert!(store.observe_automation_worker(&lease, &AutomationWorkerObservation::Preparing, 1_001).unwrap());
    assert_eq!(store.observe_automation_execution("run").unwrap().phase, AutomationExecutionPhase::Preparing);
    assert!(store.observe_automation_worker(&lease, &AutomationWorkerObservation::AdmissionRequested, 1_002).unwrap());
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    assert_eq!(store.observe_automation_execution("run").unwrap().phase, AutomationExecutionPhase::AdmissionPending);
    assert!(!store.retry_automation_run("run", "lease", 1_500, r#"{"code":"ack_unknown"}"#, 1_003).unwrap());
    let mut wrong_revision = lease.clone();
    wrong_revision.definition_revision = 2;
    assert!(!store.observe_automation_worker(&wrong_revision,
        &AutomationWorkerObservation::LaunchFailed { detail: "wrong revision".into() }, 1_004).unwrap());
    assert!(store.observe_automation_worker(&lease,
        &AutomationWorkerObservation::LaunchFailed { detail: "actual startup exception".into() }, 1_005).unwrap());
    assert!(store.validate_automation_execution(&lease, 1_006).is_err(), "Failure fencing revokes delayed admission");
    assert!(!store.claim_automation_run("run", "replacement", "replacement", 2_001, 3_000).unwrap());
    assert_eq!(store.automation_run("run").unwrap().unwrap().status, "launch_failed");
}

#[test]
fn occurrence_revision_fences_enqueue_claim_and_admission_across_connections() {
    let root = tempdir().unwrap();
    let path = root.path().join("definition-fence.db");
    let mut original = NolaStore::open(&path).unwrap();
    seed_agent_definition(&mut original);
    let first = queued_run("original", "original-occurrence");
    original.enqueue_automation_run(&first).unwrap();
    assert!(original.claim_automation_run(&first.id, "worker", "lease", 1_000, 2_000).unwrap());
    let mut editor = NolaStore::open(&path).unwrap();
    let mut changed = editor.automation_definition("morning-brief").unwrap().unwrap();
    changed.name = "Changed definition".into();
    changed.updated_at_ms = 1_001;
    let saved = editor.upsert_automation_definition(&changed).unwrap();
    assert_eq!(saved.revision, 2);
    // An old occurrence cannot borrow a fresh worker's current revision.
    assert!(!editor.claim_automation_run(&first.id, "new-worker", "new-lease", 2_001, 3_000).unwrap());
    for revision in [1, 2] {
        assert!(original.validate_automation_execution(&AutomationExecutionLease {
            occurrence_id:first.id.clone(),definition_revision:revision,worker_id:"worker".into(),lease_token:"lease".into()
        }, 1_002).is_err());
    }
    let stale = queued_run("stale", "stale-occurrence");
    assert!(original.enqueue_automation_run(&stale).is_err());
    assert!(original.automation_run("stale").unwrap().is_none());
    assert_eq!(original.enqueue_automation_run(&first).unwrap(), original.automation_run(&first.id).unwrap().unwrap());
    let mut hijack = first.clone();
    hijack.definition_revision = 2;
    assert!(editor.enqueue_automation_run(&hijack).is_err());
    let mut fresh = queued_run("fresh", "fresh-occurrence");
    fresh.definition_revision = 2;
    editor.enqueue_automation_run(&fresh).unwrap();
    assert!(editor.claim_automation_run("fresh", "worker", "fresh-lease", 2_010, 3_000).unwrap());
    let mut disabled = saved;
    disabled.enabled = false;
    disabled.updated_at_ms = 2_011;
    let disabled = editor.upsert_automation_definition(&disabled).unwrap();
    let mut blocked = queued_run("disabled", "disabled-occurrence");
    blocked.definition_revision = disabled.revision;
    assert!(original.enqueue_automation_run(&blocked).is_err());
    assert!(original.automation_run("disabled").unwrap().is_none());
}

#[test]
fn user_task_is_persisted_but_cannot_enter_agent_occurrence_execution() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("user-task.db");
    let mut store = NolaStore::open(&path).unwrap();
    seed_agent_definition(&mut store);
    store.enqueue_automation_run(&queued_run("previous-agent", "old")).unwrap();
    let mut definition = store.automation_definition("morning-brief").unwrap().unwrap();
    definition.execution_spec = serde_json::from_value(serde_json::json!({"kind":"user_task","task":{
        "task_type":"debt","description":null,"target_at_ms":null,"recurrence":null,
        "related_person":null,"amount_decimal":"1234567890.123456789","currency":"CNY",
        "due_at_ms":null,"completed":false,"notification_ids":[]
    }})).unwrap();
    definition.updated_at_ms = 2;
    let saved = store.upsert_automation_definition(&definition).unwrap();
    assert!(store.enqueue_automation_run(&queued_run("forbidden", "new")).unwrap_err().to_string().contains("wrong-definition-kind"));
    assert!(store.claim_automation_run("previous-agent", "worker", "lease", 1000, 2000).unwrap_err().to_string().contains("wrong-definition-kind"));
    assert!(store.automation_run("forbidden").unwrap().is_none());
    drop(store);
    let reopened = NolaStore::open(&path).unwrap();
    assert_eq!(reopened.automation_definition("morning-brief").unwrap().unwrap().execution_spec, saved.execution_spec);
}

#[test]
fn expired_owner_cannot_schedule_retry_or_be_replaced_without_admission_evidence() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("lease-fence.db");
    {
        let mut store = NolaStore::open(&database).unwrap();
        seed_agent_definition(&mut store);
        store.enqueue_automation_run(&queued_run("run", "occurrence")).unwrap();
        assert!(store.claim_automation_run("run", "old-worker", "old-lease", 1_000, 1_100).unwrap());
    }
    let mut store = NolaStore::open(&database).unwrap();
    let before = store.automation_run("run").unwrap().unwrap();
    for now in [1_100, 1_101] {
        assert!(!store.retry_automation_run("run", "old-lease", 1_500,
            r#"{"code":"transport_failed"}"#, now).unwrap());
    }
    let unchanged = store.automation_run("run").unwrap().unwrap();
    assert_eq!(unchanged.status, before.status);
    assert_eq!(unchanged.updated_at_ms, before.updated_at_ms);
    assert_eq!(unchanged.lease_token, before.lease_token);
    assert_eq!(unchanged.result_json, before.result_json);
    assert_eq!(unchanged.error_json, before.error_json);
    assert!(!store.claim_automation_run("run", "new-worker", "new-lease", 1_101, 1_400).unwrap());
    assert!(!store.observe_automation_worker(&AutomationExecutionLease {
        occurrence_id: "run".into(), definition_revision: 1,
        worker_id: "new-worker".into(), lease_token: "new-lease".into(),
    }, &AutomationWorkerObservation::LaunchFailed { detail: "not the owner".into() }, 1_200).unwrap());
}

#[test]
fn occurrence_is_idempotent_and_cold_recovery_closes_unadmitted_work_without_reexecution() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("automation.db");

    {
        let mut store = NolaStore::open(&database).unwrap();
        seed_agent_definition(&mut store);
        assert_eq!(store.schema_version().unwrap(), STORE_SCHEMA_VERSION);
        let first = store
            .enqueue_automation_run(&queued_run("run-1", "2026-08-18T08:00"))
            .unwrap();
        let duplicate = store
            .enqueue_automation_run(&queued_run("run-duplicate", "2026-08-18T08:00"))
            .unwrap();
        assert_eq!(first.id, "run-1");
        assert_eq!(duplicate.id, "run-1");

        assert!(store
            .claim_automation_run("run-1", "worker-a", "lease-a", 1_000, 1_100)
            .unwrap());
        assert!(!store
            .claim_automation_run("run-1", "worker-b", "lease-b", 1_050, 1_200)
            .unwrap());
    }

    {
        let mut reopened = NolaStore::open(&database).unwrap();
        assert!(!reopened
            .claim_automation_run("run-1", "worker-b", "lease-b", 1_101, 1_300)
            .unwrap());
        let claimed = reopened.automation_run("run-1").unwrap().unwrap();
        assert_eq!(claimed.attempt, 1);
        assert_eq!(claimed.worker_id.as_deref(), Some("worker-a"));
        assert!(reopened.observe_automation_worker(&AutomationExecutionLease {
            occurrence_id: "run-1".into(), definition_revision: 1,
            worker_id: "worker-a".into(), lease_token: "lease-a".into(),
        }, &AutomationWorkerObservation::LaunchFailed { detail: "Recovered unadmitted occurrence; original startup exception is unknown".into() }, 1_600).unwrap());
        let observed = reopened.observe_automation_execution("run-1").unwrap();
        assert!(observed.binding.is_none());
        assert!(observed.terminal.is_none(), "startup failure is not an admitted Run result");
        assert_eq!(observed.phase, nola_domain::automation_execution::AutomationExecutionPhase::LaunchFailed);
        assert!(matches!(observed.latest_worker_observation, Some(AutomationWorkerObservation::LaunchFailed { .. })));
    }

    let final_store = NolaStore::open(&database).unwrap();
    let run = final_store.automation_run("run-1").unwrap().unwrap();
    assert_eq!(run.status, "launch_failed");
    assert_eq!(run.attempt, 1);
    assert!(run.result_json.is_none());
    assert!(run.lease_token.is_none());
    assert!(run.error_json.unwrap().contains("original startup exception is unknown"));
    assert_eq!(
        final_store
            .list_automation_runs(Some("morning-brief"), 10)
            .unwrap()
            .len(),
        1
    );
}
