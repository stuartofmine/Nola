// SPDX-License-Identifier: Apache-2.0
//! Opt-in regression against a private recorded failure. Never opens the source
//! with application adapters and never executes tools or calls a model.
mod support;

#[test]
#[ignore = "requires NOLA_RECOVERY_SOURCE_DB and NOLA_RECOVERY_CONVERSATION"]
fn recorded_pending_model_can_be_materialized_and_cancelled_on_an_isolated_copy() {
    use nola_application::{ApplicationCommand, RequestRunCancellation, RunCancellationReceipt};
    let source = std::env::var("NOLA_RECOVERY_SOURCE_DB").unwrap();
    let conversation = std::env::var("NOLA_RECOVERY_CONVERSATION").unwrap();
    let directory = tempfile::tempdir().unwrap();
    let copy = directory.path().join("recorded.db");
    let status = std::process::Command::new("sqlite3")
        .args(["-readonly", &source, &format!(".backup '{}'", copy.display())])
        .status().unwrap();
    assert!(status.success());
    let db = rusqlite::Connection::open(&copy).unwrap();
    let (run, assistant): (String, String) = db.query_row(
        "SELECT id,assistant_message_id FROM runs WHERE conversation_id=?1 AND status='running' ORDER BY created_at_ms DESC LIMIT 1",
        [&conversation], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    drop(db);
    let mut runtime = support::open_runtime(&copy).unwrap();
    runtime.execute(ApplicationCommand::LoadCheckpoint { conversation_id: conversation.clone() }).unwrap();
    let exact: nola_application::ExactRunInvocation = serde_json::from_value(runtime.execute(
        ApplicationCommand::LoadExactRunInvocation { conversation_id: conversation.clone(),
            expected_run_id: run.clone(), assistant_message_id: assistant.clone() }).unwrap()).unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let original: nola_application::CommittedHostRequests = serde_json::from_value(runtime.execute(
        ApplicationCommand::LoadCommittedHostRequests { run_id: run.clone() }).unwrap()).unwrap();
    runtime.execute(ApplicationCommand::RecoverExpiredRunDeliveries {
        conversation_id: conversation.clone(), expected_run_id: run.clone(),
        host_invocation: exact.host_invocation, interrupted_at: now.to_string(), interrupted_at_ms: now,
    }).unwrap();
    // The shared Application command selects only authorized interrupted tails.
    let prepare = || ApplicationCommand::PrepareRunEffectRecovery {
        input: nola_application::PrepareRunEffectRecovery {
            conversation_id: conversation.clone(), run_id: run.clone(),
            invocation_config_digest: exact.invocation_config_digest.clone(), now_ms: now,
        },
    };
    for (owner, digest) in [("another-run".to_owned(), exact.invocation_config_digest.clone()),
        (run.clone(), "wrong-frozen-digest".to_owned())] {
        assert!(runtime.execute(ApplicationCommand::PrepareRunEffectRecovery {
            input: nola_application::PrepareRunEffectRecovery {
                conversation_id: conversation.clone(), run_id: owner,
                invocation_config_digest: digest, now_ms: now,
            },
        }).is_err());
        let unchanged: nola_application::CommittedHostRequests = serde_json::from_value(runtime.execute(
            ApplicationCommand::LoadCommittedHostRequests { run_id: run.clone() }).unwrap()).unwrap();
        assert_eq!(unchanged.requests.iter().map(|r| &r.request_id).collect::<Vec<_>>(),
            original.requests.iter().map(|r| &r.request_id).collect::<Vec<_>>(),
            "rejected recovery must not rebind another owner's requests");
    }
    let rebound = runtime.execute(prepare()).unwrap()["rebound_count"].as_u64().unwrap();
    assert!(rebound > 0, "recorded interrupted stream must be rebound");
    assert_eq!(runtime.execute(prepare()).unwrap()["rebound_count"].as_u64(), Some(0), "replay must not allocate another request");
    let claimed: nola_application::CommittedHostRequests = serde_json::from_value(runtime.execute(
        ApplicationCommand::ClaimCommittedHostRequests { input: nola_application::ClaimCommittedHostRequests {
            run_id: run.clone(), worker_generation: "isolated-generation".into(), lease_owner: "isolated-owner".into(),
            claimed_at_ms: now + 1, lease_expires_at_ms: now + 60_000, limit: 1,
        } }).unwrap()).unwrap();
    assert_eq!(claimed.requests.len(), 1, "the original pending request must be reclaimable");
    let resumed = &claimed.requests[0];
    let frozen = original.requests.iter().find(|r| r.effect_id == resumed.effect_id).unwrap();
    assert_eq!(serde_json::to_value(&resumed.payload).unwrap(), serde_json::to_value(&frozen.payload).unwrap(),
        "recovery must preserve the exact original model input");
    assert_eq!(resumed.turn_id.as_deref(), Some(run.as_str()));
    // Reclaim only: no Host executor runs and no model request is sent.
    let make_command = || ApplicationCommand::RequestRunCancellation { input: RequestRunCancellation {
        conversation_id: conversation.clone(), expected_run_id: run.clone(), assistant_message_id: assistant.clone(),
        cancellation_id: "isolated-recorded-cancel".into(), reason: None,
        requested_at_ms: 1 + std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64,
    } };
    let first: RunCancellationReceipt = serde_json::from_value(runtime.execute(make_command()).unwrap()).unwrap();
    assert_eq!(first.run_id, run);
    assert_eq!(first.assistant_message_id, assistant);
    assert_eq!(first.disposition, nola_application::RunCancellationDisposition::CancellationCommitted);
    assert!(first.terminal_result.is_none(), "intent is not cancellation proof");
    drop(runtime);
    let mut runtime = support::open_runtime(&copy).unwrap();
    let second: RunCancellationReceipt = serde_json::from_value(runtime.execute(make_command()).unwrap()).unwrap();
    assert_eq!(second.disposition, nola_application::RunCancellationDisposition::AlreadyCancelling);
    assert!(second.terminal_result.is_none());
}
