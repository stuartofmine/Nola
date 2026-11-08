// SPDX-License-Identifier: Apache-2.0
//! Strict JSON validation without retaining a parsed object tree.
//! Store string validation is wired here; chunk validation is a separate seam.
//!
//! serde_json 1.0.151's IgnoredAny skips string storage, but also skips surrogate,
//! finite-number and recursion-limit validation. Instead, a lexical string view
//! validates each escape with serde_json and replaces string contents *only in
//! the parser's input*. The original chunks are never rewritten or re-encoded.
//! A no-collection visitor delegates all container/number/depth rules to the same
//! Deserializer used by the existing Value-based Store validator.
use crate::{StoreError, StoreResult};
use serde::de::{self, Deserialize, MapAccess, SeqAccess, Visitor};
use std::io::{self, Read};

const CHUNK_BYTES: usize = 32 * 1024;

/// Validate an already resident JSON string without building maps, arrays or
/// owned values. Escaped strings may use the deserializer's scratch buffer;
/// this is not a claim of bounded memory for an arbitrarily large single string.
pub(crate) fn validate_json_str(value: &str) -> Result<(), serde_json::Error> {
    serde_json::from_str::<ValidateValue>(value).map(|_| ())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonValidationStats {
    pub total_utf8_bytes: u64,
    pub chunk_count: u64,
    /// Maximum resident source chunk; excludes the caller's iterator storage.
    pub maximum_chunk_bytes: usize,
}

/// Validate one original JSON value with the Store's existing serde_json rules.
/// The caller owns immutable input and is responsible for persistence. This
/// function performs no writes, retains no body, and consumes at most one 32 KiB
/// source chunk plus a 14-byte escape token and serde's fixed-depth parser state.
/// Errors identify offsets in the original byte stream, not the lexical view.
pub fn validate_json_chunks<I>(
    chunks: I,
    expected_utf8_bytes: u64,
) -> StoreResult<JsonValidationStats>
where
    I: IntoIterator<Item = Result<(u64, String), String>>,
{
    let mut view = StringView {
        source: ChunkSource {
            chunks: chunks.into_iter(),
            current: String::new(),
            cursor: 0,
            expected: expected_utf8_bytes,
            received: 0,
            consumed: 0,
            count: 0,
            maximum_chunk_bytes: 0,
            eof: false,
        },
        closing_quote: None,
        last_original_offset: 0,
    };
    let result = {
        let mut parser = serde_json::Deserializer::from_reader(&mut view);
        ValidateValue::deserialize(&mut parser).and_then(|_| parser.end())
    };
    result.map_err(|error| {
        StoreError::Invariant(format!(
            "invalid original JSON near byte {}: {error}",
            view.last_original_offset
        ))
    })?;
    Ok(JsonValidationStats {
        total_utf8_bytes: view.source.consumed,
        chunk_count: view.source.count,
        maximum_chunk_bytes: view.source.maximum_chunk_bytes,
    })
}

struct ChunkSource<I> {
    chunks: I,
    current: String,
    cursor: usize,
    expected: u64,
    received: u64,
    consumed: u64,
    count: u64,
    maximum_chunk_bytes: usize,
    eof: bool,
}
impl<I: Iterator<Item = Result<(u64, String), String>>> ChunkSource<I> {
    fn next(&mut self) -> io::Result<Option<u8>> {
        if self.cursor == self.current.len() {
            if self.eof {
                return Ok(None);
            }
            // Release before advancing a lazy source, so two chunks are not
            // resident simultaneously inside the validator.
            self.current = String::new();
            self.cursor = 0;
            match self.chunks.next() {
                Some(Ok((offset, text))) => {
                    if offset != self.received || text.is_empty() || text.len() > CHUNK_BYTES {
                        return Err(self.error("source chunks must be contiguous, nonempty and at most 32768 UTF-8 bytes"));
                    }
                    self.received = self
                        .received
                        .checked_add(text.len() as u64)
                        .filter(|bytes| *bytes <= self.expected)
                        .ok_or_else(|| self.error("source bytes exceed declared length"))?;
                    self.maximum_chunk_bytes = self.maximum_chunk_bytes.max(text.len());
                    self.count += 1;
                    self.current = text;
                }
                Some(Err(error)) => return Err(self.error(&error)),
                None => {
                    self.eof = true;
                    if self.received != self.expected {
                        return Err(self.error("source bytes do not match declared length"));
                    }
                    return Ok(None);
                }
            }
        }
        let value = self.current.as_bytes()[self.cursor];
        self.cursor += 1;
        self.consumed += 1;
        Ok(Some(value))
    }
    fn required(&mut self) -> io::Result<u8> {
        self.next()?
            .ok_or_else(|| self.error("unexpected end of original JSON string"))
    }
    fn error(&self, message: &str) -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("original byte {}: {message}", self.consumed),
        )
    }
}

struct StringView<I> {
    source: ChunkSource<I>,
    closing_quote: Option<u64>,
    last_original_offset: u64,
}
impl<I: Iterator<Item = Result<(u64, String), String>>> StringView<I> {
    fn escape(&mut self) -> io::Result<()> {
        // Quoted single escape, or a complete surrogate pair, is at most 14
        // original bytes including the two synthetic quotes. Let serde enforce
        // its own escape/codepoint acceptance rather than duplicating those rules.
        let mut token = [0_u8; 14];
        token[0] = b'"';
        token[1] = b'\\';
        token[2] = self.source.required()?;
        let mut end = 3;
        if token[2] == b'u' {
            for slot in &mut token[3..7] {
                *slot = self.source.required()?;
            }
            end = 7;
            let code = std::str::from_utf8(&token[3..7])
                .ok()
                .and_then(|digits| u16::from_str_radix(digits, 16).ok());
            if code.is_some_and(|code| (0xD800..=0xDBFF).contains(&code)) {
                for slot in &mut token[7..13] {
                    *slot = self.source.required()?;
                }
                end = 13;
            }
        }
        token[end] = b'"';
        serde_json::from_slice::<String>(&token[..=end]).map_err(|error| {
            self.source
                .error(&format!("invalid string escape: {error}"))
        })?;
        Ok(())
    }

    fn next(&mut self) -> io::Result<Option<u8>> {
        if let Some(offset) = self.closing_quote.take() {
            self.last_original_offset = offset;
            return Ok(Some(b'"'));
        }
        let start = self.source.consumed;
        let Some(byte) = self.source.next()? else {
            self.last_original_offset = self.source.consumed;
            return Ok(None);
        };
        self.last_original_offset = start;
        if byte != b'"' {
            return Ok(Some(byte));
        }
        loop {
            match self.source.required()? {
                b'"' => {
                    self.closing_quote = Some(self.source.consumed - 1);
                    return Ok(Some(b'"'));
                }
                b'\\' => self.escape()?,
                0..=0x1F => return Err(self.source.error("unescaped control byte in JSON string")),
                // Each input String is already valid UTF-8. No substring is
                // returned or decoded, so multibyte scalars need no scratch.
                _ => {}
            }
        }
    }
}
impl<I: Iterator<Item = Result<(u64, String), String>>> Read for StringView<I> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        match self.next()? {
            Some(byte) => {
                output[0] = byte;
                Ok(1)
            }
            None => Ok(0),
        }
    }
}

struct ValidateValue;
impl<'de> Deserialize<'de> for ValidateValue {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ValidateVisitor)
    }
}
struct ValidateVisitor;
impl<'de> Visitor<'de> for ValidateVisitor {
    type Value = ValidateValue;
    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("one JSON value")
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(ValidateValue)
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Ok(ValidateValue)
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
        Ok(ValidateValue)
    }
    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
        Ok(ValidateValue)
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
        Ok(ValidateValue)
    }
    fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Ok(ValidateValue)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        while sequence.next_element::<ValidateValue>()?.is_some() {}
        Ok(ValidateValue)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        while map.next_key::<ValidateValue>()?.is_some() {
            map.next_value::<ValidateValue>()?;
        }
        Ok(ValidateValue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunks(text: &str, size: usize) -> Vec<Result<(u64, String), String>> {
        let mut result = Vec::new();
        let mut offset = 0;
        while offset < text.len() {
            let mut end = (offset + size).min(text.len());
            while !text.is_char_boundary(end) {
                end += 1;
            }
            result.push(Ok((offset as u64, text[offset..end].into())));
            offset = end;
        }
        result
    }
    fn equivalent(text: &str) {
        // Keep the old Value parser as an independent oracle now that Store's
        // production validator uses the no-collection visitor itself.
        let expected = serde_json::from_str::<serde_json::Value>(text).is_ok();
        assert_eq!(validate_json_str(text).is_ok(), expected, "resident input: {text:?}");
        assert_eq!(crate::validate_json("fixture", text).is_ok(), expected);
        for size in [1, 2, 3, 4, 7, 12, 31, 32768] {
            let actual = validate_json_chunks(chunks(text, size), text.len() as u64);
            assert_eq!(
                actual.is_ok(),
                expected,
                "chunk size {size}, {text:?}: {actual:?}"
            );
            if let Ok(stats) = actual {
                assert_eq!(stats.total_utf8_bytes, text.len() as u64);
            }
        }
    }
    #[test]
    fn grammar_escape_surrogate_and_number_rules_match_existing_value_validator() {
        for text in [
            "",
            " ",
            "null",
            "true",
            "false",
            "true false",
            "[null,true,false]",
            "{}",
            "[]",
            "[1,]",
            "{\"x\":1,}",
            "{x:1}",
            "{\"x\" 1}",
            "{\"x\":1,\"x\":2}",
            "[}",
            "[1 2]",
            "\"中😀\"",
            "\"\\uD800\"",
            "\"\\uDC00\"",
            "\"\\uD800\\uDC00\"",
            "\"\\uD800\\u0041\"",
            "\"\\uDBFF\\uDFFF\"",
            "\"\\uD800x\"",
            "\"\\uXXXX\"",
            "\"\\q\"",
            "\"\\\"\\\\\\/\\b\\f\\n\\r\\t\"",
            "\"unfinished",
            "\"line\ncontrol\"",
            "01",
            "-01",
            "1.",
            ".1",
            "1e",
            "1e+",
            "1e-2",
            "-0",
            "1e309",
            "0e99999",
            "-0e-99999",
            "1e-99999",
            "18446744073709551616",
            "NaN",
            "Infinity",
        ] {
            equivalent(text);
        }
        for byte in 0u8..=127 {
            equivalent(&format!("\"x{}y\"", char::from(byte)));
            equivalent(&format!("\"x\\{}y\"", char::from(byte)));
        }
        for value in [0, 1, 0xD7FF, 0xD800, 0xDBFF, 0xDC00, 0xDFFF, 0xE000, 0xFFFF] {
            equivalent(&format!("{{\"\\u{value:04x}\":\"\\u{value:04x}\"}}"));
        }
    }
    #[test]
    fn depth_and_long_numbers_use_serde_rules_without_ignored_any_loopholes() {
        for depth in [1, 126, 127, 128, 129, 256] {
            equivalent(&format!("{}0{}", "[".repeat(depth), "]".repeat(depth)));
            equivalent(&format!(
                "{}0{}",
                "{\"k\":".repeat(depth),
                "}".repeat(depth)
            ));
        }
        for text in [
            "1".repeat(10000),
            format!("0.{}1", "0".repeat(10000)),
            format!("1e{}", "9".repeat(10000)),
        ] {
            equivalent(&text);
        }
        for text in [
            "\"\\uD800\"",
            "1e309",
            &format!("{}0{}", "[".repeat(129), "]".repeat(129)),
        ] {
            assert!(serde_json::from_str::<de::IgnoredAny>(text).is_ok());
            assert!(validate_json_chunks(chunks(text, 2), text.len() as u64).is_err());
        }
    }
    #[test]
    fn source_layout_and_original_error_offsets_are_enforced() {
        for (chunks, bytes) in [
            (vec![Ok((1, "null".into()))], 4),
            (vec![Ok((0, String::new()))], 0),
            (vec![Ok((0, "null".into()))], 5),
            (vec![Ok((0, "null".into()))], 3),
            (vec![Ok((0, " ".repeat(32769)))], 32769),
            (vec![Err("source failed".into())], 4),
        ] {
            assert!(validate_json_chunks(chunks, bytes).is_err());
        }
        let prefix = format!("[\"{}\",", "x".repeat(100_000));
        let original = format!("{prefix}?");
        let error = validate_json_chunks(chunks(&original, 32768), original.len() as u64)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(&format!("near byte {}", prefix.len())),
            "{error}"
        );
    }
}
