// SPDX-License-Identifier: Apache-2.0
//! Shared creation contract for every Goal admission adapter.
use super::{GoalRuntimeCommandEnvelope, GoalRuntimeCommand, GoalRequirement, GoalRecordStatus, GoalAcceptancePolicy};

/// Shared creation contract. A caller nonce identifies one user Start action;
/// conversation identity never substitutes for a new Goal operation identity.
pub fn create_goal_command(nonce: &str, objective: &str, constraints: &str, now_epoch_ms: i64) -> GoalRuntimeCommandEnvelope {
    let id = format!("goal-{nonce}");
    GoalRuntimeCommandEnvelope { operation_id: format!("{id}:start"), command: GoalRuntimeCommand::Start {
        id, objective: objective.into(), constraints: constraints.lines().map(str::trim)
            .filter(|line| !line.is_empty()).map(str::to_string).collect(),
        requirements: goal_requirement_records(objective, constraints), now_epoch_ms,
    }}
}

pub fn goal_requirement_records(goal_target: &str, goal_requirements: &str) -> Vec<GoalRequirement> {
    let mut descriptions = vec![format!("The goal is fully achieved: {goal_target}")];
    descriptions.extend(
        goal_requirements
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string),
    );
    descriptions
        .into_iter()
        .enumerate()
        .map(|(index, description)| GoalRequirement {
            acceptance_policy: GoalAcceptancePolicy::ModelJudgment,
            semantic_revision: 1, id: format!("requirement-{}", index + 1),
            description,
            dependencies: Vec::new(),
            status: GoalRecordStatus::Pending,
            evidence_refs: Vec::new(),
        })
        .collect()
}
