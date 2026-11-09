// SPDX-License-Identifier: Apache-2.0
//! Model-facing task results bound to the exact Core-owned request references.
use crate::assistant_journal_operations::JournalNumber;
use crate::run_requirements::RequirementVerdict;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementRequestBinding {
    pub scope_id: String,
    pub expected_requirement_revision: JournalNumber,
    pub expected_input_frontier_token: String,
    pub requirements: BTreeMap<String, String>,
    pub evidence: BTreeMap<String, String>,
    pub blockers: BTreeMap<String, String>,
    pub sources: BTreeMap<String, String>,
    pub user_sources: BTreeMap<String, String>,
}

/// Resolve the optional scope declaration against the exact model request.
/// No model-provided frontier, raw IDs or role assertions are accepted.
pub fn bind_model_task_scope(arguments: &Value, binding: &RequirementRequestBinding) -> Result<Value, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ScopeInput { requirements: Vec<Value>, dispositions: Vec<Value> }
    let mut input: ScopeInput = serde_json::from_value(arguments.clone()).map_err(|e| format!("invalid_task_scope: {e}"))?;
    fn fields(value: &Value, allowed: &[&str]) -> Result<(), String> {
        let object=value.as_object().ok_or("invalid_task_scope: object required")?;
        let unexpected: Vec<_> = object.keys().filter(|key| !allowed.contains(&key.as_str())).collect();
        if !unexpected.is_empty() {
            return Err(format!("invalid_task_scope: unexpected fields {unexpected:?}; allowed fields {allowed:?}. retain preserves an existing requirement unchanged; to refine its resource scope, use replace with a new local_key successor."));
        }
        Ok(())
    }
    fn refs(value: &mut Value, map: &BTreeMap<String,String>) -> Result<(),String> {
        let values = value.as_array_mut().ok_or("invalid_task_scope: expected reference array")?;
        for reference in values {
            *reference = json!(map.get(reference.as_str().ok_or("invalid_task_scope: reference must be string")?)
                .ok_or("invalid_task_scope: unknown frozen source reference")?);
        }
        Ok(())
    }
    fn control_ref(value: &mut Value, binding: &RequirementRequestBinding) -> Result<(),String> {
        match value["kind"].as_str() {
            Some("local_key") => Ok(()),
            Some("existing_id") => {
                value["id"] = json!(binding.requirements.get(value["id"].as_str().ok_or("invalid scope requirement")?)
                    .ok_or("unknown frozen scope requirement")?);
                Ok(())
            }
            _ => Err("invalid scope dependency".into()),
        }
    }
    for requirement in &mut input.requirements {
        fields(requirement,&["key","description","source_refs","dependencies","resource_scope"])?;
        refs(&mut requirement["source_refs"], &binding.sources)?;
        for dependency in requirement["dependencies"].as_array_mut().ok_or("missing scope dependencies")? { control_ref(dependency, binding)?; }
        let scope = &mut requirement["resource_scope"];
        if scope.get("interpreted_user_frontier").is_some() { return Err("model cannot supply scope frontier".into()); }
        if scope["kind"] == "declared" {
            for claim in scope["required"].as_array_mut().ok_or("missing required resources")? {
                refs(&mut claim["source_refs"], &binding.sources)?;
                if let Some(review) = claim.get_mut("review").filter(|value| !value.is_null()) {
                    if !review.is_object() { return Err("invalid review finding: object required".into()); }
                    refs(&mut review["source_evidence"], &binding.evidence)?;
                    if let Some(reference) = review.get_mut("counterexample_evidence").filter(|value| !value.is_null()) {
                        *reference = json!(binding.evidence.get(reference.as_str().ok_or("invalid counterexample evidence")?)
                            .ok_or("unknown frozen counterexample evidence")?);
                    }
                }
            }
            for exclusion in scope["exclusions"].as_array_mut().ok_or("missing exclusions")? {
                let map = if exclusion["provenance"] == "user_input_interpretation" { &binding.user_sources } else { &binding.sources };
                refs(&mut exclusion["source_refs"], map)?;
            }
            scope["interpreted_user_frontier"] = json!(binding.user_sources.values().collect::<BTreeSet<_>>());
        }
        let scope: crate::run_requirements::RequirementResourceScope = serde_json::from_value(scope.clone()).map_err(|e| format!("invalid resource scope: {e}"))?;
        scope.validate(&binding.sources.values().cloned().collect::<Vec<_>>())?;
    }
    for disposition in &mut input.dispositions {
        let allowed: &[&str] = match disposition["kind"].as_str() {
            Some("retain") => &["kind","requirement_id"],
            Some("replace") => &["kind","requirement_id","successors","reason","user_input_refs"],
            Some("withdraw") => &["kind","requirement_id","reason","user_input_refs"],
            _ => return Err("invalid_task_scope: unknown disposition".into()),
        };
        fields(disposition,allowed)?;
        disposition["requirement_id"] = json!(binding.requirements.get(disposition["requirement_id"].as_str().ok_or("missing scope disposition")?)
            .ok_or("unknown frozen requirement disposition")?);
        if disposition["kind"] != "retain" { refs(&mut disposition["user_input_refs"], &binding.user_sources)?; }
        if disposition["kind"] == "replace" {
            for successor in disposition["successors"].as_array_mut().ok_or("missing successors")? { control_ref(successor,binding)?; }
        }
    }
    Ok(json!({"scope_id":binding.scope_id,"expected_requirement_revision":binding.expected_requirement_revision,
        "expected_input_frontier_token":binding.expected_input_frontier_token,
        "operations":[{"kind":"revise_scope","requirements":input.requirements,"dispositions":input.dispositions}]}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelTaskResult {
    outcome: ModelTaskOutcome,
    answer: String,
    #[serde(default)]
    assessments: Vec<ModelAssessment>,
    #[serde(default)]
    blocking_facts: Vec<String>,
    resolution: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ModelTaskOutcome { Answered, Completed, Incomplete, Blocked }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelAssessment {
    requirement: String,
    verdict: RequirementVerdict,
    rationale: String,
    evidence: Vec<String>,
}

/// Core execution and Store provenance auditing must interpret the original
/// response identically. This resolves only frozen aliases; it grants no new
/// evidence, execution authority or permission to revise the input scope.
pub fn bind_model_task_result(arguments: &Value, binding: &RequirementRequestBinding)
    -> Result<Value, String>
{
    let result: ModelTaskResult = serde_json::from_value(arguments.clone())
        .map_err(|error| format!("invalid_task_result: {error}"))?;
    let resolve = |references: &[String], map: &BTreeMap<String, String>, field: &str| -> Result<Vec<String>, String> {
        references.iter().map(|reference| map.get(reference).cloned()
            .ok_or_else(|| format!("invalid_reference: unknown current reference {reference} in {field}; valid references: {:?}. Evidence (eN) and blocking facts (fN) are distinct; use incomplete when evidence is insufficient and no blocking fact exists.", map.keys().collect::<Vec<_>>()))).collect()
    };
    let mut operations = Vec::new();
    let mut accepted = Vec::new();
    let mut assessed = BTreeSet::new();
    for (i, assessment) in result.assessments.iter().enumerate() {
        if !assessed.insert(&assessment.requirement) {
            return Err("invalid_task_result: duplicate requirement assessment".into());
        }
        let requirement = binding.requirements.get(&assessment.requirement)
            .ok_or("invalid_reference: unknown current requirement")?;
        let key = format!("assessment-{i}");
        operations.push(json!({"kind":"assess","key":key,
            "requirement":{"kind":"existing_id","id":requirement},
            "verdict":assessment.verdict,"rationale":assessment.rationale,
            "evidence_refs":resolve(&assessment.evidence, &binding.evidence, "assessments.evidence (eN)")?}));
        accepted.push(json!({"kind":"local_key","key":key}));
    }
    let proposed = match result.outcome {
        ModelTaskOutcome::Answered => json!({"kind":"answered"}),
        ModelTaskOutcome::Completed => json!({"kind":"action_completed","assessment_refs":accepted}),
        ModelTaskOutcome::Incomplete => json!({"kind":"incomplete","reason":result.answer,
            "remaining_requirement_refs":binding.requirements.values().collect::<Vec<_>>()}),
        ModelTaskOutcome::Blocked => json!({"kind":"blocked","reason":result.answer,
            "blocking_fact_refs":resolve(&result.blocking_facts, &binding.blockers, "blocking_facts (fN)")?,
            "resolution_condition":result.resolution.unwrap_or_default()}),
    };
    operations.push(json!({"kind":"propose_result","result":proposed}));
    Ok(json!({"scope_id":binding.scope_id,
        "expected_requirement_revision":binding.expected_requirement_revision,
        "expected_input_frontier_token":binding.expected_input_frontier_token,
        "operations":operations,"answer":result.answer}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> RequirementRequestBinding {
        RequirementRequestBinding {
            scope_id: "scope".into(), expected_requirement_revision: JournalNumber(3),
            expected_input_frontier_token: "frontier".into(),
            requirements: [("r1".into(), "requirement-original".into())].into(),
            evidence: [("e1".into(), "evidence-original".into())].into(),
            blockers: [("f1".into(), "blocker-original".into())].into(),
            sources: [("s1".into(), "input-original".into())].into(),
            user_sources: [("s1".into(), "input-original".into())].into(),
        }
    }

    #[test]
    fn scope_declaration_binds_sources_and_rejects_forged_frontier() {
        let args = json!({"requirements":[{"key":"files","description":"Read specified files","source_refs":["s1"],"dependencies":[],
            "resource_scope":{"kind":"declared","required":[{"resource_uri":"file:///a","operation":"read","version":null,"predicate":null,"source_refs":["s1"]}],"exclusions":[]}}],
            "dispositions":[{"kind":"replace","requirement_id":"r1","successors":[{"kind":"local_key","key":"files"}],"reason":"declare scope","user_input_refs":["s1"]}]});
        let bound = bind_model_task_scope(&args,&binding()).unwrap();
        assert_eq!(bound["operations"][0]["requirements"][0]["resource_scope"]["interpreted_user_frontier"],json!(["input-original"]));
        assert_eq!(bound["operations"][0]["dispositions"][0]["requirement_id"],"requirement-original");
        let mut forged=args.clone();
        forged["requirements"][0]["resource_scope"]["interpreted_user_frontier"]=json!(["input-original"]);
        assert!(bind_model_task_scope(&forged,&binding()).is_err());
        forged=args;
        forged["requirements"][0]["source_refs"]=json!(["input-original"]);
        assert!(bind_model_task_scope(&forged,&binding()).is_err());
    }

    #[test]
    fn review_evidence_is_bound_to_current_observations_without_conferring_user_authority() {
        let mut args = json!({"requirements":[{"key":"review","description":"Repair adopted finding","source_refs":["s1"],"dependencies":[],
            "resource_scope":{"kind":"declared","required":[{"resource_uri":"file:///a","operation":"write",
                "predicate":"regression_process_exit:sha256:example","source_refs":["s1"],
                "review":{"source_evidence":["e1"],"counterexample_evidence":"e1"}}],"exclusions":[]}}],
            "dispositions":[{"kind":"retain","requirement_id":"r1"}]});
        let bound = bind_model_task_scope(&args, &binding()).unwrap();
        let review = &bound["operations"][0]["requirements"][0]["resource_scope"]["required"][0]["review"];
        assert_eq!(review["source_evidence"], json!(["evidence-original"]));
        assert_eq!(review["counterexample_evidence"], "evidence-original");
        let target = &mut args["requirements"][0]["resource_scope"]["required"][0]["review"];
        target["counterexample_evidence"] = json!("evidence-original");
        assert!(bind_model_task_scope(&args, &binding()).is_err(), "raw evidence IDs are not aliases");
        args["requirements"][0]["resource_scope"]["required"][0]["review"] = json!("not-an-object");
        assert!(bind_model_task_scope(&args, &binding()).is_err());
    }

    #[test]
    fn invalid_retain_explains_the_rejected_fields_and_correct_operation() {
        let args = json!({"requirements":[],"dispositions":[{"kind":"retain","requirement_id":"r1",
            "reason":"refine scope","successors":[],"user_input_refs":["s1"]}]});
        let error = bind_model_task_scope(&args,&binding()).unwrap_err();
        for expected in ["reason", "successors", "user_input_refs", "use replace"] {
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn compact_result_resolves_only_the_frozen_request_references() {
        let arguments = json!({"outcome":"completed","answer":"已验证。",
            "assessments":[{"requirement":"r1","verdict":"satisfied",
                "rationale":"Observed result","evidence":["e1"]}]});
        let bound = bind_model_task_result(&arguments, &binding()).unwrap();
        assert_eq!(bound["operations"][0]["requirement"]["id"], "requirement-original");
        assert_eq!(bound["operations"][0]["evidence_refs"], json!(["evidence-original"]));
        assert_eq!(bound["operations"][1]["result"]["kind"], "action_completed");
        for evidence in ["e2", "evidence-original", "foreign-run:e1"] {
            let mut changed = arguments.clone();
            changed["assessments"][0]["evidence"] = json!([evidence]);
            assert!(bind_model_task_result(&changed, &binding()).is_err());
        }
        let mut changed = arguments.clone();
        changed["assessments"][0]["requirement"] = json!("r2");
        assert!(bind_model_task_result(&changed, &binding()).is_err());
        let mut duplicate = arguments.clone();
        duplicate["assessments"].as_array_mut().unwrap().push(arguments["assessments"][0].clone());
        assert!(bind_model_task_result(&duplicate, &binding()).is_err());
        let mut forged = arguments;
        forged["operations"] = json!([]);
        assert!(bind_model_task_result(&forged, &binding()).is_err());
    }

    #[test]
    fn blocked_results_bind_original_blockers_and_incomplete_retains_scope() {
        let bound = bind_model_task_result(&json!({"outcome":"blocked","answer":"Missing access",
            "blocking_facts":["f1"],"resolution":"Grant access"}), &binding()).unwrap();
        assert_eq!(bound["operations"][0]["result"]["blocking_fact_refs"], json!(["blocker-original"]));
        assert!(bind_model_task_result(&json!({"outcome":"blocked","answer":"Missing access",
            "blocking_facts":["f2"],"resolution":"Grant access"}), &binding()).is_err());
        let bound = bind_model_task_result(&json!({"outcome":"incomplete","answer":"Still unverified"}), &binding()).unwrap();
        assert_eq!(bound["operations"][0]["result"]["remaining_requirement_refs"], json!(["requirement-original"]));
    }
}
