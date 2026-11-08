// SPDX-License-Identifier: Apache-2.0
//! Persisted generation intent, distinct from resolved GenerationOptions.
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Absence of this field inherits. A configuration patch's null removes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedOverride {
    Clear,
    Value(i64),
}

impl SeedOverride {
    pub fn resolved(self) -> Option<i64> {
        match self {
            Self::Clear => None,
            Self::Value(value) => Some(value),
        }
    }
}

/// Exact cross-client decimal representation, never a floating-point conversion.
pub fn parse_seed_decimal(value: &str) -> Result<i64, &'static str> {
    let parsed = value
        .parse::<i64>()
        .map_err(|_| "seed must be a canonical i64 decimal string")?;
    if parsed.to_string() != value {
        return Err("seed must be a canonical i64 decimal string");
    }
    Ok(parsed)
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Wire {
    Clear {},
    Value { value: String },
}

impl Serialize for SeedOverride {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Clear => Wire::Clear {},
            Self::Value(value) => Wire::Value {
                value: value.to_string(),
            },
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SeedOverride {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Wire::deserialize(deserializer)? {
            Wire::Clear {} => Ok(Self::Clear),
            Wire::Value { value } => parse_seed_decimal(&value)
                .map(Self::Value)
                .map_err(serde::de::Error::custom),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exact_decimal_round_trips_full_i64_range() {
        for value in [
            i64::MIN,
            -9_007_199_254_740_993,
            -1,
            0,
            1,
            9_007_199_254_740_993,
            i64::MAX,
        ] {
            let wire = json!({"kind":"value","value":value.to_string()});
            let decoded: SeedOverride = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(decoded.resolved(), Some(value));
            assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
        }
        assert_eq!(
            serde_json::to_value(SeedOverride::Clear).unwrap(),
            json!({"kind":"clear"})
        );
        assert_eq!(
            serde_json::from_value::<SeedOverride>(json!({"kind":"clear"}))
                .unwrap()
                .resolved(),
            None
        );
    }

    #[test]
    fn rejects_noncanonical_decimals_and_ambiguous_shapes() {
        for value in [
            "",
            "+1",
            "-0",
            "01",
            "-01",
            " 1",
            "1 ",
            "1.0",
            "1e3",
            "9223372036854775808",
            "-9223372036854775809",
        ] {
            assert!(
                serde_json::from_value::<SeedOverride>(json!({"kind":"value","value":value}))
                    .is_err(),
                "{value}"
            );
        }
        for invalid in [
            json!(0),
            json!(null),
            json!({}),
            json!({"kind":"inherit"}),
            json!({"kind":"value","value":0}),
            json!({"kind":"value"}),
            json!({"kind":"clear","value":"0"}),
            json!({"kind":"clear","unknown":true}),
            json!({"kind":"value","value":"0","unknown":true}),
        ] {
            assert!(
                serde_json::from_value::<SeedOverride>(invalid.clone()).is_err(),
                "{invalid}"
            );
        }
    }
}
