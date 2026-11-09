// SPDX-License-Identifier: Apache-2.0
//! Structural contracts for immutable Core requirement/assessment events.
//! Validation proves neither reference existence nor provenance, authentication,
//! target freshness or natural-language satisfaction. Store must resolve those
//! facts in the result transaction. These types do not authorize sealing.
use crate::assistant_journal_operations::JournalNumber;
use crate::run_input_commitment::RunInputCommitment;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementDecisionRef {
    pub core_session_id: String,
    pub turn_id: String,
    pub revision: JournalNumber,
    pub event_cursor: JournalNumber,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalRequirementRef {
    pub goal_id: String,
    pub requirement_id: String,
    pub semantic_revision: JournalNumber,
}

/// An exact command reference, not a model claim that a user accepted work.
/// Authentication and ownership must be resolved against committed commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementUserCommandRef {
    pub conversation_id: String,
    pub command_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequirement {
    pub requirement_id: String,
    pub description: String,
    pub source_refs: Vec<String>,
    pub dependency_ids: Vec<String>,
    pub goal_requirement_ref: Option<GoalRequirementRef>,
    pub resource_scope: RequirementResourceScope,
}

/// A model's explicitly recorded interpretation, never user acceptance or
/// permission. Undeclared is not an assertion that the request has no resources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RequirementResourceScope {
    Undeclared,
    Declared {
        required: Vec<RequirementResourceClaim>,
        exclusions: Vec<RequirementResourceExclusion>,
        interpreted_user_frontier: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementResourceClaim {
    pub resource_uri: String,
    pub operation: String,
    pub version: Option<String>,
    pub predicate: Option<String>,
    /// An adopted review finding remains pending until its original failing
    /// check is followed by the same check on the repaired current resource.
    pub review: Option<RequirementReviewFinding>,
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementReviewFinding {
    /// Actual observations containing the adopted finding, not user authority.
    pub source_evidence: Vec<String>,
    /// May be absent while the reproducer is still being constructed.
    pub counterexample_evidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementResourceExclusion {
    pub resource_uri: String,
    pub reason: String,
    pub source_refs: Vec<String>,
    pub provenance: RequirementScopeInterpretationSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementScopeInterpretationSource {
    UserInputInterpretation,
    AssistantSuggestion,
}

impl RequirementResourceScope {
    /// Required only for an assessed action-completion claim, never for a reply.
    /// An absent or empty declaration cannot establish resource coverage.
    pub fn validate_action_coverage_declaration(&self) -> Result<(), String> {
        match self {
            Self::Declared { exclusions, .. } if exclusions.iter().any(|exclusion|
                exclusion.provenance == RequirementScopeInterpretationSource::AssistantSuggestion
            ) => Err("scope_coverage_unresolved: assistant-suggested exclusions are unresolved scope, not user exclusions. Reconcile those resources with the user's request before claiming completed; report incomplete if the scope remains unresolved. Do not relabel suggestions as user instructions without supporting input.".into()),
            Self::Declared { required, .. } if !required.is_empty() => Ok(()),
            _ => Err("scope_coverage_unknown: declare concrete required resources with task_scope_update before claiming completed; use answered for an unassessed reply or incomplete for unfinished work".into()),
        }
    }

    pub fn validate(&self, sources: &[String]) -> Result<(), String> {
        let Self::Declared { required, exclusions, interpreted_user_frontier } = self else { return Ok(()) };
        references("interpreted_user_frontier", interpreted_user_frontier, true)?;
        if required.len() > 256 || exclusions.len() > 256 { return Err("resource declaration exceeds limit".into()); }
        let check_sources = |refs: &[String]| -> Result<(), String> {
            references("resource source_refs", refs, true)?;
            if refs.iter().any(|id| !sources.contains(id)) { return Err("resource source outside requirement inputs".into()); }
            Ok(())
        };
        if interpreted_user_frontier.iter().any(|id| !sources.contains(id)) { return Err("resource frontier outside scope inputs".into()); }
        let mut identities = BTreeSet::new();
        for claim in required {
            identity("resource_uri", &claim.resource_uri)?;
            url::Url::parse(&claim.resource_uri).map_err(|_| "resource_uri must be an absolute URI (for local files use file:///...), not a filesystem path")?;
            identity("operation", &claim.operation)?;
            if !identities.insert((&claim.resource_uri, &claim.operation, &claim.version, &claim.predicate)) { return Err("duplicate resource claim".into()); }
            if let Some(version) = &claim.version { identity("version", version)?; }
            if let Some(predicate) = &claim.predicate { identity("predicate", predicate)?; }
            if let Some(review) = &claim.review {
                references("review.source_evidence", &review.source_evidence, true)?;
                if let Some(id) = &review.counterexample_evidence { identity("review.counterexample_evidence", id)?; }
            }
            check_sources(&claim.source_refs)?;
        }
        let mut excluded = BTreeSet::new();
        for exclusion in exclusions {
            identity("resource_uri", &exclusion.resource_uri)?;
            text("exclusion reason", &exclusion.reason)?;
            if !excluded.insert(&exclusion.resource_uri) || required.iter().any(|claim| claim.resource_uri == exclusion.resource_uri) {
                return Err("duplicate or contradictory resource exclusion".into());
            }
            check_sources(&exclusion.source_refs)?;
        }
        Ok(())
    }
}

fn preserves_review(previous: Option<&RequirementReviewFinding>, next: Option<&RequirementReviewFinding>) -> bool {
    match (previous, next) {
        (None, _) => true,
        (Some(old), Some(new)) => old.source_evidence.iter().all(|id| new.source_evidence.contains(id))
            && old.counterexample_evidence.as_ref().is_none_or(|id| new.counterexample_evidence.as_ref() == Some(id)),
        (Some(_), None) => false,
    }
}

/// Same-input rephrasing cannot silently narrow a previously declared scope.
/// New user input permits a new interpretation, not an assertion of acceptance.
pub fn validate_resource_scope_revision(previous: &[RunRequirement], next: &[RunRequirement], user_frontier: &[String]) -> Result<(), String> {
    let current: BTreeSet<_> = user_frontier.iter().collect();
    for requirement in next {
        if let RequirementResourceScope::Declared { interpreted_user_frontier, .. } = &requirement.resource_scope {
            let retained = previous.iter().any(|old| old.requirement_id == requirement.requirement_id && old.resource_scope == requirement.resource_scope);
            if !retained && interpreted_user_frontier.iter().collect::<BTreeSet<_>>() != current {
                return Err("new resource interpretation has stale user frontier".into());
            }
        }
    }
    for old in previous {
        if let RequirementResourceScope::Declared { required, interpreted_user_frontier, .. } = &old.resource_scope {
            if interpreted_user_frontier.iter().collect::<BTreeSet<_>>() != current { continue; }
            for claim in required {
                let preserved = next.iter().any(|requirement| match &requirement.resource_scope {
                    RequirementResourceScope::Declared { required, .. } => required.iter().any(|candidate|
                        candidate.resource_uri == claim.resource_uri && candidate.operation == claim.operation
                        && candidate.version == claim.version
                        && (candidate.predicate == claim.predicate || (claim.review.is_some() && claim.predicate.is_none()))
                        && preserves_review(claim.review.as_ref(), candidate.review.as_ref())),
                    RequirementResourceScope::Undeclared => false,
                });
                if !preserved { return Err("resource scope narrowed without new user input".into()); }
            }
        }
    }
    Ok(())
}

pub fn validate_resource_scope_retirement(old: &RunRequirement, next: &[RunRequirement], user_refs: &[String]) -> Result<(), String> {
    let RequirementResourceScope::Declared { required, interpreted_user_frontier, .. } = &old.resource_scope else { return Ok(()) };
    let removed = required.iter().any(|claim| !next.iter().any(|requirement| match &requirement.resource_scope {
        RequirementResourceScope::Declared { required, .. } => required.iter().any(|candidate|
            candidate.resource_uri == claim.resource_uri && candidate.operation == claim.operation
            && candidate.version == claim.version
            && (candidate.predicate == claim.predicate || (claim.review.is_some() && claim.predicate.is_none()))
            && preserves_review(claim.review.as_ref(), candidate.review.as_ref())),
        RequirementResourceScope::Undeclared => false,
    }));
    if removed && !user_refs.iter().any(|reference| !interpreted_user_frontier.contains(reference)) {
        return Err("narrowed resource scope must cite newly applied user input".into());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementSetRevision {
    pub conversation_id: String,
    pub run_id: String,
    pub revision: JournalNumber,
    pub input: RunInputCommitment,
    pub source_input_refs: Vec<String>,
    pub source_decision_ref: RequirementDecisionRef,
    pub requirements: Vec<RunRequirement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RequirementAssessmentMethod {
    ModelJudgment {},
    EvidenceInspection {
        target_version_refs: Vec<String>,
    },
    UserAcceptance {
        user_command_ref: RequirementUserCommandRef,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementVerdict {
    Satisfied,
    Unsatisfied,
    InsufficientEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementAssessment {
    pub assessment_id: String,
    pub run_id: String,
    pub requirement_id: String,
    pub requirement_set_revision: JournalNumber,
    pub assessed_input: RunInputCommitment,
    pub evidence_frontier: JournalNumber,
    pub source_decision_ref: RequirementDecisionRef,
    pub method: RequirementAssessmentMethod,
    pub evidence_refs: Vec<String>,
    pub verdict: RequirementVerdict,
    pub rationale: String,
}

impl RequirementDecisionRef {
    fn validate(&self) -> Result<(), String> {
        identity("decision.core_session_id", &self.core_session_id)?;
        identity("decision.turn_id", &self.turn_id)?;
        number("decision.revision", self.revision, true)?;
        number("decision.event_cursor", self.event_cursor, true)
    }
}

impl RequirementSetRevision {
    pub fn validate(&self) -> Result<(), String> {
        identity("conversation_id", &self.conversation_id)?;
        identity("run_id", &self.run_id)?;
        number("revision", self.revision, true)?;
        self.input.validate()?;
        references("source_input_refs", &self.source_input_refs, true)?;
        self.source_decision_ref.validate()?;
        if self.requirements.is_empty() {
            return Err("requirement set must retain its accepted scope".into());
        }
        let mut pending = BTreeMap::new();
        for requirement in &self.requirements {
            identity("requirement_id", &requirement.requirement_id)?;
            text("description", &requirement.description)?;
            references("source_refs", &requirement.source_refs, true)?;
            references("dependency_ids", &requirement.dependency_ids, false)?;
            requirement.resource_scope.validate(&self.source_input_refs)?;
            if pending
                .insert(
                    requirement.requirement_id.as_str(),
                    requirement.dependency_ids.len(),
                )
                .is_some()
            {
                return Err("duplicate requirement identity".into());
            }
            if let Some(goal) = &requirement.goal_requirement_ref {
                identity("goal_id", &goal.goal_id)?;
                identity("goal.requirement_id", &goal.requirement_id)?;
                number("goal.semantic_revision", goal.semantic_revision, true)?;
            }
        }
        let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for requirement in &self.requirements {
            for dependency in &requirement.dependency_ids {
                if !pending.contains_key(dependency.as_str()) {
                    return Err("requirement dependency does not exist in this revision".into());
                }
                dependents
                    .entry(dependency)
                    .or_default()
                    .push(&requirement.requirement_id);
            }
        }
        // Iterative topological validation avoids recursion on caller-sized DAGs.
        let mut ready: VecDeque<_> = pending
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(id, _)| *id)
            .collect();
        let mut visited = 0;
        while let Some(id) = ready.pop_front() {
            visited += 1;
            if let Some(children) = dependents.get(id) {
                for child in children {
                    let count = pending.get_mut(child).expect("validated dependency graph");
                    *count -= 1;
                    if *count == 0 {
                        ready.push_back(*child);
                    }
                }
            }
        }
        if visited != pending.len() {
            return Err("requirement dependencies contain a cycle".into());
        }
        Ok(())
    }
}

impl RequirementAssessment {
    /// Verdict classification only; not proof of evidence or authority to seal.
    pub fn is_satisfied(&self) -> bool {
        self.verdict == RequirementVerdict::Satisfied
    }

    pub fn validate(&self) -> Result<(), String> {
        identity("assessment_id", &self.assessment_id)?;
        identity("run_id", &self.run_id)?;
        identity("requirement_id", &self.requirement_id)?;
        number(
            "requirement_set_revision",
            self.requirement_set_revision,
            true,
        )?;
        self.assessed_input.validate()?;
        number("evidence_frontier", self.evidence_frontier, false)?;
        self.source_decision_ref.validate()?;
        references("evidence_refs", &self.evidence_refs, false)?;
        text("rationale", &self.rationale)?;
        match &self.method {
            RequirementAssessmentMethod::ModelJudgment {} => {}
            RequirementAssessmentMethod::EvidenceInspection {
                target_version_refs,
            } => {
                references("target_version_refs", target_version_refs, true)?;
                references("inspection.evidence_refs", &self.evidence_refs, true)?;
            }
            RequirementAssessmentMethod::UserAcceptance { user_command_ref } => {
                identity(
                    "user_command.conversation_id",
                    &user_command_ref.conversation_id,
                )?;
                identity("user_command.command_id", &user_command_ref.command_id)?;
                if self.verdict != RequirementVerdict::Satisfied {
                    return Err(
                        "user acceptance must represent an explicit satisfied judgment".into(),
                    );
                }
            }
        }
        Ok(())
    }

    /// Compares the supplied immutable scope, not whatever happens to be latest.
    /// This still cannot prove either object is committed or its references real.
    pub fn validate_against(&self, set: &RequirementSetRevision) -> Result<(), String> {
        self.validate()?;
        set.validate()?;
        if self.run_id != set.run_id
            || self.requirement_set_revision != set.revision
            || self.assessed_input != set.input
        {
            return Err(
                "assessment does not match the exact Run scope revision and input watermark".into(),
            );
        }
        if !set
            .requirements
            .iter()
            .any(|requirement| requirement.requirement_id == self.requirement_id)
        {
            return Err("assessed requirement does not exist in this revision".into());
        }
        if self.source_decision_ref.core_session_id != set.source_decision_ref.core_session_id
            || self.source_decision_ref.revision.0 < set.source_decision_ref.revision.0
            || self.source_decision_ref.event_cursor.0 < set.source_decision_ref.event_cursor.0
        {
            return Err(
                "assessment decision predates or belongs outside its accepted scope".into(),
            );
        }
        if self.source_decision_ref.event_cursor == set.source_decision_ref.event_cursor
            && self.source_decision_ref != set.source_decision_ref
        {
            return Err(
                "one Core event cursor cannot identify different decision envelopes".into(),
            );
        }
        if let RequirementAssessmentMethod::UserAcceptance { user_command_ref } = &self.method {
            if user_command_ref.conversation_id != set.conversation_id {
                return Err("user command belongs to another conversation".into());
            }
        }
        Ok(())
    }
}

fn identity(field: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err(format!("{field} must be a nonempty exact identity"));
    }
    Ok(())
}
fn text(field: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    Ok(())
}
fn number(field: &str, value: JournalNumber, positive: bool) -> Result<(), String> {
    if value.0 > i64::MAX as u64 || (positive && value.0 == 0) {
        return Err(format!("{field} is outside its durable range"));
    }
    Ok(())
}
fn references(field: &str, values: &[String], required: bool) -> Result<(), String> {
    if required && values.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    let mut seen = BTreeSet::new();
    for value in values {
        identity(field, value)?;
        if !seen.insert(value) {
            return Err(format!("{field} contains duplicate references"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adopted_review_cannot_be_dropped_or_its_counterexample_replaced_on_same_input() {
        let mut old = serde_json::from_value::<RunRequirement>(serde_json::json!({
            "requirement_id":"review","description":"Fix lifecycle defect","source_refs":["input"],"dependency_ids":[],
            "goal_requirement_ref":null,"resource_scope":{"kind":"declared","required":[{
                "resource_uri":"file:///artifact","operation":"write","predicate":"case-1","source_refs":["input"],
                "review":{"source_evidence":["review-observation"],"counterexample_evidence":null}}],
                "exclusions":[],"interpreted_user_frontier":["input"]}})).unwrap();
        let mut next = old.clone();
        if let RequirementResourceScope::Declared { required, .. } = &mut next.resource_scope {
            required[0].review.as_mut().unwrap().counterexample_evidence = Some("failed-case".into());
        }
        assert!(validate_resource_scope_revision(&[old.clone()], &[next.clone()], &["input".into()]).is_ok());
        old = next;
        for review in [None, Some(RequirementReviewFinding { source_evidence: vec!["other".into()], counterexample_evidence: Some("failed-case".into()) }),
            Some(RequirementReviewFinding { source_evidence: vec!["review-observation".into()], counterexample_evidence: Some("weaker-case".into()) })] {
            let mut weakened = old.clone();
            if let RequirementResourceScope::Declared { required, .. } = &mut weakened.resource_scope { required[0].review = review; }
            assert!(validate_resource_scope_revision(&[old.clone()], &[weakened.clone()], &["input".into()]).is_err());
            assert!(validate_resource_scope_retirement(&old, &[weakened], &["input".into()]).is_err());
        }
        let mut pending = old.clone();
        if let RequirementResourceScope::Declared { required, .. } = &mut pending.resource_scope {
            required[0].predicate = None;
            required[0].review.as_mut().unwrap().counterexample_evidence = None;
        }
        assert!(validate_resource_scope_revision(&[pending.clone()], &[old.clone()], &["input".into()]).is_ok());
        assert!(validate_resource_scope_retirement(&pending, &[old.clone()], &["input".into()]).is_ok());
        let mut invalid = old.resource_scope.clone();
        if let RequirementResourceScope::Declared { required, .. } = &mut invalid { required[0].resource_uri = "/path/is/not/a/uri".into(); }
        assert!(invalid.validate(&["input".into()]).is_err());
        let mut invalid = old.resource_scope.clone();
        if let RequirementResourceScope::Declared { required, .. } = &mut invalid { required[0].review.as_mut().unwrap().source_evidence.clear(); }
        assert!(invalid.validate(&["input".into()]).is_err());
    }

    #[test]
    fn assistant_exclusions_can_be_recorded_but_cannot_prove_action_completion() {
        let mut declaration = RequirementResourceScope::Declared {
            required: vec![RequirementResourceClaim {
                resource_uri: "file:///requested".into(), operation: "export".into(),
                version: None, predicate: None, review: None, source_refs: vec!["input-3".into()],
            }],
            exclusions: vec![RequirementResourceExclusion {
                resource_uri: "file:///appendix".into(), reason: "not recommended".into(),
                source_refs: vec!["input-3".into()],
                provenance: RequirementScopeInterpretationSource::AssistantSuggestion,
            }],
            interpreted_user_frontier: vec!["input-3".into()],
        };
        assert!(declaration.validate(&["input-3".into()]).is_ok());
        assert!(declaration.validate_action_coverage_declaration().unwrap_err().contains("scope_coverage_unresolved"));
        // An explicitly interpreted user exclusion has a different contract.
        // This checks provenance, not the truth of the model's interpretation.
        if let RequirementResourceScope::Declared { exclusions, .. } = &mut declaration {
            exclusions[0].provenance = RequirementScopeInterpretationSource::UserInputInterpretation;
        }
        assert!(declaration.validate_action_coverage_declaration().is_ok());
    }

    #[test]
    fn declared_resource_scope_cannot_shrink_on_same_input() {
        let mut old = scope().requirements.remove(0);
        let claim = RequirementResourceClaim { resource_uri:"file:///a".into(),operation:"read".into(),version:None,predicate:None,review:None,source_refs:vec!["input-3".into()] };
        old.resource_scope = RequirementResourceScope::Declared { required:vec![claim],exclusions:vec![],interpreted_user_frontier:vec!["input-3".into()] };
        let mut changed=old.clone();
        changed.resource_scope=RequirementResourceScope::Undeclared;
        assert!(validate_resource_scope_revision(&[old.clone()],&[changed.clone()],&["input-3".into()]).is_err());
        assert!(validate_resource_scope_revision(&[old.clone()],&[old.clone()],&["input-3".into()]).is_ok());
        assert!(validate_resource_scope_retirement(&old,&[changed.clone()],&["input-3".into()]).is_err());
        assert!(validate_resource_scope_retirement(&old,&[changed.clone()],&["input-4".into()]).is_ok());
        assert!(validate_resource_scope_revision(&[old],&[changed],&["input-3".into(),"input-4".into()]).is_ok());
    }

    #[test]
    fn resource_scope_is_required_and_rejects_contradictory_claims() {
        let mut value=serde_json::to_value(scope()).unwrap();
        value["requirements"][0].as_object_mut().unwrap().remove("resource_scope");
        assert!(serde_json::from_value::<RequirementSetRevision>(value).is_err());
        let scope=RequirementResourceScope::Declared { required:vec![RequirementResourceClaim { resource_uri:"file:///a".into(),operation:"read".into(),version:None,predicate:None,review:None,source_refs:vec!["input-3".into()] }],
            exclusions:vec![RequirementResourceExclusion {resource_uri:"file:///a".into(),reason:"suggestion".into(),source_refs:vec!["input-3".into()],provenance:RequirementScopeInterpretationSource::AssistantSuggestion}],interpreted_user_frontier:vec!["input-3".into()] };
        assert!(scope.validate(&["input-3".into()]).is_err());
    }
    use serde_json::json;

    fn scope() -> RequirementSetRevision {
        RequirementSetRevision {
            conversation_id: "conversation".into(),
            run_id: "run".into(),
            revision: JournalNumber(1),
            input: RunInputCommitment { watermark: JournalNumber(3), applications_digest: crate::assistant_journal::AssistantDigest::parse_hex(&"a".repeat(64)).unwrap() },
            source_input_refs: vec!["input-3".into()],
            source_decision_ref: RequirementDecisionRef {
                core_session_id: "session".into(),
                turn_id: "turn".into(),
                revision: JournalNumber(4),
                event_cursor: JournalNumber(5),
            },
            requirements: vec![RunRequirement {
                requirement_id: "requirement".into(),
                description: "The user's actual scope".into(),
                source_refs: vec!["input-3".into()],
                dependency_ids: vec![],
                goal_requirement_ref: None,
                resource_scope: RequirementResourceScope::Undeclared,
            }],
        }
    }
    fn assessment() -> RequirementAssessment {
        RequirementAssessment {
            assessment_id: "assessment".into(),
            run_id: "run".into(),
            requirement_id: "requirement".into(),
            requirement_set_revision: JournalNumber(1),
            assessed_input: RunInputCommitment { watermark: JournalNumber(3), applications_digest: crate::assistant_journal::AssistantDigest::parse_hex(&"a".repeat(64)).unwrap() },
            evidence_frontier: JournalNumber(0),
            source_decision_ref: scope().source_decision_ref,
            method: RequirementAssessmentMethod::ModelJudgment {},
            evidence_refs: vec![],
            verdict: RequirementVerdict::Satisfied,
            rationale: "Semantic assessment, not independent verification".into(),
        }
    }

    #[test]
    fn ordinary_scope_needs_no_goal_or_extra_verification_method() {
        let set = scope();
        set.validate().unwrap();
        assessment().validate_against(&set).unwrap();
        let wire = serde_json::to_value(&set).unwrap();
        assert!(wire["requirements"][0]["goal_requirement_ref"].is_null());
        assert_eq!(
            serde_json::from_value::<RequirementSetRevision>(wire).unwrap(),
            set
        );
        for verdict in [
            RequirementVerdict::Satisfied,
            RequirementVerdict::Unsatisfied,
            RequirementVerdict::InsufficientEvidence,
        ] {
            let mut value = assessment();
            value.verdict = verdict;
            value.validate_against(&set).unwrap();
            assert_eq!(
                serde_json::from_value::<RequirementAssessment>(
                    serde_json::to_value(&value).unwrap()
                )
                .unwrap(),
                value
            );
        }
    }

    #[test]
    fn dependencies_must_exist_be_unique_and_form_a_dag() {
        let mut set = scope();
        let mut second = set.requirements[0].clone();
        second.requirement_id = "second".into();
        second.dependency_ids = vec!["requirement".into()];
        set.requirements.push(second);
        set.validate().unwrap();
        let mut invalid = set.clone();
        invalid.requirements[0].dependency_ids = vec!["second".into()];
        assert!(invalid.validate().is_err());
        let mut invalid = set.clone();
        invalid.requirements[0].dependency_ids = vec!["requirement".into()];
        assert!(invalid.validate().is_err());
        let mut invalid = set.clone();
        invalid.requirements[1].dependency_ids = vec!["missing".into()];
        assert!(invalid.validate().is_err());
        let mut invalid = set.clone();
        invalid.requirements[1]
            .dependency_ids
            .push("requirement".into());
        assert!(invalid.validate().is_err());
        let duplicate = set.requirements[0].clone();
        set.requirements.push(duplicate);
        assert!(set.validate().is_err());
    }

    #[test]
    fn scope_identity_sources_and_semantic_goal_version_cannot_be_omitted() {
        for field in ["conversation_id", "run_id"] {
            let mut wire = serde_json::to_value(scope()).unwrap();
            wire[field] = json!(" ");
            assert!(serde_json::from_value::<RequirementSetRevision>(wire)
                .unwrap()
                .validate()
                .is_err());
        }
        let mut set = scope();
        set.source_input_refs.clear();
        assert!(set.validate().is_err());
        let mut set = scope();
        set.source_input_refs.push("input-3".into());
        assert!(set.validate().is_err());
        let mut set = scope();
        set.requirements[0].source_refs.clear();
        assert!(set.validate().is_err());
        let mut set = scope();
        set.requirements[0].description = " \n".into();
        assert!(set.validate().is_err());
        let mut set = scope();
        set.requirements.clear();
        assert!(set.validate().is_err());
        let mut set = scope();
        set.source_decision_ref.turn_id = "bad\nid".into();
        assert!(set.validate().is_err());
        let mut set = scope();
        set.requirements[0].goal_requirement_ref = Some(GoalRequirementRef {
            goal_id: "goal".into(),
            requirement_id: "goal-requirement".into(),
            semantic_revision: JournalNumber(0),
        });
        assert!(set.validate().is_err());
        set.requirements[0]
            .goal_requirement_ref
            .as_mut()
            .unwrap()
            .semantic_revision = JournalNumber(1);
        set.validate().unwrap(); // Does not prove the Goal revision exists.
    }

    #[test]
    fn all_methods_keep_distinct_evidence_and_command_requirements() {
        let mut value = assessment();
        value.method = RequirementAssessmentMethod::EvidenceInspection {
            target_version_refs: vec!["target-v1".into()],
        };
        assert!(value.validate().is_err());
        value.evidence_refs = vec!["check-receipt".into()];
        value.evidence_frontier = JournalNumber(6);
        for verdict in [
            RequirementVerdict::Satisfied,
            RequirementVerdict::Unsatisfied,
            RequirementVerdict::InsufficientEvidence,
        ] {
            value.verdict = verdict;
            value.validate_against(&scope()).unwrap();
        }
        value.method = RequirementAssessmentMethod::EvidenceInspection {
            target_version_refs: vec![],
        };
        assert!(value.validate().is_err());
        value.method = RequirementAssessmentMethod::EvidenceInspection {
            target_version_refs: vec!["v".into(), "v".into()],
        };
        assert!(value.validate().is_err());
        value.method = RequirementAssessmentMethod::UserAcceptance {
            user_command_ref: RequirementUserCommandRef {
                conversation_id: "conversation".into(),
                command_id: "user-command".into(),
            },
        };
        assert!(value.validate().is_err()); // Insufficient evidence is not acceptance.
        value.verdict = RequirementVerdict::Unsatisfied;
        assert!(value.validate().is_err());
        value.verdict = RequirementVerdict::Satisfied;
        value.validate_against(&scope()).unwrap();
        if let RequirementAssessmentMethod::UserAcceptance { user_command_ref } = &mut value.method
        {
            user_command_ref.conversation_id = "other".into();
        }
        assert!(value.validate_against(&scope()).is_err());
        if let RequirementAssessmentMethod::UserAcceptance { user_command_ref } = &mut value.method
        {
            user_command_ref.command_id = "".into();
        }
        assert!(value.validate().is_err());
    }

    #[test]
    fn count_only_requirement_wires_are_not_accepted() {
        let mut set = serde_json::to_value(scope()).unwrap();
        set.as_object_mut().unwrap().remove("input");
        set["input_watermark"] = json!("3");
        assert!(serde_json::from_value::<RequirementSetRevision>(set).is_err());
        let mut value = serde_json::to_value(assessment()).unwrap();
        value.as_object_mut().unwrap().remove("assessed_input");
        value["assessed_input_watermark"] = json!("3");
        assert!(serde_json::from_value::<RequirementAssessment>(value).is_err());
    }

    #[test]
    fn assessment_requires_exact_scope_not_old_or_future_input() {
        let set = scope();
        let mut same_count_other_content = assessment();
        same_count_other_content.assessed_input.applications_digest =
            crate::assistant_journal::AssistantDigest::parse_hex(&"b".repeat(64)).unwrap();
        assert_eq!(same_count_other_content.assessed_input.watermark, set.input.watermark);
        assert!(same_count_other_content.validate_against(&set).is_err());
        for revision in [0, 2, u64::MAX] {
            let mut value = assessment();
            value.requirement_set_revision = JournalNumber(revision);
            assert!(value.validate_against(&set).is_err());
        }
        for watermark in [2, 4, u64::MAX] {
            let mut value = assessment();
            value.assessed_input.watermark = JournalNumber(watermark);
            assert!(value.validate_against(&set).is_err());
        }
        for field in ["run_id", "requirement_id"] {
            let mut wire = serde_json::to_value(assessment()).unwrap();
            wire[field] = json!("other");
            assert!(serde_json::from_value::<RequirementAssessment>(wire)
                .unwrap()
                .validate_against(&set)
                .is_err());
        }
        let mut value = assessment();
        value.source_decision_ref.revision = JournalNumber(3);
        assert!(value.validate_against(&set).is_err());
        let mut value = assessment();
        value.source_decision_ref.event_cursor = JournalNumber(4);
        assert!(value.validate_against(&set).is_err());
        let mut value = assessment();
        value.source_decision_ref.core_session_id = "other".into();
        assert!(value.validate_against(&set).is_err());
        let mut value = assessment();
        value.source_decision_ref.turn_id = "later-turn".into();
        value.source_decision_ref.event_cursor = JournalNumber(6);
        value.source_decision_ref.revision = JournalNumber(5);
        value.validate_against(&set).unwrap(); // Same Run may cite a later Turn; Store resolves ownership.
        let mut value = assessment();
        value.evidence_refs = vec!["fact".into(), "fact".into()];
        assert!(value.validate().is_err());
        let mut value = assessment();
        value.rationale = " ".into();
        assert!(value.validate().is_err());
    }

    #[test]
    fn identical_decision_cursor_requires_identical_envelope() {
        let set = scope();
        let mut value = assessment();
        value.source_decision_ref.turn_id = "another-turn".into();
        assert!(value.validate_against(&set).is_err());
        let mut value = assessment();
        value.source_decision_ref.revision = JournalNumber(5);
        assert!(value.validate_against(&set).is_err());
        value.source_decision_ref.event_cursor = JournalNumber(6);
        value.validate_against(&set).unwrap();
    }

    #[test]
    fn wire_rejects_missing_method_and_implicit_success_and_keeps_large_frontiers() {
        let mut value = assessment();
        value.evidence_frontier = JournalNumber(9_007_199_254_740_993);
        let wire = serde_json::to_value(&value).unwrap();
        assert_eq!(wire["evidence_frontier"], json!("9007199254740993"));
        assert_eq!(
            serde_json::from_value::<RequirementAssessment>(wire.clone()).unwrap(),
            value
        );
        for field in ["method", "verdict", "source_decision_ref"] {
            let mut missing = wire.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<RequirementAssessment>(missing).is_err());
        }
        let mut unknown = wire.clone();
        unknown["accepted"] = json!(true);
        assert!(serde_json::from_value::<RequirementAssessment>(unknown).is_err());
        let mut mixed = wire.clone();
        mixed["method"] = json!({"kind":"model_judgment","user_command_ref":{"conversation_id":"c","command_id":"u"}});
        assert!(serde_json::from_value::<RequirementAssessment>(mixed).is_err());
        let mut numeric = wire;
        numeric["evidence_frontier"] = json!(9_007_199_254_740_993u64);
        assert!(serde_json::from_value::<RequirementAssessment>(numeric).is_err());
        value.evidence_frontier = JournalNumber(u64::MAX);
        assert!(value.validate().is_err());
        let mut set = scope();
        set.revision = JournalNumber(0);
        assert!(set.validate().is_err());
        set.revision = JournalNumber(u64::MAX);
        assert!(set.validate().is_err());
        let mut set = scope();
        set.source_decision_ref.event_cursor = JournalNumber(0);
        assert!(set.validate().is_err());
    }
}
