// SPDX-License-Identifier: Apache-2.0
//! Complete reducer facts, never a second copy of a frozen request or prompt.
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub const HOST_REPLAY_FACTS_VERSION: u16 = 1;
// Hard admission bound for a valid recorded tail. Keep separate from the
// normal writer cadence: a readable tail is not necessarily cheap to replay.
pub const HOST_REPLAY_MAX_RECORDS: usize = 4095;
/// Periodic writer checkpoint cadence, including a single long model stream.
pub const HOST_REPLAY_CHECKPOINT_RECORDS: usize = 128;
pub const HOST_REPLAY_MAX_BYTES: usize = 4 * 1024 * 1024;

pub use nola_domain::host_control::{CoreHostBindingOverlay, CoreHostPayloadBinding};

/// Migration marks unsupported historical evidence explicitly. Only Replayable
/// can enter a live recovery tail; neither other variant manufactures authority.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostReplayReduction {
    Replayable {
        protocol_version: u16,
        decision_time_ms: i64,
        normalized_response: Box<RawValue>,
        binding: Option<CoreHostBindingOverlay>,
    },
    BaselineCovered {
        protocol_version: u16,
        decision_time_ms: i64,
        normalized_response: Box<RawValue>,
    },
    RecoveryRequired {
        protocol_version: u16,
        decision_time_ms: i64,
        normalized_response: Box<RawValue>,
        reason: String,
    },
}

// Deserialize the raw envelope directly from JSON's map visitor. Serde's
// internally-tagged enum buffering would first materialize generic Content and
// lose the original RawValue token/bytes before selecting a variant.
impl<'de> Deserialize<'de> for HostReplayReduction {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        fn present_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
        where
            D: serde::Deserializer<'de>,
            T: Deserialize<'de>,
        {
            Option::deserialize(deserializer).map(Some)
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            status: String,
            protocol_version: u16,
            decision_time_ms: i64,
            normalized_response: Box<RawValue>,
            #[serde(default, deserialize_with = "present_option")]
            binding: Option<Option<CoreHostBindingOverlay>>,
            #[serde(default)]
            reason: Option<String>,
        }
        let wire = Wire::deserialize(deserializer)?;
        match (wire.status.as_str(), wire.binding, wire.reason) {
            ("replayable", Some(binding), None) => Ok(Self::Replayable {
                protocol_version: wire.protocol_version,
                decision_time_ms: wire.decision_time_ms,
                normalized_response: wire.normalized_response,
                binding,
            }),
            ("baseline_covered", None, None) => Ok(Self::BaselineCovered {
                protocol_version: wire.protocol_version,
                decision_time_ms: wire.decision_time_ms,
                normalized_response: wire.normalized_response,
            }),
            ("recovery_required", None, Some(reason)) if !reason.is_empty() => {
                Ok(Self::RecoveryRequired {
                    protocol_version: wire.protocol_version,
                    decision_time_ms: wire.decision_time_ms,
                    normalized_response: wire.normalized_response,
                    reason,
                })
            }
            _ => Err(serde::de::Error::custom(
                "invalid or incomplete Host replay facts",
            )),
        }
    }
}

impl HostReplayReduction {
    pub fn protocol_version(&self) -> u16 {
        match self {
            Self::Replayable {
                protocol_version, ..
            }
            | Self::BaselineCovered {
                protocol_version, ..
            }
            | Self::RecoveryRequired {
                protocol_version, ..
            } => *protocol_version,
        }
    }

    pub fn decision_time_ms(&self) -> i64 {
        match self {
            Self::Replayable {
                decision_time_ms, ..
            }
            | Self::BaselineCovered {
                decision_time_ms, ..
            }
            | Self::RecoveryRequired {
                decision_time_ms, ..
            } => *decision_time_ms,
        }
    }

    pub fn normalized_response(&self) -> &RawValue {
        match self {
            Self::Replayable {
                normalized_response,
                ..
            }
            | Self::BaselineCovered {
                normalized_response,
                ..
            }
            | Self::RecoveryRequired {
                normalized_response,
                ..
            } => normalized_response,
        }
    }

    pub fn validate_clock(&self, stored_decision_time_ms: i64) -> Result<(), &'static str> {
        if self.protocol_version() != HOST_REPLAY_FACTS_VERSION {
            return Err("unsupported Host replay facts version");
        }
        if self.decision_time_ms() != stored_decision_time_ms {
            return Err("Host replay decision clock differs from its committed input");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_no_bind_and_original_normalized_bytes_are_required() {
        let original = r#"{ "payload" : {"type":"accepted"} }"#;
        let facts = HostReplayReduction::Replayable {
            protocol_version: HOST_REPLAY_FACTS_VERSION,
            decision_time_ms: 7,
            normalized_response: RawValue::from_string(original.into()).unwrap(),
            binding: None,
        };
        let encoded = serde_json::to_string(&facts).unwrap();
        let decoded: HostReplayReduction = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.normalized_response().get(), original);
        decoded.validate_clock(7).unwrap();
        assert!(decoded.validate_clock(8).is_err());
        let mut missing: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        missing.as_object_mut().unwrap().remove("binding");
        assert!(serde_json::from_str::<HostReplayReduction>(&missing.to_string()).is_err());
    }
}
