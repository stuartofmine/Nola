// SPDX-License-Identifier: Apache-2.0
use super::*;
use crate::assistant_journal_planner::AssistantJournalPlanner;
use nola_domain::assistant_journal_operations::{
    JournalContentField, JournalNumber, JournalOrderKey, JournalPartMetadata, JournalSection,
    JournalTextValue,
};
use nola_agent_engine::agent::parts::{
    AgentPart, PartOwnership, PartUpdate, StepPart, StepStatus, TextPart, TextReviewState,
    ToolPart, ToolPartState,
};
use nola_store::{BeginTurnInput, CommitTransitionInput, NolaStore};

fn transition(store: &mut NolaStore, revision: i64) -> AssistantSourceCommitStamp {
    let transition_id = format!("transition-{revision}");
    assert!(
        store
            .commit_transition(&CommitTransitionInput {
                assistant_turn_bindings: vec![],
                transition_id: transition_id.clone(),
                run_id: "run".into(),
                expected_checkpoint_revision: revision - 1,
                checkpoint_revision: revision,
                checkpoint_event_cursor: 0,
                checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline {
                    checkpoint_json: "{}".into()
                },
                submission_reduction_json: None, host_event: None,
                events: vec![],
                ensured_messages: vec![],
                part_message_owners: Default::default(),
                part_mutations: vec![],
                outbox_effects: vec![],
                model_attempt_mutations: vec![],
                delegated_invocation_mutations: vec![],
                delegated_permission_mutations: vec![],
                effect_delivery_attempt_mutations: vec![],
                acknowledged_effect_ids: vec![],
                memory_usage_entries: vec![],
                run_status: None,
                assistant_status: None,
                assistant_status_message_id: None,
                committed_at_ms: revision,
            })
            .unwrap()
            .applied
    );
    AssistantSourceCommitStamp::Transition { transition_id }
}

fn begin(store: &mut NolaStore) -> AssistantJournalIdentity {
    store
        .begin_turn(&BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            initial_events: Vec::new(),
            initial_assistant_part_mutations: vec![],
            assistant_turn_bindings: vec![
                nola_domain::AssistantTurnBinding::Admission {
                    turn_id: "run".into(),
                    assistant_message_id: "assistant".into(),
                },
                nola_domain::AssistantTurnBinding::Part {
                    turn_id: "run".into(),
                    assistant_message_id: "assistant".into(),
                    part_id: "text".into(),
                },
            ],
            conversation_id: "conversation".into(),
            conversation_title: "Journal".into(),
            user_message_id: "user".into(),
            user_part_id: "user-text".into(),
            user_text: "hello".into(),
            user_content_json: None,
            additional_user_parts: vec![],
            assistant_message_id: "assistant".into(),
            assistant_part_id: "text".into(),
            run_id: "run".into(),
            context_checkpoint_id: None,
            checkpoint_revision: 1,
            checkpoint_event_cursor: 0,
            checkpoint_json: "{}".into(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: "test".into(),
            outbox_effects: vec![],
            model_attempt_mutations: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            memory_usage_entries: vec![],
            created_at_ms: 1,
        })
        .unwrap();
    AssistantJournalIdentity {
        source_store_epoch: store.assistant_source_epoch().unwrap(),
        conversation_id: "conversation".into(),
        run_id: "run".into(),
        assistant_message_id: "assistant".into(),
    }
}
fn stamp() -> AssistantSourceCommitStamp {
    AssistantSourceCommitStamp::Admission {
        run_id: "run".into(),
    }
}
fn order() -> JournalOrderKey {
    JournalOrderKey {
        section: JournalSection::Transcript,
        position: JournalNumber(1),
    }
}
fn replacement(
    store: &NolaStore,
    identity: &AssistantJournalIdentity,
    value: &str,
) -> VisibleJournalPlan {
    let reader = StoreJournalPlanningReader(store);
    let mut planner = AssistantJournalPlanner::new(&reader, identity).unwrap();
    planner.begin_text("text", order()).unwrap();
    let part = AgentPart::Text(TextPart {
        id: "text".into(),
        ownership: PartOwnership {
            message_id: "assistant".into(),
            turn_id: "run".into(),
        },
        text: value.into(),
        review: None,
        is_complete: true,
    });
    planner
        .part_update(
            &PartUpdate::TextReplaced {
                part_id: "text".into(),
                text: value.into(),
                is_complete: true,
            },
            &part,
            order(),
        )
        .unwrap();
    planner.finish().unwrap()
}

#[test]
fn transport_budget_resume_uses_presentation_order_after_cold_reopen() {
    use crate::assistant_journal_read::{AssistantJournalPartPage, AssistantJournalSnapshotHandle};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ordered-wire.db");
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_write_unit_of_work().unwrap();
    let identity = begin(&mut store);
    let expected = store.assistant_journal_head(&identity).unwrap();
    // Deliberately descending IDs: advancing the cursor cannot mean ID > last.
    let base = expected.clone();
    let ids: Vec<String> = ["z", "m", "a"]
        .iter()
        .map(|prefix| prefix.repeat(200))
        .collect();
    let reader = StoreJournalPlanningReader(&store);
    let mut planner = AssistantJournalPlanner::new(&reader, &identity).unwrap();
    for (position, id) in ids.iter().enumerate() {
        planner
            .begin_text(
                id,
                JournalOrderKey {
                    section: JournalSection::Transcript,
                    position: JournalNumber(position as u64),
                },
            )
            .unwrap();
    }
    let plan = planner.finish().unwrap();
    publish_assistant_publication_set(&mut store, stamp(), vec![(expected, plan)]).unwrap();
    let snapshot = store.open_assistant_journal_snapshot(&identity).unwrap();
    let all = store
        .read_assistant_snapshot_parts(&snapshot, None, 128, 262144)
        .unwrap();
    assert_eq!(
        all.parts
            .iter()
            .map(|part| &part.part_id)
            .collect::<Vec<_>>(),
        ids.iter().collect::<Vec<_>>()
    );
    let budget = serde_json::to_vec(&AssistantJournalPartPage {
        at: snapshot.at.clone(),
        parts: vec![all.parts[0].clone()],
        next_part_id: Some(ids[0].clone()),
    })
    .unwrap()
    .len()
    .max(1024);
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    let handle = AssistantJournalSnapshotHandle {
        snapshot_id: snapshot.snapshot_id,
        at: snapshot.at,
    };
    let mut cursor = None;
    let mut seen = Vec::new();
    for index in 0..3 {
        let mut runtime = crate::ApplicationRuntime::open(&path).unwrap();
        let wire = runtime
            .execute(crate::ApplicationCommand::ReadAssistantJournalParts {
                snapshot: handle.clone(),
                after_part_id: cursor.clone(),
                max_records: 128,
                max_encoded_bytes: budget,
            })
            .unwrap();
        assert!(serde_json::to_vec(&wire).unwrap().len() <= budget);
        let page: AssistantJournalPartPage = serde_json::from_value(wire).unwrap();
        assert_eq!(page.at, handle.at);
        assert_eq!(
            page.parts.len(),
            1,
            "actual wire budget must trim complete heads"
        );
        seen.push(page.parts[0].part_id.clone());
        cursor = page.next_part_id;
        assert_eq!(cursor.is_none(), index == 2);
    }
    assert_eq!(seen, ids);
    // Changed-Part pagination deliberately uses lexicographic Part IDs, not
    // display order; the consumer merges then sorts canonical order keys.
    let mut changed_ids = ids.clone();
    changed_ids.sort();
    let first = all
        .parts
        .iter()
        .find(|head| head.part_id == changed_ids[0])
        .unwrap()
        .clone();
    let change_budget = serde_json::to_vec(&crate::AssistantJournalPartChangePage {
        base: base.clone(),
        at: handle.at.clone(),
        changes: vec![crate::AssistantJournalPartChange {
            part_id: first.part_id.clone(),
            head: Some(first),
        }],
        next_part_id: Some(changed_ids[0].clone()),
    })
    .unwrap()
    .len()
    .max(1024);
    let mut after = None;
    let mut changed = Vec::new();
    for index in 0..3 {
        let mut runtime = crate::ApplicationRuntime::open(&path).unwrap();
        let wire = runtime
            .execute(crate::ApplicationCommand::ReadAssistantJournalPartChanges {
                snapshot: handle.clone(),
                base: base.clone(),
                after_part_id: after.clone(),
                max_records: 128,
                max_encoded_bytes: change_budget,
            })
            .unwrap();
        assert!(serde_json::to_vec(&wire).unwrap().len() <= change_budget);
        let page: crate::AssistantJournalPartChangePage = serde_json::from_value(wire).unwrap();
        assert_eq!(page.base, base);
        assert_eq!(page.at, handle.at);
        assert_eq!(page.changes.len(), 1);
        assert!(page.changes[0].head.is_some());
        changed.push(page.changes[0].part_id.clone());
        after = page.next_part_id;
        assert_eq!(after.is_none(), index == 2);
    }
    assert_eq!(changed, changed_ids);
}

#[test]
fn real_store_publication_streams_large_preparations_into_fixed_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("journal.db");
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_write_unit_of_work().unwrap();
    let identity = begin(&mut store);
    let cursor = store.assistant_journal_head(&identity).unwrap();
    let text = "你好🙂\n".repeat(40_000); // More than a 256 KiB preparation frame.
    let plan = replacement(&store, &identity, &text);
    assert!(matches!(
        publish_assistant_publication_set(&mut store, stamp(), vec![(cursor, plan)]).unwrap(),
        Some(AssistantPublicationCommit::Applied(cursors)) if cursors.len() == 1
    ));
    let snapshot = store.open_assistant_journal_snapshot(&identity).unwrap();
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    let head = store
        .read_assistant_snapshot_parts(&snapshot, None, 10, 65536)
        .unwrap()
        .parts
        .remove(0);
    assert_eq!(head.generation.0, 2);
    let content = &head.bodies[0];
    let mut actual = String::new();
    for index in 0..content.chunk_count.0 {
        actual.push_str(
            store
                .read_assistant_snapshot_chunk(&snapshot, &head.part_id, content, index)
                .unwrap()
                .text(),
        );
    }
    assert_eq!(actual, text);
    let reader = StoreJournalPlanningReader(&store);
    let mut planner = AssistantJournalPlanner::new(&reader, &identity).unwrap();
    planner.close_public_part("text", "completed").unwrap();
    assert!(matches!(
        planner.finish().unwrap(),
        VisibleJournalPlan::Unchanged
    ));
}

#[test]
fn canonical_owner_large_delta_publishes_all_bounded_frames_and_survives_cold_open() {
    use nola_store_port::assistant_journal::AssistantJournalPage;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large-delta.db");
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_write_unit_of_work().unwrap();
    let identity = begin(&mut store);
    let cursor = store.assistant_journal_head(&identity).unwrap();
    let text = "界🙂\n".repeat(80_000);
    let part = AgentPart::Text(TextPart {
        id: "text".into(),
        ownership: PartOwnership::for_turn("run"),
        text: text.clone(),
        review: None,
        is_complete: true,
    });
    let canonical = crate::canonical_part_projection::CanonicalPartProjection::new(
        &part,
        "run",
        Some("assistant"),
    )
    .unwrap();
    let plan = {
        let reader = StoreJournalPlanningReader(&store);
        let mut planner = AssistantJournalPlanner::new(&reader, &identity).unwrap();
        planner.begin_text("text", order()).unwrap();
        planner
            .part_update_view(
                &PartUpdate::TextDelta {
                    part_id: "text".into(),
                    delta: text.clone(),
                },
                &canonical,
                order(),
            )
            .unwrap();
        planner.finish().unwrap()
    };
    let VisibleJournalPlan::Changed(ref frames) = plan else {
        panic!("changed frames")
    };
    let frame_count = frames.len();
    assert!(frame_count > 1);
    for frame in frames {
        assert!(frame.batch.operations.len() <= 128);
        assert!(frame.batch.canonical_bytes().unwrap().len() <= 128 * 1024);
        assert!(frame.supplied_chunks.len() <= 128);
        assert!(
            frame
                .supplied_chunks
                .iter()
                .map(|chunk| chunk.byte_length())
                .sum::<usize>()
                <= 256 * 1024
        );
        assert!(frame.preparations.is_empty());
    }
    let Some(AssistantPublicationCommit::Applied(final_cursors)) =
        publish_assistant_publication_set(&mut store, stamp(), vec![(cursor.clone(), plan)])
            .unwrap()
    else {
        panic!("applied complete set")
    };
    assert_eq!(final_cursors.len(), 1);
    assert_eq!(final_cursors[0].sequence, frame_count as u64);
    let snapshot = store.open_assistant_journal_snapshot(&identity).unwrap();
    store.commit_write_unit_of_work().unwrap();
    drop(store);

    let store = NolaStore::open(&path).unwrap();
    let head = store
        .assistant_journal_part(&identity, "text")
        .unwrap()
        .unwrap();
    assert_eq!(head.generation.0, 1);
    assert_eq!(head.bodies[0].utf8_bytes.0, text.len() as u64);
    assert!(matches!(&head.metadata, JournalPartMetadata::Text(value) if value.is_final));
    let mut actual = String::new();
    for index in 0..head.bodies[0].chunk_count.0 {
        actual.push_str(
            store
                .read_assistant_snapshot_chunk(&snapshot, "text", &head.bodies[0], index)
                .unwrap()
                .text(),
        );
    }
    assert_eq!(actual, text);
    let mut window = store.open_assistant_journal_read(&cursor).unwrap();
    let mut record_count = 0;
    loop {
        let AssistantJournalPage::Records {
            records,
            next,
            complete,
        } = store
            .read_assistant_journal_page(&window, 1, 256 * 1024)
            .unwrap()
        else {
            panic!("retained bounded journal records")
        };
        assert_eq!(records.len(), 1);
        record_count += records.len();
        assert_eq!(next.sequence, record_count as u64);
        if complete {
            break;
        }
        window.after = next;
    }
    assert_eq!(record_count, frame_count);
    assert_eq!(
        store.assistant_journal_head(&identity).unwrap(),
        final_cursors[0]
    );
    let reader = StoreJournalPlanningReader(&store);
    let mut planner = AssistantJournalPlanner::new(&reader, &identity).unwrap();
    planner.close_public_part("text", "completed").unwrap();
    assert!(matches!(
        planner.finish().unwrap(),
        VisibleJournalPlan::Unchanged
    ));
}

#[test]
fn canonical_public_planner_rejects_foreign_run_context_and_message_before_read() {
    struct NoRead;
    impl JournalPlanningReader for NoRead {
        fn load_head(
            &self,
            _: &AssistantJournalIdentity,
            _: &str,
        ) -> Result<AssistantJournalPartState, PlannerError> {
            panic!("foreign owner must be rejected before Store head lookup")
        }
        fn load_manifest_page(
            &self,
            _: &AssistantJournalIdentity,
            _: &str,
            _: &JournalContentRef,
            _: u64,
            _: usize,
        ) -> Result<AssistantManifestPage, PlannerError> {
            panic!("foreign owner must be rejected before Store manifest lookup")
        }
    }
    let identity = AssistantJournalIdentity {
        source_store_epoch: "epoch".into(),
        conversation_id: "conversation".into(),
        run_id: "run".into(),
        assistant_message_id: "assistant".into(),
    };
    for (run, message) in [("foreign-run", "assistant"), ("run", "foreign-assistant")] {
        let part = AgentPart::Text(TextPart {
            id: "text".into(),
            ownership: PartOwnership {
                message_id: message.into(),
                turn_id: "child-turn".into(),
            },
            text: "hello".into(),
            review: None,
            is_complete: false,
        });
        let view = crate::canonical_part_projection::CanonicalPartProjection::new(
            &part,
            run,
            Some("assistant"),
        )
        .unwrap();
        let mut planner = AssistantJournalPlanner::new(&NoRead, &identity).unwrap();
        assert_eq!(
            planner.part_update_view(&PartUpdate::Created { part: part.clone() }, &view, order()),
            Err(PlannerError::OwnerMismatch)
        );
    }
}

#[test]
fn step_publication_keeps_large_title_and_old_snapshot_across_control_completion() {
    let directory = tempfile::tempdir().unwrap();
    for (status, expected) in [
        (StepStatus::Completed, nola_domain::ActivityState::Completed),
        (StepStatus::Failed, nola_domain::ActivityState::Failed),
        (StepStatus::Cancelled, nola_domain::ActivityState::Cancelled),
    ] {
        let path = directory.path().join(format!("step-{status:?}.db"));
        let mut store = NolaStore::open(&path).unwrap();
        store.begin_write_unit_of_work().unwrap();
        let identity = begin(&mut store);
        let title = "真实步骤长标题🙂\n".repeat(40_000);
        let mut step = StepPart {
            id: "step".into(),
            ownership: PartOwnership {
                message_id: "assistant".into(),
                turn_id: "run".into(),
            },
            title: title.clone(),
            status: StepStatus::Started,
        };
        let part = AgentPart::Step(step.clone());
        let cursor = store.assistant_journal_head(&identity).unwrap();
        let plan = {
            let reader = StoreJournalPlanningReader(&store);
            let mut planner = AssistantJournalPlanner::new(&reader, &identity).unwrap();
            planner
                .part_update(&PartUpdate::Created { part: part.clone() }, &part, order())
                .unwrap();
            planner.finish().unwrap()
        };
        publish_assistant_publication_set(&mut store, stamp(), vec![(cursor, plan)]).unwrap();
        let old_snapshot = store.open_assistant_journal_snapshot(&identity).unwrap();
        store.commit_write_unit_of_work().unwrap();
        let old = store
            .read_assistant_snapshot_parts(&old_snapshot, None, 10, 65536)
            .unwrap()
            .parts
            .remove(0);
        let JournalPartMetadata::Step(metadata) = &old.metadata else {
            panic!("Step metadata")
        };
        assert_eq!(metadata.state, nola_domain::ActivityState::Running);
        assert_eq!(
            metadata.title,
            JournalTextValue::Field(JournalContentField::Title)
        );
        assert_eq!(old.bodies[0].utf8_bytes.0, title.len() as u64);

        store.begin_write_unit_of_work().unwrap();
        let commit_stamp = transition(&mut store, 2);
        let cursor = store.assistant_journal_head(&identity).unwrap();
        step.status = status;
        let plan = {
            let reader = StoreJournalPlanningReader(&store);
            let mut planner = AssistantJournalPlanner::new(&reader, &identity).unwrap();
            planner
                .part_update(
                    &PartUpdate::Completed {
                        part_id: step.id.clone(),
                    },
                    &AgentPart::Step(step),
                    order(),
                )
                .unwrap();
            planner.finish().unwrap()
        };
        let VisibleJournalPlan::Changed(ref frames) = plan else {
            panic!("changed Step state")
        };
        assert_eq!(frames.len(), 1);
        let prepared = &frames[0];
        assert!(prepared.preparations.is_empty());
        assert!(prepared.supplied_chunks.is_empty());
        publish_assistant_publication_set(&mut store, commit_stamp, vec![(cursor, plan)]).unwrap();
        let final_snapshot = store.open_assistant_journal_snapshot(&identity).unwrap();
        store.commit_write_unit_of_work().unwrap();
        drop(store);

        let store = NolaStore::open(&path).unwrap();
        let old_after = store
            .read_assistant_snapshot_parts(&old_snapshot, None, 10, 65536)
            .unwrap()
            .parts
            .remove(0);
        assert_eq!(old_after, old);
        let final_head = store
            .read_assistant_snapshot_parts(&final_snapshot, None, 10, 65536)
            .unwrap()
            .parts
            .remove(0);
        assert!(
            matches!(&final_head.metadata, JournalPartMetadata::Step(m) if m.state == expected)
        );
        assert_eq!(final_head.bodies, old.bodies);
        assert_eq!(final_head.generation, old.generation);
        assert_eq!(final_head.revision.0, old.revision.0 + 1);
        let body = &old.bodies[0];
        let mut actual = String::new();
        for index in 0..body.chunk_count.0 {
            actual.push_str(
                store
                    .read_assistant_snapshot_chunk(&final_snapshot, "step", body, index)
                    .unwrap()
                    .text(),
            );
        }
        assert_eq!(actual, title);
    }
}

#[test]
fn failed_publication_requires_outer_rollback_and_leaves_no_visible_admission() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("rollback.db")).unwrap();
    store.begin_write_unit_of_work().unwrap();
    let identity = begin(&mut store);
    let cursor = store.assistant_journal_head(&identity).unwrap();
    let plan = replacement(&store, &identity, "prepared but never published");
    let wrong_stamp = AssistantSourceCommitStamp::Transition {
        transition_id: "not-applied".into(),
    };
    assert!(
        publish_assistant_publication_set(&mut store, wrong_stamp, vec![(cursor, plan)]).is_err()
    );
    store.rollback_write_unit_of_work().unwrap();
    assert!(store.load_conversation("conversation").unwrap().is_none());
    store.begin_write_unit_of_work().unwrap();
    let identity = begin(&mut store);
    let cursor = store.assistant_journal_head(&identity).unwrap();
    assert_eq!(cursor.sequence, 0);
    assert!(store
        .assistant_journal_part(&identity, "text")
        .unwrap()
        .is_none());
    let plan = replacement(&store, &identity, "retry succeeds");
    assert!(
        publish_assistant_publication_set(&mut store, stamp(), vec![(cursor, plan)])
            .unwrap()
            .is_some()
    );
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn review_reveal_is_atomic_and_old_snapshot_never_acquires_hidden_body() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("review.db")).unwrap();
    store.begin_write_unit_of_work().unwrap();
    let identity = begin(&mut store);
    let cursor = store.assistant_journal_head(&identity).unwrap();
    let pending = AgentPart::Text(TextPart {
        id: "text".into(),
        ownership: PartOwnership {
            message_id: "assistant".into(),
            turn_id: "run".into(),
        },
        text: "private candidate".into(),
        review: Some(TextReviewState::Pending),
        is_complete: false,
    });
    let reader = StoreJournalPlanningReader(&store);
    let mut planner = AssistantJournalPlanner::new(&reader, &identity).unwrap();
    planner
        .part_update(
            &PartUpdate::Created {
                part: pending.clone(),
            },
            &pending,
            order(),
        )
        .unwrap();
    let plan = planner.finish().unwrap();
    publish_assistant_publication_set(&mut store, stamp(), vec![(cursor, plan)]).unwrap();
    let hidden_snapshot = store.open_assistant_journal_snapshot(&identity).unwrap();
    store.commit_write_unit_of_work().unwrap();

    store.begin_write_unit_of_work().unwrap();
    let commit_stamp = transition(&mut store, 2);
    let cursor = store.assistant_journal_head(&identity).unwrap();
    let mut approved = pending;
    if let AgentPart::Text(part) = &mut approved {
        part.review = Some(TextReviewState::Approved);
        part.is_complete = true;
    }
    let reader = StoreJournalPlanningReader(&store);
    let mut planner = AssistantJournalPlanner::new(&reader, &identity).unwrap();
    planner
        .part_update(
            &PartUpdate::TextReviewChanged {
                part_id: "text".into(),
                state: TextReviewState::Approved,
            },
            &approved,
            order(),
        )
        .unwrap();
    let plan = planner.finish().unwrap();
    publish_assistant_publication_set(&mut store, commit_stamp, vec![(cursor, plan)]).unwrap();
    let visible_snapshot = store.open_assistant_journal_snapshot(&identity).unwrap();
    store.commit_write_unit_of_work().unwrap();
    let hidden = store
        .read_assistant_snapshot_parts(&hidden_snapshot, None, 10, 65536)
        .unwrap()
        .parts
        .remove(0);
    let visible = store
        .read_assistant_snapshot_parts(&visible_snapshot, None, 10, 65536)
        .unwrap()
        .parts
        .remove(0);
    assert!(hidden.bodies.is_empty());
    assert_eq!(visible.generation.0, 2);
    assert_eq!(
        store
            .read_assistant_snapshot_chunk(&visible_snapshot, "text", &visible.bodies[0], 0)
            .unwrap()
            .text(),
        "private candidate"
    );
    // Knowing the new reference does not authorize reading it via the old snapshot.
    assert!(store
        .read_assistant_snapshot_chunk(&hidden_snapshot, "text", &visible.bodies[0], 0)
        .is_err());
}

#[test]
fn rich_tool_publication_cold_reopens_independent_results_and_keeps_progress_input_root() {
    use nola_domain::assistant_journal_content::{
        decode_assistant_artifact_content, decode_assistant_structured_content,
    };
    use nola_domain::{
        ArtifactReference, HostCapabilityOrigin, HostCapabilityOriginKind, StructuredValue,
        ToolPermissionDisplay,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rich-tool.db");
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_write_unit_of_work().unwrap();
    let identity = begin(&mut store);
    let cursor = store.assistant_journal_head(&identity).unwrap();
    let mut tool = ToolPart {
        execution_source: Default::default(),
        id: "tool".into(),
        ownership: PartOwnership {
            message_id: "assistant".into(),
            turn_id: "run".into(),
        },
        call_id: "call".into(),
        capability_id: "execute".into(),
        title: "真正标题🙂".repeat(400),
        summary: Some(String::new()),
        origin: HostCapabilityOrigin {
            kind: HostCapabilityOriginKind::Mcp,
            provider_id: Some("provider".into()),
            provider_title: Some("来源名称🙂".repeat(400)),
            namespace: Some(String::new()),
        },
        state: ToolPartState::Running,
        input: serde_json::json!({"large": "i".repeat(8 * 1024 * 1024)}),
        raw_input: "PRIVATE_RAW_FRAGMENT".into(),
        permission: Some(ToolPermissionDisplay {
            request_id: "permission".into(),
            permission_domain: "files".into(),
            rationale: "权限说明🙂".repeat(400),
            decision: Some(String::new()),
        }),
        output_preview: Some("start".into()),
        structured_output: Some(serde_json::Value::Null),
        full_output_ref: Some("/private/host/PRIVATE_OUTPUT_PATH".into()),
        artifacts: vec![ArtifactReference {
            uri: "artifact-ref".into(),
            name: "原始名称🙂".into(),
            mime_type: Some("application/pdf".into()),
            operation: None,
        }],
        started_at_ms: Some(u64::MAX),
        finished_at_ms: None,
    };
    let part = AgentPart::Tool(tool.clone());
    let plan = {
        let reader = StoreJournalPlanningReader(&store);
        let mut planner = AssistantJournalPlanner::new(&reader, &identity).unwrap();
        planner
            .part_update(&PartUpdate::Created { part: part.clone() }, &part, order())
            .unwrap();
        planner.finish().unwrap()
    };
    let VisibleJournalPlan::Changed(ref frames) = plan else {
        panic!("Tool creation")
    };
    assert_eq!(frames.len(), 1);
    let prepared = &frames[0];
    assert!(!serde_json::to_string(&prepared.batch)
        .unwrap()
        .contains("PRIVATE_OUTPUT_PATH"));
    assert!(!prepared
        .preparations
        .iter()
        .flat_map(|preparation| &preparation.chunks)
        .any(|chunk| chunk.text().contains("PRIVATE_RAW_FRAGMENT")
            || chunk.text().contains("PRIVATE_OUTPUT_PATH")));
    publish_assistant_publication_set(&mut store, stamp(), vec![(cursor, plan)]).unwrap();
    let old_snapshot = store.open_assistant_journal_snapshot(&identity).unwrap();
    store.commit_write_unit_of_work().unwrap();
    let old = store
        .read_assistant_snapshot_parts(&old_snapshot, None, 10, 65536)
        .unwrap()
        .parts
        .remove(0);
    let input = old
        .bodies
        .iter()
        .find(|body| body.field == JournalContentField::ToolInput)
        .unwrap()
        .clone();
    let structured = old
        .bodies
        .iter()
        .find(|body| body.field == JournalContentField::ToolOutput)
        .unwrap()
        .clone();
    let title = old
        .bodies
        .iter()
        .find(|body| body.field == JournalContentField::Title)
        .unwrap()
        .clone();
    store.begin_write_unit_of_work().unwrap();
    let commit_stamp = transition(&mut store, 2);
    let cursor = store.assistant_journal_head(&identity).unwrap();
    tool.output_preview = Some("start新增".into());
    let plan = {
        let reader = StoreJournalPlanningReader(&store);
        let mut planner = AssistantJournalPlanner::new(&reader, &identity).unwrap();
        planner
            .part_update(
                &PartUpdate::ToolProgress {
                    part_id: "tool".into(),
                    title: None,
                    detail: None,
                    output_delta: Some("新增".into()),
                },
                &AgentPart::Tool(tool.clone()),
                order(),
            )
            .unwrap();
        planner.finish().unwrap()
    };
    let VisibleJournalPlan::Changed(ref frames) = plan else {
        panic!("Tool progress")
    };
    assert_eq!(frames.len(), 1);
    let prepared = &frames[0];
    assert!(prepared.preparations.is_empty());
    assert_eq!(
        prepared
            .supplied_chunks
            .iter()
            .map(|chunk| chunk.text())
            .collect::<String>(),
        "新增"
    );
    publish_assistant_publication_set(&mut store, commit_stamp, vec![(cursor, plan)]).unwrap();
    let final_snapshot = store.open_assistant_journal_snapshot(&identity).unwrap();
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    assert_eq!(
        store
            .read_assistant_snapshot_parts(&old_snapshot, None, 10, 65536)
            .unwrap()
            .parts
            .remove(0),
        old
    );
    let head = store
        .read_assistant_snapshot_parts(&final_snapshot, None, 10, 65536)
        .unwrap()
        .parts
        .remove(0);
    assert!(
        head.bodies.contains(&input)
            && head.bodies.contains(&structured)
            && head.bodies.contains(&title)
    );
    let JournalPartMetadata::Tool(metadata) = &head.metadata else {
        panic!("Tool metadata")
    };
    assert_eq!(
        metadata.summary,
        Some(JournalTextValue::Inline(String::new()))
    );
    assert_eq!(
        metadata.title,
        JournalTextValue::Field(JournalContentField::Title)
    );
    assert_eq!(
        metadata.permission.as_ref().unwrap().rationale,
        JournalTextValue::Field(JournalContentField::PermissionRationale)
    );
    assert_eq!(
        metadata.full_output.as_ref().unwrap().message_id.0,
        "assistant"
    );
    assert_eq!(metadata.full_output.as_ref().unwrap().part_id.0, "tool");
    assert_eq!(
        metadata.full_output.as_ref().unwrap().tool_call_id.0,
        "call"
    );
    assert_eq!(metadata.started_at_ms, Some(JournalNumber(u64::MAX)));
    let read = |field| {
        let body = head.bodies.iter().find(|body| body.field == field).unwrap();
        (0..body.chunk_count.0)
            .map(|index| {
                store
                    .read_assistant_snapshot_chunk(&final_snapshot, "tool", body, index)
                    .unwrap()
                    .text()
                    .to_owned()
            })
            .collect::<String>()
    };
    assert_eq!(
        decode_assistant_structured_content(&read(JournalContentField::ToolOutput)).unwrap(),
        StructuredValue::Null
    );
    assert_eq!(read(JournalContentField::ToolOutputText), "start新增");
    assert_eq!(read(JournalContentField::Title), tool.title);
    assert_eq!(
        decode_assistant_artifact_content(&read(JournalContentField::ToolArtifacts)).unwrap(),
        tool.artifacts
    );
}

#[test]
fn real_core_tool_state_nochange_survives_published_append_and_cold_reopen() {
    use nola_agent_engine::agent::parts::ConversationProjection;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("semantic-nochange.db");
    let mut store = NolaStore::open(&path).unwrap();
    let mut core = ConversationProjection::default();
    store.begin_write_unit_of_work().unwrap();
    let identity = begin(&mut store);
    let cursor = store.assistant_journal_head(&identity).unwrap();
    let created = PartUpdate::Created {
        part: AgentPart::Tool(ToolPart {
            execution_source: Default::default(),
            id: "tool".into(),
            ownership: PartOwnership {
                message_id: "assistant".into(),
                turn_id: "run".into(),
            },
            call_id: "call".into(),
            capability_id: "execute".into(),
            title: "标题🙂".into(),
            summary: Some("human summary".into()),
            origin: Default::default(),
            state: ToolPartState::Running,
            input: serde_json::json!({"input": "i".repeat(100_000)}),
            raw_input: String::new(),
            permission: None,
            output_preview: Some("🙂".repeat(16_000)),
            structured_output: Some(serde_json::Value::Null),
            full_output_ref: None,
            artifacts: vec![],
            started_at_ms: None,
            finished_at_ms: None,
        }),
    };
    assert!(core.apply(created.clone()).unwrap());
    let plan = {
        let reader = StoreJournalPlanningReader(&store);
        let mut planner = AssistantJournalPlanner::new(&reader, &identity).unwrap();
        planner
            .part_update(&created, &core.parts["tool"], order())
            .unwrap();
        planner.finish().unwrap()
    };
    publish_assistant_publication_set(&mut store, stamp(), vec![(cursor, plan)]).unwrap();
    store.commit_write_unit_of_work().unwrap();

    store.begin_write_unit_of_work().unwrap();
    let stamp = transition(&mut store, 2);
    let cursor = store.assistant_journal_head(&identity).unwrap();
    let progress = PartUpdate::ToolProgress {
        part_id: "tool".into(),
        title: None,
        detail: None,
        output_delta: Some("尾🙂".into()),
    };
    assert!(core.apply(progress.clone()).unwrap());
    let plan = {
        let reader = StoreJournalPlanningReader(&store);
        let mut planner = AssistantJournalPlanner::new(&reader, &identity).unwrap();
        planner
            .part_update(&progress, &core.parts["tool"], order())
            .unwrap();
        planner.finish().unwrap()
    };
    let VisibleJournalPlan::Changed(ref frames) = plan else {
        panic!("append")
    };
    assert_eq!(frames.len(), 1);
    let prepared = &frames[0];
    assert_eq!(prepared.batch.operations.len(), 1);
    assert!(prepared.preparations.is_empty());
    publish_assistant_publication_set(&mut store, stamp, vec![(cursor, plan)]).unwrap();
    let snapshot = store.open_assistant_journal_snapshot(&identity).unwrap();
    store.commit_write_unit_of_work().unwrap();
    let original = store
        .assistant_journal_part(&identity, "tool")
        .unwrap()
        .unwrap();
    let original_cursor = store.assistant_journal_head(&identity).unwrap();
    let preview = original
        .bodies
        .iter()
        .find(|value| value.field == JournalContentField::ToolOutputText)
        .unwrap()
        .clone();
    assert_eq!(preview.chunk_count, JournalNumber(2));
    let core_bytes = serde_json::to_vec(&core).unwrap();
    drop(core);
    drop(store);

    let mut store = NolaStore::open(&path).unwrap();
    let mut core: ConversationProjection = serde_json::from_slice(&core_bytes).unwrap();
    let repeated = PartUpdate::ToolState {
        part_id: "tool".into(),
        state: ToolPartState::Running,
        title: None,
        summary: None,
        output_preview: Some(format!("{}尾🙂", "🙂".repeat(16_000))),
        structured_output: None,
        full_output_ref: None,
        artifacts: vec![],
        timestamp_ms: None,
    };
    store.begin_write_unit_of_work().unwrap();
    let source = transition(&mut store, 3);
    for _ in 0..3 {
        assert!(core.apply(repeated.clone()).unwrap());
        let plan = {
            let reader = StoreJournalPlanningReader(&store);
            let mut planner = AssistantJournalPlanner::new(&reader, &identity).unwrap();
            planner
                .part_update(&repeated, &core.parts["tool"], order())
                .unwrap();
            planner.finish().unwrap()
        };
        assert!(matches!(plan, VisibleJournalPlan::Unchanged));
        assert_eq!(
            publish_assistant_publication_set(
                &mut store,
                source.clone(),
                vec![(original_cursor.clone(), plan)]
            )
            .unwrap(),
            None
        );
        assert_eq!(
            store.assistant_journal_head(&identity).unwrap(),
            original_cursor
        );
        assert_eq!(
            store
                .assistant_journal_part(&identity, "tool")
                .unwrap()
                .unwrap(),
            original
        );
    }
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    assert_eq!(
        store.assistant_journal_head(&identity).unwrap(),
        original_cursor
    );
    assert_eq!(
        store
            .assistant_journal_part(&identity, "tool")
            .unwrap()
            .unwrap(),
        original
    );

    // Different bytes with the same total length must still replace only this
    // field. Equality is semantic, not a length check or generation shortcut.
    let mut changed = repeated;
    if let PartUpdate::ToolState {
        output_preview: Some(value),
        ..
    } = &mut changed
    {
        value.replace_range(.."🙂".len(), "🙃");
    }
    assert!(core.apply(changed.clone()).unwrap());
    store.begin_write_unit_of_work().unwrap();
    let source = transition(&mut store, 4);
    let plan = {
        let reader = StoreJournalPlanningReader(&store);
        let mut planner = AssistantJournalPlanner::new(&reader, &identity).unwrap();
        planner
            .part_update(&changed, &core.parts["tool"], order())
            .unwrap();
        planner.finish().unwrap()
    };
    let VisibleJournalPlan::Changed(ref frames) = plan else {
        panic!("replacement")
    };
    assert_eq!(frames.len(), 1);
    let prepared = &frames[0];
    assert_eq!(prepared.batch.operations.len(), 1);
    assert!(
        matches!(&prepared.batch.operations[0], nola_domain::assistant_journal_operations::AssistantJournalOperationV1::ReplaceContent(value) if value.field == JournalContentField::ToolOutputText)
    );
    assert!(prepared
        .preparations
        .iter()
        .all(|value| value.resulting.field == JournalContentField::ToolOutputText));
    publish_assistant_publication_set(&mut store, source, vec![(original_cursor, plan)]).unwrap();
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    let after = store
        .assistant_journal_part(&identity, "tool")
        .unwrap()
        .unwrap();
    assert_eq!(after.generation, original.generation);
    assert_eq!(after.revision.0, original.revision.0 + 1);
    assert_eq!(after.metadata, original.metadata);
    for value in &original.bodies {
        if value.field != JournalContentField::ToolOutputText {
            assert!(after.bodies.contains(value));
        }
    }
    assert_eq!(
        store
            .read_assistant_snapshot_parts(&snapshot, None, 10, 65536)
            .unwrap()
            .parts
            .remove(0),
        original
    );
}
