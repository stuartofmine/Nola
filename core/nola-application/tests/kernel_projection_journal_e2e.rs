// SPDX-License-Identifier: Apache-2.0
// Real Application receipts and SQLite journal consumed by the shared Kernel.

mod support;
use nola_application::{
    ApplicationCommand, ApplicationCommandReceiptOutcome, ApplicationProjectionScope,
    ExecuteReceivedCommand,
};
use nola_kernel::{ConversationProjectionReducer, ProjectionApplyError, ProjectionApplyOutcome};
use nola_store::{CreateConversationInput, NolaStore};

#[test]
fn damaged_journal_rejects_the_whole_page_instead_of_skipping_a_sequence() {
    use nola_domain::{
        ConversationId, ConversationProjectionEvent, ConversationProjectionEventIdentity,
        ConversationProjectionMutation, RunEventId,
    };
    use nola_store::{AppendProjectionEventInput, ProjectionScopeKey};

    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("sequence-gap.db");
    let mut store = NolaStore::open(&database).unwrap();
    for sequence in 1..=3 {
        let event = ConversationProjectionEvent {
            identity: ConversationProjectionEventIdentity {
                event_id: RunEventId(format!("gap-{sequence}")),
                conversation_id: ConversationId("gap".into()),
                run_id: None,
                delegated_invocation_id: None,
                attempt_id: None,
                sequence,
            },
            mutation: ConversationProjectionMutation::SetTitle {
                title: format!("Title {sequence}"),
                revision: sequence,
            },
        };
        store
            .append_projection_event(&AppendProjectionEventInput {
                event_id: event.identity.event_id.0.clone(),
                scope: ProjectionScopeKey {
                    scope_type: "conversation".into(),
                    scope_id: "gap".into(),
                },
                expected_after_sequence: sequence - 1,
                payload_json: serde_json::to_string(&event).unwrap(),
                occurred_at: "2026-09-05T00:00:00Z".into(),
            })
            .unwrap();
    }
    drop(store);
    let connection = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        connection
            .execute("DELETE FROM projection_events WHERE event_id = 'gap-2'", [],)
            .unwrap(),
        1
    );
    drop(connection);
    let runtime = support::open_runtime(&database).unwrap();
    let scope = || ApplicationProjectionScope {
        scope_type: "conversation".into(),
        scope_id: "gap".into(),
    };
    let first = runtime.projection_event_suffix(scope(), 0, 1).unwrap();
    assert_eq!(first.events.len(), 1);
    assert!(first.has_more);
    for after_sequence in [0, 1] {
        let error = runtime
            .projection_event_suffix(scope(), after_sequence, 10)
            .unwrap_err();
        assert!(error.contains("expected 2, received 3"), "{error}");
    }
    assert!(runtime
        .projection_event_suffix(scope(), 4, 10)
        .unwrap_err()
        .contains("ahead of the journal cursor"));
    assert!(runtime
        .projection_event_suffix(scope(), 0, 0)
        .unwrap_err()
        .contains("page limit must be positive"));
    let caught_up = runtime.projection_event_suffix(scope(), 3, 10).unwrap();
    assert!(caught_up.events.is_empty());
    assert!(!caught_up.has_more);

    drop(runtime);
    let connection = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        connection
            .execute("DELETE FROM projection_events WHERE event_id = 'gap-3'", [],)
            .unwrap(),
        1
    );
    drop(connection);
    let runtime = support::open_runtime(&database).unwrap();
    let error = runtime.projection_event_suffix(scope(), 1, 10).unwrap_err();
    assert!(error.contains("suffix is missing after 1"), "{error}");
}

#[test]
fn journal_rejects_foreign_conversation_payload_before_delivery() {
    use nola_domain::{
        ConversationId, ConversationProjectionEvent, ConversationProjectionEventIdentity,
        ConversationProjectionMutation, RunEventId,
    };
    use nola_store::{AppendProjectionEventInput, ProjectionScopeKey};

    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("foreign-event.db");
    let mut store = NolaStore::open(&database).unwrap();
    let event = ConversationProjectionEvent {
        identity: ConversationProjectionEventIdentity {
            event_id: RunEventId("foreign-event".into()),
            conversation_id: ConversationId("other-conversation".into()),
            run_id: None,
            delegated_invocation_id: None,
            attempt_id: None,
            sequence: 1,
        },
        mutation: ConversationProjectionMutation::SetTitle {
            title: "Must not be delivered".into(),
            revision: 1,
        },
    };
    store
        .append_projection_event(&AppendProjectionEventInput {
            event_id: "foreign-event".into(),
            scope: ProjectionScopeKey {
                scope_type: "conversation".into(),
                scope_id: "requested".into(),
            },
            expected_after_sequence: 0,
            payload_json: serde_json::to_string(&event).unwrap(),
            occurred_at: "2026-09-05T00:00:00Z".into(),
        })
        .unwrap();
    drop(store);
    let runtime = support::open_runtime(&database).unwrap();
    let error = runtime
        .projection_event_suffix(
            ApplicationProjectionScope {
                scope_type: "conversation".into(),
                scope_id: "requested".into(),
            },
            0,
            10,
        )
        .unwrap_err();
    assert!(error.contains("belongs to another conversation"), "{error}");
    for (scope_type, scope_id) in [("conversations", ""), ("conversation", "")] {
        assert!(runtime
            .projection_event_suffix(
                ApplicationProjectionScope {
                    scope_type: scope_type.into(),
                    scope_id: scope_id.into(),
                },
                0,
                10
            )
            .unwrap_err()
            .contains("nonempty conversation scope"));
    }
}

#[test]
fn metadata_invalidation_requires_fresh_snapshot_after_restart_and_redelivery() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("kernel-journal.db");
    let mut store = NolaStore::open(&database).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "journal".into(),
            title: "Initial".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    drop(store);
    let mut runtime = support::open_runtime(&database).unwrap();
    let initial = runtime.conversation_projection_snapshot("journal").unwrap();
    let mut reducer = ConversationProjectionReducer::restore(
        initial.projection.unwrap(),
        initial.current_sequence,
    );

    for (index, title) in ["First", "Second", "Final"].into_iter().enumerate() {
        let command = ApplicationCommand::RenameConversation {
            title_source: None,
            conversation_id: "journal".into(),
            title: title.into(),
            updated_at_ms: index as i64 + 2,
        };
        let receipt = runtime
            .execute_received_command(
                ExecuteReceivedCommand {
                    client_id: "kernel-client".into(),
                    request_id: format!("rename-{index}"),
                    command_type: "rename_conversation".into(),
                    command_json: serde_json::to_string(&command).unwrap(),
                    expected_revision: None,
                    received_at: "2026-09-05T00:00:00Z".into(),
                },
                command,
            )
            .unwrap();
        assert!(matches!(
            receipt.outcome,
            ApplicationCommandReceiptOutcome::Committed { .. }
        ));
        drop(runtime);
        runtime = support::open_runtime(&database).unwrap();
        reducer = ConversationProjectionReducer::resume(reducer.into_checkpoint());
        let authoritative = runtime.conversation_projection_snapshot("journal").unwrap();
        assert!(authoritative.current_sequence > reducer.last_sequence());
        let mut pages = 0;
        while reducer.last_sequence() < authoritative.current_sequence {
            pages += 1;
            assert!(pages <= 32, "journal paging must make bounded progress");
            let suffix = runtime
                .projection_event_suffix(
                    ApplicationProjectionScope {
                        scope_type: "conversation".into(),
                        scope_id: "journal".into(),
                    },
                    reducer.last_sequence(),
                    1,
                )
                .unwrap();
            assert_eq!(suffix.events.len(), 1);
            assert_eq!(suffix.current_sequence, authoritative.current_sequence);
            let events: Vec<_> = suffix.events.into_iter().map(|value| value.event).collect();
            let before = reducer.projection().clone();
            let before_sequence = reducer.last_sequence();
            let error = reducer.apply_batch(events.clone()).unwrap_err();
            assert!(
                matches!(error, ProjectionApplyError::SnapshotRequired { revision }
                if revision == index as u64 + 2)
            );
            assert_eq!(
                reducer.projection(),
                &before,
                "invalidation cannot publish a partial title-only state"
            );
            assert_eq!(
                reducer.last_sequence(),
                before_sequence,
                "invalidation is unacknowledged until snapshot refresh"
            );
            let refresh = runtime.conversation_projection_snapshot("journal").unwrap();
            reducer = ConversationProjectionReducer::restore(
                refresh.projection.unwrap(),
                refresh.current_sequence,
            );
            let committed = reducer.projection().clone();
            let cursor = reducer.last_sequence();
            assert_eq!(
                reducer.apply_batch(events),
                Ok(vec![ProjectionApplyOutcome::Duplicate])
            );
            assert_eq!(reducer.last_sequence(), cursor);
            assert_eq!(reducer.projection(), &committed);
        }
        assert_eq!(
            reducer.projection(),
            authoritative.projection.as_ref().unwrap()
        );
        assert_eq!(reducer.last_sequence(), authoritative.current_sequence);
    }
}
