// SPDX-License-Identifier: Apache-2.0
//! Consolidated automation lifecycle suite merging `automation_e2e`, `automation_revision_e2e`,
//! `automation_run_e2e`, and `automation_delete_e2e` into dense multi-scenario Store tests.

use nola_domain::automation_execution::{
    AutomationExecutionLease, AutomationExecutionPhase, AutomationWorkerObservation,
};
use nola_store::{
    NolaStore, StoredAutomationDefinition, StoredAutomationRun, STORE_SCHEMA_VERSION,
};

fn agent_definition(id: &str, name: &str, format: &str, updated_at_ms: i64) -> StoredAutomationDefinition {
    StoredAutomationDefinition {
        execution_spec: serde_json::from_value(serde_json::json!({
            "kind": "agent",
            "spec": {
                "prompt": "Execute this task once",
                "execution_scope": {"kind": "local_workspace", "root": "/fixture"},
                "permission_profile": "cautious",
                "model_selection": "use_current_default_at_admission",
                "context_refs": [],
                "skill_refs": []
            }
        }))
        .unwrap(),
        id: id.into(),
        name: name.into(),
        schedule_format: format.into(),
        enabled: true,
        revision: 0,
        metadata_json: format!(r#"{{"id":"{id}","name":"{name}","format":"{format}"}}"#),
        created_at_ms: 100,
        updated_at_ms,
        deleted_at_ms: None,
    }
}

fn queued_run_for(automation_id: &str, id: &str, occurrence: &str) -> StoredAutomationRun {
    StoredAutomationRun {
        definition_revision: 1,
        id: id.into(),
        automation_id: automation_id.into(),
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

#[test]
fn automation_definition_lifecycle_reconcile_cas_concurrency_and_deletion_hold_across_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("automation-lifecycle.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    let mut second = NolaStore::open(&path).unwrap();

    // 1. Scoped reconcile preserves other schedule formats and rejects identity takeover atomically.
    let mac = agent_definition("mac", "Mac task", "nola.mac.automation.v1", 100);
    let committed_mac = store.upsert_automation_definition(&mac).unwrap();
    let web = agent_definition("web", "Web task", "cron", 100);
    store.upsert_automation_definition(&web).unwrap();
    let cron_scope = ["cron".to_string()];

    let result = store
        .reconcile_automation_definitions(&[web.clone()], &cron_scope, 200)
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id, "web");
    assert_eq!(
        store.automation_definition("mac").unwrap(),
        Some(committed_mac.clone())
    );
    let before = store.list_automation_definitions().unwrap();
    let mut changed_web = web.clone();
    changed_web.name = "Must roll back".into();
    let collision = agent_definition("mac", "Foreign overwrite", "cron", 300);
    assert!(store
        .reconcile_automation_definitions(&[changed_web, collision], &cron_scope, 300)
        .is_err());
    assert_eq!(store.list_automation_definitions().unwrap(), before);
    assert!(store
        .reconcile_automation_definitions(&[mac], &cron_scope, 300)
        .is_err());
    assert!(store
        .reconcile_automation_definitions(&[], &[], 300)
        .is_err());
    assert_eq!(store.list_automation_definitions().unwrap(), before);
    assert!(store
        .reconcile_automation_definitions(&[], &cron_scope, 400)
        .unwrap()
        .is_empty());
    assert!(store.automation_definition("web").unwrap().is_none());
    assert_eq!(
        store.automation_definition("mac").unwrap(),
        Some(committed_mac)
    );
    assert!(!store
        .pending_sync_changes(100)
        .unwrap()
        .iter()
        .any(|change| change.entity_id == "mac" && change.deleted));

    // 2. Conditional CAS revision update rejects stale/foreign/deleted revisions across connections.
    let mut reviewed_seed = agent_definition("reviewed", "Script", "script", 1);
    reviewed_seed.enabled = false;
    reviewed_seed.metadata_json = r#"{"script":"old","approved":false}"#.into();
    reviewed_seed.created_at_ms = 1;
    let original_reviewed = store.upsert_automation_definition(&reviewed_seed).unwrap();
    let mut edited = original_reviewed.clone();
    edited.metadata_json = r#"{"script":"changed","approved":false}"#.into();
    edited.updated_at_ms = 2;
    let current_reviewed = second.upsert_automation_definition(&edited).unwrap();
    let outbox_count = store.pending_sync_changes(100).unwrap().len();
    let mut stale_approval = original_reviewed.clone();
    stale_approval.enabled = true;
    stale_approval.metadata_json = r#"{"script":"old","approved":true}"#.into();
    stale_approval.updated_at_ms = 3;
    assert!(store
        .update_automation_definition_if_revision(&stale_approval, original_reviewed.revision)
        .unwrap()
        .is_none());
    assert_eq!(
        store.automation_definition("reviewed").unwrap().unwrap(),
        current_reviewed
    );
    assert_eq!(store.pending_sync_changes(100).unwrap().len(), outbox_count);
    let mut foreign = current_reviewed.clone();
    foreign.schedule_format = "nola.mac.automation.v1".into();
    assert!(store
        .update_automation_definition_if_revision(&foreign, current_reviewed.revision)
        .unwrap()
        .is_none());
    assert_eq!(store.pending_sync_changes(100).unwrap().len(), outbox_count);
    let mut approved = current_reviewed.clone();
    approved.enabled = true;
    let saved_reviewed = store
        .update_automation_definition_if_revision(&approved, current_reviewed.revision)
        .unwrap()
        .unwrap();
    assert_eq!(saved_reviewed.revision, current_reviewed.revision + 1);
    assert!(second
        .update_automation_definition_if_revision(&approved, current_reviewed.revision)
        .unwrap()
        .is_none());
    store.delete_automation_definition("reviewed", 10).unwrap();
    assert!(second
        .update_automation_definition_if_revision(&approved, saved_reviewed.revision)
        .unwrap()
        .is_none());
    assert!(second.automation_definition("reviewed").unwrap().is_none());

    // 3. Concurrent writers racing on CAS commit exactly one winner.
    let mut race_seed = agent_definition("race", "Script", "script", 1);
    race_seed.enabled = false;
    race_seed.metadata_json = "{}".into();
    let race = store.upsert_automation_definition(&race_seed).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers = (0..2)
        .map(|index| {
            let db_path = path.clone();
            let barrier = barrier.clone();
            let mut input = race.clone();
            std::thread::spawn(move || {
                let mut conn = NolaStore::open(db_path).unwrap();
                input.metadata_json = format!("{{\"writer\":{index}}}");
                barrier.wait();
                conn.update_automation_definition_if_revision(&input, input.revision)
                    .unwrap()
                    .is_some()
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        workers
            .into_iter()
            .map(|worker| usize::from(worker.join().unwrap()))
            .sum::<usize>(),
        1
    );
    assert_eq!(
        store.automation_definition("race").unwrap().unwrap().revision,
        race.revision + 1
    );

    // 4. Atomic idempotent deletion + versioned cron reconcile across cold reopen.
    let mut daily_review = agent_definition("daily-review", "Daily review", "manual", 1_000);
    daily_review.revision = 99;
    daily_review.metadata_json = "{}".into();
    daily_review.created_at_ms = 1_000;
    store.upsert_automation_definition(&daily_review).unwrap();
    assert!(store.delete_automation_definition("daily-review", 2_000).unwrap());
    assert!(!store.delete_automation_definition("daily-review", 3_000).unwrap());
    assert!(store.automation_definition("daily-review").unwrap().is_none());

    let first_daily = store
        .upsert_automation_definition(&agent_definition("daily", "Daily brief", "cron", 100))
        .unwrap();
    assert_eq!(first_daily.revision, 1);
    let unchanged_daily = store
        .upsert_automation_definition(&agent_definition("daily", "Daily brief", "cron", 200))
        .unwrap();
    assert_eq!(unchanged_daily.revision, 1);
    assert_eq!(unchanged_daily.updated_at_ms, 100);
    let reconciled = store
        .reconcile_automation_definitions(
            &[
                agent_definition("daily", "Daily briefing", "cron", 300),
                agent_definition("weekly", "Weekly review", "cron", 300),
            ],
            &cron_scope,
            300,
        )
        .unwrap();
    assert_eq!(reconciled.len(), 2);
    assert_eq!(
        store.automation_definition("daily").unwrap().unwrap().revision,
        2
    );
    store
        .reconcile_automation_definitions(
            &[agent_definition("weekly", "Weekly review", "cron", 300)],
            &cron_scope,
            400,
        )
        .unwrap();
    assert!(store.automation_definition("daily").unwrap().is_none());

    let before_invalid = store.list_automation_definitions().unwrap();
    let mut invalid = agent_definition("broken", "", "cron", 500);
    invalid.metadata_json = "not-json".into();
    assert!(store
        .reconcile_automation_definitions(&[invalid], &cron_scope, 500)
        .is_err());
    assert_eq!(store.list_automation_definitions().unwrap(), before_invalid);

    drop(second);
    drop(store);
    let mut reopened = NolaStore::open(&path).unwrap();
    assert_eq!(
        reopened.list_automation_definitions().unwrap(),
        before_invalid
    );
    assert!(reopened.automation_definition("daily-review").unwrap().is_none());
    let restored = reopened
        .reconcile_automation_definitions(
            &[
                agent_definition("daily", "Daily briefing", "cron", 600),
                agent_definition("weekly", "Weekly review", "cron", 300),
            ],
            &cron_scope,
            600,
        )
        .unwrap();
    assert_eq!(restored.len(), 2);
    assert_eq!(
        reopened
            .automation_definition("daily")
            .unwrap()
            .unwrap()
            .revision,
        4
    );
}

#[test]
fn automation_run_preparation_revision_fencing_user_task_guard_and_cold_recovery_hold() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("automation-runs.db");
    let mut store = NolaStore::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), STORE_SCHEMA_VERSION);

    for id in [
        "brief-prep",
        "brief-fence",
        "brief-user-task",
        "brief-expired",
        "brief-cold",
    ] {
        let mut def = agent_definition(id, "Brief", "manual", 1);
        def.metadata_json = "{}".into();
        def.created_at_ms = 1;
        store.upsert_automation_definition(&def).unwrap();
    }

    // 1. Worker preparation & admission request durability ("brief-prep").
    store
        .enqueue_automation_run(&queued_run_for("brief-prep", "run-prep", "slot"))
        .unwrap();
    assert!(store
        .claim_automation_run("run-prep", "worker", "lease", 1_000, 2_000)
        .unwrap());
    let prep_lease = AutomationExecutionLease {
        occurrence_id: "run-prep".into(),
        definition_revision: 1,
        worker_id: "worker".into(),
        lease_token: "lease".into(),
    };
    assert_eq!(
        store.observe_automation_execution("run-prep").unwrap().phase,
        AutomationExecutionPhase::Claimed
    );
    assert!(store
        .observe_automation_worker(&prep_lease, &AutomationWorkerObservation::Preparing, 1_001)
        .unwrap());
    assert_eq!(
        store.observe_automation_execution("run-prep").unwrap().phase,
        AutomationExecutionPhase::Preparing
    );
    assert!(store
        .observe_automation_worker(
            &prep_lease,
            &AutomationWorkerObservation::AdmissionRequested,
            1_002
        )
        .unwrap());

    // 2. Occurrence revision fences enqueue, claim, and admission across connections ("brief-fence").
    let mut editor = NolaStore::open(&path).unwrap();
    let first_fenced = queued_run_for("brief-fence", "original", "original-occurrence");
    store.enqueue_automation_run(&first_fenced).unwrap();
    assert!(store
        .claim_automation_run(&first_fenced.id, "worker", "lease", 1_000, 2_000)
        .unwrap());
    let mut changed = editor.automation_definition("brief-fence").unwrap().unwrap();
    changed.name = "Changed definition".into();
    changed.updated_at_ms = 1_001;
    let saved_fence = editor.upsert_automation_definition(&changed).unwrap();
    assert_eq!(saved_fence.revision, 2);
    assert!(!editor
        .claim_automation_run(&first_fenced.id, "new-worker", "new-lease", 2_001, 3_000)
        .unwrap());
    for revision in [1, 2] {
        assert!(store
            .validate_automation_execution(
                &AutomationExecutionLease {
                    occurrence_id: first_fenced.id.clone(),
                    definition_revision: revision,
                    worker_id: "worker".into(),
                    lease_token: "lease".into(),
                },
                1_002
            )
            .is_err());
    }
    let stale = queued_run_for("brief-fence", "stale", "stale-occurrence");
    assert!(store.enqueue_automation_run(&stale).is_err());
    assert!(store.automation_run("stale").unwrap().is_none());
    assert_eq!(
        store.enqueue_automation_run(&first_fenced).unwrap(),
        store.automation_run(&first_fenced.id).unwrap().unwrap()
    );
    let mut hijack = first_fenced.clone();
    hijack.definition_revision = 2;
    assert!(editor.enqueue_automation_run(&hijack).is_err());
    let mut fresh = queued_run_for("brief-fence", "fresh", "fresh-occurrence");
    fresh.definition_revision = 2;
    editor.enqueue_automation_run(&fresh).unwrap();
    assert!(editor
        .claim_automation_run("fresh", "worker", "fresh-lease", 2_010, 3_000)
        .unwrap());
    let mut disabled = saved_fence;
    disabled.enabled = false;
    disabled.updated_at_ms = 2_011;
    let disabled = editor.upsert_automation_definition(&disabled).unwrap();
    let mut blocked = queued_run_for("brief-fence", "disabled", "disabled-occurrence");
    blocked.definition_revision = disabled.revision;
    assert!(store.enqueue_automation_run(&blocked).is_err());
    assert!(store.automation_run("disabled").unwrap().is_none());

    // 3. User-task definition is persisted but forbidden from agent occurrence execution ("brief-user-task").
    store
        .enqueue_automation_run(&queued_run_for("brief-user-task", "previous-agent", "old"))
        .unwrap();
    let mut user_task_def = store.automation_definition("brief-user-task").unwrap().unwrap();
    user_task_def.execution_spec = serde_json::from_value(serde_json::json!({
        "kind": "user_task",
        "task": {
            "task_type": "debt", "description": null, "target_at_ms": null, "recurrence": null,
            "related_person": null, "amount_decimal": "1234567890.123456789", "currency": "CNY",
            "due_at_ms": null, "completed": false, "notification_ids": []
        }
    }))
    .unwrap();
    user_task_def.updated_at_ms = 2;
    let saved_user_task = store.upsert_automation_definition(&user_task_def).unwrap();
    assert!(store
        .enqueue_automation_run(&queued_run_for("brief-user-task", "forbidden", "new"))
        .unwrap_err()
        .to_string()
        .contains("wrong-definition-kind"));
    assert!(store
        .claim_automation_run("previous-agent", "worker", "lease", 1000, 2000)
        .unwrap_err()
        .to_string()
        .contains("wrong-definition-kind"));
    assert!(store.automation_run("forbidden").unwrap().is_none());

    // 4. Seed expired-owner ("brief-expired") and cold-recovery ("brief-cold") runs before reopen.
    store
        .enqueue_automation_run(&queued_run_for("brief-expired", "run-expired", "occurrence"))
        .unwrap();
    assert!(store
        .claim_automation_run("run-expired", "old-worker", "old-lease", 1_000, 1_100)
        .unwrap());

    let cold_first = store
        .enqueue_automation_run(&queued_run_for("brief-cold", "run-1", "2026-08-18T08:00"))
        .unwrap();
    let cold_dup = store
        .enqueue_automation_run(&queued_run_for(
            "brief-cold",
            "run-duplicate",
            "2026-08-18T08:00",
        ))
        .unwrap();
    assert_eq!(cold_first.id, "run-1");
    assert_eq!(cold_dup.id, "run-1");
    assert!(store
        .claim_automation_run("run-1", "worker-a", "lease-a", 1_000, 1_100)
        .unwrap());
    assert!(!store
        .claim_automation_run("run-1", "worker-b", "lease-b", 1_050, 1_200)
        .unwrap());

    drop(editor);
    drop(store);

    // Reopen #1: verify AdmissionPending, user-task spec, expired-owner fencing, and cold recovery!
    let mut reopened = NolaStore::open(&path).unwrap();
    assert_eq!(
        reopened.observe_automation_execution("run-prep").unwrap().phase,
        AutomationExecutionPhase::AdmissionPending
    );
    assert!(!reopened
        .retry_automation_run("run-prep", "lease", 1_500, r#"{"code":"ack_unknown"}"#, 1_003)
        .unwrap());
    let mut wrong_revision = prep_lease.clone();
    wrong_revision.definition_revision = 2;
    assert!(!reopened
        .observe_automation_worker(
            &wrong_revision,
            &AutomationWorkerObservation::LaunchFailed {
                detail: "wrong revision".into()
            },
            1_004
        )
        .unwrap());
    assert!(reopened
        .observe_automation_worker(
            &prep_lease,
            &AutomationWorkerObservation::LaunchFailed {
                detail: "actual startup exception".into()
            },
            1_005
        )
        .unwrap());
    assert!(reopened
        .validate_automation_execution(&prep_lease, 1_006)
        .is_err());
    assert!(!reopened
        .claim_automation_run("run-prep", "replacement", "replacement", 2_001, 3_000)
        .unwrap());
    assert_eq!(
        reopened.automation_run("run-prep").unwrap().unwrap().status,
        "launch_failed"
    );

    assert_eq!(
        reopened
            .automation_definition("brief-user-task")
            .unwrap()
            .unwrap()
            .execution_spec,
        saved_user_task.execution_spec
    );

    let before_expired = reopened.automation_run("run-expired").unwrap().unwrap();
    for now in [1_100, 1_101] {
        assert!(!reopened
            .retry_automation_run(
                "run-expired",
                "old-lease",
                1_500,
                r#"{"code":"transport_failed"}"#,
                now
            )
            .unwrap());
    }
    let unchanged = reopened.automation_run("run-expired").unwrap().unwrap();
    assert_eq!(unchanged.status, before_expired.status);
    assert_eq!(unchanged.updated_at_ms, before_expired.updated_at_ms);
    assert_eq!(unchanged.lease_token, before_expired.lease_token);
    assert!(!reopened
        .claim_automation_run("run-expired", "new-worker", "new-lease", 1_101, 1_400)
        .unwrap());
    assert!(!reopened
        .observe_automation_worker(
            &AutomationExecutionLease {
                occurrence_id: "run-expired".into(),
                definition_revision: 1,
                worker_id: "new-worker".into(),
                lease_token: "new-lease".into(),
            },
            &AutomationWorkerObservation::LaunchFailed {
                detail: "not the owner".into()
            },
            1_200
        )
        .unwrap());

    assert!(!reopened
        .claim_automation_run("run-1", "worker-b", "lease-b", 1_101, 1_300)
        .unwrap());
    let claimed_cold = reopened.automation_run("run-1").unwrap().unwrap();
    assert_eq!(claimed_cold.attempt, 1);
    assert_eq!(claimed_cold.worker_id.as_deref(), Some("worker-a"));
    assert!(reopened
        .observe_automation_worker(
            &AutomationExecutionLease {
                occurrence_id: "run-1".into(),
                definition_revision: 1,
                worker_id: "worker-a".into(),
                lease_token: "lease-a".into(),
            },
            &AutomationWorkerObservation::LaunchFailed {
                detail: "Recovered unadmitted occurrence; original startup exception is unknown"
                    .into()
            },
            1_600
        )
        .unwrap());
    let observed_cold = reopened.observe_automation_execution("run-1").unwrap();
    assert!(observed_cold.binding.is_none());
    assert!(observed_cold.terminal.is_none());
    assert_eq!(observed_cold.phase, AutomationExecutionPhase::LaunchFailed);

    drop(reopened);
    let final_store = NolaStore::open(&path).unwrap();
    let run = final_store.automation_run("run-1").unwrap().unwrap();
    assert_eq!(run.status, "launch_failed");
    assert_eq!(run.attempt, 1);
    assert!(run.result_json.is_none());
    assert!(run.lease_token.is_none());
    assert!(run
        .error_json
        .unwrap()
        .contains("original startup exception is unknown"));
    assert_eq!(
        final_store
            .list_automation_runs(Some("brief-cold"), 10)
            .unwrap()
            .len(),
        1
    );
}
