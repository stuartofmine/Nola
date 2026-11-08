// SPDX-License-Identifier: Apache-2.0
//! Original-field derivation claims. Transported records are not evidence of
//! local authorship and never authorize semantic ownership or tool execution.
use crate::{CanonicalPartOwner, PartBodyField};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartFieldFingerprint {
    pub field: PartBodyField,
    pub total_utf8_bytes: u64,
    pub sha256: String,
}
/// Fingerprints describe raw bytes, independently of inline/reference storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartDerivationSource {
    pub owner: CanonicalPartOwner,
    pub run_id: Option<String>,
    pub kind: String,
    pub state: String,
    pub fields: [Option<PartFieldFingerprint>; 2],
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartDerivation {
    pub version: u32,
    pub operation_id: String,
    pub target: PartDerivationSource,
    pub immediate_source: PartDerivationSource,
    pub root_source: Option<PartDerivationSource>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transported_claim_cannot_select_receiver_owned_provenance() {
        let source = PartDerivationSource {
            owner: CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap(),
            run_id: None,
            kind: "text".into(),
            state: "completed".into(),
            fields: [
                Some(PartFieldFingerprint {
                    field: PartBodyField::ContentText,
                    total_utf8_bytes: 1,
                    sha256: "a".repeat(64),
                }),
                None,
            ],
        };
        let claim = PartDerivation {
            version: 1,
            operation_id: "operation".into(),
            target: source.clone(),
            immediate_source: source,
            root_source: None,
        };
        let mut wire = serde_json::to_value(&claim).unwrap();
        assert_eq!(
            serde_json::from_value::<PartDerivation>(wire.clone()).unwrap(),
            claim
        );
        wire["provenance"] = "local_branch".into();
        assert!(serde_json::from_value::<PartDerivation>(wire).is_err());
        let mut wire = serde_json::to_value(&claim).unwrap();
        wire["target"]["owner"]["part_id"] = "".into();
        assert!(serde_json::from_value::<PartDerivation>(wire).is_err());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartDerivationProvenance {
    LocalBranch,
    Replica,
    Archive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredPartDerivation {
    pub derivation: PartDerivation,
    pub provenance: PartDerivationProvenance,
}
