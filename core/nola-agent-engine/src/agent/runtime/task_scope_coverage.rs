// SPDX-License-Identifier: Apache-2.0
//! Checks declared object coverage, never completeness of natural-language intent.
use super::requirements::ObservedEvidenceReference;
use nola_domain::execution_evidence::{ExecutionCheckKind, ExecutionState};
use nola_domain::run_requirements::{RequirementResourceClaim, RequirementResourceScope};
use std::collections::BTreeMap;

pub(super) fn validate_satisfied_coverage(
    scope: &RequirementResourceScope,
    cited: &[String],
    observations: &BTreeMap<String, ObservedEvidenceReference>,
) -> Result<(), String> {
    let RequirementResourceScope::Declared { required, .. } = scope else {
        return Ok(());
    };
    for claim in required {
        let covered = cited
            .iter()
            .filter_map(|id| observations.get(id))
            .any(|observation| {
                let Some(receipt) = &observation.receipt else {
                    return false;
                };
                if receipt.state != ExecutionState::Completed
                    || !matches!(
                        observation.outcome,
                        nola_domain::host_effect::HostToolOutcome::Succeeded
                    )
                {
                    return false;
                }
                let versions: Vec<(&String, &ObservedEvidenceReference, bool)> =
                    if let Some(predicate) = &claim.predicate {
                        let mut versions = Vec::new();
                        for check in receipt.checks.iter().filter(|check| {
                            accepted_check_kind(check, claim)
                                && check.passed
                                && check.resource_uri == claim.resource_uri
                                && &check.predicate == predicate
                        }) {
                            // Preserve atomic receipts, e.g. a repository commit effect
                            // accompanied by exact committed-path postconditions.
                            if receipt
                                .effects
                                .iter()
                                .any(|effect| effect.operation == claim.operation)
                            {
                                versions.push((&check.version, observation, false));
                            }
                            // A read-only validator must not invent a write effect.
                            // Both actual receipts must be explicitly cited and ordered.
                            for action in cited.iter().filter_map(|id| observations.get(id)) {
                                if action.turn_id != observation.turn_id
                                    || action.source.host_id != observation.source.host_id
                                    || action.recorded_core_revision
                                        >= observation.recorded_core_revision
                                    || !matches!(
                                        action.outcome,
                                        nola_domain::host_effect::HostToolOutcome::Succeeded
                                    )
                                {
                                    continue;
                                }
                                if action.receipt.as_ref().is_some_and(|prior| {
                                    prior.state == ExecutionState::Completed
                                        && prior.effects.iter().any(|effect| {
                                            effect.resource_uri == claim.resource_uri
                                                && effect.operation == claim.operation
                                                && effect.after_version.as_ref()
                                                    == Some(&check.version)
                                        })
                                }) {
                                    versions.push((&check.version, action, true));
                                }
                            }
                        }
                        versions
                    } else {
                        receipt
                            .effects
                            .iter()
                            .filter(|effect| {
                                effect.resource_uri == claim.resource_uri
                                    && effect.operation == claim.operation
                            })
                            .filter_map(|effect| effect.after_version.as_ref())
                            .map(|version| (version, observation, false))
                            .collect()
                    };
                versions.into_iter().any(|(version, action, joint)| {
                    if claim
                        .version
                        .as_ref()
                        .is_some_and(|expected| expected != version)
                    {
                        return false;
                    }
                    // A generic process-exit check cannot stand in for a declared
                    // resource predicate. No model text can manufacture this check.
                    if claim.predicate.as_ref().is_some_and(|predicate| {
                        !receipt.checks.iter().any(|check| {
                            accepted_check_kind(check, claim)
                                && check.passed
                                && check.resource_uri == claim.resource_uri
                                && &check.version == version
                                && &check.predicate == predicate
                        })
                    }) {
                        return false;
                    }
                    if !review_regression_covered(claim, cited, observations, action, observation, version) {
                        return false;
                    }
                    // Only a demonstrated transition of this version invalidates
                    // current-state credit. A read is not a mutation, and an
                    // immutable commit identity is not a file-content hash.
                    !observations.values().any(|later| {
                        later.evidence_id != action.evidence_id
                            && later.source.host_id == action.source.host_id
                            && later.recorded_core_revision >= action.recorded_core_revision
                            && later.receipt.as_ref().is_some_and(|later_receipt| {
                                later_receipt.effects.iter().any(|changed| {
                                    changed.resource_uri == claim.resource_uri
                                        && changed.before_version != changed.after_version
                                        && (joint
                                            || changed.before_version.as_ref() == Some(version)
                                            || (claim.predicate.is_none()
                                                && changed.operation == claim.operation))
                                        && changed.after_version.as_ref() != Some(version)
                                })
                            })
                    })
                })
            });
        if !covered {
            return Err(format!("insufficient_resource_coverage: {} operation {} lacks current cited receipt coverage", claim.resource_uri, claim.operation));
        }
    }
    Ok(())
}

/// Typed process-exit evidence is useful for one pinned regression, never a
/// claim that arbitrary process success establishes business acceptance.
fn accepted_check_kind(check: &nola_domain::execution_evidence::PostconditionCheck, claim: &RequirementResourceClaim) -> bool {
    check.kind == ExecutionCheckKind::Postcondition
        || (claim.review.is_some() && check.kind == ExecutionCheckKind::ProcessExit
            && check.predicate.strip_prefix("regression_process_exit:sha256:")
                .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())))
}

fn review_regression_covered(
    claim: &RequirementResourceClaim,
    cited: &[String],
    observations: &BTreeMap<String, ObservedEvidenceReference>,
    repair: &ObservedEvidenceReference,
    validation: &ObservedEvidenceReference,
    version: &str,
) -> bool {
    let Some(review) = &claim.review else { return true; };
    let Some(id) = &review.counterexample_evidence else { return false; };
    if !cited.contains(id) { return false; }
    let Some(failure) = observations.get(id) else { return false; };
    if !matches!(failure.outcome, nola_domain::host_effect::HostToolOutcome::Succeeded | nola_domain::host_effect::HostToolOutcome::Failed)
        || failure.source.host_id != validation.source.host_id || failure.turn_id != validation.turn_id
        || failure.recorded_core_revision >= repair.recorded_core_revision
        || repair.recorded_core_revision >= validation.recorded_core_revision {
        return false;
    }
    let Some(failed_receipt) = &failure.receipt else { return false; };
    if failed_receipt.state != ExecutionState::Completed { return false; }
    let fixed = failed_receipt.checks.iter().any(|failed| {
        accepted_check_kind(failed, claim) && !failed.passed
            && failed.resource_uri == claim.resource_uri && Some(&failed.predicate) == claim.predicate.as_ref()
            && failed.version != version
            && validation.receipt.as_ref().is_some_and(|receipt| receipt.checks.iter().any(|passed|
                passed.passed && passed.kind == failed.kind && passed.resource_uri == failed.resource_uri
                    && passed.predicate == failed.predicate && passed.version == version))
    });
    fixed && !observations.values().any(|later| {
        later.source.host_id == validation.source.host_id && later.turn_id == validation.turn_id
            && later.recorded_core_revision > validation.recorded_core_revision
            && later.receipt.as_ref().is_some_and(|receipt| receipt.checks.iter().any(|check|
                !check.passed && check.resource_uri == claim.resource_uri
                    && Some(&check.predicate) == claim.predicate.as_ref()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_domain::assistant_journal_operations::JournalNumber;
    use nola_domain::execution_evidence::*;
    use nola_domain::run_requirements::RequirementResourceClaim;

    fn scope() -> RequirementResourceScope {
        RequirementResourceScope::Declared {
            required: ["file:///one", "file:///two"]
                .into_iter()
                .map(|uri| RequirementResourceClaim {
                    resource_uri: uri.into(),
                    operation: "commit".into(),
                    version: Some("v1".into()),
                    predicate: Some("committed_path".into()),
                    review: None,
                    source_refs: vec!["user".into()],
                })
                .collect(),
            exclusions: vec![],
            interpreted_user_frontier: vec!["user".into()],
        }
    }
    fn observation(id: &str, uri: &str, revision: u64) -> ObservedEvidenceReference {
        ObservedEvidenceReference::from((
            &ExecutionEvidenceObservation {
                evidence_id: id.into(),
                call_id: id.into(),
                tool_name: "tool".into(),
                source: ExecutionEvidenceSource {
                    effect_id: id.into(),
                    request_id: id.into(),
                    host_id: "host".into(),
                    attempt_id: None,
                    response_sequence: JournalNumber(1),
                    catalog_revision: JournalNumber(1),
                    availability_epoch: JournalNumber(0),
                },
                outcome: nola_domain::host_effect::HostToolOutcome::Succeeded,
                receipt: Some(ExecutionReceipt {
                    local_stop: None,
                    state: ExecutionState::Completed,
                    effects: vec![ResourceEffect {
                        resource_uri: uri.into(),
                        operation: "commit".into(),
                        before_version: None,
                        after_version: Some("v1".into()),
                    }],
                    checks: vec![PostconditionCheck {
                        kind: ExecutionCheckKind::Postcondition,
                        resource_uri: uri.into(),
                        version: "v1".into(),
                        predicate: "committed_path".into(),
                        passed: true,
                    }],
                }),
            },
            "turn",
            revision,
        ))
    }
    fn regression_fixture() -> (RequirementResourceScope, BTreeMap<String, ObservedEvidenceReference>, Vec<String>) {
        use nola_domain::run_requirements::RequirementReviewFinding;
        let predicate = format!("regression_process_exit:sha256:{}", "a".repeat(64));
        let scope = RequirementResourceScope::Declared { required: vec![RequirementResourceClaim {
            resource_uri: "file:///one".into(), operation:"commit".into(), version:None,
            predicate:Some(predicate.clone()), review:Some(RequirementReviewFinding {
                source_evidence:vec!["review".into()], counterexample_evidence:Some("failure".into()) }), source_refs:vec!["user".into()],
        }], exclusions:vec![], interpreted_user_frontier:vec!["user".into()] };
        let review = observation("review", "file:///one", 1);
        let mut failure = observation("failure", "file:///one", 2);
        failure.outcome = nola_domain::host_effect::HostToolOutcome::Failed;
        failure.receipt.as_mut().unwrap().effects.clear();
        failure.receipt.as_mut().unwrap().checks[0] = PostconditionCheck { kind:ExecutionCheckKind::ProcessExit,
            resource_uri:"file:///one".into(), version:"broken".into(), predicate:predicate.clone(), passed:false };
        let mut repair = observation("repair", "file:///one", 3);
        repair.receipt.as_mut().unwrap().checks.clear();
        repair.receipt.as_mut().unwrap().effects[0].before_version = Some("broken".into());
        let mut pass = observation("pass", "file:///one", 4);
        pass.receipt.as_mut().unwrap().effects.clear();
        pass.receipt.as_mut().unwrap().checks[0].kind = ExecutionCheckKind::ProcessExit;
        pass.receipt.as_mut().unwrap().checks[0].predicate = predicate;
        let observations = [review, failure, repair, pass].into_iter().map(|item| (item.evidence_id.clone(),item)).collect();
        (scope, observations, vec!["failure".into(),"repair".into(),"pass".into()])
    }

    #[test]
    fn review_closure_requires_original_failure_ordered_repair_and_same_current_check() {
        let (scope, original, refs) = regression_fixture();
        assert!(validate_satisfied_coverage(&scope, &refs, &original).is_ok());
        for variant in 0..14 {
            let mut observations = original.clone();
            match variant {
                0 => { observations.get_mut("failure").unwrap().receipt = None; }, // child says tests passed
                1 => { observations.get_mut("failure").unwrap().receipt.as_mut().unwrap().checks[0].passed = true; },
                2 => { observations.get_mut("pass").unwrap().receipt.as_mut().unwrap().checks[0].predicate = "process_exit_zero_with_stable_resource".into(); },
                3 => { observations.get_mut("pass").unwrap().receipt.as_mut().unwrap().checks[0].predicate.push('b'); }, // changed test
                4 => { observations.get_mut("failure").unwrap().receipt.as_mut().unwrap().checks[0].resource_uri = "file:///different".into(); },
                5 => { observations.get_mut("failure").unwrap().receipt.as_mut().unwrap().checks[0].version = "v1".into(); }, // no repaired version
                6 => { observations.get_mut("failure").unwrap().recorded_core_revision = 5; },
                7 => { observations.get_mut("repair").unwrap().recorded_core_revision = 5; },
                8 => { observations.get_mut("failure").unwrap().source.host_id = "another-host".into(); },
                9 => { observations.get_mut("failure").unwrap().turn_id = "another-turn".into(); },
                10 => { observations.get_mut("failure").unwrap().receipt.as_mut().unwrap().state = ExecutionState::Unknown; },
                11 => { observations.get_mut("pass").unwrap().outcome = nola_domain::host_effect::HostToolOutcome::Failed; },
                12 => { observations.get_mut("failure").unwrap().outcome = nola_domain::host_effect::HostToolOutcome::Deferred; },
                13 => { observations.get_mut("failure").unwrap().outcome = nola_domain::host_effect::HostToolOutcome::Cancelled; },
                _ => unreachable!(),
            }
            assert!(validate_satisfied_coverage(&scope, &refs, &observations).is_err(), "variant {variant}");
        }
        assert!(validate_satisfied_coverage(&scope, &refs[1..], &original).is_err(), "must cite original counterexample");
        let mut pending = scope.clone();
        if let RequirementResourceScope::Declared { required, .. } = &mut pending {
            required[0].review.as_mut().unwrap().counterexample_evidence = None;
        }
        assert!(validate_satisfied_coverage(&pending, &refs, &original).is_err());
        let cold_scope: RequirementResourceScope = serde_json::from_str(&serde_json::to_string(&scope).unwrap()).unwrap();
        let cold_observations = serde_json::from_str(&serde_json::to_string(&original).unwrap()).unwrap();
        assert!(validate_satisfied_coverage(&cold_scope, &refs, &cold_observations).is_ok());
    }

    #[test]
    fn later_edit_or_failure_reopens_review_and_unrelated_evidence_cannot_close_it() {
        let (scope, original, refs) = regression_fixture();
        for later_failure in [false, true] {
            let mut observations = original.clone();
            let mut later = if later_failure { original["failure"].clone() } else { observation("later", "file:///one", 5) };
            later.evidence_id = "later".into();
            later.recorded_core_revision = 5;
            if !later_failure {
                later.receipt.as_mut().unwrap().effects[0].before_version = Some("v1".into());
                later.receipt.as_mut().unwrap().effects[0].after_version = Some("v2".into());
                later.receipt.as_mut().unwrap().checks.clear();
            }
            observations.insert("later".into(), later);
            observations.insert("scale-test".into(), observation("scale-test", "file:///unrelated", 6));
            assert!(validate_satisfied_coverage(&scope, &refs, &observations).is_err());
        }
        let mut observations = original;
        let mut unrelated = observation("unrelated", "file:///different", 5);
        unrelated.receipt.as_mut().unwrap().effects[0].after_version = Some("new".into());
        observations.insert("unrelated".into(), unrelated);
        assert!(validate_satisfied_coverage(&scope, &refs, &observations).is_ok());
    }

    #[test]
    fn subset_and_process_exit_do_not_cover_declared_objects() {
        let mut observations = BTreeMap::from([("e1".into(), observation("e1", "file:///one", 1))]);
        assert!(validate_satisfied_coverage(&scope(), &["e1".into()], &observations).is_err());
        observations.insert("e2".into(), observation("e2", "file:///two", 2));
        let refs = vec!["e1".into(), "e2".into()];
        assert!(validate_satisfied_coverage(&scope(), &refs, &observations).is_ok());
        for observation in observations.values_mut() {
            observation.receipt.as_mut().unwrap().effects[0].resource_uri =
                "file:///repository/.git".into();
        }
        assert!(
            validate_satisfied_coverage(&scope(), &refs, &observations).is_ok(),
            "Host postconditions need not invent source mutations"
        );
        observations
            .get_mut("e2")
            .unwrap()
            .receipt
            .as_mut()
            .unwrap()
            .checks[0]
            .kind = ExecutionCheckKind::ProcessExit;
        assert!(validate_satisfied_coverage(&scope(), &refs, &observations).is_err());
    }
    #[test]
    fn later_failed_or_unknown_mutation_invalidates_historical_coverage() {
        let mut observations = BTreeMap::from([
            ("e1".into(), observation("e1", "file:///one", 1)),
            ("e2".into(), observation("e2", "file:///two", 2)),
        ]);
        let mut later = observation("e3", "file:///one", 3);
        later.outcome = nola_domain::host_effect::HostToolOutcome::Failed;
        later.receipt.as_mut().unwrap().effects[0].after_version = None;
        later.receipt.as_mut().unwrap().effects[0].before_version = Some("v1".into());
        observations.insert("e3".into(), later);
        assert!(
            validate_satisfied_coverage(&scope(), &["e1".into(), "e2".into()], &observations)
                .is_err()
        );
        assert!(validate_satisfied_coverage(
            &RequirementResourceScope::Undeclared,
            &[],
            &observations
        )
        .is_ok());
    }

    #[test]
    fn reads_do_not_invalidate_historical_fact_and_wrong_operation_does_not_cover() {
        let mut observations = BTreeMap::from([
            ("e1".into(), observation("e1", "file:///one", 1)),
            ("e2".into(), observation("e2", "file:///two", 2)),
        ]);
        let mut read = observation("read", "file:///one", 3);
        let effect = &mut read.receipt.as_mut().unwrap().effects[0];
        effect.operation = "read".into();
        effect.before_version = Some("content-hash".into());
        effect.after_version = Some("content-hash".into());
        observations.insert("read".into(), read);
        let refs = vec!["e1".into(), "e2".into()];
        assert!(validate_satisfied_coverage(&scope(), &refs, &observations).is_ok());
        observations
            .get_mut("e1")
            .unwrap()
            .receipt
            .as_mut()
            .unwrap()
            .effects[0]
            .operation = "different-action".into();
        assert!(validate_satisfied_coverage(&scope(), &refs, &observations).is_err());
    }

    #[test]
    fn independent_validation_requires_exact_cited_ordered_successful_action() {
        let mut scope = scope();
        let RequirementResourceScope::Declared { required, .. } = &mut scope else {
            unreachable!()
        };
        required.truncate(1);
        let mut action = observation("action", "file:///one", 1);
        action.receipt.as_mut().unwrap().checks.clear();
        let mut check = observation("check", "file:///one", 2);
        check.receipt.as_mut().unwrap().effects.clear();
        let observations = BTreeMap::from([("action".into(), action), ("check".into(), check)]);
        let refs = vec!["action".into(), "check".into()];
        assert!(validate_satisfied_coverage(&scope, &refs, &observations).is_ok());
        for cited in [vec!["action".into()], vec!["check".into()]] {
            assert!(
                validate_satisfied_coverage(&scope, &cited, &observations).is_err(),
                "uncited observations cannot complete coverage"
            );
        }
        for case in 0..12 {
            let mut changed = observations.clone();
            match case {
                0 => {
                    changed
                        .get_mut("action")
                        .unwrap()
                        .receipt
                        .as_mut()
                        .unwrap()
                        .effects[0]
                        .resource_uri = "file:///other".into()
                }
                1 => {
                    changed
                        .get_mut("action")
                        .unwrap()
                        .receipt
                        .as_mut()
                        .unwrap()
                        .effects[0]
                        .operation = "other".into()
                }
                2 => {
                    changed
                        .get_mut("action")
                        .unwrap()
                        .receipt
                        .as_mut()
                        .unwrap()
                        .effects[0]
                        .after_version = Some("v0".into())
                }
                3 => {
                    changed
                        .get_mut("check")
                        .unwrap()
                        .receipt
                        .as_mut()
                        .unwrap()
                        .checks[0]
                        .resource_uri = "file:///other".into()
                }
                4 => {
                    changed
                        .get_mut("check")
                        .unwrap()
                        .receipt
                        .as_mut()
                        .unwrap()
                        .checks[0]
                        .version = "v0".into()
                }
                5 => {
                    changed
                        .get_mut("check")
                        .unwrap()
                        .receipt
                        .as_mut()
                        .unwrap()
                        .checks[0]
                        .predicate = "other".into()
                }
                6 => {
                    changed
                        .get_mut("check")
                        .unwrap()
                        .receipt
                        .as_mut()
                        .unwrap()
                        .checks[0]
                        .kind = ExecutionCheckKind::ProcessExit
                }
                7 => changed.get_mut("check").unwrap().recorded_core_revision = 1,
                8 => changed.get_mut("check").unwrap().recorded_core_revision = 0,
                9 => changed.get_mut("check").unwrap().turn_id = "other-turn".into(),
                10 => changed.get_mut("check").unwrap().source.host_id = "other-host".into(),
                _ => {
                    changed
                        .get_mut("check")
                        .unwrap()
                        .receipt
                        .as_mut()
                        .unwrap()
                        .checks[0]
                        .passed = false
                }
            }
            assert!(
                validate_satisfied_coverage(&scope, &refs, &changed).is_err(),
                "invalid joint evidence case {case}"
            );
        }
        for id in ["action", "check"] {
            let mut changed = observations.clone();
            changed.get_mut(id).unwrap().outcome =
                nola_domain::host_effect::HostToolOutcome::Failed;
            assert!(validate_satisfied_coverage(&scope, &refs, &changed).is_err());
            let mut changed = observations.clone();
            changed.get_mut(id).unwrap().receipt.as_mut().unwrap().state = ExecutionState::Unknown;
            assert!(validate_satisfied_coverage(&scope, &refs, &changed).is_err());
        }
        let mut wrong_scope = scope.clone();
        let RequirementResourceScope::Declared { required, .. } = &mut wrong_scope else {
            unreachable!()
        };
        required[0].version = Some("v2".into());
        assert!(validate_satisfied_coverage(&wrong_scope, &refs, &observations).is_err());
        for after in [Some("v2".to_owned()), None] {
            let mut changed = observations.clone();
            let mut later = observation("later", "file:///one", 3);
            later.outcome = nola_domain::host_effect::HostToolOutcome::Failed;
            let effect = &mut later.receipt.as_mut().unwrap().effects[0];
            effect.before_version = Some("v1".into());
            effect.after_version = after;
            changed.insert("later".into(), later);
            assert!(
                validate_satisfied_coverage(&scope, &refs, &changed).is_err(),
                "later failed/unknown mutation invalidates old checks even when uncited"
            );
        }
        let mut changed = observations.clone();
        let mut read = observation("read", "file:///one", 3);
        let effect = &mut read.receipt.as_mut().unwrap().effects[0];
        effect.operation = "read".into();
        effect.before_version = effect.after_version.clone();
        changed.insert("read".into(), read);
        assert!(validate_satisfied_coverage(&scope, &refs, &changed).is_ok());
    }

    #[test]
    fn joint_coverage_isolates_hosts_and_requires_new_cited_pair_after_version_restoration() {
        let mut scope = scope();
        let RequirementResourceScope::Declared { required, .. } = &mut scope else {
            unreachable!()
        };
        required.truncate(1);
        let mut action = observation("action", "file:///one", 1);
        action.receipt.as_mut().unwrap().checks.clear();
        let mut check = observation("check", "file:///one", 2);
        check.receipt.as_mut().unwrap().effects.clear();
        let mut values = BTreeMap::from([("action".into(), action), ("check".into(), check)]);
        let refs = vec!["action".into(), "check".into()];
        let mut mutation = observation("mutation", "file:///one", 3);
        mutation.receipt.as_mut().unwrap().checks.clear();
        let effect = &mut mutation.receipt.as_mut().unwrap().effects[0];
        effect.before_version = Some("v1".into());
        effect.after_version = Some("v2".into());
        mutation.source.host_id = "other-host".into();
        values.insert("mutation".into(), mutation.clone());
        assert!(
            validate_satisfied_coverage(&scope, &refs, &values).is_ok(),
            "same URI on another Host does not invalidate this owner"
        );
        mutation.source.host_id = "host".into();
        mutation.turn_id = "later-turn".into();
        values.insert("mutation".into(), mutation);
        assert!(validate_satisfied_coverage(&scope,&refs,&values).is_err(),"same Host mutation invalidates even across turns when present in the observation frontier");
        let mut restored = observation("restored", "file:///one", 4);
        restored.receipt.as_mut().unwrap().checks.clear();
        restored.receipt.as_mut().unwrap().effects[0].before_version = Some("v2".into());
        values.insert("restored".into(), restored);
        assert!(
            validate_satisfied_coverage(&scope, &refs, &values).is_err(),
            "restoring content does not revive the original proof"
        );
        let mut rechecked = observation("rechecked", "file:///one", 5);
        rechecked.receipt.as_mut().unwrap().effects.clear();
        values.insert("rechecked".into(), rechecked);
        assert!(
            validate_satisfied_coverage(
                &scope,
                &vec!["action".into(), "rechecked".into()],
                &values
            )
            .is_err(),
            "a new check cannot hide the uncited restoration action"
        );
        assert!(
            validate_satisfied_coverage(&scope, &vec!["restored".into(), "check".into()], &values)
                .is_err(),
            "an old check cannot validate a later restoration"
        );
        assert!(
            validate_satisfied_coverage(
                &scope,
                &vec!["restored".into(), "rechecked".into()],
                &values
            )
            .is_ok(),
            "new exact ordered cited proof is valid after restoration"
        );
    }
}
