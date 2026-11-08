// SPDX-License-Identifier: Apache-2.0
//! Reject ambiguous object members before materializing protocol JSON.
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::fmt;

struct UniqueValue(Value);
impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON with unique object keys")
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> { Ok(UniqueValue(Value::Bool(value))) }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> { Ok(UniqueValue(Value::Number(value.into()))) }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> { Ok(UniqueValue(Value::Number(value.into()))) }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Number::from_f64(value).map(|n| UniqueValue(Value::Number(n))).ok_or_else(|| E::custom("non-finite JSON number"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> { Ok(UniqueValue(Value::String(value.into()))) }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> { Ok(UniqueValue(Value::String(value))) }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> { Ok(UniqueValue(Value::Null)) }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueValue(value)) = seq.next_element()? { values.push(value); }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) { return Err(de::Error::custom("duplicate JSON object key")); }
                    let UniqueValue(value) = map.next_value()?;
                    values.insert(key, value);
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

pub fn decode(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    serde_json::from_slice::<UniqueValue>(bytes).map(|value| value.0)
}

#[cfg(test)]
mod tests {
    use super::decode;
    #[test]
    fn rejects_duplicate_members_at_every_depth_and_after_escape_decoding() {
        for raw in [r#"{"version":"0.9.1","version":"0.9.1"}"#,
            r#"{"value":{"name":1,"na\u006de":2}}"#,
            r#"[{"value":null,"value":3}]"#] {
            assert!(decode(raw.as_bytes()).is_err());
        }
    }
    #[test]
    fn preserves_json_types_and_allows_equal_keys_in_distinct_objects() {
        let raw = br#"[{"x":null},{"x":[true,-1,2,1.25,"text"]}]"#;
        assert_eq!(decode(raw).unwrap(), serde_json::from_slice::<serde_json::Value>(raw).unwrap());
        assert!(decode(b"{} {}").is_err());
    }
}
