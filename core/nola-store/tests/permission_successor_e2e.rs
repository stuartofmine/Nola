// SPDX-License-Identifier: Apache-2.0
//! Persistent permission grant succession; no model or live user database.
use nola_store::{NolaStore, RuntimeTaskLeaseWitness};
use nola_store_port::runtime_task::*;
use serde_json::json;

struct Fixture {
    _root: tempfile::TempDir,
    path: std::path::PathBuf,
    store: NolaStore,
    original: RuntimeTaskAttempt,
    first: RuntimeTaskAttempt,
    suspension: RuntimeTaskPermissionSuspension,
}

fn fixture() -> Fixture {
    // Scope: generic durable permission succession and exact lease fencing.
    // The SQL Run below is an identity row, not a Core agent promotion proof.
    // Real agent admission/suspension is exercised through Application tests;
    // keep this fixture a process task rather than forging an agent seal.
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("successor.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    store.create_conversation(&nola_store::CreateConversationInput {
        id: "child".into(), title: "Child".into(), revision: 1, settings_json: None,
        created_at_ms: 100, updated_at_ms: 100,
    }).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms)
        VALUES('run','child','conversation','running',1,100,100)", []).unwrap();
    store.create_runtime_task_with_event(&serde_json::from_value(json!({
        "id":"task", "kind":"shell_process", "status":"queued", "title":"Permission successor",
        "project_root":"/fixture", "owner_session_id":"parent", "child_session_id":"child",
        "unread":false,"cancel_requested":false,"retry_count":0,"input_watermark":7,
        "created_at_ms":100,"updated_at_ms":100,"metadata_json":"{}"
    })).unwrap()).unwrap();
    let original = store.claim_runtime_task("task", "worker-0", "generation-0", "token-0", 200, 1000).unwrap().unwrap();
    let suspension = store.suspend_runtime_task_for_permission(&SuspendRuntimeTaskForPermission {
        task_id: "task".into(), attempt_id: original.id.clone(), expected_attempt_revision: original.revision,
        worker_generation: original.worker_generation.clone(), lease_token: original.lease_token.clone(),
        current_expiry_ms: original.lease_expires_at_ms,
        permission_request: nola_store_port::permission::StoredPermissionRequest {
            closure: None,
            request_id:"permission".into(), task_id:"task".into(), child_session_id:"child".into(),
            capability:"file.write".into(), input_json:"{}".into(), scope_rule:"file.write:/fixture".into(),
            status:"pending".into(), decision:None, custom_response:None, created_at_ms:250,
            resolved_at_ms:None, core_permission_request_id:Some("core-request".into()), permission_instructions_json:"[]".into(),
        },
        continuation_id:"continuation".into(),
        continuation_json:json!({"child_session_id":"child","run_id":"run","core_permission_request_id":"core-request"}).to_string(),
        progress_summary:"Waiting".into(), last_activity:None, now_ms:250,
    }).unwrap().unwrap();
    db.execute("UPDATE permission_requests SET status='resolved',decision='allow_once',resolved_at_ms=300 WHERE request_id='permission'", []).unwrap();
    let first = store.resume_runtime_task_after_permission_fair(&ResumeRuntimeTaskAfterPermissionFair {
        task_id:"task".into(), suspension_id:suspension.id.clone(), expected_suspension_revision:suspension.revision,
        permission_request_id:"permission".into(), continuation_id:"continuation".into(),
        worker_id:"worker-1".into(),worker_generation:"generation-1".into(),lease_token:"token-1".into(),
        now_ms:300,lease_until_ms:500,max_running_global:2,max_running_project:2,
    }).unwrap().unwrap();
    let suspension = store.runtime_task_permission_suspension(&suspension.id).unwrap().unwrap();
    Fixture { _root:root,path,store,original,first,suspension }
}

fn proposal(f: &Fixture) -> RecoverRuntimeTaskPermissionContinuationFair {
    RecoverRuntimeTaskPermissionContinuationFair {
        suspension_id:f.suspension.id.clone(), expected_suspension_revision:f.suspension.revision,
        permission_request_id:"permission".into(), continuation_id:"continuation".into(),
        expected_child_session_id:"child".into(), expected_run_id:"run".into(),
        predecessor:RuntimeTaskLeaseWitness::from(&f.first), grant_attempt_id:"grant-2".into(),
        worker_id:"worker-2".into(),worker_generation:"generation-2".into(),lease_token:"token-2".into(),
        now_ms:600,lease_until_ms:900,max_running_global:2,max_running_project:2,max_recoveries:3,
    }
}

#[test]
fn repeated_permission_successors_keep_exact_grants_across_reopen() {
    let mut f = fixture();
    let input = proposal(&f);
    let second = f.store.recover_runtime_task_permission_continuation_fair(&input).unwrap().unwrap();
    assert!(!f.store.retry_runtime_task(&RuntimeTaskLeaseWitness::from(&f.first),700,"old worker",650).unwrap());
    assert!(f.store.recover_runtime_task_permission_continuation_fair(&input).unwrap().is_none());
    let mut next = input.clone();
    next.expected_suspension_revision += 1;
    next.predecessor = RuntimeTaskLeaseWitness::from(&second);
    next.grant_attempt_id = "grant-3".into(); next.worker_generation = "generation-3".into();
    next.lease_token = "token-3".into(); next.now_ms = 950; next.lease_until_ms = 1300;
    let third = f.store.recover_runtime_task_permission_continuation_fair(&next).unwrap().unwrap();
    drop(f.store);
    let store = NolaStore::open(&f.path).unwrap();
    let proof = store.runtime_task_permission_continuation_grant("task",&f.suspension.id,
        &PermissionContinuationGrantSelector::ExactAttempt { attempt_id:third.id.clone() }).unwrap().unwrap();
    assert_eq!(proof.granted_attempt,third);
    assert_eq!(proof.ancestors.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
        vec![f.original.id.as_str(),f.first.id.as_str(),second.id.as_str()]);
    assert_eq!(store.runtime_task_permission_resume("task",&f.suspension.id,"permission","continuation")
        .unwrap().unwrap().granted_attempt,f.first);
    assert_eq!(store.runtime_task("task").unwrap().unwrap().retry_count,2);
}

#[test]
fn successor_event_failure_and_wrong_run_do_not_consume_predecessor() {
    let mut f = fixture();
    let input = proposal(&f);
    let mut wrong = input.clone(); wrong.expected_run_id = "foreign".into();
    assert!(f.store.recover_runtime_task_permission_continuation_fair(&wrong).unwrap().is_none());
    let db = rusqlite::Connection::open(&f.path).unwrap();
    db.execute_batch("CREATE TRIGGER reject_successor BEFORE INSERT ON durable_task_events
        WHEN NEW.kind='permission_recovered' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(f.store.recover_runtime_task_permission_continuation_fair(&input).is_err());
    assert_eq!(f.store.runtime_task_attempt(&f.first.id).unwrap().unwrap(),f.first);
    assert_eq!(f.store.runtime_task_permission_suspension(&f.suspension.id).unwrap().unwrap(),f.suspension);
    assert!(f.store.runtime_task_attempt(&input.grant_attempt_id).unwrap().is_none());
    db.execute_batch("DROP TRIGGER reject_successor;").unwrap();
    assert!(f.store.recover_runtime_task_permission_continuation_fair(&input).unwrap().is_some());
}

#[test]
fn successor_requires_uncancelled_exact_expired_owner_and_fresh_identity() {
    let mut f = fixture();
    let input = proposal(&f);
    let mut wrong = input.clone(); wrong.predecessor.expected_attempt_revision += 1;
    assert!(f.store.recover_runtime_task_permission_continuation_fair(&wrong).unwrap().is_none());
    wrong = input.clone(); wrong.worker_generation = f.original.worker_generation.clone();
    assert!(f.store.recover_runtime_task_permission_continuation_fair(&wrong).unwrap().is_none());
    wrong = input.clone(); wrong.lease_token = f.first.lease_token.clone();
    assert!(f.store.recover_runtime_task_permission_continuation_fair(&wrong).unwrap().is_none());
    wrong = input.clone(); wrong.now_ms = 499;
    assert!(f.store.recover_runtime_task_permission_continuation_fair(&wrong).is_err());
    assert_eq!(f.store.runtime_task_attempt(&f.first.id).unwrap().unwrap(),f.first);
    assert!(f.store.request_runtime_task_cancel("task",550).unwrap());
    assert!(f.store.recover_runtime_task_permission_continuation_fair(&input).unwrap().is_none());
    assert!(f.store.runtime_task_attempt(&input.grant_attempt_id).unwrap().is_none());
}

#[test]
fn competing_successor_proposals_have_one_winner() {
    let f = fixture();
    let input = proposal(&f);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers = (0..2).map(|i| {
        let path = f.path.clone(); let mut input = input.clone(); let barrier = barrier.clone();
        input.grant_attempt_id = format!("competitor-{i}"); input.worker_generation = format!("competitor-generation-{i}");
        input.lease_token = format!("competitor-token-{i}");
        std::thread::spawn(move || {
            let mut store = NolaStore::open(&path).unwrap(); barrier.wait();
            store.recover_runtime_task_permission_continuation_fair(&input).unwrap()
        })
    }).collect::<Vec<_>>();
    let winners = workers.into_iter().filter_map(|worker| worker.join().unwrap()).collect::<Vec<_>>();
    assert_eq!(winners.len(),1);
    let proof = f.store.runtime_task_permission_continuation_grant("task",&f.suspension.id,
        &PermissionContinuationGrantSelector::RecoveryCandidate).unwrap().unwrap();
    assert_eq!(proof.granted_attempt,winners[0]);
}
