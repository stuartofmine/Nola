// SPDX-License-Identifier: Apache-2.0
//! Core-local Goal bookkeeping; never a Host tool or authorization grant.
use super::*;
use crate::agent::goal::{GoalWorkEntry, GoalWorkOrigin};

pub(super) const TOOL: &str = "goal_update";

#[cfg(test)]
mod evidence_tests {
    use super::*;
    #[test]
    fn declared_checks_reject_text_untyped_failed_and_wrong_version_sources_after_restore() {
        use crate::agent::goal::*;
        use nola_domain::execution_evidence::*;
        use nola_domain::assistant_journal_operations::JournalNumber;
        let mut projection = GoalProjection::default();
        projection.start("g".into(), "Check a resource".into(), vec![], vec![GoalRequirement {
            acceptance_policy: GoalAcceptancePolicy::ExecutionChecks { checks: vec![GoalRequiredCheck {
                kind: ExecutionCheckKind::Postcondition,
                resource_uri: "file:output".into(), version: "sha256:v1".into(), predicate: "test.suite".into(),
            }] }, semantic_revision: 1, id: "r".into(), description: "Actual check".into(),
            dependencies: vec![], status: GoalRecordStatus::Pending, evidence_refs: vec![],
        }], 0).unwrap();
        let goal = projection.active.as_mut().unwrap();
        goal.work.source_turn_ids.insert("turn".into());
        let mut claim = GoalCompletionClaim { requirement_id:"r".into(), requirement_revision:1,
            summary:"Claimed passed".into(), evidence_refs:vec!["response".into()],
            execution_evidence_refs:vec![], task_replacements:vec![] };
        assert!(validate_execution_claim(goal, &goal.requirements["r"], &claim, 1).is_err());
        goal.execution_observations.insert("observed".into(), GoalExecutionObservation {
            recorded_goal_revision: 1,
            turn_id:"turn".into(), source:ExecutionEvidenceSource { effect_id:"effect".into(), request_id:"request".into(),
                host_id:"host".into(), attempt_id:Some("attempt".into()), response_sequence:JournalNumber(1),
                catalog_revision:JournalNumber(1), availability_epoch:JournalNumber(1) },
            state:Some(ExecutionState::Completed), outcome:nola_domain::host_effect::HostToolOutcome::Succeeded,
            checks:vec![PostconditionCheck { kind:nola_domain::execution_evidence::ExecutionCheckKind::Postcondition, resource_uri:"file:output".into(), version:"sha256:v1".into(),
                predicate:"test.suite".into(), passed:true }],
        });
        claim.execution_evidence_refs.push("observed".into());
        let restored: GoalState = serde_json::from_slice(&serde_json::to_vec(goal).unwrap()).unwrap();
        validate_execution_claim(&restored, &restored.requirements["r"], &claim, 1).unwrap();
        let mut process_only = restored.clone();
        process_only.execution_observations.get_mut("observed").unwrap().checks[0].kind = ExecutionCheckKind::ProcessExit;
        if let GoalAcceptancePolicy::ExecutionChecks { checks } = &mut process_only.requirements.get_mut("r").unwrap().acceptance_policy {
            checks[0].kind = ExecutionCheckKind::ProcessExit;
        }
        validate_execution_claim(&process_only, &process_only.requirements["r"], &claim, 1).unwrap();
        process_only.execution_observations.get_mut("observed").unwrap().checks[0].kind = ExecutionCheckKind::Postcondition;
        assert!(validate_execution_claim(&process_only, &process_only.requirements["r"], &claim, 1).is_err());
        for mutation in ["untyped", "failed", "version", "process_exit", "foreign"] {
            let mut invalid = restored.clone();
            let evidence = invalid.execution_observations.get_mut("observed").unwrap();
            match mutation {
                "untyped" => evidence.state = None,
                "failed" => evidence.checks[0].passed = false,
                "version" => evidence.checks[0].version = "sha256:v2".into(),
                "process_exit" => evidence.checks[0].kind = ExecutionCheckKind::ProcessExit,
                _ => evidence.turn_id = "foreign".into(),
            }
            assert!(validate_execution_claim(&invalid, &invalid.requirements["r"], &claim, 1).is_err(), "{mutation}");
        }
        let mut newer = restored.clone();
        let mut failed = newer.execution_observations["observed"].clone();
        failed.recorded_goal_revision = 2;
        failed.checks[0].passed = false;
        newer.execution_observations.insert("new-failure".into(), failed);
        let newer: GoalState = serde_json::from_slice(&serde_json::to_vec(&newer).unwrap()).unwrap();
        assert!(validate_execution_claim(&newer, &newer.requirements["r"], &claim, 2).is_err());
        let mut recovered = newer.clone();
        let mut passed = recovered.execution_observations["observed"].clone();
        passed.recorded_goal_revision = 3;
        recovered.execution_observations.insert("new-pass".into(), passed.clone());
        claim.execution_evidence_refs = vec!["new-pass".into()];
        validate_execution_claim(&recovered, &recovered.requirements["r"], &claim, 3).unwrap();
        assert!(validate_execution_claim(&recovered, &recovered.requirements["r"], &claim, 2).is_err());
        passed.recorded_goal_revision = 4;
        passed.checks[0].version = "other-version".into();
        passed.checks[0].passed = false;
        recovered.execution_observations.insert("different-version".into(), passed);
        validate_execution_claim(&recovered, &recovered.requirements["r"], &claim, 4).unwrap();
        let duplicate = recovered.execution_observations["new-pass"].clone();
        recovered.execution_observations.insert("same-revision".into(), duplicate);
        assert!(validate_execution_claim(&recovered, &recovered.requirements["r"], &claim, 4).is_err());
    }
    #[test]
    fn evidence_survives_summary_only_context_and_rejects_ambiguous_or_foreign_refs() {
        use crate::agent::{goal::*, session::*};
        let mut state = AgentThreadState::new(
            SessionId::from_string("session".into()),
            AgentSessionConfiguration::from_runtime(&AgentCoreConfig::default()),
        );
        state
            .goal_projection
            .start(
                "g".into(),
                "verify".into(),
                vec![],
                vec![GoalRequirement {
                    acceptance_policy: crate::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                    semantic_revision: 1, id: "r".into(),
                    description: "verify".into(),
                    dependencies: vec![],
                    status: GoalRecordStatus::Pending,
                    evidence_refs: vec![],
                }],
                0,
            )
            .unwrap();
        for (turn, failed) in [
            ("first", false),
            ("second", false),
            ("failed", true),
            ("foreign", false),
        ] {
            if turn != "foreign" {
                state
                    .goal_projection
                    .active
                    .as_mut()
                    .unwrap()
                    .work
                    .source_turn_ids
                    .insert(turn.into());
            }
            state.item_ownership.insert(
                turn.into(),
                AgentItemOwnership {
                    source: Default::default(),
                    turn_id: turn.into(),
                    message_id: turn.into(),
                },
            );
            state.conversation.transcript.push(AgentItem::ToolResult {
                id: ItemId::from_string(turn.into()),
                call_id: CallId::from_string("read".into()),
                name: "read_file".into(),
                output: json!({"content":format!("original-{turn}")}),
                is_error: failed,
                full_output_ref: None,
                artifacts: vec![],
            });
        }
        state.conversation.model_context = vec![AgentItem::user_message(
            ItemId::from_string("summary".into()),
            "Summary claims everything passed",
        )];
        let encoded = serde_json::to_vec(&state).unwrap();
        let restored: AgentThreadState = serde_json::from_slice(&encoded).unwrap();
        let evidence = observed_goal_evidence(&restored);
        assert_eq!(evidence.len(), 2);
        assert!(evidence["tool:first:read"].1.contains("original-first"));
        assert!(evidence["tool:second:read"].1.contains("original-second"));
        assert!(!evidence.contains_key("tool:read"));
        assert!(!evidence.contains_key("tool:foreign:read"));
        assert!(!evidence.contains_key("tool:failed:read"));
        let mut single = restored.clone();
        single.conversation.transcript.retain(|item| item.id().as_str() != "second");
        let aliases = observed_goal_evidence(&single);
        assert!(aliases.contains_key("tool:first:read") && aliases.contains_key("tool:read"));
        let index = super::super::model_planner::goal_evidence_index(&single);
        assert_eq!(index.len(), 1, "aliases must not repeat the original evidence preview");
        assert_eq!(index[0]["source_item_id"], "first");
        assert_eq!(observed_goal_evidence(&single).len(), 2, "claim aliases remain available");
    }
}

/// Rebuilt from the lossless checkpoint transcript, never from a compaction summary.
pub(super) fn observed_goal_evidence(
    session: &crate::agent::session::AgentThreadState,
) -> std::collections::BTreeMap<String, (String, String, String)> {
    let Some(goal) = session.goal_projection.active.as_ref() else {
        return Default::default();
    };
    let mut available = std::collections::BTreeMap::<String, (String, String, String)>::new();
    let mut ambiguous = std::collections::BTreeSet::new();
    for item in &session.conversation.transcript {
        if session
            .item_ownership
            .get(item.id().as_str())
            .is_none_or(|owner| !goal.work.source_turn_ids.contains(&owner.turn_id))
        {
            continue;
        }
        let source = session.item_ownership[item.id().as_str()].turn_id.clone();
        match item {
            AgentItem::ToolResult {
                call_id,
                name,
                output,
                is_error: false,
                ..
            } if !matches!(
                name.as_str(),
                "goal_complete" | "goal_update" | "request_completion_review"
            ) =>
            {
                available.insert(
                    format!("tool:{source}:{}", call_id.as_str()),
                    (source.clone(), output.to_string(), item.id().as_str().to_owned()),
                );
                if available
                    .insert(
                        format!("tool:{}", call_id.as_str()),
                        (source, output.to_string(), item.id().as_str().to_owned()),
                    )
                    .is_some()
                {
                    ambiguous.insert(format!("tool:{}", call_id.as_str()));
                }
            }
            AgentItem::Message {
                role: AgentRole::Assistant,
                content,
                ..
            } if !content.trim().is_empty() => {
                available.insert(
                    format!("response:{}", item.id().as_str()),
                    (source.clone(), content.clone(), item.id().as_str().to_owned()),
                );
                available.insert("response".into(), (source, content.clone(), item.id().as_str().to_owned()));
            }
            _ => {}
        }
    }
    for id in ambiguous {
        available.remove(&id);
    }
    available
}

impl AgentRuntime {
    pub(super) fn accept_goal_completion(
        &mut self,
        expected_revision: u64,
        contract_id: &str,
        claims: Vec<crate::agent::goal::GoalCompletionClaim>,
        now: i64,
    ) -> Result<(), String> {
        use crate::agent::goal::*;
        let projection = &self.session.goal_projection;
        if projection.revision != expected_revision {
            return Err("Goal changed before acceptance".into());
        }
        let goal = projection.active.as_ref().ok_or("No active Goal")?;
        let turn = self
            .session
            .execution
            .active_turn
            .as_ref()
            .ok_or("No source turn for Goal acceptance")?;
        if turn.status != AgentTurnStatus::Completed {
            return Err("Goal acceptance requires a completed source turn".into());
        }
        if !matches!(
            turn.completion_outcome,
            Some(nola_domain::TaskCompletionOutcome::Answered
                | nola_domain::TaskCompletionOutcome::ActionCompleted)
        ) {
            return Err("Goal acceptance cannot promote an unfinished source task outcome or a missing result".into());
        }
        // A legacy status flag is not the result decision. Missing or negative
        // canonical decisions cannot be upgraded by a Goal acceptance claim.
        let decision_matches = self.session.execution.requirements.decision.as_ref()
            .is_some_and(|decision| matches!((&decision.value, turn.completion_outcome),
                (nola_domain::run_result::RunResultValue::Answered { .. }, Some(nola_domain::TaskCompletionOutcome::Answered))
                | (nola_domain::run_result::RunResultValue::ActionCompleted { .. }, Some(nola_domain::TaskCompletionOutcome::ActionCompleted))));
        if !decision_matches {
            return Err("Goal acceptance requires the matching canonical source result decision".into());
        }
        if !goal.work.source_turn_ids.contains(turn.turn_id.as_str()) {
            return Err("Completion source turn does not belong to this Goal".into());
        }
        if goal.work.owned_task_ids.iter().any(|id| {
            self.session
                .task_projection
                .tasks
                .get(id)
                .is_none_or(|task| !task.state.is_terminal())
        }) {
            return Err("Goal has unfinished or unreconciled child tasks".into());
        }
        let expected: std::collections::BTreeSet<_> = goal
            .requirements
            .values()
            .filter(|r| r.status != GoalRecordStatus::Completed)
            .map(|r| r.id.as_str())
            .collect();
        let supplied: std::collections::BTreeSet<_> =
            claims.iter().map(|c| c.requirement_id.as_str()).collect();
        if supplied.len() != claims.len() || supplied != expected || claims.is_empty() {
            return Err(
                "Completion claims must cover each pending requirement exactly once".into(),
            );
        }
        let available = observed_goal_evidence(&self.session);
        let retained_claims = claims.clone();
        let negative_tasks: std::collections::BTreeSet<_> = goal.work.owned_task_ids.iter().filter(|id| {
            let task = &self.session.task_projection.tasks[*id];
            if task.kind == crate::agent::task::TaskKind::Agent {
                !task.durability.run_result.as_ref().is_some_and(|result| matches!(result.value,
                    nola_domain::run_result::RunResultValue::Answered { .. }
                    | nola_domain::run_result::RunResultValue::ActionCompleted { .. }))
            } else { task.state != crate::agent::task::TaskState::Completed }
        }).cloned().collect();
        let replacements: std::collections::BTreeSet<_> = claims.iter().flat_map(|claim|
            claim.task_replacements.iter().map(|replacement| replacement.task_id.clone())).collect();
        if replacements != negative_tasks {
            return Err("Every unsuccessful child result requires an explicit requirement replacement; successful children are not replacement targets".into());
        }
        let mut evidence = std::collections::BTreeMap::new();
        let mut findings = Vec::new();
        for claim in claims {
            let requirement = goal.requirements.get(&claim.requirement_id).ok_or("Unknown Goal requirement")?;
            if claim.requirement_revision != requirement.semantic_revision {
                return Err("Goal requirement changed before completion claim".into());
            }
            validate_execution_claim(goal, requirement, &claim, expected_revision)?;
            let mut replaced = std::collections::BTreeSet::new();
            for replacement in &claim.task_replacements {
                if !replaced.insert(&replacement.task_id) { return Err("Duplicate child result replacement".into()); }
                let task = self.session.task_projection.tasks.get(&replacement.task_id).ok_or("Unknown child result")?;
                if replacement.terminal_receipt_id != task.durability.terminal_receipt_id
                    || replacement.result_digest != task.durability.result_digest
                    || replacement.execution_evidence_refs.is_empty() {
                    return Err("Replacement must bind the exact child result and actual parent execution evidence".into());
                }
                for reference in &replacement.execution_evidence_refs {
                    let observation = goal.execution_observations.get(reference).ok_or("Unknown replacement execution evidence")?;
                    if !claim.execution_evidence_refs.contains(reference) || observation.turn_id != turn.turn_id.as_str()
                        || !successful_execution_observation(observation) {
                        return Err("Replacement must cite successful execution from this parent Turn".into());
                    }
                }
            }
            if claim.summary.trim().is_empty()
                || claim.summary.len() > 8192
                || (claim.evidence_refs.is_empty() && claim.execution_evidence_refs.is_empty())
            {
                return Err(
                    "Each requirement needs a bounded explanation and retained evidence".into(),
                );
            }
            for reference in &claim.evidence_refs {
                let (source, observed, source_item_id) = available
                    .get(reference)
                    .ok_or_else(|| format!("Unknown or failed completion evidence: {reference}"))?;
                evidence.insert(
                    reference.clone(),
                    GoalEvidence {
                        id: reference.clone(),
                        source: serde_json::to_string(&(self.session.session_id.as_str(), source, source_item_id)).expect("source identity strings"),
                        summary: observed.chars().take(4096).collect(),
                        observed_at_epoch_ms: now,
                        // Core observed the referenced output, not an executor's
                        // claim that a nonexistent tool ran. Semantic relevance
                        // is the explicit per-requirement model judgment.
                        source_verified: true,
                    },
                );
            }
            for reference in &claim.execution_evidence_refs {
                let observation = &goal.execution_observations[reference];
                evidence.insert(reference.clone(), GoalEvidence {
                    id: reference.clone(), source: serde_json::to_string(&(self.session.session_id.as_str(),
                        &observation.turn_id, reference, &observation.source)).expect("typed evidence source"),
                    summary: serde_json::to_string(&observation.checks).expect("typed checks"),
                    observed_at_epoch_ms: now, source_verified: true,
                });
            }
            let mut all_refs = claim.evidence_refs;
            all_refs.extend(claim.execution_evidence_refs);
            findings.push(GoalAuditFinding {
                requirement_revision: Some(claim.requirement_revision),
                assessment_method: match &requirement.acceptance_policy {
                    GoalAcceptancePolicy::ModelJudgment => GoalAssessmentMethod::ModelJudgment,
                    GoalAcceptancePolicy::ExecutionChecks { .. } => GoalAssessmentMethod::EvidenceInspection,
                },
                record_kind: GoalRecordKind::Requirement,
                record_id: claim.requirement_id,
                status: GoalRecordStatus::Completed,
                evidence_refs: all_refs,
                summary: claim.summary,
            });
        }
        let mut next = projection.clone();
        next.apply_audit(GoalAuditReport {
            assessor: GoalAssessor::ModelTurn { session_id: self.session.session_id.as_str().to_owned(), turn_id: turn.turn_id.as_str().to_owned() },
            expected_goal_revision: expected_revision,
            id: format!("acceptance:{contract_id}"),
            contract_id: contract_id.into(),
            verdict: GoalAuditVerdict::Passed,
            summary: "Explicit requirement acceptance grounded in committed source output".into(),
            findings,
            unmet_requirement_ids: vec![],
            integrity_violations: vec![],
            evidence: evidence.into_values().collect(),
            audited_at_epoch_ms: now,
        })?;
        next.active.as_mut().expect("active Goal").completion_claims.insert(format!("acceptance:{contract_id}"), retained_claims);
        next.complete(now)?;
        self.session.goal_projection = next;
        Ok(())
    }

    pub(super) fn record_goal_user_updates(
        &mut self,
        items: &[AgentItem],
        events: &mut Vec<RuntimeEvent>,
    ) {
        let Some(goal) = self
            .session
            .goal_projection
            .active
            .as_mut()
            .filter(|goal| !goal.status.is_terminal())
        else {
            return;
        };
        let mut changed = false;
        for item in items {
            let user_update = match item {
                AgentItem::Message { id, role: AgentRole::User, content } => Some((id, content)),
                AgentItem::UiAction { id, action } => Some((id, &action.summary)),
                _ => None,
            };
            if let Some((id, content)) = user_update {
                if !content.trim().is_empty()
                    && !goal
                        .work
                        .user_updates
                        .iter()
                        .any(|entry| entry.message_id == id.as_str())
                {
                    goal.work
                        .user_updates
                        .push(crate::agent::goal::GoalUserUpdate {
                            message_id: id.as_str().to_string(),
                            content: content.clone(),
                        });
                    changed = true;
                }
            }
        }
        if changed {
            self.session.goal_projection.revision =
                self.session.goal_projection.revision.saturating_add(1);
            events.push(RuntimeEvent::GoalUpdated {
                goal_projection_revision: self.session.goal_projection.revision,
                goal: self.session.goal_projection.clone(),
            });
        }
    }
}

fn successful_execution_observation(observation: &crate::agent::goal::GoalExecutionObservation) -> bool {
    observation.outcome == nola_domain::host_effect::HostToolOutcome::Succeeded
        && observation.state == Some(nola_domain::execution_evidence::ExecutionState::Completed)
}

fn validate_execution_claim(goal: &crate::agent::goal::GoalState,
    requirement: &crate::agent::goal::GoalRequirement,
    claim: &crate::agent::goal::GoalCompletionClaim,
    expected_goal_revision: u64) -> Result<(), String> {
    use crate::agent::goal::GoalAcceptancePolicy;
    let mut seen = std::collections::BTreeSet::new();
    for reference in &claim.execution_evidence_refs {
        if !seen.insert(reference) { return Err("Duplicate execution evidence reference".into()); }
        let observation = goal.execution_observations.get(reference).ok_or("Unknown execution evidence reference")?;
        if observation.recorded_goal_revision == 0 || observation.recorded_goal_revision > expected_goal_revision
            || !goal.work.source_turn_ids.contains(&observation.turn_id) || !successful_execution_observation(observation) {
            return Err("Execution evidence is not a completed typed observation of this Goal".into());
        }
    }
    if let GoalAcceptancePolicy::ExecutionChecks { checks } = &requirement.acceptance_policy {
        for expected in checks {
            let matching = |check: &nola_domain::execution_evidence::PostconditionCheck|
                check.kind == expected.kind
                    && check.resource_uri == expected.resource_uri && check.version == expected.version
                    && check.predicate == expected.predicate;
            let mut latest: Option<(&String, &crate::agent::goal::GoalExecutionObservation)> = None;
            let mut observed_revisions = std::collections::BTreeSet::new();
            for (id, observation) in &goal.execution_observations {
                if !observation.checks.iter().any(&matching) { continue; }
                if observation.recorded_goal_revision == 0 || observation.recorded_goal_revision > expected_goal_revision
                    || !goal.work.source_turn_ids.contains(&observation.turn_id) {
                    return Err("Goal check observation lies outside the accepted source snapshot".into());
                }
                if !observed_revisions.insert(observation.recorded_goal_revision) {
                    return Err("Goal check observation order is ambiguous".into());
                }
                if latest.is_none_or(|(_, prior)| prior.recorded_goal_revision < observation.recorded_goal_revision) {
                    latest = Some((id, observation));
                }
            }
            let Some((id, observation)) = latest else {
                return Err("Required execution check has no exact successful typed observation".into());
            };
            if !claim.execution_evidence_refs.contains(id) || !successful_execution_observation(observation)
                || !observation.checks.iter().filter(|check| matching(check)).all(|check| check.passed) {
                return Err("Required execution check needs the latest successful observation, not an older pass".into());
            }
        }
    }
    Ok(())
}

pub(super) fn tool_schema() -> Value {
    json!({"type":"function","function":{
        "name":TOOL,
        "description":"Persist Goal requirement details, constraints, decisions, reported progress, blockers and next actions before context compaction. Call alone. These are model-authored plans/claims, never user authorization or verified completion. Use fresh entry ids; supersedes replaces only a current model record of the same kind. Read goal_id and expected_revision from durable_goal_state. Entries accept only the fields declared below; do not copy origin or source_ref from stored work records. Core assigns those provenance fields.",
        "parameters":{"type":"object","additionalProperties":false,
            "required":["goal_id","expected_revision","entries"],
            "properties":{
                "goal_id":{"type":"string"},"expected_revision":{"type":"integer","minimum":0},
                "entries":{"type":"array","minItems":1,"maxItems":32,"items":{
                    "type":"object","additionalProperties":false,
                    "required":["id","kind","content","requirement_ids","task_ids","evidence_refs","progress","supersedes"],
                    "properties":{
                        "id":{"type":"string"},
                        "kind":{"type":"string","enum":["constraint","requirement_detail","decision","progress","blocker","next_action"]},
                        "content":{"type":"string"},
                        "requirement_ids":{"type":"array","items":{"type":"string"}},
                        "task_ids":{"type":"array","items":{"type":"string"}},
                        "evidence_refs":{"type":"array","items":{"type":"string"}},
                        "progress":{"type":["string","null"],"enum":["planned","in_progress","reported_done","blocked",null]},
                        "supersedes":{"type":["string","null"]}
                    }
                }}
            }
        }
    }})
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    goal_id: String,
    expected_revision: u64,
    entries: Vec<Value>,
}

pub(super) fn apply_model_update(
    projection: &mut crate::agent::goal::GoalProjection,
    arguments: &Value,
    source_ref: &str,
    now_ms: i64,
) -> Result<(), String> {
    let update: Update = serde_json::from_value(arguments.clone()).map_err(|e| e.to_string())?;
    let entries = update
        .entries
        .into_iter()
        .map(|mut value| {
            let object = value
                .as_object_mut()
                .ok_or("Goal entry must be an object")?;
            if object.contains_key("origin") || object.contains_key("source_ref") {
                return Err("Goal record provenance is assigned by Core, not the model; remove origin and source_ref from entries and retry with the current goal revision".into());
            }
            object.insert(
                "origin".into(),
                serde_json::to_value(GoalWorkOrigin::Model).unwrap(),
            );
            object.insert("source_ref".into(), Value::String(source_ref.into()));
            serde_json::from_value::<GoalWorkEntry>(value).map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>, String>>()?;
    projection.update_work(&update.goal_id, update.expected_revision, entries, now_ms)
}
