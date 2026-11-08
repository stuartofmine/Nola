// SPDX-License-Identifier: Apache-2.0
//! Versioned working agreement. Model plans are never user authorization or
//! verified completion; requirement completion remains owned by Goal evidence.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalWorkKind {
    Constraint,
    RequirementDetail,
    Decision,
    Progress,
    Blocker,
    NextAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalWorkOrigin {
    User,
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalWorkProgress {
    Planned,
    InProgress,
    ReportedDone,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalWorkEntry {
    pub id: String,
    pub kind: GoalWorkKind,
    pub origin: GoalWorkOrigin,
    pub source_ref: String,
    pub content: String,
    pub requirement_ids: Vec<String>,
    pub task_ids: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub progress: Option<GoalWorkProgress>,
    pub supersedes: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalWorkState {
    pub source_turn_ids: BTreeSet<String>,
    pub owned_task_ids: BTreeSet<String>,
    /// Immutable revision history; superseded records remain inspectable.
    pub entries: Vec<GoalWorkEntry>,
    /// Exact user messages, captured at the explicit Goal turn boundary.
    /// Not model paraphrases and not inferred from ordinary chat keywords.
    pub user_updates: Vec<GoalUserUpdate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalUserUpdate {
    pub message_id: String,
    pub content: String,
}

impl GoalWorkState {
    pub fn current(&self) -> Vec<&GoalWorkEntry> {
        let superseded: BTreeSet<_> = self
            .entries
            .iter()
            .filter_map(|entry| entry.supersedes.as_deref())
            .collect();
        self.entries
            .iter()
            .filter(|entry| !superseded.contains(entry.id.as_str()))
            .collect()
    }
}

impl GoalProjection {
    pub fn update_work(
        &mut self,
        goal_id: &str,
        expected_revision: u64,
        entries: Vec<GoalWorkEntry>,
        now_epoch_ms: i64,
    ) -> Result<(), String> {
        if self.revision != expected_revision {
            return Err("Goal revision changed; reload before updating".into());
        }
        let goal = self.active.as_ref().ok_or("No Goal is active")?;
        if goal.id != goal_id || goal.status.is_terminal() {
            return Err("Goal identity or lifecycle does not permit updates".into());
        }
        if now_epoch_ms < goal.updated_at_epoch_ms {
            return Err("Goal update time moved backwards".into());
        }
        if entries.is_empty() || entries.len() > 32 {
            return Err("Goal update requires 1..32 entries".into());
        }
        let mut work = goal.work.clone();
        let mut invalidated = BTreeSet::new();
        for entry in entries {
            for (name, value, limit) in [
                ("id", &entry.id, 256),
                ("source", &entry.source_ref, 512),
                ("content", &entry.content, 8192),
            ] {
                if value.trim().is_empty() || value.len() > limit {
                    return Err(format!("Invalid Goal work {name}"));
                }
            }
            if work.entries.iter().any(|old| old.id == entry.id) {
                return Err("Goal work id already exists".into());
            }
            if entry
                .requirement_ids
                .iter()
                .any(|id| !goal.requirements.contains_key(id))
            {
                return Err("Goal work references an unknown requirement".into());
            }
            if entry.kind == GoalWorkKind::RequirementDetail && entry.requirement_ids.is_empty() {
                return Err("Requirement details must refine an existing requirement".into());
            }
            if (entry.kind == GoalWorkKind::Progress) != entry.progress.is_some() {
                return Err("Only progress entries carry progress state".into());
            }
            if entry.progress == Some(GoalWorkProgress::ReportedDone)
                && entry.evidence_refs.is_empty()
            {
                return Err(
                    "Reported progress needs evidence references; it does not verify completion"
                        .into(),
                );
            }
            for references in [
                &entry.requirement_ids,
                &entry.task_ids,
                &entry.evidence_refs,
            ] {
                if references.len() > 32
                    || references
                        .iter()
                        .any(|id| id.trim().is_empty() || id.len() > 512)
                {
                    return Err("Invalid Goal work references".into());
                }
            }
            if let Some(previous) = &entry.supersedes {
                let active = work.current();
                let old = active
                    .iter()
                    .find(|old| &old.id == previous)
                    .ok_or("Superseded Goal record is not current")?;
                if old.kind != entry.kind
                    || old.origin != entry.origin
                {
                    return Err(
                        "Model updates cannot replace user requirements or change record kind"
                            .into(),
                    );
                }
                collect_changed_requirements(goal, old, &mut invalidated);
            }
            collect_changed_requirements(goal, &entry, &mut invalidated);
            work.entries.push(entry);
        }
        // Reject rather than silently dropping constraints when constructing
        // model context. Historical versions are not repeated in every request.
        if serde_json::to_vec(&work.current())
            .map_err(|e| e.to_string())?
            .len()
            > 48 * 1024
        {
            return Err(
                "Active Goal work exceeds 48 KiB; explicitly consolidate model records".into(),
            );
        }
        let goal = self.active.as_mut().expect("validated Goal");
        invalidate_requirement_acceptances(goal, invalidated)?;
        goal.work = work;
        goal.updated_at_epoch_ms = now_epoch_ms;
        goal.last_progress_at_epoch_ms = Some(now_epoch_ms);
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }
}

/// Only explicit contract changes invalidate acceptance. Progress, next-action
/// notes and arbitrary user words are not requirement-routing signals.
fn collect_changed_requirements(
    goal: &GoalState,
    entry: &GoalWorkEntry,
    affected: &mut BTreeSet<String>,
) {
    if !matches!(entry.kind, GoalWorkKind::Constraint | GoalWorkKind::RequirementDetail) {
        return;
    }
    if entry.requirement_ids.is_empty() {
        affected.extend(goal.requirements.keys().cloned());
    } else {
        affected.extend(entry.requirement_ids.iter().cloned());
    }
}

fn invalidate_requirement_acceptances(goal: &mut GoalState, mut affected: BTreeSet<String>) -> Result<(), String> {
    // Include dependants: an acceptance relying on an invalidated prerequisite
    // must not remain current. Retain historical reports and cited evidence.
    let mut dependants = BTreeMap::<String, Vec<String>>::new();
    for requirement in goal.requirements.values() {
        for dependency in &requirement.dependencies {
            dependants.entry(dependency.clone()).or_default().push(requirement.id.clone());
        }
    }
    let mut pending: Vec<_> = affected.iter().cloned().collect();
    while let Some(id) = pending.pop() {
        for dependant in dependants.get(&id).into_iter().flatten() {
            if affected.insert(dependant.clone()) {
                pending.push(dependant.clone());
            }
        }
    }
    if affected.iter().any(|id| goal.requirements.get(id).is_some_and(|requirement| requirement.semantic_revision == u64::MAX)) {
        return Err("Goal requirement revision exhausted".into());
    }
    for id in affected {
        if let Some(requirement) = goal.requirements.get_mut(&id) {
            requirement.semantic_revision += 1;
            if requirement.status == GoalRecordStatus::Completed {
                requirement.status = GoalRecordStatus::Pending;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn projection() -> GoalProjection {
        let mut goal = GoalProjection::default();
        goal.start(
            "g".into(),
            "Deliver the requested result".into(),
            vec!["No deployment".into()],
            vec![GoalRequirement {
                acceptance_policy: crate::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                semantic_revision: 1, id: "r".into(),
                description: "Verify output".into(),
                dependencies: vec![],
                status: GoalRecordStatus::Pending,
                evidence_refs: vec![],
            }],
            1,
        )
        .unwrap();
        goal
    }
    fn entry(id: &str, origin: GoalWorkOrigin) -> GoalWorkEntry {
        GoalWorkEntry {
            id: id.into(),
            kind: GoalWorkKind::Decision,
            origin,
            source_ref: "message:1".into(),
            content: "Keep the existing data model".into(),
            requirement_ids: vec!["r".into()],
            task_ids: vec![],
            evidence_refs: vec![],
            progress: None,
            supersedes: None,
        }
    }
    #[test]
    fn model_cannot_overwrite_user_records_and_batch_rejection_is_atomic() {
        let mut goal = projection();
        goal.update_work("g", 1, vec![entry("user", GoalWorkOrigin::User)], 2)
            .unwrap();
        let before = goal.clone();
        let mut replacement = entry("replacement", GoalWorkOrigin::Model);
        replacement.supersedes = Some("user".into());
        assert!(goal
            .update_work(
                "g",
                2,
                vec![entry("new", GoalWorkOrigin::Model), replacement],
                3
            )
            .is_err());
        assert_eq!(goal, before);
        assert!(goal
            .update_work("g", 1, vec![entry("stale", GoalWorkOrigin::Model)], 3)
            .is_err());
        assert!(goal
            .update_work(
                "other-goal",
                2,
                vec![entry("foreign", GoalWorkOrigin::Model)],
                3
            )
            .is_err());
    }
    #[test]
    fn revisions_preserve_history_and_reported_progress_does_not_complete_requirements() {
        let mut goal = projection();
        let control_token = goal.control_token();
        let mut first = entry("step1", GoalWorkOrigin::Model);
        first.kind = GoalWorkKind::Progress;
        first.progress = Some(GoalWorkProgress::InProgress);
        goal.update_work("g", 1, vec![first.clone()], 2).unwrap();
        let mut done = first;
        done.id = "step2".into();
        done.supersedes = Some("step1".into());
        done.progress = Some(GoalWorkProgress::ReportedDone);
        assert!(goal.update_work("g", 2, vec![done.clone()], 3).is_err());
        done.evidence_refs = vec!["tool-result:42".into()];
        goal.update_work("g", 2, vec![done], 3).unwrap();
        assert_eq!(goal.control_token(), control_token, "progress must not invalidate user controls");
        let restored: GoalProjection =
            serde_json::from_slice(&serde_json::to_vec(&goal).unwrap()).unwrap();
        assert_eq!(restored, goal);
        let state = restored.active.unwrap();
        assert_eq!(state.work.entries.len(), 2);
        assert_eq!(state.work.current()[0].id, "step2");
        assert_eq!(state.requirements["r"].status, GoalRecordStatus::Pending);
        assert!(goal.complete(4).is_err());
    }

    fn accepted_projection() -> GoalProjection {
        let mut goal = projection();
        let state = goal.active.as_mut().unwrap();
        for (id, dependencies) in [("dependent", vec!["r".into()]), ("unrelated", vec![])] {
            state.requirements.insert(id.into(), GoalRequirement {
                acceptance_policy: crate::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                semantic_revision: 1, id: id.into(), description: id.into(), dependencies,
                status: GoalRecordStatus::Completed, evidence_refs: vec![format!("evidence:{id}")],
            });
        }
        let requirement = state.requirements.get_mut("r").unwrap();
        requirement.status = GoalRecordStatus::Completed;
        requirement.evidence_refs = vec!["evidence:r".into()];
        goal
    }

    #[test]
    fn typed_requirement_change_invalidates_dependants_but_retains_evidence() {
        let mut goal = accepted_projection();
        let before = goal.active.as_ref().unwrap().clone();
        let mut detail = entry("new-detail", GoalWorkOrigin::Model);
        detail.kind = GoalWorkKind::RequirementDetail;
        goal.update_work("g", goal.revision, vec![detail], 2).unwrap();
        let restored: GoalProjection = serde_json::from_slice(&serde_json::to_vec(&goal).unwrap()).unwrap();
        let state = restored.active.unwrap();
        for id in ["r", "dependent"] {
            assert_eq!(state.requirements[id].status, GoalRecordStatus::Pending);
            assert_eq!(state.requirements[id].semantic_revision, before.requirements[id].semantic_revision + 1);
            assert_eq!(state.requirements[id].evidence_refs, before.requirements[id].evidence_refs);
        }
        assert_eq!(state.requirements["unrelated"], before.requirements["unrelated"]);
        assert_eq!(state.audit_reports, before.audit_reports);
        assert_eq!(state.requirement_acceptances, before.requirement_acceptances);
    }

    #[test]
    fn global_constraint_invalidates_all_but_progress_does_not() {
        let mut goal = accepted_projection();
        let requirements = goal.active.as_ref().unwrap().requirements.clone();
        let mut progress = entry("progress", GoalWorkOrigin::Model);
        progress.kind = GoalWorkKind::Progress;
        progress.progress = Some(GoalWorkProgress::InProgress);
        goal.update_work("g", goal.revision, vec![progress], 2).unwrap();
        assert_eq!(goal.active.as_ref().unwrap().requirements, requirements);
        let mut constraint = entry("constraint", GoalWorkOrigin::User);
        constraint.kind = GoalWorkKind::Constraint;
        constraint.requirement_ids.clear();
        goal.update_work("g", goal.revision, vec![constraint], 3).unwrap();
        assert!(goal.active.as_ref().unwrap().requirements.values().all(|r| r.status == GoalRecordStatus::Pending));
    }

    #[test]
    fn semantic_revision_overflow_rejects_entire_work_batch() {
        let mut goal = accepted_projection();
        goal.active.as_mut().unwrap().requirements.get_mut("dependent").unwrap().semantic_revision = u64::MAX;
        let before = goal.clone();
        let mut detail = entry("detail", GoalWorkOrigin::Model);
        detail.kind = GoalWorkKind::RequirementDetail;
        assert!(goal.update_work("g", goal.revision, vec![detail], 2).unwrap_err().contains("revision exhausted"));
        assert_eq!(goal, before);
    }

    #[test]
    fn invalid_batch_does_not_invalidate_acceptance_and_supersession_covers_old_scope() {
        let mut goal = accepted_projection();
        let mut detail = entry("detail", GoalWorkOrigin::Model);
        detail.kind = GoalWorkKind::RequirementDetail;
        let before = goal.clone();
        let mut invalid = detail.clone();
        invalid.id = "invalid".into();
        invalid.requirement_ids = vec!["missing".into()];
        assert!(goal.update_work("g", goal.revision, vec![detail.clone(), invalid], 2).is_err());
        assert_eq!(goal, before);
        goal.update_work("g", goal.revision, vec![detail.clone()], 2).unwrap();
        // Isolate a later acceptance of the first scope; superseding the record
        // with a different scope must invalidate both, not only the new scope.
        for r in goal.active.as_mut().unwrap().requirements.values_mut() {
            r.status = GoalRecordStatus::Completed;
        }
        detail.id = "replacement".into();
        detail.supersedes = Some("detail".into());
        detail.requirement_ids = vec!["unrelated".into()];
        goal.update_work("g", goal.revision, vec![detail], 3).unwrap();
        assert!(goal.active.as_ref().unwrap().requirements.values().all(|r| r.status == GoalRecordStatus::Pending));
    }
}
