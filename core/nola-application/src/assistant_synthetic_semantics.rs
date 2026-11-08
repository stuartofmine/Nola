// SPDX-License-Identifier: Apache-2.0
//! Pure, borrowed public semantics for one accepted Task, Skill or Attempt.
//! Ownership, keyed dependency reads, durable order and removals belong to the
//! caller. These views do not scan a projection or establish write authority.
use nola_domain::{
    ActivityState, MessagePartId, MessagePartView, ModelAttemptFailure, ModelAttemptRecord,
    ModelAttemptState, RunAttemptId,
};
use nola_agent_engine::agent::capability::{SkillExecutionMode, SkillRun, SkillRunStatus};
use nola_agent_engine::agent::task::{TaskHandle, TaskState};

#[derive(Debug, Clone)]
pub(crate) struct PublicTaskPart<'a> {
    pub task_id: &'a str,
    /// Ordinary DTO source revision, not proof that visible fields changed.
    pub revision: u64,
    pub display_key: &'static str,
    pub state: ActivityState,
    pub progress: Option<f64>,
    pub current_step: Option<&'a str>,
    pub result_ref: Option<&'a str>,
    pub cancel_requested: bool,
    pub supports_guidance: bool,
}
impl PublicTaskPart<'_> {
    pub(crate) fn part_id(&self) -> String {
        format!("task:{}", self.task_id)
    }
    pub(crate) fn to_message_part(self) -> MessagePartView {
        MessagePartView::Task {
            part_id: MessagePartId(self.part_id()),
            revision: self.revision,
            task_id: self.task_id.into(),
            display_key: self.display_key.into(),
            state: self.state,
            progress: self.progress,
            current_step: self.current_step.map(str::to_owned),
            result_ref: self.result_ref.map(str::to_owned),
            cancel_requested: self.cancel_requested,
        }
    }
}

pub(crate) fn task_part(task: &TaskHandle, revision: u64) -> PublicTaskPart<'_> {
    let TaskHandle {
        id,
        state,
        progress,
        current_step,
        result_ref,
        cancel_requested,
        // Ownership is resolved independently. Execution kind, lease, lineage
        // and cancellation timestamps are not ordinary assistant-card fields.
        owner_session_id: _,
        owner_turn_id: _,
        kind: _,
        cancel_requested_at_epoch_ms: _,
        lineage: _,
        durability: _,
    } = task;
    PublicTaskPart {
        task_id: id,
        revision,
        display_key: "task.background",
        state: match state {
            TaskState::Queued => ActivityState::Pending,
            TaskState::Running => ActivityState::Running,
            TaskState::Waiting | TaskState::Suspended | TaskState::RetryWaiting => {
                ActivityState::Pending
            }
            TaskState::Completed => ActivityState::Completed,
            TaskState::Sealed => ActivityState::Sealed,
            TaskState::Failed => ActivityState::Failed,
            TaskState::Cancelled => ActivityState::Cancelled,
        },
        progress: *progress,
        current_step: current_step.as_deref(),
        result_ref: result_ref.as_deref(),
        cancel_requested: *cancel_requested,
        // A pure Task projection has no durable child-Run authority.
        supports_guidance: false,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PublicSkillPart<'a> {
    pub run_id: &'a str,
    pub revision: u64,
    pub skill_id: &'a str,
    pub display_name: &'a str,
    pub mode: &'static str,
    pub state: ActivityState,
    pub model_turns: u32,
    pub tool_calls: u32,
    pub elapsed_millis: u64,
    pub result_ref: Option<&'a str>,
    pub failure: Option<&'a str>,
}
impl PublicSkillPart<'_> {
    pub(crate) fn part_id(&self) -> String {
        format!("skill:{}", self.run_id)
    }
    pub(crate) fn to_message_part(self) -> MessagePartView {
        MessagePartView::Skill {
            part_id: MessagePartId(self.part_id()),
            revision: self.revision,
            run_id: self.run_id.into(),
            skill_id: self.skill_id.into(),
            display_name: self.display_name.into(),
            mode: self.mode.into(),
            state: self.state,
            model_turns: self.model_turns,
            tool_calls: self.tool_calls,
            elapsed_millis: self.elapsed_millis,
            result_ref: self.result_ref.map(str::to_owned),
            failure: self.failure.map(str::to_owned),
        }
    }
}

pub(crate) fn skill_part(run: &SkillRun, revision: u64) -> PublicSkillPart<'_> {
    let SkillRun {
        run_id,
        descriptor,
        mode,
        status,
        usage,
        result_ref,
        failure,
        // Execution policy and completion evidence stay in their own typed
        // contracts. The ordinary Skill card has never exposed these values.
        owner: _,
        budget: _,
        missing_tools: _,
        catalog_revision: _,
        current_progress_step: _,
        completed_progress_steps: _,
        artifacts: _,
        verifications: _,
        resume_point: _,
    } = run;
    PublicSkillPart {
        run_id,
        revision,
        skill_id: &descriptor.skill_id,
        // Keep the complete dynamic display name. The journal adapter uses a
        // typed Title field for a name too large for bounded inline metadata.
        display_name: &descriptor.display_name,
        mode: match mode {
            SkillExecutionMode::Inline => "inline",
            SkillExecutionMode::Fork => "fork",
        },
        state: match status {
            SkillRunStatus::Running => ActivityState::Running,
            SkillRunStatus::WaitingForCatalog => ActivityState::Pending,
            SkillRunStatus::Completed => ActivityState::Completed,
            SkillRunStatus::Failed => ActivityState::Failed,
            SkillRunStatus::Cancelled => ActivityState::Cancelled,
        },
        model_turns: usage.model_turns,
        tool_calls: usage.tool_calls,
        elapsed_millis: usage.elapsed_millis,
        result_ref: result_ref.as_deref(),
        failure: failure.as_deref(),
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PublicModelAttemptPart<'a> {
    pub attempt_id: &'a str,
    pub revision: u64,
    pub state: ActivityState,
    pub provider: Option<&'a str>,
    pub model: Option<&'a str>,
    pub failure: &'a ModelAttemptFailure,
    pub fallback_attempt_id: Option<&'a str>,
    pub fallback_provider: Option<&'a str>,
    pub fallback_model: Option<&'a str>,
}
impl PublicModelAttemptPart<'_> {
    pub(crate) fn part_id(&self) -> String {
        format!("model-attempt:{}", self.attempt_id)
    }
    pub(crate) fn to_message_part(self) -> MessagePartView {
        MessagePartView::ModelAttempt {
            part_id: MessagePartId(self.part_id()),
            revision: self.revision,
            state: self.state,
            attempt_id: RunAttemptId(self.attempt_id.into()),
            provider: self.provider.map(str::to_owned),
            model: self.model.map(str::to_owned),
            // This is the existing bounded public failure record, not provider
            // diagnostic JSON. Never reconstruct it from trace/output strings.
            failure: self.failure.clone(),
            fallback_attempt_id: self.fallback_attempt_id.map(|id| RunAttemptId(id.into())),
            fallback_provider: self.fallback_provider.map(str::to_owned),
            fallback_model: self.fallback_model.map(str::to_owned),
        }
    }
}

/// `successor` must be a keyed dependency of this exact Attempt, not the last
/// attempt in a Run. None means no visible failure card; an incremental caller
/// must explicitly remove a previously published card when failure is cleared.
pub(crate) fn model_attempt_part<'a>(
    attempt: &'a ModelAttemptRecord,
    successor: Option<&'a ModelAttemptRecord>,
) -> Result<Option<PublicModelAttemptPart<'a>>, &'static str> {
    let ModelAttemptRecord {
        id,
        run_id,
        invocation_id,
        effect_id,
        state,
        provider,
        model,
        failure,
        revision,
        // Canonical Part ownership and lineage are checked by the caller.
        // Usage and timestamps are ledger facts, not this failure-card contract.
        text_part_id: _,
        reasoning_part_id: _,
        predecessor_id: _,
        usage: _,
        created_at_ms: _,
        started_at_ms: _,
        completed_at_ms: _,
    } = attempt;
    let Some(failure) = failure.as_ref() else {
        return Ok(None);
    };
    if let Some(next) = successor {
        if next.id == *id
            || next.predecessor_id.as_deref() != Some(id.as_str())
            || next.run_id != *run_id
            || next.effect_id != *effect_id
            || next.invocation_id != *invocation_id
        {
            return Err("Attempt projection successor belongs to another execution");
        }
    }
    Ok(Some(PublicModelAttemptPart {
        attempt_id: id,
        revision: u64::try_from(*revision).unwrap_or_default(),
        state: match state {
            ModelAttemptState::Prepared => ActivityState::Pending,
            ModelAttemptState::Running | ModelAttemptState::Interrupted => ActivityState::Running,
            ModelAttemptState::Succeeded => ActivityState::Completed,
            ModelAttemptState::Cancelled => ActivityState::Cancelled,
            ModelAttemptState::Failed | ModelAttemptState::Superseded => ActivityState::Failed,
        },
        provider: provider.as_deref(),
        model: model.as_deref(),
        failure,
        fallback_attempt_id: successor.map(|next| next.id.as_str()),
        fallback_provider: successor.and_then(|next| next.provider.as_deref()),
        fallback_model: successor.and_then(|next| next.model.as_deref()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn task() -> TaskHandle {
        serde_json::from_value(json!({"id":"task-1", "owner_session_id":"session",
            "owner_turn_id":"turn", "kind":"tool", "state":"running",
            "current_step":"检查输入", "result_ref":"file:///tmp/result.json",
            "cancel_requested":true, "durability":{"worker_id":"private-worker",
                "ownership_token":"private-lease-token"}}))
        .unwrap()
    }
    fn skill() -> SkillRun {
        serde_json::from_value(json!({"run_id":"skill-run", "owner": {
            "session_id":"session", "parent_turn_id":"turn"}, "descriptor": {
            "skill_id":"document", "version":"1", "display_name":"文档整理",
            "instructions_ref":"private-instructions", "instructions_digest":"a".repeat(64)},
            "mode":"inline", "status":"running",
            "budget":{"max_model_turns":10,"max_tool_calls":20},
            "usage":{"model_turns":3,"tool_calls":5,"elapsed_millis":u64::MAX},
            "catalog_revision":7, "result_ref":"file:///tmp/report.pdf", "failure":"真实限制"}))
        .unwrap()
    }
    fn attempt(id: &str) -> ModelAttemptRecord {
        ModelAttemptRecord {
            id: id.into(),
            run_id: "run".into(),
            invocation_id: "invocation".into(),
            effect_id: "effect".into(),
            text_part_id: format!("{id}:text"),
            reasoning_part_id: format!("{id}:reasoning"),
            predecessor_id: None,
            state: ModelAttemptState::Failed,
            provider: Some("opaque.provider.identity".into()),
            model: Some("model-a".into()),
            failure: Some(ModelAttemptFailure {
                code: "provider_failed".into(),
                message: "Safe readable failure".into(),
                retryable: true,
                http_status: Some(503),
            }),
            usage: json!({"private_diagnostic":"not-public"}),
            created_at_ms: 1,
            started_at_ms: Some(2),
            completed_at_ms: Some(3),
            revision: 4,
        }
    }

    #[test]
    fn pure_task_projection_cannot_grant_guidance_without_durable_binding() {
        use nola_agent_engine::agent::task::TaskKind;
        let mut source = task();
        for kind in [TaskKind::Agent, TaskKind::Tool, TaskKind::Skill, TaskKind::DataRefresh] {
            source.kind = kind;
            for state in [TaskState::Queued, TaskState::Running, TaskState::Completed] {
                source.state = state;
                assert!(!task_part(&source, 1).supports_guidance);
            }
        }
    }

    #[test]
    fn task_mapper_borrows_exact_fields_and_preserves_every_state_and_progress_bits() {
        let mut source = task();
        for (state, expected) in [
            (TaskState::Queued, ActivityState::Pending),
            (TaskState::Running, ActivityState::Running),
            (TaskState::Waiting, ActivityState::Pending),
            (TaskState::Suspended, ActivityState::Pending),
            (TaskState::RetryWaiting, ActivityState::Pending),
            (TaskState::Completed, ActivityState::Completed),
            (TaskState::Sealed, ActivityState::Sealed),
            (TaskState::Failed, ActivityState::Failed),
            (TaskState::Cancelled, ActivityState::Cancelled),
        ] {
            source.state = state;
            for progress in [f64::from_bits(1), -0.0, 1.0] {
                source.progress = Some(progress);
                let part = task_part(&source, 12);
                assert_eq!(part.part_id(), "task:task-1");
                assert_eq!(part.state, expected);
                assert_eq!(part.progress.unwrap().to_bits(), progress.to_bits());
                assert_eq!(
                    part.current_step.unwrap().as_ptr(),
                    source.current_step.as_ref().unwrap().as_ptr()
                );
                let MessagePartView::Task {
                    revision,
                    display_key,
                    progress,
                    current_step,
                    result_ref,
                    cancel_requested,
                    ..
                } = part.to_message_part()
                else {
                    panic!("Task DTO");
                };
                assert_eq!(revision, 12);
                assert_eq!(display_key, "task.background");
                assert_eq!(
                    progress.unwrap().to_bits(),
                    source.progress.unwrap().to_bits()
                );
                assert_eq!(current_step, source.current_step);
                assert_eq!(result_ref, source.result_ref);
                assert!(cancel_requested);
            }
        }
        let serialized = serde_json::to_string(&task_part(&source, 12).to_message_part()).unwrap();
        assert!(!serialized.contains("private-lease-token"));
    }

    #[test]
    fn task_optional_fields_are_not_reconstructed_or_collapsed() {
        let mut source = task();
        source.progress = None;
        for value in [None, Some(String::new()), Some("精确当前步骤".into())] {
            source.current_step = value.clone();
            source.result_ref = value.clone();
            let part = task_part(&source, 1);
            assert_eq!(part.current_step, value.as_deref());
            assert_eq!(part.result_ref, value.as_deref());
            assert_eq!(part.progress, None);
        }
    }

    #[test]
    fn skill_mapper_borrows_dynamic_text_and_preserves_full_public_state() {
        let mut source = skill();
        source.descriptor.display_name = "长显示名称".repeat(2_000);
        for (state, expected) in [
            (SkillRunStatus::Running, ActivityState::Running),
            (SkillRunStatus::WaitingForCatalog, ActivityState::Pending),
            (SkillRunStatus::Completed, ActivityState::Completed),
            (SkillRunStatus::Failed, ActivityState::Failed),
            (SkillRunStatus::Cancelled, ActivityState::Cancelled),
        ] {
            source.status = state;
            for (mode, label) in [
                (SkillExecutionMode::Inline, "inline"),
                (SkillExecutionMode::Fork, "fork"),
            ] {
                source.mode = mode;
                let part = skill_part(&source, 17);
                assert_eq!(part.part_id(), "skill:skill-run");
                assert_eq!(
                    part.display_name.as_ptr(),
                    source.descriptor.display_name.as_ptr()
                );
                assert_eq!(part.state, expected);
                assert_eq!(part.mode, label);
                let MessagePartView::Skill {
                    display_name,
                    model_turns,
                    tool_calls,
                    elapsed_millis,
                    result_ref,
                    failure,
                    ..
                } = part.to_message_part()
                else {
                    panic!("Skill DTO");
                };
                assert_eq!(display_name, source.descriptor.display_name);
                assert_eq!((model_turns, tool_calls, elapsed_millis), (3, 5, u64::MAX));
                assert_eq!(result_ref, source.result_ref);
                assert_eq!(failure, source.failure);
            }
        }
        let serialized = serde_json::to_string(&skill_part(&source, 17).to_message_part()).unwrap();
        assert!(!serialized.contains("private-instructions"));
        for value in [None, Some(String::new()), Some("确切失败".into())] {
            source.failure = value.clone();
            source.result_ref = value.clone();
            let part = skill_part(&source, 17);
            assert_eq!(part.failure, value.as_deref());
            assert_eq!(part.result_ref, value.as_deref());
        }
    }

    #[test]
    fn attempt_mapper_requires_failure_and_only_accepts_an_exact_successor() {
        let mut source = attempt("first");
        let mut successor = attempt("next");
        successor.predecessor_id = Some(source.id.clone());
        successor.provider = Some("fallback-provider".into());
        successor.model = Some("model-b".into());
        let part = model_attempt_part(&source, Some(&successor))
            .unwrap()
            .unwrap();
        assert_eq!(part.part_id(), "model-attempt:first");
        assert!(std::ptr::eq(part.failure, source.failure.as_ref().unwrap()));
        assert_eq!(part.fallback_attempt_id, Some("next"));
        assert_eq!(part.fallback_provider, Some("fallback-provider"));
        assert_eq!(part.fallback_model, Some("model-b"));
        let MessagePartView::ModelAttempt {
            failure,
            provider,
            fallback_attempt_id,
            ..
        } = part.to_message_part()
        else {
            panic!("Attempt DTO");
        };
        assert_eq!(failure, source.failure.clone().unwrap());
        assert_eq!(provider, source.provider);
        assert_eq!(fallback_attempt_id, Some(RunAttemptId("next".into())));
        for invalid in ["predecessor", "run", "effect", "invocation", "self"] {
            let mut foreign = successor.clone();
            match invalid {
                "predecessor" => foreign.predecessor_id = Some("other".into()),
                "run" => foreign.run_id = "foreign".into(),
                "effect" => foreign.effect_id = "foreign".into(),
                "invocation" => foreign.invocation_id = "foreign".into(),
                "self" => foreign.id = source.id.clone(),
                _ => unreachable!(),
            }
            assert!(model_attempt_part(&source, Some(&foreign)).is_err());
        }
        assert_eq!(
            model_attempt_part(&source, None)
                .unwrap()
                .unwrap()
                .fallback_attempt_id,
            None
        );
        source.failure = None;
        assert!(model_attempt_part(&source, None).unwrap().is_none());
    }

    #[test]
    fn attempt_state_mapping_keeps_the_existing_failure_only_display_policy() {
        let mut source = attempt("one");
        for (state, expected) in [
            (ModelAttemptState::Prepared, ActivityState::Pending),
            (ModelAttemptState::Running, ActivityState::Running),
            (ModelAttemptState::Interrupted, ActivityState::Running),
            (ModelAttemptState::Succeeded, ActivityState::Completed),
            (ModelAttemptState::Cancelled, ActivityState::Cancelled),
            (ModelAttemptState::Failed, ActivityState::Failed),
            (ModelAttemptState::Superseded, ActivityState::Failed),
        ] {
            source.state = state;
            let part = model_attempt_part(&source, None).unwrap().unwrap();
            assert_eq!(part.state, expected);
            assert_eq!(part.revision, 4);
            let serialized = serde_json::to_string(&part.to_message_part()).unwrap();
            assert!(serialized.contains("Safe readable failure"));
            assert!(!serialized.contains("private_diagnostic"));
        }
    }
}
