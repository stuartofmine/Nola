// SPDX-License-Identifier: Apache-2.0
//! Checks declared object coverage, never completeness of natural-language intent.
use super::requirements::ObservedEvidenceReference;
use nola_domain::execution_evidence::{ExecutionCheckKind, ExecutionState};
use nola_domain::run_requirements::RequirementResourceScope;
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
                            check.kind == ExecutionCheckKind::Postcondition
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
                            check.kind == ExecutionCheckKind::Postcondition
                                && check.passed
                                && check.resource_uri == claim.resource_uri
                                && &check.version == version
                                && &check.predicate == predicate
                        })
                    }) {
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
