// SPDX-License-Identifier: Apache-2.0
//! Compare recognized embedded identities with canonical owner metadata.
use nola_domain::{CanonicalPartOwner, SemanticIdentityAssessment};

#[derive(Clone, Copy)]
pub enum SemanticOwnerClaim<'a> {
    Part {
        part_id: &'a str,
    },
    AgentPart {
        part_id: &'a str,
        message_id: &'a str,
        turn_id: &'a str,
    },
}

pub fn assess_semantic_identity(
    owner: &CanonicalPartOwner,
    canonical_run_id: Option<&str>,
    claim: SemanticOwnerClaim<'_>,
) -> SemanticIdentityAssessment {
    use SemanticIdentityAssessment::*;
    match claim {
        SemanticOwnerClaim::Part { part_id } => {
            if part_id == owner.part_id() {
                Canonical
            } else {
                DerivationRequired
            }
        }
        SemanticOwnerClaim::AgentPart {
            part_id,
            message_id,
            turn_id,
        } => {
            if part_id != owner.part_id() || message_id != owner.message_id() {
                DerivationRequired
            } else {
                match canonical_run_id {
                    Some(run) if run == turn_id => Canonical,
                    Some(_) => DerivationRequired,
                    None => CanonicalTurnUnavailable,
                }
            }
        }
    }
}

/// The caller must revalidate the stored target's raw fields and metadata in
/// the same snapshot as extraction. A transported record is not this evidence.
/// Historical binding does not authorize tools, permissions, or Run recovery.
pub fn assess_semantic_identity_with_derivation(
    owner: &CanonicalPartOwner,
    canonical_run_id: Option<&str>,
    claim: SemanticOwnerClaim<'_>,
    validated_record: Option<&nola_domain::StoredPartDerivation>,
) -> SemanticIdentityAssessment {
    use nola_domain::{PartDerivationProvenance, SemanticIdentityAssessment::*};
    let direct = assess_semantic_identity(owner, canonical_run_id, claim);
    if direct == Canonical {
        return direct;
    }
    let Some(stored) = validated_record else {
        return direct;
    };
    let record = &stored.derivation;
    if stored.provenance != PartDerivationProvenance::LocalBranch
        || record.version != 1
        || record.target.owner != *owner
        || record.target.run_id.as_deref() != canonical_run_id
    {
        return direct;
    }
    let Some(root) = &record.root_source else {
        return direct;
    };
    if assess_semantic_identity(&root.owner, root.run_id.as_deref(), claim) == Canonical {
        VerifiedLocalDerivation
    } else {
        direct
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SemanticIdentityAssessment::*;
    #[test]
    fn embedded_owner_never_supplies_missing_or_conflicting_canonical_identity() {
        let owner = CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap();
        assert_eq!(
            assess_semantic_identity(&owner, None, SemanticOwnerClaim::Part { part_id: "p" }),
            Canonical
        );
        assert_eq!(
            assess_semantic_identity(&owner, None, SemanticOwnerClaim::Part { part_id: "old" }),
            DerivationRequired
        );
        for (part, message, turn, canonical, expected) in [
            ("p", "m", "run", Some("run"), Canonical),
            ("p", "m", "run", None, CanonicalTurnUnavailable),
            ("old", "m", "run", Some("run"), DerivationRequired),
            ("p", "old", "run", Some("run"), DerivationRequired),
            ("p", "m", "old", Some("run"), DerivationRequired),
        ] {
            assert_eq!(
                assess_semantic_identity(
                    &owner,
                    canonical,
                    SemanticOwnerClaim::AgentPart {
                        part_id: part,
                        message_id: message,
                        turn_id: turn,
                    }
                ),
                expected
            );
        }
    }
    #[test]
    fn derivation_binding_requires_local_origin_current_target_and_complete_root_identity() {
        use nola_domain::{
            PartDerivation, PartDerivationProvenance, PartDerivationSource, StoredPartDerivation,
        };
        let owner = CanonicalPartOwner::new("branch".into(), "bm".into(), "bp".into()).unwrap();
        let root = PartDerivationSource {
            owner: CanonicalPartOwner::new("root".into(), "m".into(), "p".into()).unwrap(),
            run_id: Some("run".into()),
            kind: "text".into(),
            state: "completed".into(),
            fields: [None, None],
        };
        let mut record = StoredPartDerivation {
            provenance: PartDerivationProvenance::LocalBranch,
            derivation: PartDerivation {
                version: 1,
                operation_id: "branch-op".into(),
                target: PartDerivationSource {
                    owner: owner.clone(),
                    run_id: None,
                    ..root.clone()
                },
                immediate_source: root.clone(),
                root_source: Some(root.clone()),
            },
        };
        let claim = SemanticOwnerClaim::AgentPart {
            part_id: "p",
            message_id: "m",
            turn_id: "run",
        };
        let check = |record: &StoredPartDerivation| {
            assess_semantic_identity_with_derivation(&owner, None, claim, Some(record))
        };
        assert_eq!(check(&record), VerifiedLocalDerivation);
        record.derivation.immediate_source.owner =
            CanonicalPartOwner::new("intermediate".into(), "im".into(), "ip".into()).unwrap();
        assert_eq!(check(&record), VerifiedLocalDerivation);
        assert_eq!(
            assess_semantic_identity_with_derivation(
                &owner,
                None,
                SemanticOwnerClaim::Part { part_id: "ip" },
                Some(&record)
            ),
            DerivationRequired
        );
        assert_eq!(
            assess_semantic_identity_with_derivation(
                &owner,
                None,
                SemanticOwnerClaim::AgentPart {
                    part_id: "p",
                    message_id: "m",
                    turn_id: "wrong-run"
                },
                Some(&record)
            ),
            DerivationRequired
        );
        for origin in [
            PartDerivationProvenance::Replica,
            PartDerivationProvenance::Archive,
        ] {
            record.provenance = origin;
            assert_eq!(check(&record), DerivationRequired);
        }
        record.provenance = PartDerivationProvenance::LocalBranch;
        record.derivation.root_source.as_mut().unwrap().run_id = None;
        assert_eq!(check(&record), DerivationRequired);
        record.derivation.root_source = None;
        assert_eq!(check(&record), DerivationRequired);
        record.derivation.root_source = Some(root);
        record.derivation.target.run_id = Some("new-run".into());
        assert_eq!(check(&record), DerivationRequired);
        record.derivation.target.run_id = None;
        record.derivation.target.owner = record.derivation.immediate_source.owner.clone();
        assert_eq!(check(&record), DerivationRequired);
        assert_eq!(
            assess_semantic_identity_with_derivation(
                &owner,
                None,
                SemanticOwnerClaim::Part { part_id: "bp" },
                Some(&record)
            ),
            Canonical
        );
    }
}
