// SPDX-License-Identifier: Apache-2.0
use super::*;
use nola_domain::generation::ModelProtocol;

fn record() -> GatewayAttemptRecord {
    GatewayAttemptRecord {
        id: uuid::Uuid::new_v4().to_string(),
        profile_id: "profile".into(),
        profile_revision: 1,
        connection_id: "connection".into(),
        account_id: Some("account".into()),
        client_access_revision: 2,
        protocol: ModelProtocol::Responses,
        started_at_ms: 1,
        finished_at_ms: None,
        outcome: GatewayAttemptOutcome::InFlight,
        http_status: None,
        usage: None,
    }
}

#[test]
fn streamed_usage_snapshots_are_not_summed_and_partial_usage_stays_unknown() {
    let root = tempfile::tempdir().unwrap();
    let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
    let mut tracker = GatewayUsageTracker::start(&store, record()).unwrap();
    tracker.response(200, true);
    let event = b"data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":17,\"cache_read_input_tokens\":5}}}\n\n";
    for bytes in event.chunks(3) {
        tracker.observe(bytes);
    }
    tracker.observe(event);
    tracker.observe(b"data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":4}}\n\ndata: {\"type\":\"message_stop\"}\n\n");
    tracker.eof().unwrap();
    let attempts = list_gateway_attempts(&store, 20, 0).unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].record.outcome, GatewayAttemptOutcome::Completed);
    let usage = attempts[0].record.usage.as_ref().unwrap();
    assert_eq!(usage.input_tokens, Some(17));
    assert_eq!(usage.output_tokens, Some(4));
    assert_eq!(usage.cached_input_tokens, Some(5));
    assert_eq!(usage.reasoning_output_tokens, None);
    assert_eq!(attempts[0].record.client_access_revision, 2);
    assert!(store
        .application()
        .unwrap()
        .execute::<serde_json::Value>(ApplicationCommand::Model {
            command: ModelCommand::RecordGatewayAttempt {
                input: attempts[0].record.clone(),
                expected_revision: Some(attempts[0].revision)
            }
        })
        .is_err());
}

#[test]
fn eof_without_terminal_and_downstream_drop_are_interrupted_not_success_or_zero() {
    let root = tempfile::tempdir().unwrap();
    let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
    {
        let mut tracker = GatewayUsageTracker::start(&store, record()).unwrap();
        tracker.response(200, true);
        tracker.observe(b"data: {\"delta\":\"private-output-never-stored\"}\n\n");
        tracker.eof().unwrap();
    }
    {
        let mut tracker = GatewayUsageTracker::start(&store, record()).unwrap();
        tracker.response(200, true);
        tracker.observe(b"data: {\"usage\":{\"input_tokens\":3}}\n\n");
    }
    drop(store);
    let reopened = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
    let attempts = list_gateway_attempts(&reopened, 20, 0).unwrap();
    assert_eq!(attempts.len(), 2);
    assert!(attempts
        .iter()
        .all(|attempt| attempt.record.outcome == GatewayAttemptOutcome::Interrupted));
    assert!(attempts
        .iter()
        .any(|attempt| attempt.record.usage.is_none()));
    assert!(attempts.iter().any(|attempt| attempt
        .record
        .usage
        .as_ref()
        .is_some_and(|usage| usage.input_tokens == Some(3) && usage.output_tokens.is_none())));
    assert!(!serde_json::to_string(&attempts)
        .unwrap()
        .contains("private-output"));
}

#[test]
fn json_unknown_usage_and_provider_error_have_distinct_outcomes() {
    let root = tempfile::tempdir().unwrap();
    let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
    for (status, body, outcome) in [
        (
            200,
            b"{\"output\":[],\"error\":null}".as_slice(),
            GatewayAttemptOutcome::Completed,
        ),
        (
            429,
            b"{\"error\":{\"message\":\"quota\"}}".as_slice(),
            GatewayAttemptOutcome::UpstreamRejected,
        ),
    ] {
        let mut tracker = GatewayUsageTracker::start(&store, record()).unwrap();
        tracker.response(status, false);
        tracker.observe(body);
        tracker.eof().unwrap();
        assert_eq!(tracker.snapshot.record.outcome, outcome);
        assert!(tracker.snapshot.record.usage.is_none());
    }
}
