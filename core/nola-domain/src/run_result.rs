// SPDX-License-Identifier: Apache-2.0
//! Immutable Run result proposal. Structural validation grants no evidence credit.
//! Store must validate referenced ownership/revisions in the sealing transaction.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use crate::assistant_journal_operations::JournalNumber;
use crate::assistant_journal::AssistantDigest;
use crate::run_input_commitment::RunInputCommitment;
use sha2::{Digest, Sha256};

/// A result is distinct from request termination, Part closure and a live wait.
/// No default outcome: old or incomplete payloads must not imply success.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunResult {
    pub result_id: String,
    pub run_id: String,
    pub conversation_id: String,
    pub originating_submission_id: String,
    pub input: RunInputCommitment,
    pub decided_at_revision: JournalNumber,
    pub decision_event_cursor: JournalNumber,
    pub requirement_revision: JournalNumber,
    pub evidence_frontier: JournalNumber,
    pub value: RunResultValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunResultValue {
    Answered {
        answer_part_refs: Vec<String>,
    },
    /// Explicit user delegation command admitted a durable child Task; not child completion.
    DelegationAccepted { task_id: String, parent_call_id: String },
    ActionCompleted {
        requirement_acceptance_refs: Vec<String>,
    },
    Incomplete {
        reason: RunResultReason,
        remaining_requirement_refs: Vec<String>,
        obligation_refs: Vec<String>,
    },
    Blocked {
        reason: RunResultReason,
        blocking_fact_refs: Vec<String>,
        resolution_condition: String,
    },
    Cancelled {
        cancellation_id: String,
        observed_effect_refs: Vec<String>,
        unknown_effect_refs: Vec<String>,
    },
    Failed {
        error_ref: String,
        observed_effect_refs: Vec<String>,
        unknown_effect_refs: Vec<String>,
    },
}

/// Stable reason code and optional explanation, never a localized status label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunResultReason {
    pub code: String,
    pub detail: Option<String>,
}

impl RunResult {
    /// Versioned, fixed-field-order JSON. Set-valued references are sorted;
    /// answer Parts retain presentation order. Never hash caller-provided JSON.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut canonical = self.clone();
        match &mut canonical.value {
            RunResultValue::Answered { .. } | RunResultValue::DelegationAccepted { .. } => {},
            RunResultValue::ActionCompleted { requirement_acceptance_refs } => {
                requirement_acceptance_refs.sort();
            }
            RunResultValue::Incomplete { remaining_requirement_refs, obligation_refs, .. } => {
                remaining_requirement_refs.sort();
                obligation_refs.sort();
            }
            RunResultValue::Blocked { blocking_fact_refs, .. } => blocking_fact_refs.sort(),
            RunResultValue::Cancelled { observed_effect_refs, unknown_effect_refs, .. }
            | RunResultValue::Failed { observed_effect_refs, unknown_effect_refs, .. } => {
                observed_effect_refs.sort();
                unknown_effect_refs.sort();
            }
        }
        serde_json::to_vec(&canonical).map_err(|error| error.to_string())
    }

    /// A content digest, not a signature or permission to seal. The future seal
    /// must bind this digest to its committed identity/frontiers atomically.
    pub fn computed_digest(&self) -> Result<AssistantDigest, String> {
        let bytes = self.canonical_bytes()?;
        let length = u64::try_from(bytes.len()).map_err(|_| "Run result length overflow")?;
        let mut hash = Sha256::new();
        hash.update(b"NOLA-RUN-RESULT-V1");
        hash.update(length.to_be_bytes());
        hash.update(bytes);
        AssistantDigest::parse_hex(&format!("{:x}", hash.finalize())).map_err(|error| error.to_string())
    }

    /// Checks only payload structure. A valid proposal still requires committed
    /// evidence, exact ownership, matching versions and legal state to seal.
    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [
            ("result_id", self.result_id.as_str()),
            ("run_id", self.run_id.as_str()),
            ("conversation_id", self.conversation_id.as_str()),
            ("originating_submission_id", self.originating_submission_id.as_str()),
        ] {
            identity(field, value)?;
        }
        // Zero is a valid empty evidence/requirement frontier. It does not prove
        // that a referenced acceptance or decision exists; Store checks that.
        self.input.validate()?;
        for (field, value) in [
            ("decided_at_revision", self.decided_at_revision),
            ("decision_event_cursor", self.decision_event_cursor),
            ("requirement_revision", self.requirement_revision),
            ("evidence_frontier", self.evidence_frontier),
        ] {
            if value.0 > i64::MAX as u64 {
                return Err(format!("{field} exceeds durable integer range"));
            }
        }
        self.value.validate()
    }
}

impl RunResult {
    /// Structural coverage gate for an action result, not a source lookup.
    /// The sealing transaction must supply the authoritative current set and
    /// assessments and independently verify their committed evidence. In
    /// particular, a historical Satisfied assessment must not hide a newer
    /// contradiction; this method cannot discover omitted database records.
    pub fn validate_action_requirement_coverage(
        &self,
        requirements: &crate::run_requirements::RequirementSetRevision,
        assessments: &[crate::run_requirements::RequirementAssessment],
    ) -> Result<(), String> {
        self.validate()?;
        requirements.validate()?;
        let RunResultValue::ActionCompleted { requirement_acceptance_refs } = &self.value else {
            return Err("action coverage requires an action_completed result".into());
        };
        for requirement in &requirements.requirements {
            requirement.resource_scope.validate_action_coverage_declaration()?;
        }
        if self.run_id != requirements.run_id
            || self.conversation_id != requirements.conversation_id
            || self.requirement_revision != requirements.revision
            || self.input != requirements.input
        {
            return Err("action result differs from its exact requirement scope".into());
        }
        if requirements.source_decision_ref.revision.0 > self.decided_at_revision.0
            || requirements.source_decision_ref.event_cursor.0 > self.decision_event_cursor.0
        {
            return Err("requirement source is beyond the result decision frontier".into());
        }
        let mut available = std::collections::BTreeMap::new();
        let mut latest = std::collections::BTreeMap::new();
        let mut positions = BTreeSet::new();
        let mut source_envelopes = std::collections::BTreeMap::new();
        for assessment in assessments {
            assessment.validate_against(requirements)?;
            if assessment.evidence_frontier.0 > self.evidence_frontier.0
                || assessment.source_decision_ref.revision.0 > self.decided_at_revision.0
                || assessment.source_decision_ref.event_cursor.0 > self.decision_event_cursor.0
                || assessment.source_decision_ref.core_session_id != requirements.source_decision_ref.core_session_id
            {
                return Err("assessment is outside the result source/frontier".into());
            }
            if available.insert(assessment.assessment_id.as_str(), assessment).is_some() {
                return Err("duplicate assessment identity".into());
            }
            let cursor = assessment.source_decision_ref.event_cursor.0;
            if !positions.insert((assessment.requirement_id.as_str(), cursor)) {
                return Err("one requirement has ambiguous assessments at the same decision cursor".into());
            }
            if let Some(previous) = source_envelopes.insert(cursor, &assessment.source_decision_ref) {
                if previous != &assessment.source_decision_ref {
                    return Err("one Core event cursor cannot identify different decision envelopes".into());
                }
            }
            let current = latest.entry(assessment.requirement_id.as_str()).or_insert(assessment);
            if cursor > current.source_decision_ref.event_cursor.0 { *current = assessment; }
        }
        let mut previous_revision = requirements.source_decision_ref.revision.0;
        for source in source_envelopes.values() {
            if source.revision.0 < previous_revision {
                return Err("Core decision revisions decrease as event cursors advance".into());
            }
            previous_revision = source.revision.0;
        }
        let mut covered = BTreeSet::new();
        for reference in requirement_acceptance_refs {
            let assessment = available.get(reference.as_str())
                .ok_or("action acceptance reference is missing")?;
            if !assessment.is_satisfied() {
                return Err("action acceptance must be satisfied".into());
            }
            if latest[assessment.requirement_id.as_str()].assessment_id != assessment.assessment_id {
                return Err("action acceptance is superseded within the supplied assessment set".into());
            }
            if !covered.insert(assessment.requirement_id.as_str()) {
                return Err("action result repeats acceptance of one requirement".into());
            }
        }
        let required: BTreeSet<_> = requirements.requirements.iter()
            .map(|requirement| requirement.requirement_id.as_str()).collect();
        if covered != required {
            return Err("action result must cover every requirement, not a successful subset".into());
        }
        Ok(())
    }
}

impl RunResultValue {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::DelegationAccepted { task_id, parent_call_id } => {
                references("task_id", std::slice::from_ref(task_id), true)?;
                references("parent_call_id", std::slice::from_ref(parent_call_id), true)
            }
            Self::Answered { answer_part_refs } => references("answer_part_refs", answer_part_refs, true),
            Self::ActionCompleted { requirement_acceptance_refs } => {
                references("requirement_acceptance_refs", requirement_acceptance_refs, true)
            }
            Self::Incomplete { reason, remaining_requirement_refs, obligation_refs } => {
                reason.validate()?;
                references("remaining_requirement_refs", remaining_requirement_refs, false)?;
                references("obligation_refs", obligation_refs, false)?;
                if remaining_requirement_refs.is_empty() && obligation_refs.is_empty() {
                    return Err("incomplete result must identify remaining work".into());
                }
                Ok(())
            }
            Self::Blocked { reason, blocking_fact_refs, resolution_condition } => {
                reason.validate()?;
                references("blocking_fact_refs", blocking_fact_refs, true)?;
                if resolution_condition.trim().is_empty() {
                    return Err("blocked result requires a resolution condition".into());
                }
                Ok(())
            }
            Self::Cancelled { cancellation_id, observed_effect_refs, unknown_effect_refs } => {
                identity("cancellation_id", cancellation_id)?;
                effect_references(observed_effect_refs, unknown_effect_refs)
            }
            Self::Failed { error_ref, observed_effect_refs, unknown_effect_refs } => {
                identity("error_ref", error_ref)?;
                effect_references(observed_effect_refs, unknown_effect_refs)
            }
        }
    }
}

impl RunResultReason {
    fn validate(&self) -> Result<(), String> {
        if self.code.is_empty() || !self.code.bytes().all(|byte|
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'.')
        {
            return Err("result reason requires a stable lowercase code".into());
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

fn references(field: &str, values: &[String], required: bool) -> Result<(), String> {
    if required && values.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    let mut seen = BTreeSet::new();
    for value in values {
        identity(field, value)?;
        if !seen.insert(value) {
            return Err(format!("{field} contains a duplicate reference"));
        }
    }
    Ok(())
}

fn effect_references(observed: &[String], unknown: &[String]) -> Result<(), String> {
    references("observed_effect_refs", observed, false)?;
    references("unknown_effect_refs", unknown, false)?;
    // These refer to effects, not to individual observations of an effect.
    let observed: BTreeSet<_> = observed.iter().collect();
    if unknown.iter().any(|effect| observed.contains(effect)) {
        return Err("an effect cannot be both observed and unknown in one result".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn delegation_acknowledgement_requires_exact_task_and_parent_call() {
        let valid = RunResultValue::DelegationAccepted { task_id: "task".into(), parent_call_id: "call".into() };
        assert!(valid.validate().is_ok());
        for value in [
            RunResultValue::DelegationAccepted { task_id: "".into(), parent_call_id: "call".into() },
            RunResultValue::DelegationAccepted { task_id: "task".into(), parent_call_id: "".into() },
        ] { assert!(value.validate().is_err()); }
        assert_eq!(serde_json::to_value(valid).unwrap()["kind"], "delegation_accepted");
    }

    fn reason() -> RunResultReason {
        RunResultReason { code: "evidence.unavailable".into(), detail: None }
    }

    fn proposal(value: RunResultValue) -> RunResult {
        RunResult {
            result_id: "result".into(), run_id: "run".into(), conversation_id: "conversation".into(),
            originating_submission_id: "submission".into(), input: RunInputCommitment {
                watermark: JournalNumber(1), applications_digest: AssistantDigest::parse_hex(&"a".repeat(64)).unwrap(),
            },
            decided_at_revision: JournalNumber(2), decision_event_cursor: JournalNumber(3),
            requirement_revision: JournalNumber(0), evidence_frontier: JournalNumber(0), value,
        }
    }

    fn values() -> Vec<RunResultValue> {
        vec![
            RunResultValue::Answered { answer_part_refs: vec!["answer".into()] },
            RunResultValue::ActionCompleted { requirement_acceptance_refs: vec!["acceptance".into()] },
            RunResultValue::DelegationAccepted { task_id: "task".into(), parent_call_id: "spawn-call".into() },
            RunResultValue::Incomplete { reason: reason(), remaining_requirement_refs: vec!["requirement".into()], obligation_refs: vec![] },
            RunResultValue::Blocked { reason: reason(), blocking_fact_refs: vec!["fact".into()], resolution_condition: "Provide an observable resource version".into() },
            RunResultValue::Cancelled { cancellation_id: "cancel".into(), observed_effect_refs: vec!["effect-1".into()], unknown_effect_refs: vec!["effect-2".into()] },
            RunResultValue::Failed { error_ref: "error".into(), observed_effect_refs: vec![], unknown_effect_refs: vec!["effect-2".into()] },
        ]
    }

    fn action_scope() -> (crate::run_requirements::RequirementSetRevision, Vec<crate::run_requirements::RequirementAssessment>, RunResult) {
        let mut set: crate::run_requirements::RequirementSetRevision = serde_json::from_value(json!({
            "conversation_id":"conversation","run_id":"run","revision":"1","input":{"watermark":"1","applications_digest":"a".repeat(64)},
            "source_input_refs":["input"],
            "source_decision_ref":{"core_session_id":"core","turn_id":"turn","revision":"1","event_cursor":"1"},
            "requirements":[
                {"requirement_id":"a","description":"First requirement","source_refs":["input"],"dependency_ids":[],"goal_requirement_ref":null,"resource_scope":{"kind":"undeclared"}},
                {"requirement_id":"b","description":"Second requirement","source_refs":["input"],"dependency_ids":["a"],"goal_requirement_ref":null,"resource_scope":{"kind":"undeclared"}}
            ]
        })).unwrap();
        for requirement in &mut set.requirements {
            requirement.resource_scope = crate::run_requirements::RequirementResourceScope::Declared {
                required: vec![crate::run_requirements::RequirementResourceClaim {
                    resource_uri: format!("file:///{}", requirement.requirement_id),
                    operation: "write".into(), version: None, predicate: None,
                    review: None,
                    source_refs: vec!["input".into()],
                }], exclusions: vec![], interpreted_user_frontier: vec!["input".into()],
            };
        }
        let assessments = ["a", "b"].into_iter().map(|id| serde_json::from_value(json!({
            "assessment_id":format!("accept-{id}"),"run_id":"run","requirement_id":id,
            "requirement_set_revision":"1","assessed_input":{"watermark":"1","applications_digest":"a".repeat(64)},"evidence_frontier":"2",
            "source_decision_ref":{"core_session_id":"core","turn_id":"turn","revision":"2","event_cursor":"2"},
            "method":{"kind":"model_judgment"},"evidence_refs":[],"verdict":"satisfied","rationale":"A model judgment, not independent evidence"
        })).unwrap()).collect();
        let mut result = proposal(RunResultValue::ActionCompleted {
            requirement_acceptance_refs: vec!["accept-a".into(), "accept-b".into()],
        });
        result.requirement_revision = JournalNumber(1);
        result.evidence_frontier = JournalNumber(2);
        (set, assessments, result)
    }

    #[test]
    fn action_coverage_rejects_unknown_or_empty_scope_despite_satisfied_judgment() {
        use crate::run_requirements::RequirementResourceScope;
        let (mut set, assessments, result) = action_scope();
        for scope in [RequirementResourceScope::Undeclared,
            RequirementResourceScope::Declared { required:vec![],exclusions:vec![],interpreted_user_frontier:vec!["input".into()] }] {
            set.requirements[0].resource_scope = scope;
            assert!(result.validate_action_requirement_coverage(&set, &assessments)
                .unwrap_err().contains("scope_coverage_unknown"));
        }
    }

    #[test]
    fn action_coverage_cannot_hide_newer_contradictions_or_same_cursor_ambiguity() {
        let (set, assessments, mut result) = action_scope();
        result.decided_at_revision = JournalNumber(4);
        result.decision_event_cursor = JournalNumber(4);
        for verdict in [crate::run_requirements::RequirementVerdict::Unsatisfied,
            crate::run_requirements::RequirementVerdict::InsufficientEvidence,
            crate::run_requirements::RequirementVerdict::Satisfied] {
            let mut newer = assessments[0].clone();
            newer.assessment_id = "new-a".into(); newer.verdict = verdict;
            newer.source_decision_ref.event_cursor = JournalNumber(3);
            newer.source_decision_ref.revision = JournalNumber(3);
            let mut supplied = assessments.clone(); supplied.push(newer);
            for reverse in [false, true] {
                if reverse { supplied.reverse(); }
                assert!(result.validate_action_requirement_coverage(&set, &supplied).is_err(), "older success must not bypass the supplied latest assessment");
            }
            let latest = supplied.iter_mut().find(|item| item.assessment_id == "new-a").unwrap();
            latest.source_decision_ref = assessments[0].source_decision_ref.clone();
            assert!(result.validate_action_requirement_coverage(&set, &supplied).is_err(), "same-cursor assessments are ambiguous even when verdicts agree");
        }
    }

    #[test]
    fn action_coverage_accepts_new_satisfied_after_old_failure_across_turns_in_any_slice_order() {
        let (set, mut assessments, mut result) = action_scope();
        let mut old = assessments[0].clone();
        old.verdict = crate::run_requirements::RequirementVerdict::Unsatisfied;
        old.assessment_id = "old-failed-a".into();
        assessments[0].source_decision_ref.event_cursor = JournalNumber(3);
        assessments[0].source_decision_ref.revision = JournalNumber(3);
        assessments[0].source_decision_ref.turn_id = "later-turn".into();
        assessments.push(old);
        result.decided_at_revision = JournalNumber(4);
        result.decision_event_cursor = JournalNumber(4);
        for _ in 0..assessments.len() {
            result.validate_action_requirement_coverage(&set, &assessments).unwrap();
            assessments.rotate_left(1);
        }
        assessments.reverse();
        result.validate_action_requirement_coverage(&set, &assessments).unwrap();
    }

    #[test]
    fn action_coverage_rejects_inconsistent_envelopes_at_one_shared_source_cursor() {
        let (set, assessments, mut result) = action_scope();
        result.decided_at_revision = JournalNumber(4);
        for changed_field in ["turn", "revision"] {
            let mut supplied = assessments.clone();
            if changed_field == "turn" { supplied[1].source_decision_ref.turn_id = "foreign-turn".into(); }
            else { supplied[1].source_decision_ref.revision = JournalNumber(3); }
            assert!(result.validate_action_requirement_coverage(&set, &supplied).is_err());
            supplied.reverse();
            assert!(result.validate_action_requirement_coverage(&set, &supplied).is_err());
        }
    }

    #[test]
    fn action_coverage_requires_nondecreasing_revision_in_cursor_order() {
        let (set, mut assessments, mut result) = action_scope();
        result.decided_at_revision = JournalNumber(5);
        result.decision_event_cursor = JournalNumber(5);
        assessments[0].source_decision_ref.revision = JournalNumber(4);
        assessments[1].source_decision_ref.event_cursor = JournalNumber(3);
        assessments[1].source_decision_ref.revision = JournalNumber(3);
        assessments[1].source_decision_ref.turn_id = "later-turn".into();
        for _ in 0..2 {
            assert!(result.validate_action_requirement_coverage(&set, &assessments).is_err());
            assessments.reverse();
        }
        // Several events in one Core reduction share a revision. Turn identity
        // does not reset session cursor/revision ordering.
        assessments[1].source_decision_ref.revision = JournalNumber(4);
        result.validate_action_requirement_coverage(&set, &assessments).unwrap();
        assessments[1].source_decision_ref.revision = JournalNumber(5);
        assessments.reverse();
        result.validate_action_requirement_coverage(&set, &assessments).unwrap();
    }

    #[test]
    fn action_coverage_rejects_successful_subset_and_missing_or_duplicate_acceptances() {
        let (set, assessments, mut result) = action_scope();
        result.validate_action_requirement_coverage(&set, &assessments).unwrap();
        result.value = RunResultValue::ActionCompleted { requirement_acceptance_refs: vec!["accept-a".into()] };
        assert!(result.validate_action_requirement_coverage(&set, &assessments).is_err());
        result.value = RunResultValue::ActionCompleted { requirement_acceptance_refs: vec!["accept-a".into(), "missing".into()] };
        assert!(result.validate_action_requirement_coverage(&set, &assessments).is_err());
        let mut repeated = assessments.clone();
        repeated[1].requirement_id = "a".into();
        result.value = RunResultValue::ActionCompleted { requirement_acceptance_refs: vec!["accept-a".into(), "accept-b".into()] };
        assert!(result.validate_action_requirement_coverage(&set, &repeated).is_err());
        repeated = assessments.clone();
        repeated.push(assessments[0].clone());
        assert!(result.validate_action_requirement_coverage(&set, &repeated).is_err());
    }

    #[test]
    fn action_coverage_rejects_stale_scope_future_evidence_and_unsatisfied_verdicts() {
        let (set, assessments, result) = action_scope();
        let mut changed = result.clone();
        changed.input.watermark = JournalNumber(2);
        assert!(changed.validate_action_requirement_coverage(&set, &assessments).is_err());
        let mut changed = result.clone();
        changed.input.applications_digest = AssistantDigest::parse_hex(&"b".repeat(64)).unwrap();
        assert_eq!(changed.input.watermark, set.input.watermark);
        assert!(changed.validate_action_requirement_coverage(&set, &assessments).is_err());
        let mut changed = assessments.clone();
        changed[0].assessed_input.applications_digest = AssistantDigest::parse_hex(&"b".repeat(64)).unwrap();
        assert!(result.validate_action_requirement_coverage(&set, &changed).is_err());
        for verdict in [crate::run_requirements::RequirementVerdict::Unsatisfied,
            crate::run_requirements::RequirementVerdict::InsufficientEvidence] {
            let mut invalid = assessments.clone();
            invalid[1].verdict = verdict;
            assert!(result.validate_action_requirement_coverage(&set, &invalid).is_err());
        }
        let mut invalid = assessments.clone();
        invalid[0].evidence_frontier = JournalNumber(3);
        assert!(result.validate_action_requirement_coverage(&set, &invalid).is_err());
        for field in ["run_id", "conversation_id", "requirement_revision"] {
            let mut value = serde_json::to_value(&result).unwrap();
            value[field] = json!(if field.ends_with("id") { "foreign" } else { "2" });
            let changed: RunResult = serde_json::from_value(value).unwrap();
            assert!(changed.validate_action_requirement_coverage(&set, &assessments).is_err());
        }
        let mut invalid = assessments.clone();
        invalid[0].source_decision_ref.event_cursor = JournalNumber(4);
        assert!(result.validate_action_requirement_coverage(&set, &invalid).is_err());
        let mut invalid = assessments.clone();
        invalid[0].source_decision_ref.core_session_id = "foreign".into();
        assert!(result.validate_action_requirement_coverage(&set, &invalid).is_err());
    }

    #[test]
    fn six_result_kinds_round_trip_without_defaulting_or_erasing_references() {
        for value in values() {
            let result = proposal(value);
            result.validate().unwrap();
            let encoded = serde_json::to_value(&result).unwrap();
            assert_eq!(serde_json::from_value::<RunResult>(encoded.clone()).unwrap(), result);
            let mut missing = encoded.clone();
            missing.as_object_mut().unwrap().remove("value");
            assert!(serde_json::from_value::<RunResult>(missing).is_err());
            let mut foreign = encoded;
            foreign["status"] = json!("completed");
            assert!(serde_json::from_value::<RunResult>(foreign).is_err());
        }
    }

    #[test]
    fn wait_is_not_a_sealed_result_and_variant_fields_cannot_be_mixed() {
        for kind in ["waiting", "needs_input", "awaiting_permission", "completed"] {
            assert!(serde_json::from_value::<RunResultValue>(json!({"kind":kind})).is_err());
        }
        assert!(serde_json::from_value::<RunResultValue>(json!({
            "kind":"answered", "answer_part_refs":["answer"], "error_ref":"error"
        })).is_err());
    }

    #[test]
    fn no_success_without_references_or_incomplete_without_remaining_work() {
        for value in [
            RunResultValue::Answered { answer_part_refs: vec![] },
            RunResultValue::ActionCompleted { requirement_acceptance_refs: vec![] },
            RunResultValue::Incomplete { reason: reason(), remaining_requirement_refs: vec![], obligation_refs: vec![] },
            RunResultValue::Blocked { reason: reason(), blocking_fact_refs: vec![], resolution_condition: "new evidence".into() },
            RunResultValue::Blocked { reason: reason(), blocking_fact_refs: vec!["fact".into()], resolution_condition: " ".into() },
        ] {
            assert!(proposal(value).validate().is_err());
        }
    }

    #[test]
    fn malformed_duplicate_and_contradictory_effect_references_are_rejected() {
        for refs in [vec!["".into()], vec![" ref".into()], vec!["a\nb".into()], vec!["a".into(), "a".into()]] {
            assert!(proposal(RunResultValue::Answered { answer_part_refs: refs }).validate().is_err());
        }
        for value in [
            RunResultValue::Cancelled { cancellation_id: "cancel".into(), observed_effect_refs: vec!["effect".into()], unknown_effect_refs: vec!["effect".into()] },
            RunResultValue::Failed { error_ref: "".into(), observed_effect_refs: vec![], unknown_effect_refs: vec![] },
        ] {
            assert!(proposal(value).validate().is_err());
        }
    }

    #[test]
    fn identity_and_durable_frontiers_are_validated_without_inventing_evidence() {
        let mut result = proposal(values().remove(0));
        result.run_id = " ".into();
        assert!(result.validate().is_err());
        result.run_id = "run".into();
        result.evidence_frontier = JournalNumber(u64::MAX);
        assert!(result.validate().is_err());
        result.evidence_frontier = JournalNumber(0);
        result.validate().unwrap(); // Does not prove the answer reference exists.
        assert!(RunResultReason { code: "任务失败".into(), detail: None }.validate().is_err());
    }

    #[test]
    fn frontier_wire_is_lossless_above_javascript_integer_precision() {
        let mut result = proposal(values().remove(0));
        result.decision_event_cursor = JournalNumber(9_007_199_254_740_993);
        result.validate().unwrap();
        let mut wire = serde_json::to_value(&result).unwrap();
        assert_eq!(wire["decision_event_cursor"], json!("9007199254740993"));
        assert_eq!(serde_json::from_value::<RunResult>(wire.clone()).unwrap(), result);
        wire["decision_event_cursor"] = json!(9_007_199_254_740_993u64);
        assert!(serde_json::from_value::<RunResult>(wire).is_err());
    }

    #[test]
    fn canonical_digest_sorts_sets_but_preserves_answer_order() {
        let forward = proposal(RunResultValue::ActionCompleted {
            requirement_acceptance_refs: vec!["a".into(), "b".into()],
        });
        let reversed = proposal(RunResultValue::ActionCompleted {
            requirement_acceptance_refs: vec!["b".into(), "a".into()],
        });
        assert_eq!(forward.canonical_bytes().unwrap(), reversed.canonical_bytes().unwrap());
        assert_eq!(forward.computed_digest().unwrap(), reversed.computed_digest().unwrap());
        let forward = proposal(RunResultValue::Answered { answer_part_refs: vec!["a".into(), "b".into()] });
        let reversed = proposal(RunResultValue::Answered { answer_part_refs: vec!["b".into(), "a".into()] });
        assert_ne!(forward.computed_digest().unwrap(), reversed.computed_digest().unwrap());
    }

    #[test]
    fn count_only_input_wire_is_not_accepted() {
        let mut wire = serde_json::to_value(proposal(values().remove(0))).unwrap();
        wire.as_object_mut().unwrap().remove("input");
        wire["input_watermark"] = json!("1");
        assert!(serde_json::from_value::<RunResult>(wire).is_err());
    }

    #[test]
    fn result_digest_matches_independent_fixed_wire_vector() {
        let result = proposal(RunResultValue::Answered { answer_part_refs: vec!["answer".into()] });
        let expected = br#"{"result_id":"result","run_id":"run","conversation_id":"conversation","originating_submission_id":"submission","input":{"watermark":"1","applications_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"decided_at_revision":"2","decision_event_cursor":"3","requirement_revision":"0","evidence_frontier":"0","value":{"kind":"answered","answer_part_refs":["answer"]}}"#;
        assert_eq!(result.canonical_bytes().unwrap(), expected);
        // Independently derived with Python hashlib/struct for the v1 domain and a big-endian u64
        // length prefix, not generated by this implementation at test time.
        assert_eq!(result.computed_digest().unwrap(), AssistantDigest::parse_hex(
            "ee00eaa7d13f627f105608ff5522732633d659f024e8e10c8af5580a0e37dd4e"
        ).unwrap());
    }

    #[test]
    fn result_digest_binds_identity_frontiers_reason_and_evidence() {
        let result = proposal(RunResultValue::Blocked {
            reason: reason(),
            blocking_fact_refs: vec!["fact".into()],
            resolution_condition: "Provide an observable resource version".into(),
        });
        let digest = result.computed_digest().unwrap();
        for field in ["watermark", "applications_digest"] {
            let mut wire = serde_json::to_value(&result).unwrap();
            wire["input"][field] = if field == "watermark" { json!("17") } else { json!("b".repeat(64)) };
            let changed: RunResult = serde_json::from_value(wire).unwrap();
            assert_ne!(digest, changed.computed_digest().unwrap(), "{field}");
        }
        for field in ["result_id", "run_id", "conversation_id", "originating_submission_id"] {
            let mut wire = serde_json::to_value(&result).unwrap();
            wire[field] = json!("different");
            let changed: RunResult = serde_json::from_value(wire).unwrap();
            assert_ne!(digest, changed.computed_digest().unwrap(), "{field}");
        }
        for field in ["decided_at_revision", "decision_event_cursor", "requirement_revision", "evidence_frontier"] {
            let mut wire = serde_json::to_value(&result).unwrap();
            wire[field] = json!("17");
            let changed: RunResult = serde_json::from_value(wire).unwrap();
            assert_ne!(digest, changed.computed_digest().unwrap(), "{field}");
        }
        for (field, value) in [
            ("reason", json!({"code":"evidence.changed", "detail":null})),
            ("blocking_fact_refs", json!(["different-fact"])),
            ("resolution_condition", json!("different condition")),
        ] {
            let mut wire = serde_json::to_value(&result).unwrap();
            wire["value"][field] = value;
            let changed: RunResult = serde_json::from_value(wire).unwrap();
            assert_ne!(digest, changed.computed_digest().unwrap(), "{field}");
        }
        assert!(proposal(RunResultValue::Answered { answer_part_refs: vec![] }).computed_digest().is_err());
    }
}
