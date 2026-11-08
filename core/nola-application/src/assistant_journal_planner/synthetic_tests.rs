// SPDX-License-Identifier: Apache-2.0
//! Exercise the same single-item semantics as ordinary presentation, including
//! independent source revisions, optional empty values and permanent removals.
use super::*;
use crate::assistant_synthetic_semantics::{
    PublicModelAttemptPart, PublicSkillPart, PublicTaskPart,
};

fn section(section: JournalSection) -> JournalOrderKey {
    JournalOrderKey {
        section,
        position: JournalNumber(1),
    }
}

fn validate_plan(reader: &Reader, plan: &PreparedAssistantJournal) {
    validate_assistant_batch(
        &ProofReader {
            original: reader,
            plan,
        },
        &plan.batch,
    )
    .unwrap();
}

#[test]
fn task_control_upgrade_and_capability_changes_are_exact_incremental_updates() {
    let empty = Reader::default();
    let mut seed = AssistantJournalPlanner::new(&empty, &identity()).unwrap();
    let mut part = PublicTaskPart {
        task_id: "background", revision: 1, display_key: "task.background",
        state: nola_domain::ActivityState::Running, progress: None,
        current_step: None, result_ref: None, cancel_requested: false,
        supports_guidance: false,
    };
    let order = section(JournalSection::Tasks);
    seed.task_update(&part, order.clone()).unwrap();
    let mut legacy = seed.head("task:background").unwrap().unwrap();
    let JournalPartMetadata::TaskControl(controls) = legacy.metadata.clone() else { panic!("task") };
    legacy.metadata = JournalPartMetadata::Task(controls.task);
    let mut reader = Reader::default();
    reader.heads.insert(legacy.part_id.clone(), legacy.clone());
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    planner.task_update(&part, order.clone()).unwrap();
    assert_eq!(planner.operations.len(), 1);
    assert!(matches!(&planner.operations[0], AssistantJournalOperationV1::Replace(_)));
    let upgraded = planner.head(&legacy.part_id).unwrap().unwrap();
    assert_eq!(upgraded.generation.0, legacy.generation.0 + 1);
    assert_eq!(upgraded.order_key, legacy.order_key);
    assert_eq!(upgraded.bodies, legacy.bodies);
    part.supports_guidance = true;
    planner.task_update(&part, order.clone()).unwrap();
    assert_eq!(planner.operations.len(), 2);
    assert!(matches!(&planner.operations[1], AssistantJournalOperationV1::Update(_)));
    assert_eq!(planner.head(&legacy.part_id).unwrap().unwrap().generation, upgraded.generation);
    part.revision += 1;
    planner.task_update(&part, order).unwrap();
    assert_eq!(planner.operations.len(), 2, "source-only revision is not a visible change");
    validate_plan(&reader, &changed(planner.finish().unwrap()));
}

#[test]
fn task_exact_fraction_optional_empty_and_source_only_revision_changes() {
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let mut part = PublicTaskPart {
        task_id: "background",
        revision: 1,
        display_key: "task.background",
        state: nola_domain::ActivityState::Running,
        progress: Some(f64::from_bits(1)),
        current_step: None,
        result_ref: Some(""),
        cancel_requested: false,
        supports_guidance: true,
    };
    let order = section(JournalSection::Tasks);
    planner.task_update(&part, order.clone()).unwrap();
    let before = planner.head("task:background").unwrap().unwrap();
    assert!(!before
        .bodies
        .iter()
        .any(|value| value.field == JournalContentField::TaskCurrentStep));
    assert_eq!(before.bodies[0].field, JournalContentField::ResultReference);
    assert_eq!(before.bodies[0].utf8_bytes.0, 0);
    let JournalPartMetadata::TaskControl(controls) = &before.metadata else {
        panic!("Task metadata")
    };
    assert!(controls.supports_guidance && controls.supports_cancellation);
    let metadata = &controls.task;
    assert_eq!(metadata.progress.unwrap().fraction().to_bits(), 1);
    let operations = planner.operations.len();
    part.revision = 100;
    planner.task_update(&part, order.clone()).unwrap();
    assert_eq!(planner.operations.len(), operations);
    assert_eq!(planner.head("task:background").unwrap().unwrap(), before);

    part.current_step = Some("");
    part.progress = Some(-0.0);
    part.result_ref = None;
    planner.task_update(&part, order).unwrap();
    let after = planner.head("task:background").unwrap().unwrap();
    assert_eq!(after.bodies.len(), 1);
    assert_eq!(after.bodies[0].field, JournalContentField::TaskCurrentStep);
    assert_eq!(after.bodies[0].utf8_bytes.0, 0);
    let JournalPartMetadata::TaskControl(controls) = &after.metadata else {
        panic!("Task metadata")
    };
    let metadata = &controls.task;
    assert_eq!(
        metadata.progress.unwrap().fraction().to_bits(),
        (-0.0f64).to_bits()
    );
    assert_eq!(reader.reads.get(), 1);
    validate_plan(&reader, &changed(planner.finish().unwrap()));
}

#[test]
fn skill_dynamic_name_survives_usage_failure_updates_and_inline_replacement() {
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let long_name = "多语言任务🙂".repeat(2000);
    let mut part = PublicSkillPart {
        run_id: "skill-run",
        revision: 1,
        skill_id: "skill",
        display_name: &long_name,
        mode: "inline",
        state: nola_domain::ActivityState::Running,
        model_turns: 0,
        tool_calls: 0,
        elapsed_millis: 0,
        result_ref: None,
        failure: Some(""),
    };
    let order = section(JournalSection::Skills);
    planner.skill_update(&part, order.clone()).unwrap();
    let before = planner.head("skill:skill-run").unwrap().unwrap();
    let title = before
        .bodies
        .iter()
        .find(|value| value.field == JournalContentField::Title)
        .unwrap()
        .clone();
    assert_eq!(title.utf8_bytes.0, long_name.len() as u64);
    part.model_turns = u32::MAX;
    part.tool_calls = 5;
    part.elapsed_millis = u64::MAX;
    part.failure = Some("明确失败原因");
    planner.skill_update(&part, order.clone()).unwrap();
    let after = planner.head("skill:skill-run").unwrap().unwrap();
    assert_eq!(
        after
            .bodies
            .iter()
            .find(|value| value.field == JournalContentField::Title),
        Some(&title)
    );
    let JournalPartMetadata::Skill(metadata) = &after.metadata else {
        panic!("Skill metadata")
    };
    assert_eq!(metadata.elapsed_millis.0, u64::MAX);
    assert_eq!(metadata.model_turns.0, u64::from(u32::MAX));
    assert_eq!(
        metadata.display_name,
        JournalTextValue::Field(JournalContentField::Title)
    );
    let operations = planner.operations.len();
    part.revision = 900;
    planner.skill_update(&part, order.clone()).unwrap();
    assert_eq!(planner.operations.len(), operations);
    part.display_name = "短名称";
    part.result_ref = Some("");
    planner.skill_update(&part, order).unwrap();
    let final_head = planner.head("skill:skill-run").unwrap().unwrap();
    assert!(!final_head
        .bodies
        .iter()
        .any(|value| value.field == JournalContentField::Title));
    assert!(final_head
        .bodies
        .iter()
        .any(|value| value.field == JournalContentField::FailureMessage));
    assert!(final_head
        .bodies
        .iter()
        .any(|value| value.field == JournalContentField::ResultReference));
    assert_eq!(reader.reads.get(), 1);
    validate_plan(&reader, &changed(planner.finish().unwrap()));
}

#[test]
fn failed_attempt_retains_public_message_and_exact_successor_then_clears_once() {
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let failure = nola_domain::ModelAttemptFailure {
        code: "provider_unavailable".into(),
        message: "可公开的失败摘要".into(),
        retryable: true,
        http_status: Some(503),
    };
    let mut part = PublicModelAttemptPart {
        attempt_id: "physical-attempt",
        revision: 1,
        state: nola_domain::ActivityState::Failed,
        provider: Some("provider"),
        model: Some("model"),
        failure: &failure,
        fallback_attempt_id: None,
        fallback_provider: None,
        fallback_model: None,
    };
    let order = section(JournalSection::ModelAttempts);
    planner.model_attempt_update(&part, order.clone()).unwrap();
    let before = planner
        .head("model-attempt:physical-attempt")
        .unwrap()
        .unwrap();
    assert_eq!(before.bodies[0].field, JournalContentField::FailureMessage);
    assert_eq!(before.bodies[0].utf8_bytes.0, failure.message.len() as u64);
    part.fallback_attempt_id = Some("successor");
    part.fallback_provider = Some("next-provider");
    part.fallback_model = Some("next-model");
    planner.model_attempt_update(&part, order).unwrap();
    let after = planner
        .head("model-attempt:physical-attempt")
        .unwrap()
        .unwrap();
    assert_eq!(before.bodies, after.bodies);
    let JournalPartMetadata::ModelAttempt(metadata) = &after.metadata else {
        panic!("Attempt metadata")
    };
    assert_eq!(metadata.fallback_attempt_id.as_deref(), Some("successor"));
    planner.remove_model_attempt(part.attempt_id).unwrap();
    let operations = planner.operations.len();
    planner.remove_model_attempt(part.attempt_id).unwrap();
    assert_eq!(planner.operations.len(), operations);
    assert_eq!(reader.reads.get(), 1);
    validate_plan(&reader, &changed(planner.finish().unwrap()));
}

#[test]
fn cold_retired_attempt_is_not_resurrected_or_repeatedly_removed() {
    let reader = Reader {
        retired: true,
        ..Reader::default()
    };
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    planner.remove_model_attempt("physical-attempt").unwrap();
    planner.remove_model_attempt("physical-attempt").unwrap();
    assert_eq!(reader.reads.get(), 1);
    assert_eq!(
        planner.head("model-attempt:physical-attempt"),
        Err(PlannerError::RetiredPart)
    );
    assert!(matches!(
        planner.finish().unwrap(),
        VisibleJournalPlan::Unchanged
    ));
}
