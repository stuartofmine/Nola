// SPDX-License-Identifier: Apache-2.0
//! Exact input-prefix commitment, not proof of source completeness or authority.
use crate::assistant_journal::AssistantDigest;
use crate::assistant_journal_operations::JournalNumber;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunInputCommitment {
    pub watermark: JournalNumber,
    pub applications_digest: AssistantDigest,
}

impl RunInputCommitment {
    /// SHA-256(domain || u64be(JSON length) || canonical empty array).
    /// This is an explicit empty-prefix value, never a deserialization default.
    pub fn empty_applications_digest() -> AssistantDigest {
        let mut hash = Sha256::new();
        hash.update(b"NOLA-RUN-INPUT-COMMITMENT-V1");
        hash.update(2_u64.to_be_bytes());
        hash.update(b"[]");
        AssistantDigest::parse_hex(&format!("{:x}", hash.finalize()))
            .expect("SHA-256 formats as a valid digest")
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.watermark.0 > i64::MAX as u64 {
            return Err("input watermark exceeds durable integer range".into());
        }
        if (self.watermark.0 == 0)
            != (self.applications_digest == Self::empty_applications_digest())
        {
            return Err("input watermark and empty-prefix commitment disagree".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_prefix_matches_independent_ruby_vector_and_is_exclusive() {
        let empty = RunInputCommitment::empty_applications_digest();
        assert_eq!(
            empty.to_hex(),
            "e0aeaeba0c6a61c02edcc186902f9b41dd82835d01b780eb0e479cc513ea66dd"
        );
        let mut value = RunInputCommitment {
            watermark: JournalNumber(0),
            applications_digest: empty,
        };
        value.validate().unwrap();
        value.watermark = JournalNumber(1);
        assert!(value.validate().is_err());
        value.applications_digest = AssistantDigest::parse_hex(&"a".repeat(64)).unwrap();
        value.validate().unwrap();
        value.watermark = JournalNumber(0);
        assert!(value.validate().is_err());
        value.watermark = JournalNumber(u64::MAX);
        assert!(value.validate().is_err());
    }

    #[test]
    fn wire_requires_both_fields_without_count_only_compatibility() {
        for wire in [
            json!({"watermark":"1"}),
            json!({"input_watermark":"1"}),
            json!({"watermark":"1","applications_digest":"invalid"}),
            json!({"watermark":"1","applications_digest":"a".repeat(64),"extra":true}),
        ] {
            assert!(serde_json::from_value::<RunInputCommitment>(wire).is_err());
        }
    }
}
