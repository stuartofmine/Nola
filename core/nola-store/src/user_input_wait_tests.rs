// SPDX-License-Identifier: Apache-2.0
use super::*;
use nola_domain::user_input_wait::*;
use nola_store_port::run_creation::InitialCoreRunEvent;
use serde_json::json;

fn question() -> UserInputWait {
    let question_set = QuestionSet {
        questions: vec![UserQuestion {
            id: "choice".into(),
            prompt: "Select an item".into(),
            options: vec![QuestionOption {
                id: "option-a".into(),
                label: "A".into(),
                description: None,
            }],
            allow_freeform: false,
            required: true,
            max_text_chars: 32,
        }],
    };
    UserInputWait {
        wait_id: "wait".into(),
        core_session_id: "core".into(),
        turn_id: "run".into(),
        originating_call_id: Some("tool-call".into()),
        revision: 1,
        created_at_revision: 10,
        response_schema_digest: question_set.digest().unwrap(),
        question_set,
        state: UserInputWaitState::Open,
    }
}
fn fixture(path: &std::path::Path) -> NolaStore {
    let mut store = NolaStore::open_archive_mode(path, false).unwrap();
    let initial = InitialCoreRunEvent {
        core_session_id: "core".into(),
        core_turn_id: Some("run".into()),
        core_cursor: 1,
        core_revision: 10,
        event_kind: "user_input_wait_changed".into(),
        payload_json: json!({"type":"user_input_wait_changed","wait":question()}).to_string(),
    };
    store
        .begin_turn(&crate::input_facts::tests::input(vec![initial]))
        .unwrap();
    store
}
fn answer(command: &str) -> RespondToUserInputWait {
    RespondToUserInputWait {
        command_id: command.into(),
        wait_id: "wait".into(),
        expected_revision: 1,
        response_schema_digest: question().response_schema_digest,
        response: UserInputWaitResponse::Answers {
            answers: vec![QuestionAnswer {
                question_id: "choice".into(),
                value: QuestionAnswerValue::Option {
                    option_id: "option-a".into(),
                },
            }],
        },
    }
}
fn resolved(answer: &RespondToUserInputWait) -> UserInputWait {
    let mut wait = question();
    wait.revision = 2;
    wait.state = UserInputWaitState::Resolved {
        command_id: answer.command_id.clone(),
        response_digest: answer.digest().unwrap(),
    };
    wait
}
fn stage(
    store: &NolaStore,
    event: &str,
    wait: UserInputWait,
    response: Option<&RespondToUserInputWait>,
) -> StoreResult<()> {
    let payload = json!({"type":"user_input_wait_changed","wait":wait,
        "core_envelope":{"session_id":"core","turn_id":"run","revision":10,"cursor":{"sequence":2}}});
    store.connection.execute("INSERT INTO run_events(event_id,run_id,sequence,event_kind,payload_json,created_at_ms) VALUES(?1,'run',(SELECT COALESCE(MAX(sequence),0)+1 FROM run_events WHERE run_id='run'),'user_input_wait_changed',?2,2)",params![event,payload.to_string()])?;
    index_event(&store.connection, event, response)
}

#[test]
fn committed_question_reopens_without_host_and_is_exactly_scoped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wait.sqlite");
    drop(fixture(&path));
    let store = NolaStore::open_archive_mode(&path, false).unwrap();
    let waits = store
        .open_user_input_waits("conversation", Some("run"))
        .unwrap();
    assert_eq!(waits.len(), 1);
    assert_eq!(waits[0].wait, question());
    assert!(store
        .open_user_input_waits("other", None)
        .unwrap()
        .is_empty());
    assert!(store
        .load_user_input_wait("other", "wait")
        .unwrap()
        .is_none());
    assert!(store
        .canonical_run_terminal("conversation", "run")
        .unwrap()
        .is_none());
}

#[test]
fn response_requires_exact_original_schema_payload_and_open_cas() {
    for case in [
        "missing_submission",
        "stale_revision",
        "unknown_option",
        "schema",
        "modified_question",
    ] {
        let mut store = fixture(std::path::Path::new(":memory:"));
        let mut response = answer("command");
        if case == "stale_revision" {
            response.expected_revision = 2;
        }
        if case == "schema" {
            response.response_schema_digest = "b".repeat(64);
        }
        if case == "unknown_option" {
            response.response = UserInputWaitResponse::Answers {
                answers: vec![QuestionAnswer {
                    question_id: "choice".into(),
                    value: QuestionAnswerValue::Option {
                        option_id: "unknown".into(),
                    },
                }],
            };
        }
        let mut wait = resolved(&response);
        if case == "modified_question" {
            wait.question_set.questions[0].prompt = "replacement".into();
            wait.response_schema_digest = wait.question_set.digest().unwrap();
        }
        store.begin_write_unit_of_work().unwrap();
        assert!(
            stage(
                &store,
                "answer",
                wait,
                if case == "missing_submission" {
                    None
                } else {
                    Some(&response)
                }
            )
            .is_err(),
            "{case}"
        );
        store.rollback_write_unit_of_work().unwrap();
        assert_eq!(
            store
                .load_user_input_wait("conversation", "wait")
                .unwrap()
                .unwrap()
                .wait,
            question()
        );
        assert_eq!(
            store
                .connection
                .query_row("SELECT COUNT(*) FROM user_input_wait_responses", [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(
            store
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM run_events WHERE event_id='answer'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
}

#[test]
fn answer_receipt_is_atomic_and_duplicate_event_is_idempotent() {
    let mut store = fixture(std::path::Path::new(":memory:"));
    let response = answer("command");
    store.connection.execute_batch("CREATE TRIGGER reject_wait_receipt BEFORE INSERT ON user_input_wait_responses BEGIN SELECT RAISE(ABORT,'receipt fault'); END;").unwrap();
    store.begin_write_unit_of_work().unwrap();
    assert!(stage(&store, "answer", resolved(&response), Some(&response)).is_err());
    store.rollback_write_unit_of_work().unwrap();
    store
        .connection
        .execute_batch("DROP TRIGGER reject_wait_receipt")
        .unwrap();
    store.begin_write_unit_of_work().unwrap();
    stage(&store, "answer", resolved(&response), Some(&response)).unwrap();
    index_event(&store.connection, "answer", Some(&response)).unwrap();
    store.commit_write_unit_of_work().unwrap();
    assert!(store
        .open_user_input_waits("conversation", None)
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM user_input_wait_responses", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
        1
    );
}

#[test]
fn cancellation_withdraws_wait_and_late_answer_cannot_reopen_it() {
    let mut store = fixture(std::path::Path::new(":memory:"));
    store.begin_write_unit_of_work().unwrap();
    store
        .connection
        // Fault injection: a lifecycle seal cannot coexist with an open wait.
        .execute("UPDATE runs SET status='sealed' WHERE id='run'", [])
        .unwrap();
    assert!(ensure_not_sealed_with_open_waits(&store.connection, "run").is_err());
    let mut withdrawn = question();
    withdrawn.revision = 2;
    withdrawn.state = UserInputWaitState::Withdrawn {
        reason: "cancelled".into(),
    };
    stage(&store, "withdraw", withdrawn, None).unwrap();
    ensure_not_sealed_with_open_waits(&store.connection, "run").unwrap();
    store.commit_write_unit_of_work().unwrap();
    store.begin_write_unit_of_work().unwrap();
    let response = answer("late");
    // A new source event must not exploit an already-withdrawn head.
    assert!(stage(&store, "late", resolved(&response), Some(&response)).is_err());
    store.rollback_write_unit_of_work().unwrap();
    assert!(matches!(
        store
            .load_user_input_wait("conversation", "wait")
            .unwrap()
            .unwrap()
            .wait
            .state,
        UserInputWaitState::Withdrawn { .. }
    ));
}

#[test]
fn two_connections_cannot_commit_two_answers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("race.sqlite");
    drop(fixture(&path));
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers = (0..2)
        .map(|index| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
                barrier.wait();
                store.begin_write_unit_of_work().unwrap();
                let response = answer(&format!("answer-{index}"));
                let result = stage(
                    &store,
                    &format!("event-{index}"),
                    resolved(&response),
                    Some(&response),
                );
                if result.is_ok() {
                    store.commit_write_unit_of_work().unwrap();
                    true
                } else {
                    store.rollback_write_unit_of_work().unwrap();
                    false
                }
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        workers
            .into_iter()
            .map(|w| w.join().unwrap())
            .filter(|ok| *ok)
            .count(),
        1
    );
    let store = NolaStore::open_archive_mode(&path, false).unwrap();
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM user_input_wait_responses", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
        1
    );
}

