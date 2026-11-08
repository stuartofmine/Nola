// SPDX-License-Identifier: Apache-2.0
//! Borrowed diagnostic-only Serialize adapter. Strings are bounded before the
//! destination serializer scans/escapes them; the destination writer remains
//! responsible for the complete encoded-byte budget. Never use for canonical
//! persistence, model requests, recovery or externally supplied contracts.
use serde::ser::{
    SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant,
};
use serde::{Serialize, Serializer};
use std::cell::Cell;
use std::fmt::{self, Display, Write as _};

const STRING_BYTES: usize = 512;
const MAX_DEPTH: usize = 64;

/// All nested values share the same flag. The caller must retain that flag in
/// its preview envelope; a bounded prefix is not a complete structured value.
pub(super) struct BoundedValue<'a, T: ?Sized> {
    value: &'a T,
    truncated: &'a Cell<bool>,
    depth: usize,
}

impl<'a, T: ?Sized> BoundedValue<'a, T> {
    pub(super) fn new(value: &'a T, truncated: &'a Cell<bool>) -> Self {
        Self {
            value,
            truncated,
            depth: 0,
        }
    }
}

impl<T: Serialize + ?Sized> Serialize for BoundedValue<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.depth >= MAX_DEPTH {
            self.truncated.set(true);
            // Unlike an actual None/null, this has an explicit diagnostic marker.
            return serializer.serialize_str("[depth omitted]");
        }
        self.value.serialize(Adapter {
            inner: serializer,
            truncated: self.truncated,
            depth: self.depth,
        })
    }
}

fn prefix<'a>(value: &'a str, truncated: &Cell<bool>) -> &'a str {
    if value.len() <= STRING_BYTES {
        return value;
    }
    truncated.set(true);
    let mut end = STRING_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

struct Adapter<'a, S> {
    inner: S,
    truncated: &'a Cell<bool>,
    depth: usize,
}

struct Compound<'a, S> {
    inner: S,
    truncated: &'a Cell<bool>,
    depth: usize,
}

macro_rules! forward_scalar {
    ($($method:ident($value:ident: $kind:ty);)*) => {$(
        fn $method(self, $value: $kind) -> Result<Self::Ok, Self::Error> {
            self.inner.$method($value)
        }
    )*};
}

impl<'a, S: Serializer> Serializer for Adapter<'a, S> {
    type Ok = S::Ok;
    type Error = S::Error;
    type SerializeSeq = Compound<'a, S::SerializeSeq>;
    type SerializeTuple = Compound<'a, S::SerializeTuple>;
    type SerializeTupleStruct = Compound<'a, S::SerializeTupleStruct>;
    type SerializeTupleVariant = Compound<'a, S::SerializeTupleVariant>;
    type SerializeMap = Compound<'a, S::SerializeMap>;
    type SerializeStruct = Compound<'a, S::SerializeStruct>;
    type SerializeStructVariant = Compound<'a, S::SerializeStructVariant>;

    forward_scalar! {
        serialize_bool(value: bool);
        serialize_i8(value: i8);
        serialize_i16(value: i16);
        serialize_i32(value: i32);
        serialize_i64(value: i64);
        serialize_i128(value: i128);
        serialize_u8(value: u8);
        serialize_u16(value: u16);
        serialize_u32(value: u32);
        serialize_u64(value: u64);
        serialize_u128(value: u128);
        serialize_f32(value: f32);
        serialize_f64(value: f64);
        serialize_char(value: char);
    }

    fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
        self.inner.serialize_str(prefix(value, self.truncated))
    }

    fn serialize_bytes(self, value: &[u8]) -> Result<Self::Ok, Self::Error> {
        if value.len() > STRING_BYTES {
            self.truncated.set(true);
        }
        self.inner
            .serialize_bytes(&value[..value.len().min(STRING_BYTES)])
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        self.inner.serialize_none()
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        self.inner.serialize_some(&BoundedValue {
            value,
            truncated: self.truncated,
            depth: self.depth + 1,
        })
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        self.inner.serialize_unit()
    }

    fn serialize_unit_struct(self, name: &'static str) -> Result<Self::Ok, Self::Error> {
        self.inner
            .serialize_unit_struct(prefix(name, self.truncated))
    }

    fn serialize_unit_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        self.inner.serialize_unit_variant(
            prefix(name, self.truncated),
            index,
            prefix(variant, self.truncated),
        )
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.inner.serialize_newtype_struct(
            prefix(name, self.truncated),
            &BoundedValue {
                value,
                truncated: self.truncated,
                depth: self.depth + 1,
            },
        )
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.inner.serialize_newtype_variant(
            prefix(name, self.truncated),
            index,
            prefix(variant, self.truncated),
            &BoundedValue {
                value,
                truncated: self.truncated,
                depth: self.depth + 1,
            },
        )
    }

    fn serialize_seq(self, length: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(Compound {
            inner: self.inner.serialize_seq(length)?,
            truncated: self.truncated,
            depth: self.depth + 1,
        })
    }

    fn serialize_tuple(self, length: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(Compound {
            inner: self.inner.serialize_tuple(length)?,
            truncated: self.truncated,
            depth: self.depth + 1,
        })
    }

    fn serialize_tuple_struct(
        self,
        name: &'static str,
        length: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(Compound {
            inner: self
                .inner
                .serialize_tuple_struct(prefix(name, self.truncated), length)?,
            truncated: self.truncated,
            depth: self.depth + 1,
        })
    }

    fn serialize_tuple_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        length: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(Compound {
            inner: self.inner.serialize_tuple_variant(
                prefix(name, self.truncated),
                index,
                prefix(variant, self.truncated),
                length,
            )?,
            truncated: self.truncated,
            depth: self.depth + 1,
        })
    }

    fn serialize_map(self, length: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(Compound {
            inner: self.inner.serialize_map(length)?,
            truncated: self.truncated,
            depth: self.depth + 1,
        })
    }

    fn serialize_struct(
        self,
        name: &'static str,
        length: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(Compound {
            inner: self
                .inner
                .serialize_struct(prefix(name, self.truncated), length)?,
            truncated: self.truncated,
            depth: self.depth + 1,
        })
    }

    fn serialize_struct_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        length: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(Compound {
            inner: self.inner.serialize_struct_variant(
                prefix(name, self.truncated),
                index,
                prefix(variant, self.truncated),
                length,
            )?,
            truncated: self.truncated,
            depth: self.depth + 1,
        })
    }

    fn collect_str<T: Display + ?Sized>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        // Do not call to_string/default collect_str: stop standard formatting
        // at a bounded UTF-8 sink before the JSON string encoder sees a value.
        let mut output = DisplayPrefix {
            value: String::with_capacity(STRING_BYTES),
            truncated: self.truncated,
            overflowed: false,
        };
        if write!(&mut output, "{value}").is_err() && !output.overflowed {
            return Err(serde::ser::Error::custom("diagnostic formatter failed"));
        }
        self.inner.serialize_str(&output.value)
    }

    fn is_human_readable(&self) -> bool {
        self.inner.is_human_readable()
    }
}

struct DisplayPrefix<'a> {
    value: String,
    truncated: &'a Cell<bool>,
    overflowed: bool,
}
impl fmt::Write for DisplayPrefix<'_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let remaining = STRING_BYTES - self.value.len();
        if value.len() <= remaining {
            self.value.push_str(value);
            return Ok(());
        }
        self.truncated.set(true);
        self.overflowed = true;
        let mut end = remaining;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        self.value.push_str(&value[..end]);
        Err(fmt::Error)
    }
}

macro_rules! compound_value {
    ($trait_name:ident, $method:ident) => {
        impl<S: $trait_name> $trait_name for Compound<'_, S> {
            type Ok = S::Ok;
            type Error = S::Error;
            fn $method<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
                self.inner.$method(&BoundedValue {
                    value,
                    truncated: self.truncated,
                    depth: self.depth,
                })
            }
            fn end(self) -> Result<Self::Ok, Self::Error> {
                self.inner.end()
            }
        }
    };
}
compound_value!(SerializeSeq, serialize_element);
compound_value!(SerializeTuple, serialize_element);
compound_value!(SerializeTupleStruct, serialize_field);
compound_value!(SerializeTupleVariant, serialize_field);

impl<S: SerializeMap> SerializeMap for Compound<'_, S> {
    type Ok = S::Ok;
    type Error = S::Error;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        self.inner.serialize_key(&BoundedValue {
            value,
            truncated: self.truncated,
            depth: self.depth,
        })
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        self.inner.serialize_value(&BoundedValue {
            value,
            truncated: self.truncated,
            depth: self.depth,
        })
    }
    fn serialize_entry<K: Serialize + ?Sized, V: Serialize + ?Sized>(
        &mut self,
        key: &K,
        value: &V,
    ) -> Result<(), Self::Error> {
        self.inner.serialize_entry(
            &BoundedValue {
                value: key,
                truncated: self.truncated,
                depth: self.depth,
            },
            &BoundedValue {
                value,
                truncated: self.truncated,
                depth: self.depth,
            },
        )
    }
    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.inner.end()
    }
}

macro_rules! compound_fields {
    ($trait_name:ident) => {
        impl<S: $trait_name> $trait_name for Compound<'_, S> {
            type Ok = S::Ok;
            type Error = S::Error;
            fn serialize_field<T: Serialize + ?Sized>(
                &mut self,
                key: &'static str,
                value: &T,
            ) -> Result<(), Self::Error> {
                self.inner.serialize_field(
                    prefix(key, self.truncated),
                    &BoundedValue {
                        value,
                        truncated: self.truncated,
                        depth: self.depth,
                    },
                )
            }
            fn skip_field(&mut self, key: &'static str) -> Result<(), Self::Error> {
                self.inner.skip_field(prefix(key, self.truncated))
            }
            fn end(self) -> Result<Self::Ok, Self::Error> {
                self.inner.end()
            }
        }
    };
}
compound_fields!(SerializeStruct);
compound_fields!(SerializeStructVariant);

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::Write;

    fn encoded<T: Serialize + ?Sized>(value: &T) -> (String, bool) {
        let truncated = Cell::new(false);
        let json = serde_json::to_string(&BoundedValue::new(value, &truncated)).unwrap();
        (json, truncated.get())
    }

    #[derive(Serialize)]
    struct Unit;
    #[derive(Serialize)]
    struct Newtype<T>(T);
    #[derive(Serialize)]
    struct TupleStruct(bool, u64, Unit);
    #[derive(Serialize)]
    enum EveryVariant<'a> {
        Unit,
        Newtype(&'a str),
        Tuple(i64, Option<&'a str>),
        Struct { title: &'a str, value: u64 },
    }

    #[test]
    fn primitives_and_every_compound_preserve_complete_wire_values() {
        #[derive(Serialize)]
        struct Values<'a> {
            bool_: bool,
            i8_: i8,
            i16_: i16,
            i32_: i32,
            i64_: i64,
            i128_: i128,
            u8_: u8,
            u16_: u16,
            u32_: u32,
            u64_: u64,
            u128_: u128,
            f32_: f32,
            f64_: f64,
            char_: char,
            str_: &'a str,
            none: Option<&'a str>,
            some: Option<&'a str>,
            unit: (),
            unit_struct: Unit,
            newtype: Newtype<u64>,
            tuple: (u8, bool, &'a str),
            tuple_struct: TupleStruct,
            sequence: Vec<EveryVariant<'a>>,
            map: BTreeMap<&'a str, Option<&'a str>>,
        }
        let value = Values {
            bool_: true,
            i8_: i8::MIN,
            i16_: i16::MIN,
            i32_: i32::MIN,
            i64_: i64::MIN,
            i128_: i128::MIN,
            u8_: u8::MAX,
            u16_: u16::MAX,
            u32_: u32::MAX,
            u64_: u64::MAX,
            u128_: u128::MAX,
            f32_: -0.0,
            f64_: 0.5,
            char_: '🙂',
            str_: "escaped\n汉🙂",
            none: None,
            some: Some(""),
            unit: (),
            unit_struct: Unit,
            newtype: Newtype(7),
            tuple: (1, false, "tuple"),
            tuple_struct: TupleStruct(true, 5, Unit),
            sequence: vec![
                EveryVariant::Unit,
                EveryVariant::Newtype("text"),
                EveryVariant::Tuple(-1, Some("")),
                EveryVariant::Struct {
                    title: "title",
                    value: u64::MAX,
                },
            ],
            map: BTreeMap::from([("null", None), ("empty", Some(""))]),
        };
        let (actual, truncated) = encoded(&value);
        assert_eq!(actual, serde_json::to_string(&value).unwrap());
        assert!(!truncated);
    }

    #[test]
    fn typed_origin_permission_artifacts_and_evidence_clip_before_string_encoding() {
        let huge = "汉🙂\n".repeat(150_000);
        #[derive(Serialize)]
        struct Origin<'a> {
            provider_id: &'a str,
            provider_title: &'a str,
            namespace: Option<&'a str>,
        }
        #[derive(Serialize)]
        struct Permission<'a> {
            rationale: &'a str,
            decision: Option<&'a str>,
        }
        #[derive(Serialize)]
        struct Artifact<'a> {
            uri: &'a str,
            name: &'a str,
            mime_type: Option<&'a str>,
        }
        #[derive(Serialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        enum Evidence<'a> {
            Memory {
                source: &'a str,
                title: Option<&'a str>,
                supersedes: Vec<&'a str>,
            },
        }
        #[derive(Serialize)]
        struct Fields<'a> {
            origin: Origin<'a>,
            permission: Permission<'a>,
            artifacts: Vec<Artifact<'a>>,
            evidence: Evidence<'a>,
        }
        let value = Fields {
            origin: Origin {
                provider_id: &huge,
                provider_title: &huge,
                namespace: Some(""),
            },
            permission: Permission {
                rationale: &huge,
                decision: None,
            },
            artifacts: vec![Artifact {
                uri: &huge,
                name: &huge,
                mime_type: None,
            }],
            evidence: Evidence::Memory {
                source: &huge,
                title: Some(&huge),
                supersedes: vec![&huge],
            },
        };
        let (json, truncated) = encoded(&value);
        assert!(truncated && json.len() < 10_000);
        let decoded: serde_json::Value = serde_json::from_str(&json).unwrap();
        for field in [
            &decoded["origin"]["provider_id"],
            &decoded["origin"]["provider_title"],
            &decoded["permission"]["rationale"],
            &decoded["artifacts"][0]["uri"],
            &decoded["artifacts"][0]["name"],
            &decoded["evidence"]["source"],
            &decoded["evidence"]["title"],
            &decoded["evidence"]["supersedes"][0],
        ] {
            let text = field.as_str().unwrap();
            assert!(text.len() <= STRING_BYTES && huge.starts_with(text));
        }
        assert_eq!(decoded["origin"]["namespace"], "");
        assert!(decoded["permission"]["decision"].is_null());
    }

    #[test]
    fn map_keys_and_enum_payloads_are_utf8_bounded() {
        let huge = "🙂汉".repeat(200_000);
        let (json, truncated) = encoded(&BTreeMap::from([(
            huge.as_str(),
            EveryVariant::Newtype(huge.as_str()),
        )]));
        assert!(truncated);
        let decoded: serde_json::Value = serde_json::from_str(&json).unwrap();
        let (key, value) = decoded.as_object().unwrap().iter().next().unwrap();
        assert!(key.len() <= STRING_BYTES && huge.starts_with(key));
        let text = value["Newtype"].as_str().unwrap();
        assert!(text.len() <= STRING_BYTES && huge.starts_with(text));
        let exact = "🙂".repeat(128);
        assert!(!encoded(&exact).1);
        assert!(encoded(&format!("{exact}x")).1);
    }

    struct Formatted<'a> {
        visited: &'a Cell<usize>,
        count: usize,
    }
    impl Display for Formatted<'_> {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            for _ in 0..self.count {
                self.visited.set(self.visited.get() + 1);
                formatter.write_str("汉")?;
            }
            Ok(())
        }
    }
    impl Serialize for Formatted<'_> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_str(self)
        }
    }

    #[test]
    fn collect_str_stops_formatting_at_byte_bound_including_map_keys() {
        let visited = Cell::new(0);
        let formatted = Formatted {
            visited: &visited,
            count: 1_000_000,
        };
        let (json, truncated) = encoded(&formatted);
        assert!(truncated);
        assert!(visited.get() <= STRING_BYTES / "汉".len() + 1);
        assert_eq!(serde_json::from_str::<String>(&json).unwrap().len(), 510);
        struct Key<'a>(&'a Formatted<'a>);
        impl Serialize for Key<'_> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(self.0, &true)?;
                map.end()
            }
        }
        visited.set(0);
        let (json, truncated) = encoded(&Key(&formatted));
        assert!(truncated && visited.get() <= STRING_BYTES / "汉".len() + 1);
        assert!(serde_json::from_str::<serde_json::Value>(&json)
            .unwrap()
            .is_object());
    }

    #[test]
    fn depth_guard_prevents_visiting_deeper_typed_values_and_preserves_null_absence() {
        struct Deep<'a> {
            remaining: usize,
            visited: &'a Cell<usize>,
        }
        impl Serialize for Deep<'_> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                self.visited.set(self.visited.get() + 1);
                if self.remaining == 0 {
                    serializer.serialize_none()
                } else {
                    serializer.serialize_some(&Deep {
                        remaining: self.remaining - 1,
                        visited: self.visited,
                    })
                }
            }
        }
        let visited = Cell::new(0);
        let (json, truncated) = encoded(&Deep {
            remaining: 10_000,
            visited: &visited,
        });
        assert!(truncated);
        assert_eq!(visited.get(), MAX_DEPTH);
        assert_eq!(
            serde_json::from_str::<String>(&json).unwrap(),
            "[depth omitted]"
        );
        #[derive(Serialize)]
        struct Optional<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            missing: Option<&'a str>,
            null: Option<&'a str>,
            empty: Option<&'a str>,
            nested: Option<Option<&'a str>>,
        }
        let value = Optional {
            missing: None,
            null: None,
            empty: Some(""),
            nested: Some(None),
        };
        let (json, truncated) = encoded(&value);
        assert!(!truncated);
        assert_eq!(json, serde_json::to_string(&value).unwrap());
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value.get("missing").is_none());
        assert!(value["null"].is_null() && value["nested"].is_null());
        assert_eq!(value["empty"], "");
    }

    #[test]
    fn raw_bytes_are_bounded_but_small_complete_bytes_stay_identical() {
        struct Bytes<'a>(&'a [u8]);
        impl Serialize for Bytes<'_> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_bytes(self.0)
            }
        }
        let small = Bytes(&[0, 1, 255]);
        assert_eq!(
            encoded(&small),
            (serde_json::to_string(&small).unwrap(), false)
        );
        let large = [7; 10_000];
        let (json, truncated) = encoded(&Bytes(&large));
        assert!(truncated);
        assert_eq!(
            serde_json::from_str::<Vec<u8>>(&json).unwrap().len(),
            STRING_BYTES
        );
    }

    #[test]
    fn static_field_names_and_unit_variants_are_bounded_and_format_errors_propagate() {
        let key: &'static str = Box::leak("键🙂".repeat(200).into_boxed_str());
        struct Named(&'static str);
        impl Serialize for Named {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut fields = serializer.serialize_struct(self.0, 1)?;
                fields.serialize_field(self.0, &true)?;
                fields.end()
            }
        }
        let (json, truncated) = encoded(&Named(key));
        assert!(truncated);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let name = value.as_object().unwrap().keys().next().unwrap();
        assert!(name.len() <= STRING_BYTES && key.starts_with(name));
        struct Variant(&'static str);
        impl Serialize for Variant {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_unit_variant(self.0, 0, self.0)
            }
        }
        let (json, truncated) = encoded(&Variant(key));
        assert!(truncated && serde_json::from_str::<String>(&json).unwrap().len() <= STRING_BYTES);
        struct Broken;
        impl Display for Broken {
            fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
                Err(fmt::Error)
            }
        }
        impl Serialize for Broken {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }
        let already_truncated = Cell::new(true);
        assert!(serde_json::to_string(&BoundedValue::new(&Broken, &already_truncated)).is_err());
    }

    #[test]
    fn total_writer_budget_stops_visiting_long_sequences() {
        struct Counted<'a> {
            visited: &'a Cell<usize>,
        }
        impl Serialize for Counted<'_> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                self.visited.set(self.visited.get() + 1);
                serializer.serialize_str("small string")
            }
        }
        struct Sequence<'a>(&'a Cell<usize>);
        impl Serialize for Sequence<'_> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut sequence = serializer.serialize_seq(None)?;
                for _ in 0..100_000 {
                    sequence.serialize_element(&Counted { visited: self.0 })?;
                }
                sequence.end()
            }
        }
        struct Budget {
            remaining: usize,
        }
        impl Write for Budget {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() > self.remaining {
                    return Err(std::io::Error::other("full"));
                }
                self.remaining -= bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let visited = Cell::new(0);
        let truncated = Cell::new(false);
        let mut writer = Budget { remaining: 64 };
        assert!(serde_json::to_writer(
            &mut writer,
            &BoundedValue::new(&Sequence(&visited), &truncated)
        )
        .is_err());
        assert!(visited.get() < 10);
        // Destination-byte exhaustion is signalled by the writer's error.
        assert!(!truncated.get());
    }
}
