// SPDX-License-Identifier: Apache-2.0
//! Non-secret provenance, not permission to resolve a secret. Hosts must match
//! their trusted local owner, current revision/revocation and exact route before
//! resolving a managed handle. No caller-selected storage queries are accepted.
use serde::{Deserialize, Serialize};

pub const EMPTY_CREDENTIAL_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostSecretAuthority {
    CliHost,
    MacApp,
    IosApp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostSecretPurpose {
    ModelCredential,
    EndpointQuery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostSecretSource {
    None,
    Ephemeral,
    Managed {
        authority: HostSecretAuthority,
        /// Locally generated opaque 128-bit identifier encoded as 32 lowercase
        /// hex digits, never a UUID/account/path copied from external input.
        owner_id: String,
        /// Independently generated opaque identifier using the same encoding.
        handle_id: String,
        /// Positive and JSON-safe across Swift/JavaScript numeric transports.
        revision: u64,
        purpose: HostSecretPurpose,
    },
}

impl<'de> Deserialize<'de> for HostSecretSource {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Internally tagged unit variants discard extra fields in Serde. Empty
        // struct variants enforce the same closed-object contract as Managed.
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            None {},
            Ephemeral {},
            Managed {
                authority: HostSecretAuthority,
                owner_id: String,
                handle_id: String,
                revision: u64,
                purpose: HostSecretPurpose,
            },
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::None {} => Self::None,
            Wire::Ephemeral {} => Self::Ephemeral,
            Wire::Managed {
                authority,
                owner_id,
                handle_id,
                revision,
                purpose,
            } => Self::Managed {
                authority,
                owner_id,
                handle_id,
                revision,
                purpose,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn managed() -> serde_json::Value {
        serde_json::json!({"kind":"managed","authority":"ios_app",
            "owner_id":"0123456789abcdef0123456789abcdef",
            "handle_id":"fedcba9876543210fedcba9876543210",
            "revision":1,"purpose":"model_credential"})
    }

    #[test]
    fn sources_are_closed_and_have_no_storage_query_escape_hatch() {
        for kind in ["none", "ephemeral"] {
            let valid = serde_json::json!({"kind":kind});
            assert!(serde_json::from_value::<HostSecretSource>(valid.clone()).is_ok());
            for key in ["service", "account", "path", "secret", "owner_id"] {
                let mut invalid = valid.clone();
                invalid[key] = serde_json::json!("private");
                assert!(serde_json::from_value::<HostSecretSource>(invalid).is_err());
            }
        }
        let mut value = managed();
        value["service"] = serde_json::json!("arbitrary-keychain-service");
        assert!(serde_json::from_value::<HostSecretSource>(value).is_err());
        for key in ["authority", "purpose", "owner_id", "handle_id", "revision"] {
            let mut value = managed();
            value.as_object_mut().unwrap().remove(key);
            assert!(serde_json::from_value::<HostSecretSource>(value).is_err());
        }
    }

    #[test]
    fn managed_identity_purpose_and_json_safe_revision_are_validated() {
        let valid: HostSecretSource = serde_json::from_value(managed()).unwrap();
        valid.validate(HostSecretPurpose::ModelCredential).unwrap();
        assert!(valid.validate(HostSecretPurpose::EndpointQuery).is_err());
        for key in ["owner_id", "handle_id"] {
            for id in [
                "/tmp/secret",
                "endpoint:account",
                "https://example.invalid",
                "01234567-89ab-cdef-0123-456789abcdef",
                "ABCDEF0123456789ABCDEF0123456789",
            ] {
                let mut value = managed();
                value[key] = serde_json::json!(id);
                let decoded: HostSecretSource = serde_json::from_value(value).unwrap();
                assert!(decoded
                    .validate(HostSecretPurpose::ModelCredential)
                    .is_err());
            }
        }
        for revision in [0, 9_007_199_254_740_992u64, u64::MAX] {
            let mut value = managed();
            value["revision"] = serde_json::json!(revision);
            let decoded: HostSecretSource = serde_json::from_value(value).unwrap();
            assert!(decoded
                .validate(HostSecretPurpose::ModelCredential)
                .is_err());
        }
        let mut value = managed();
        value["revision"] = serde_json::json!(9_007_199_254_740_991u64);
        serde_json::from_value::<HostSecretSource>(value)
            .unwrap()
            .validate(HostSecretPurpose::ModelCredential)
            .unwrap();
    }
}

impl HostSecretSource {
    pub(crate) fn validate(&self, expected: HostSecretPurpose) -> Result<(), &'static str> {
        if let Self::Managed {
            owner_id,
            handle_id,
            revision,
            purpose,
            ..
        } = self
        {
            let opaque_id = |id: &str| {
                id.len() == 32
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            };
            if !opaque_id(owner_id)
                || !opaque_id(handle_id)
                || *revision == 0
                || *revision > 9_007_199_254_740_991
                || *purpose != expected
            {
                return Err("invalid Host secret provenance");
            }
        }
        Ok(())
    }
}
