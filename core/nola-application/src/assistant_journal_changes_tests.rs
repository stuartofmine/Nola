// SPDX-License-Identifier: Apache-2.0
//! Changed-input contract tests, not production publication coverage.

use super::*;
use nola_domain::{ModelAttemptFailure, ModelAttemptState, TextReviewState};
use nola_agent_engine::agent::capability::{
    SkillDescriptor, SkillExecutionMode, SkillRunBudget, SkillRunOwner, SkillRunStatus,
};
use nola_agent_engine::agent::parts::{CompactionPart, PartOwnership, TextPart};
use nola_agent_engine::agent::protocol::{SessionId, TurnId};
use nola_domain::core_event::EventCursor;
use nola_agent_engine::agent::task::{TaskKind, TaskState};

fn scope() -> AssistantJournalChangeScope<'static> {
    AssistantJournalChangeScope {
        conversation_id: "conversation",
        core_session_id: "conversation",
        source_run_id: Some("run"),
        maintenance: false,
    }
}

#[test]
fn runtime_decision_fact_is_persisted_control_evidence_not_an_assistant_part() {
    let mut event = envelope(9, Some("run"), AgentEvent::RuntimeDecisionFactObserved {
        turn_id: TurnId::from_string("run".into()),
        fact: nola_domain::core_requirements::CoreRuntimeDecisionFact {
            fact_id: "cancel-fact".into(),
            kind: nola_domain::core_requirements::CoreRuntimeDecisionFactKind::Cancellation {
                cancellation_id: "cancel".into(), reason: "user stopped".into(),
            },
        },
    });
    let skills = SkillRuntimeState::default();
    let changes = collect_assistant_journal_changes(scope(), std::slice::from_ref(&event),
        &skills, &[], &[]).unwrap();
    assert!(changes.changes.is_empty());
    let payload = crate::committed_event_payload_for_persistence(&event).unwrap();
    assert_eq!(payload["type"], "runtime_decision_fact_observed");
    assert_eq!(payload["fact"]["fact_id"], "cancel-fact");
    assert!(payload.get("core_envelope").is_some());
    event.turn_id = Some("foreign-run".into());
    assert!(collect_assistant_journal_changes(scope(), &[event], &skills, &[], &[]).is_err());
}

fn envelope(cursor: u64, turn: Option<&str>, event: AgentEvent) -> CoreEventEnvelope<AgentEvent> {
    CoreEventEnvelope {
        session_id: "conversation".into(),
        turn_id: turn.map(str::to_owned),
        cursor: EventCursor { sequence: cursor },
        revision: cursor,
        event,
    }
}

fn text_event(update: PartUpdate, complete: bool) -> CoreEventEnvelope<AgentEvent> {
    envelope(
        1,
        Some("successor-turn"),
        AgentEvent::PartUpdated {
            turn_id: TurnId::from_string("successor-turn".into()),
            projection_revision: 1,
            update,
            part: AgentPart::Text(TextPart {
                review: None,
                id: "text".into(),
                ownership: PartOwnership::for_turn("successor-turn"),
                text: "正文🙂".into(),
                is_complete: complete,
            }),
        },
    )
}

fn task_event() -> CoreEventEnvelope<AgentEvent> {
    envelope(
        2,
        Some("older-turn"),
        AgentEvent::TaskUpdated {
            task_projection_revision: 18,
            task: TaskHandle {
                id: "task".into(),
                owner_session_id: SessionId::from_string("conversation".into()),
                owner_turn_id: TurnId::from_string("older-turn".into()),
                kind: TaskKind::Tool,
                state: TaskState::Running,
                progress: Some(f64::from_bits(0x3fd5555555555555)),
                current_step: Some("working".into()),
                result_ref: None,
                cancel_requested: false,
                cancel_requested_at_epoch_ms: None,
                lineage: Default::default(),
                durability: Default::default(),
            },
        },
    )
}

fn skill_run() -> SkillRun {
    SkillRun {
        run_id: "skill-run".into(),
        owner: SkillRunOwner {
            session_id: "conversation".into(),
            parent_turn_id: "older-turn".into(),
            child_turn_id: None,
        },
        descriptor: SkillDescriptor {
            skill_id: "skill".into(),
            version: "1".into(),
            display_name: "Long title remains borrowed".repeat(100),
            instructions_ref: "opaque".into(),
            instructions_digest: "a".repeat(64),
            required_tools: Vec::new(),
            default_mode: None,
            execution_contract: Default::default(),
        },
        mode: SkillExecutionMode::Inline,
        status: SkillRunStatus::Completed,
        budget: SkillRunBudget::default(),
        usage: Default::default(),
        missing_tools: Vec::new(),
        catalog_revision: 1,
        result_ref: Some("result".into()),
        failure: None,
        current_progress_step: None,
        completed_progress_steps: Default::default(),
        artifacts: Default::default(),
        verifications: Default::default(),
        resume_point: None,
    }
}

fn skill_event(update: SkillRuntimeEvent) -> CoreEventEnvelope<AgentEvent> {
    envelope(
        3,
        Some("older-turn"),
        AgentEvent::SkillRuntimeUpdated {
            turn_id: Some(TurnId::from_string("older-turn".into())),
            skill_runtime_revision: 19,
            update,
        },
    )
}

fn close(part_id: &str) -> PartMutation {
    PartMutation::Close {
        part_id: part_id.into(),
        state: "completed".into(),
        content_json: None,
        content_digest: None,
        updated_at_ms: 1,
    }
}

fn attempt(id: &str, revision: i64, predecessor: Option<&str>) -> ModelAttemptRecord {
    ModelAttemptRecord {
        id: id.into(),
        run_id: "run".into(),
        invocation_id: "invocation".into(),
        effect_id: "effect".into(),
        text_part_id: format!("{id}:text"),
        reasoning_part_id: format!("{id}:reasoning"),
        predecessor_id: predecessor.map(str::to_owned),
        state: ModelAttemptState::Failed,
        provider: Some("provider".into()),
        model: Some("model".into()),
        failure: Some(ModelAttemptFailure {
            code: "safe_failure".into(),
            message: "Safe failure summary".into(),
            retryable: true,
            http_status: None,
        }),
        usage: serde_json::json!({}),
        created_at_ms: 1,
        started_at_ms: Some(1),
        completed_at_ms: Some(2),
        revision,
    }
}

#[test]
fn preserves_event_order_and_borrows_exact_changed_records_and_text() {
    let events = vec![
        text_event(
            PartUpdate::TextDelta {
                part_id: "text".into(),
                delta: "🙂".into(),
            },
            false,
        ),
        task_event(),
        skill_event(SkillRuntimeEvent::UsageUpdated {
            run_id: "skill-run".into(),
            usage: Default::default(),
        }),
    ];
    let mut skills = SkillRuntimeState::default();
    let mut run = skill_run();
    run.usage.model_turns = 7;
    skills.runs.insert(run.run_id.clone(), run);
    let result = collect_assistant_journal_changes(scope(), &events, &skills, &[], &[]).unwrap();
    assert_eq!(result.changes.len(), 3);
    let AssistantJournalChange::Part {
        update,
        part,
        turn_id,
        ..
    } = &result.changes[0]
    else {
        panic!("Part first")
    };
    let AgentEvent::PartUpdated {
        part: original,
        update: original_update,
        ..
    } = &events[0].event
    else {
        panic!("fixture")
    };
    assert!(std::ptr::eq(*part, original));
    assert!(std::ptr::eq(*update, original_update));
    assert_eq!(*turn_id, "successor-turn");
    let AssistantJournalChange::Task { task, .. } = &result.changes[1] else {
        panic!("Task second")
    };
    assert_eq!(task.progress.unwrap().to_bits(), 0x3fd5555555555555);
    let AssistantJournalChange::Skill { run, .. } = &result.changes[2] else {
        panic!("Skill third")
    };
    assert!(std::ptr::eq(*run, skills.runs.get("skill-run").unwrap()));
    assert_eq!(run.usage.model_turns, 7);
}

#[test]
fn captures_idle_task_and_skill_without_any_part_update_or_active_run() {
    let events = vec![
        task_event(),
        skill_event(SkillRuntimeEvent::Completed {
            run_id: "skill-run".into(),
            result_ref: Some("result".into()),
        }),
    ];
    let mut skills = SkillRuntimeState::default();
    skills.runs.insert("skill-run".into(), skill_run());
    let result = collect_assistant_journal_changes(
        AssistantJournalChangeScope {
            source_run_id: None,
            ..scope()
        },
        &events,
        &skills,
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(result.changes.len(), 2);
}

#[test]
fn core_session_and_canonical_conversation_are_distinct_explicit_identities() {
    let mut events = vec![
        task_event(),
        skill_event(SkillRuntimeEvent::Completed {
            run_id: "skill-run".into(),
            result_ref: Some("result".into()),
        }),
    ];
    for event in &mut events {
        event.session_id = "core-session".into();
        if let AgentEvent::TaskUpdated { task, .. } = &mut event.event {
            task.owner_session_id = SessionId::from_string("core-session".into());
        }
    }
    let mut skills = SkillRuntimeState::default();
    let mut run = skill_run();
    run.owner.session_id = "core-session".into();
    skills.runs.insert("skill-run".into(), run);
    let result = collect_assistant_journal_changes(
        AssistantJournalChangeScope {
            conversation_id: "canonical-conversation",
            core_session_id: "core-session",
            source_run_id: None,
            maintenance: false,
        },
        &events,
        &skills,
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(result.changes.len(), 2);
}

#[test]
fn requires_exact_skill_final_record_and_declared_session_turn() {
    let event = skill_event(SkillRuntimeEvent::ProgressUpdated {
        run_id: "skill-run".into(),
        step_id: "step".into(),
        completed: true,
    });
    let events = vec![event];
    let mut skills = SkillRuntimeState::default();
    assert!(collect_assistant_journal_changes(scope(), &events, &skills, &[], &[]).is_err());
    let mut run = skill_run();
    run.owner.parent_turn_id = "another-turn".into();
    skills.runs.insert("skill-run".into(), run);
    assert!(collect_assistant_journal_changes(scope(), &events, &skills, &[], &[]).is_err());
    skills
        .runs
        .get_mut("skill-run")
        .unwrap()
        .owner
        .parent_turn_id = "older-turn".into();
    skills.runs.get_mut("skill-run").unwrap().owner.session_id = "another-session".into();
    assert!(collect_assistant_journal_changes(scope(), &events, &skills, &[], &[]).is_err());
}

#[test]
fn started_skill_event_cannot_borrow_a_different_final_owner() {
    let mut started = skill_run();
    started.owner.child_turn_id = Some("child-a".into());
    let events = vec![skill_event(SkillRuntimeEvent::Started { run: started })];
    let mut skills = SkillRuntimeState::default();
    skills.runs.insert("skill-run".into(), skill_run());
    assert!(collect_assistant_journal_changes(scope(), &events, &skills, &[], &[]).is_err());
}

#[test]
fn rejects_part_and_envelope_owner_mismatch() {
    let mut event = text_event(
        PartUpdate::Completed {
            part_id: "text".into(),
        },
        true,
    );
    event.turn_id = Some("foreign-turn".into());
    assert!(collect_assistant_journal_changes(
        scope(),
        &[event],
        &SkillRuntimeState::default(),
        &[],
        &[]
    )
    .is_err());
}

#[test]
fn synthetic_close_is_not_lost_after_review_only_or_nonterminal_updates() {
    let skills = SkillRuntimeState::default();
    let closes = vec![close("text"), close("text")];
    for (update, complete, expected_closes) in [
        (
            PartUpdate::Completed {
                part_id: "text".into(),
            },
            true,
            0,
        ),
        (
            PartUpdate::TextReviewChanged {
                part_id: "text".into(),
                state: TextReviewState::Approved,
            },
            true,
            1,
        ),
        (
            PartUpdate::TextDelta {
                part_id: "text".into(),
                delta: "🙂".into(),
            },
            false,
            1,
        ),
    ] {
        let events = vec![text_event(update, complete)];
        let result =
            collect_assistant_journal_changes(scope(), &events, &skills, &[], &closes).unwrap();
        assert_eq!(
            result
                .changes
                .iter()
                .filter(|change| matches!(change, AssistantJournalChange::SyntheticClose { .. }))
                .count(),
            expected_closes
        );
    }
}

#[test]
fn synthetic_close_is_collected_without_an_event_or_body_reconstruction() {
    let closes = vec![close("unrelated-existing-placeholder")];
    let skills = SkillRuntimeState::default();
    let result = collect_assistant_journal_changes(scope(), &[], &skills, &[], &closes).unwrap();
    let AssistantJournalChange::SyntheticClose { mutation } = &result.changes[0] else {
        panic!("Close")
    };
    assert!(std::ptr::eq(*mutation, &closes[0]));
}

#[test]
fn final_attempt_overlay_keeps_first_touch_order_and_explicit_failure_clear() {
    let first = attempt("parent", 2, None);
    let child = attempt("child", 1, Some("parent"));
    let mut cleared = attempt("parent", 3, None);
    cleared.failure = None;
    let mutations = vec![
        ModelAttemptMutation {
            expected_revision: Some(1),
            record: first,
        },
        ModelAttemptMutation {
            expected_revision: None,
            record: child,
        },
        ModelAttemptMutation {
            expected_revision: Some(2),
            record: cleared,
        },
    ];
    let skills = SkillRuntimeState::default();
    let result = collect_assistant_journal_changes(scope(), &[], &skills, &mutations, &[]).unwrap();
    assert_eq!(result.changes.len(), 2);
    let AssistantJournalChange::ModelAttempt { record, display } = &result.changes[0] else {
        panic!("parent")
    };
    assert_eq!(record.id, "parent");
    assert_eq!(record.revision, 3);
    assert_eq!(*display, AttemptDisplayChange::Remove);
    assert!(std::ptr::eq(*record, &mutations[2].record));
    let AssistantJournalChange::ModelAttempt { record, display } = &result.changes[1] else {
        panic!("child")
    };
    assert_eq!(record.id, "child");
    assert_eq!(*display, AttemptDisplayChange::Upsert);
    assert_eq!(result.attempt_predecessors.len(), 1);
    assert_eq!(result.attempt_predecessors[0].predecessor_id, "parent");
    assert!(std::ptr::eq(
        result.attempt_predecessors[0].successor,
        &mutations[1].record
    ));
}

#[test]
fn unmutated_predecessor_is_an_explicit_scoped_keyed_dependency() {
    let mutations = vec![ModelAttemptMutation {
        expected_revision: None,
        record: attempt("child", 1, Some("stored-parent")),
    }];
    let skills = SkillRuntimeState::default();
    let result = collect_assistant_journal_changes(scope(), &[], &skills, &mutations, &[]).unwrap();
    assert_eq!(
        result.attempt_predecessors[0].predecessor_id,
        "stored-parent"
    );
    assert_eq!(result.attempt_predecessors[0].successor.run_id, "run");
    assert_eq!(
        result.attempt_predecessors[0].successor.invocation_id,
        "invocation"
    );
    assert_eq!(result.attempt_predecessors[0].successor.effect_id, "effect");
}

#[test]
fn attempt_overlay_rejects_wrong_run_cas_and_predecessor_execution() {
    let skills = SkillRuntimeState::default();
    let mut wrong_run = attempt("attempt", 1, None);
    wrong_run.run_id = "foreign".into();
    assert!(collect_assistant_journal_changes(
        scope(),
        &[],
        &skills,
        &[ModelAttemptMutation {
            expected_revision: None,
            record: wrong_run
        }],
        &[]
    )
    .is_err());
    let bad_cas = vec![
        ModelAttemptMutation {
            expected_revision: None,
            record: attempt("attempt", 1, None),
        },
        ModelAttemptMutation {
            expected_revision: Some(2),
            record: attempt("attempt", 3, None),
        },
    ];
    assert!(collect_assistant_journal_changes(scope(), &[], &skills, &bad_cas, &[]).is_err());
    let mut child = attempt("child", 1, Some("parent"));
    child.invocation_id = "foreign-invocation".into();
    let bad_lineage = vec![
        ModelAttemptMutation {
            expected_revision: None,
            record: attempt("parent", 1, None),
        },
        ModelAttemptMutation {
            expected_revision: None,
            record: child,
        },
    ];
    assert!(collect_assistant_journal_changes(scope(), &[], &skills, &bad_lineage, &[]).is_err());
}

#[test]
fn session_catalog_and_compaction_are_not_assistant_changed_inputs() {
    let part = AgentPart::Compaction(CompactionPart {
        id: "compaction".into(),
        ownership: PartOwnership::for_turn("run"),
        summary: "summary".into(),
        retained_item_ids: Vec::new(),
        source_fingerprint: "fingerprint".into(),
        original_token_count: 100,
        compacted_token_count: 30,
    });
    let events = vec![
        envelope(
            1,
            None,
            AgentEvent::SkillRuntimeUpdated {
                turn_id: None,
                skill_runtime_revision: 1,
                update: SkillRuntimeEvent::CatalogReplaced { revision: 1 },
            },
        ),
        envelope(
            2,
            Some("run"),
            AgentEvent::PartUpdated {
                turn_id: TurnId::from_string("run".into()),
                projection_revision: 2,
                update: PartUpdate::Created { part: part.clone() },
                part,
            },
        ),
        envelope(
            3,
            Some("run"),
            AgentEvent::HostProgress {
                turn_id: Some(TurnId::from_string("run".into())),
                effect_id: "effect".into(),
                title: "not a card".into(),
                detail: None,
                metadata: serde_json::Value::Null,
            },
        ),
    ];
    let skills = SkillRuntimeState::default();
    assert!(
        collect_assistant_journal_changes(scope(), &events, &skills, &[], &[])
            .unwrap()
            .changes
            .is_empty()
    );
}

#[test]
fn maintenance_scope_does_not_publish_assistant_or_attempt_changes() {
    let events = vec![
        task_event(),
        text_event(
            PartUpdate::Completed {
                part_id: "text".into(),
            },
            true,
        ),
    ];
    let mutations = vec![ModelAttemptMutation {
        expected_revision: None,
        record: attempt("attempt", 1, None),
    }];
    let closes = vec![close("text")];
    let skills = SkillRuntimeState::default();
    let result = collect_assistant_journal_changes(
        AssistantJournalChangeScope {
            maintenance: true,
            ..scope()
        },
        &events,
        &skills,
        &mutations,
        &closes,
    )
    .unwrap();
    assert!(result.changes.is_empty());
    assert!(result.attempt_predecessors.is_empty());
}
