// SPDX-License-Identifier: Apache-2.0
//! Character-based estimates matching compact serde JSON, without a JSON string copy.
use nola_domain::item_token_estimate::ItemTokenEstimateFacts;
use std::io::{self, Write};

pub const ITEM_FRAMING_TOKENS: usize = 4;

pub fn estimated_item_tokens(facts: ItemTokenEstimateFacts<'_>) -> usize {
    let chars = match facts {
        ItemTokenEstimateFacts::Text(text) => text.chars().count(),
        ItemTokenEstimateFacts::Attachment {
            uri,
            name,
            mime_type,
        } => uri
            .chars()
            .count()
            .saturating_add(name.chars().count())
            .saturating_add(mime_type.unwrap_or_default().chars().count()),
        ItemTokenEstimateFacts::Reasoning { summary, content } => summary
            .chars()
            .count()
            .saturating_add(content.chars().count()),
        ItemTokenEstimateFacts::NamedJson { name, value } => name
            .chars()
            .count()
            .saturating_add(json_character_count(value)),
    };
    chars.saturating_add(3) / 4 + ITEM_FRAMING_TOKENS
}

pub fn json_character_count(value: &serde_json::Value) -> usize {
    let mut counter = CharacterCounter::default();
    // Value serialization is infallible for this writer: it emits existing
    // JSON values and Write never rejects data. Serializer scratch is bounded
    // per value; recursive traversal retains only its ordinary nesting stack.
    serde_json::to_writer(&mut counter, value)
        .expect("JSON Value must serialize to an infallible counter");
    counter.characters
}

#[derive(Default)]
struct CharacterCounter {
    characters: usize,
}
impl Write for CharacterCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        // UTF-8 continuation bytes never begin a scalar. Counting leading
        // bytes remains exact even if a scalar spans separate writer calls.
        self.characters = self
            .characters
            .saturating_add(bytes.iter().filter(|byte| **byte & 0xc0 != 0x80).count());
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    #[test]
    fn compact_json_oracle_covers_escaping_unicode_nested_values_and_numbers() {
        let mut values = vec![
            Value::Null,
            json!(true),
            json!(false),
            json!(i64::MIN),
            json!(u64::MAX),
            json!(-0.0),
            json!(1.25e-30),
            json!({"中文🦀":"é\\\"\n\t\r\u{0}\u{001f}","nested":[{},[],null,false,42,3.14159]}),
        ];
        for scalar in 0..=0x7f {
            values.push(json!(char::from_u32(scalar).unwrap().to_string()));
        }
        values.push(json!("🦀é中文\u{2028}\u{2029}\\/"));
        let mut nested = json!("leaf🦀\n");
        for depth in 0..32 {
            nested = json!({format!("level-{depth}"): [nested]});
        }
        values.push(nested);
        for value in values {
            let oracle = value.to_string().chars().count();
            assert_eq!(json_character_count(&value), oracle, "{value:?}");
            assert_eq!(
                estimated_item_tokens(ItemTokenEstimateFacts::NamedJson {
                    name: "工具",
                    value: &value
                }),
                (oracle + 2 + 3) / 4 + 4
            );
        }
    }
    #[test]
    fn scalar_boundaries_and_borrowed_item_fields_match_original_formula() {
        for text in ["", "a", "abc", "abcd", "abcde", "🦀中文é", " a\n"] {
            assert_eq!(
                estimated_item_tokens(ItemTokenEstimateFacts::Text(text)),
                (text.chars().count() + 3) / 4 + 4
            );
        }
        assert_eq!(
            estimated_item_tokens(ItemTokenEstimateFacts::Attachment {
                uri: "文",
                name: "🦀",
                mime_type: Some("ab")
            }),
            5
        );
        assert_eq!(
            estimated_item_tokens(ItemTokenEstimateFacts::Reasoning {
                summary: "文🦀",
                content: "abc"
            }),
            6
        );
        let mut count = CharacterCounter::default();
        for byte in "🦀é中文".as_bytes() {
            count.write_all(&[*byte]).unwrap();
        }
        assert_eq!(count.characters, 4);
    }
}
