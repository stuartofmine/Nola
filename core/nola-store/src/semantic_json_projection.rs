// SPDX-License-Identifier: Apache-2.0
//! Dormant, bounded top-level JSON token extraction. No publication or IO policy.
use crate::{StoreError, StoreResult};
use sha2::{Digest, Sha256};

// Count bytes actually pulled by both scanner passes, not declared source
// lengths. Thread isolation keeps concurrent test fixtures independent.
#[cfg(test)]
thread_local! { pub(crate) static SCANNED_JSON_BYTES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) }; }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Null,
    Bool,
    Number,
    String,
    Array,
    Object,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenSpan {
    pub start_bytes: u64,
    pub end_bytes: u64,
    pub kind: TokenKind,
}
#[derive(Debug, PartialEq)]
pub enum ScalarValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
}
#[derive(Debug, PartialEq)]
pub enum ExtractedValue {
    Complete(ScalarValue),
    Deferred,
}
#[derive(Debug)]
pub struct FieldToken {
    pub field_index: usize,
    pub span: TokenSpan,
    /// A duplicate never selects a winning value; the first span is evidence only.
    pub duplicate_known_key: bool,
    pub value: ExtractedValue,
}
#[derive(Debug)]
pub struct TokenProjection {
    pub root_kind: TokenKind,
    pub fields: Vec<FieldToken>,
    pub source_sha256: String,
    /// Bounded schema adapters can reject extras without retaining their keys
    /// or values. General token inspection remains permissive.
    pub has_unknown_fields: bool,
}
fn invalid(message: &str) -> StoreError {
    StoreError::Invariant(message.into())
}

/// Reopens the exact immutable source twice. Syntax acceptance is delegated to
/// the original validator; extraction never interprets malformed JSON. A digest
/// comparison rejects a factory that changed bytes between those two passes.
/// Output metadata and decoded scalar bytes share `maximum_output_bytes`.
/// At most 64 unique requested names (each <=256 bytes) and 32768 output bytes.
pub fn extract_top_level<F, I>(
    open: F,
    total_bytes: u64,
    known_fields: &[&str],
    maximum_output_bytes: usize,
) -> StoreResult<TokenProjection>
where
    F: FnMut() -> I,
    I: Iterator<Item = Result<(u64, String), String>>,
{
    extract_top_level_with_scalar_fields(open, total_bytes, known_fields, &[], maximum_output_bytes)
}

/// A fixed schema can reserve decoding for control fields. An empty mask means
/// decode every selected scalar; otherwise it must match known_fields exactly.
/// Masked fields retain exact spans and become Deferred even for small scalars.
pub fn extract_top_level_with_scalar_fields<F, I>(
    mut open: F,
    total_bytes: u64,
    known_fields: &[&str],
    scalar_fields: &[bool],
    maximum_output_bytes: usize,
) -> StoreResult<TokenProjection>
where
    F: FnMut() -> I,
    I: Iterator<Item = Result<(u64, String), String>>,
{
    if (!scalar_fields.is_empty() && scalar_fields.len() != known_fields.len())
        || known_fields.len() > 64
        || known_fields.iter().any(|key| key.len() > 256)
        || known_fields
            .iter()
            .enumerate()
            .any(|(i, key)| known_fields[..i].contains(key))
        || maximum_output_bytes > 32768
    {
        return Err(invalid("invalid semantic extraction field/budget limit"));
    }
    let metadata_bytes = known_fields.len() * std::mem::size_of::<FieldToken>()
        + std::mem::size_of::<TokenProjection>()
        + 64;
    let mut remaining = maximum_output_bytes
        .checked_sub(metadata_bytes)
        .ok_or_else(|| invalid("semantic extraction budget cannot hold field metadata"))?;
    let mut first_hash = Sha256::new();
    crate::structured_json_validation::validate_json_chunks(
        open().map(|item| {
            if let Ok((_, bytes)) = &item {
                #[cfg(test)]
                SCANNED_JSON_BYTES.with(|count| count.set(count.get() + bytes.len() as u64));
                first_hash.update(bytes.as_bytes());
            }
            item
        }),
        total_bytes,
    )?;
    let first_hash = first_hash.finalize();
    let mut cursor = Cursor {
        input: open(),
        current: String::new(),
        index: 0,
        received: 0,
        position: 0,
        total: total_bytes,
        hash: Sha256::new(),
    };
    cursor.whitespace()?;
    let root_kind = kind(cursor.peek()?.ok_or_else(|| invalid("missing JSON root"))?)?;
    let mut fields: Vec<FieldToken> = Vec::with_capacity(known_fields.len());
    let mut has_unknown_fields = false;
    if root_kind == TokenKind::Object {
        cursor.take()?;
        cursor.whitespace()?;
        if cursor.peek()? != Some(b'}') {
            loop {
                cursor.whitespace()?;
                let key_limit = known_fields.iter().map(|key| key.len()).max().unwrap_or(0) * 6 + 2;
                let (_, key_raw) = cursor.token(key_limit)?;
                let field = key_raw
                    .and_then(|raw| serde_json::from_slice::<String>(&raw).ok())
                    .and_then(|key| known_fields.iter().position(|known| *known == key));
                has_unknown_fields |= field.is_none();
                cursor.whitespace()?;
                if cursor.take()? != Some(b':') {
                    return Err(invalid("reopened JSON object differs"));
                }
                cursor.whitespace()?;
                let duplicate = field
                    .and_then(|index| fields.iter().position(|entry| entry.field_index == index));
                let (span, raw) = cursor.token(
                    if field.is_some_and(|index| scalar_fields.is_empty() || scalar_fields[index])
                        && duplicate.is_none()
                    {
                        remaining
                    } else {
                        0
                    },
                )?;
                if let Some(index) = duplicate {
                    let old = &mut fields[index];
                    if let ExtractedValue::Complete(ScalarValue::String(value)) = &old.value {
                        remaining += value.len();
                    }
                    old.value = ExtractedValue::Deferred;
                    old.duplicate_known_key = true;
                } else if let Some(field_index) = field {
                    let value = match (span.kind, raw) {
                        (TokenKind::Array | TokenKind::Object, _) | (_, None) => {
                            ExtractedValue::Deferred
                        }
                        (_, Some(raw)) => {
                            let scalar = match serde_json::from_slice::<serde_json::Value>(&raw)? {
                                serde_json::Value::Null => ScalarValue::Null,
                                serde_json::Value::Bool(value) => ScalarValue::Bool(value),
                                serde_json::Value::Number(value) => ScalarValue::Number(value),
                                serde_json::Value::String(value) => {
                                    remaining -= value.len();
                                    ScalarValue::String(value)
                                }
                                _ => return Err(invalid("reopened JSON scalar differs")),
                            };
                            ExtractedValue::Complete(scalar)
                        }
                    };
                    fields.push(FieldToken {
                        field_index,
                        span,
                        duplicate_known_key: false,
                        value,
                    });
                }
                cursor.whitespace()?;
                match cursor.take()? {
                    Some(b',') => {}
                    Some(b'}') => break,
                    _ => return Err(invalid("reopened JSON object delimiter differs")),
                }
            }
        } else {
            cursor.take()?;
        }
    } else {
        cursor.token(0)?;
    }
    cursor.whitespace()?;
    if cursor.peek()?.is_some() || cursor.position != total_bytes {
        return Err(invalid("reopened JSON has trailing bytes"));
    }
    if cursor.hash.finalize() != first_hash {
        return Err(invalid(
            "JSON source changed between validation and extraction",
        ));
    }
    Ok(TokenProjection {
        root_kind,
        fields,
        source_sha256: format!("{first_hash:x}"),
        has_unknown_fields,
    })
}
fn kind(byte: u8) -> StoreResult<TokenKind> {
    Ok(match byte {
        b'n' => TokenKind::Null,
        b't' | b'f' => TokenKind::Bool,
        b'"' => TokenKind::String,
        b'[' => TokenKind::Array,
        b'{' => TokenKind::Object,
        b'-' | b'0'..=b'9' => TokenKind::Number,
        _ => return Err(invalid("reopened JSON token differs")),
    })
}
struct Cursor<I> {
    input: I,
    current: String,
    index: usize,
    received: u64,
    position: u64,
    total: u64,
    hash: Sha256,
}
impl<I: Iterator<Item = Result<(u64, String), String>>> Cursor<I> {
    fn peek(&mut self) -> StoreResult<Option<u8>> {
        if self.index == self.current.len() {
            self.current = String::new();
            self.index = 0;
            match self.input.next() {
                None => {
                    if self.received != self.total {
                        return Err(invalid("reopened JSON source truncated"));
                    }
                    return Ok(None);
                }
                Some(Err(error)) => return Err(invalid(&error)),
                Some(Ok((offset, text))) => {
                    #[cfg(test)]
                    SCANNED_JSON_BYTES.with(|count| count.set(count.get() + text.len() as u64));
                    if offset != self.received || text.is_empty() || text.len() > 32768 {
                        return Err(invalid("invalid reopened JSON chunk geometry"));
                    }
                    self.received = self
                        .received
                        .checked_add(text.len() as u64)
                        .filter(|length| *length <= self.total)
                        .ok_or_else(|| invalid("reopened JSON exceeds declared length"))?;
                    self.hash.update(text.as_bytes());
                    self.current = text;
                }
            }
        }
        Ok(Some(self.current.as_bytes()[self.index]))
    }
    fn take(&mut self) -> StoreResult<Option<u8>> {
        let byte = self.peek()?;
        if byte.is_some() {
            self.index += 1;
            self.position += 1;
        }
        Ok(byte)
    }
    fn whitespace(&mut self) -> StoreResult<()> {
        while matches!(self.peek()?, Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.take()?;
        }
        Ok(())
    }
    fn token(&mut self, limit: usize) -> StoreResult<(TokenSpan, Option<Vec<u8>>)> {
        self.token_mode(limit, false)
    }
    fn token_mode(
        &mut self,
        limit: usize,
        capture_containers: bool,
    ) -> StoreResult<(TokenSpan, Option<Vec<u8>>)> {
        let start_bytes = self.position;
        let first = self
            .peek()?
            .ok_or_else(|| invalid("reopened JSON token truncated"))?;
        let kind = kind(first)?;
        let mut raw = if limit > 0
            && (capture_containers || !matches!(kind, TokenKind::Array | TokenKind::Object))
        {
            Some(Vec::with_capacity(limit))
        } else {
            None
        };
        let mut depth = 0usize;
        let mut string = false;
        let mut escaped = false;
        loop {
            let byte = self
                .peek()?
                .ok_or_else(|| invalid("reopened JSON token truncated"))?;
            if !string && depth == 0 && self.position > start_bytes {
                break;
            }
            self.take()?;
            if let Some(buffer) = &mut raw {
                if buffer.len() < limit {
                    buffer.push(byte);
                } else {
                    raw = None;
                }
            }
            if string {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    string = false;
                }
            } else {
                match byte {
                    b'"' => string = true,
                    b'[' | b'{' => {
                        depth += 1;
                        if depth > 128 {
                            return Err(invalid("reopened JSON exceeds nesting limit"));
                        }
                    }
                    b']' | b'}' => {
                        depth = depth
                            .checked_sub(1)
                            .ok_or_else(|| invalid("reopened JSON unbalanced container"))?;
                    }
                    _ => {}
                }
            }
            if !string && depth == 0 {
                if matches!(
                    kind,
                    TokenKind::String | TokenKind::Array | TokenKind::Object
                ) {
                    break;
                }
                // Scalar literal/number: scan until its grammar delimiter. The
                // first pass proves syntax, and digest equality proves identity.
                while let Some(byte) = self.peek()? {
                    if matches!(byte, b',' | b']' | b'}' | b' ' | b'\n' | b'\r' | b'\t') {
                        break;
                    }
                    self.take()?;
                    if let Some(buffer) = &mut raw {
                        if buffer.len() < limit {
                            buffer.push(byte);
                        } else {
                            raw = None;
                        }
                    }
                }
                break;
            }
        }
        Ok((
            TokenSpan {
                start_bytes,
                end_bytes: self.position,
                kind,
            },
            raw,
        ))
    }
}

#[derive(Debug)]
pub struct ArrayTokenItem {
    pub span: TokenSpan,
    /// Present only if the entire original item fits the shared raw budget.
    pub raw_json: Option<String>,
}
#[derive(Debug)]
pub struct ArrayTokenProjection {
    pub root_kind: TokenKind,
    pub total_items: u64,
    pub items: Vec<ArrayTokenItem>,
    pub source_sha256: String,
}
/// Bounded array envelope extraction for typed nested adapters. Keeps at most
/// maximum_items (hard cap 64); raw items and metadata share the output budget.
/// A skipped/oversized item never becomes an apparently complete empty object.
pub fn extract_array_items<F, I>(
    open: F,
    total_bytes: u64,
    maximum_items: usize,
    maximum_output_bytes: usize,
) -> StoreResult<ArrayTokenProjection>
where
    F: FnMut() -> I,
    I: Iterator<Item = Result<(u64, String), String>>,
{
    extract_array_items_visiting(
        open,
        total_bytes,
        maximum_items,
        maximum_output_bytes,
        false,
        |_| Ok(()),
    )
}

/// Visit every original array item without retaining the array or item bodies.
/// Validation and reopened-source digest checks are identical to extraction.
pub fn visit_array_item_spans<F, I>(
    open: F,
    total_bytes: u64,
    visitor: impl FnMut(TokenSpan) -> StoreResult<()>,
) -> StoreResult<()>
where
    F: FnMut() -> I,
    I: Iterator<Item = Result<(u64, String), String>>,
{
    extract_array_items_visiting(open, total_bytes, 0, 1024, false, visitor).map(|_| ())
}

/// Like SQLite json_each on an object, visits original immediate member values.
pub fn visit_object_value_spans<F, I>(
    open: F,
    total_bytes: u64,
    visitor: impl FnMut(TokenSpan) -> StoreResult<()>,
) -> StoreResult<()>
where
    F: FnMut() -> I,
    I: Iterator<Item = Result<(u64, String), String>>,
{
    extract_array_items_visiting(open, total_bytes, 0, 1024, true, visitor).map(|_| ())
}

fn extract_array_items_visiting<F, I>(
    mut open: F,
    total_bytes: u64,
    maximum_items: usize,
    maximum_output_bytes: usize,
    visit_object: bool,
    mut visitor: impl FnMut(TokenSpan) -> StoreResult<()>,
) -> StoreResult<ArrayTokenProjection>
where
    F: FnMut() -> I,
    I: Iterator<Item = Result<(u64, String), String>>,
{
    if maximum_items > 64 || maximum_output_bytes > 32768 {
        return Err(invalid("invalid nested array extraction budget"));
    }
    let mut remaining = maximum_output_bytes
        .checked_sub(
            maximum_items * std::mem::size_of::<ArrayTokenItem>()
                + std::mem::size_of::<ArrayTokenProjection>()
                + 64,
        )
        .ok_or_else(|| invalid("array budget cannot hold bounded item metadata"))?;
    let mut first_hash = Sha256::new();
    crate::structured_json_validation::validate_json_chunks(
        open().map(|item| {
            if let Ok((_, bytes)) = &item {
                #[cfg(test)]
                SCANNED_JSON_BYTES.with(|count| count.set(count.get() + bytes.len() as u64));
                first_hash.update(bytes.as_bytes());
            }
            item
        }),
        total_bytes,
    )?;
    let first_hash = first_hash.finalize();
    let mut cursor = Cursor {
        input: open(),
        current: String::new(),
        index: 0,
        received: 0,
        position: 0,
        total: total_bytes,
        hash: Sha256::new(),
    };
    cursor.whitespace()?;
    let root_kind = kind(
        cursor
            .peek()?
            .ok_or_else(|| invalid("missing JSON array root"))?,
    )?;
    let mut total_items = 0u64;
    let mut items = Vec::with_capacity(maximum_items);
    if root_kind == TokenKind::Array {
        cursor.take()?;
        cursor.whitespace()?;
        if cursor.peek()? == Some(b']') {
            cursor.take()?;
        } else {
            loop {
                cursor.whitespace()?;
                let retain = items.len() < maximum_items;
                let (span, raw) = cursor.token_mode(if retain { remaining } else { 0 }, true)?;
                visitor(span)?;
                total_items = total_items
                    .checked_add(1)
                    .ok_or_else(|| invalid("array item count overflow"))?;
                if retain {
                    let raw_json = raw
                        .map(|bytes| {
                            remaining -= bytes.len();
                            // Release unused capacity before retaining multiple items.
                            String::from_utf8(bytes.into_boxed_slice().into_vec())
                                .map_err(|_| invalid("invalid reopened array UTF-8"))
                        })
                        .transpose()?;
                    items.push(ArrayTokenItem { span, raw_json });
                }
                cursor.whitespace()?;
                match cursor.take()? {
                    Some(b',') => {}
                    Some(b']') => break,
                    _ => return Err(invalid("reopened JSON array delimiter differs")),
                }
            }
        }
    } else if root_kind == TokenKind::Object && visit_object {
        cursor.take()?;
        cursor.whitespace()?;
        if cursor.peek()? == Some(b'}') {
            cursor.take()?;
        } else {
            loop {
                cursor.whitespace()?;
                cursor.token(0)?;
                cursor.whitespace()?;
                if cursor.take()? != Some(b':') {
                    return Err(invalid("reopened object delimiter differs"));
                }
                cursor.whitespace()?;
                let (span, _) = cursor.token(0)?;
                visitor(span)?;
                cursor.whitespace()?;
                match cursor.take()? {
                    Some(b',') => {}
                    Some(b'}') => break,
                    _ => return Err(invalid("reopened object delimiter differs")),
                }
            }
        }
    } else {
        cursor.token(0)?;
    }
    cursor.whitespace()?;
    if cursor.peek()?.is_some() || cursor.position != total_bytes {
        return Err(invalid("reopened JSON array has trailing bytes"));
    }
    if cursor.hash.finalize() != first_hash {
        return Err(invalid(
            "JSON array source changed between validation and extraction",
        ));
    }
    Ok(ArrayTokenProjection {
        root_kind,
        total_items,
        items,
        source_sha256: format!("{first_hash:x}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chunks(text: &str, size: usize) -> Vec<Result<(u64, String), String>> {
        let mut result = Vec::new();
        let mut start = 0;
        while start < text.len() {
            let mut end = (start + size).min(text.len());
            while !text.is_char_boundary(end) {
                end += 1;
            }
            result.push(Ok((start as u64, text[start..end].to_owned())));
            start = end;
        }
        result
    }
    #[test]
    fn exact_spans_and_scalars_match_serde_independent_of_key_order_and_chunk_boundaries() {
        let input = r#" {"input":{"nested":["}","\"",null]},"te\u0078t":"中😀\uD83D\uDE00\n","nil":null,"number":-1.25e2,"flag":true,"kind":"tool"} "#;
        let fields = ["input", "text", "nil", "number", "flag", "kind"];
        let expected: serde_json::Value = serde_json::from_str(input).unwrap();
        for size in [1, 2, 3, 7, 31, 32768] {
            let result = extract_top_level(
                || chunks(input, size).into_iter(),
                input.len() as u64,
                &fields,
                8192,
            )
            .unwrap();
            assert_eq!(result.fields.len(), fields.len());
            for field in result.fields {
                assert!(!field.duplicate_known_key);
                let raw = &input[field.span.start_bytes as usize..field.span.end_bytes as usize];
                let value: serde_json::Value = serde_json::from_str(raw).unwrap();
                assert_eq!(value, expected[fields[field.field_index]]);
                match field.value {
                    ExtractedValue::Complete(ScalarValue::String(text)) => {
                        assert_eq!(value.as_str(), Some(text.as_str()))
                    }
                    ExtractedValue::Complete(ScalarValue::Null) => assert!(value.is_null()),
                    ExtractedValue::Complete(ScalarValue::Bool(boolean)) => {
                        assert_eq!(value.as_bool(), Some(boolean))
                    }
                    ExtractedValue::Complete(ScalarValue::Number(number)) => {
                        assert_eq!(value, serde_json::Value::Number(number))
                    }
                    ExtractedValue::Deferred => assert!(value.is_object()),
                }
            }
        }
    }
    #[test]
    fn syntax_acceptance_matches_original_validator_including_surrogates_depth_and_trailing_bytes()
    {
        let mut inputs = vec![
            "null".to_string(),
            "[]".into(),
            "{}".into(),
            "1e309".into(),
            "[1,]".into(),
            "{} garbage".into(),
            r#"{"x":"\uD800"}"#.into(),
            r#"{"x":"\uDC00"}"#.into(),
            r#"{"x":"\uD800\uDC00"}"#.into(),
            r#"{"x":1,"x":2}"#.into(),
        ];
        for depth in [126, 127, 128, 129] {
            inputs.push(format!("{}0{}", "[".repeat(depth), "]".repeat(depth)));
        }
        for byte in 0..128u8 {
            inputs.push(format!("{{\"x\":\"{}\"}}", char::from(byte)));
            inputs.push(format!("{{\"x\":\"\\{}\"}}", char::from(byte)));
        }
        for input in inputs {
            let expected = serde_json::from_str::<serde_json::Value>(&input).is_ok();
            for size in [1, 7, 32768] {
                assert_eq!(
                    extract_top_level(
                        || chunks(&input, size).into_iter(),
                        input.len() as u64,
                        &["x"],
                        4096
                    )
                    .is_ok(),
                    expected,
                    "size={size} input={input:?}"
                );
            }
        }
    }
    #[test]
    fn duplicate_keys_are_ambiguous_and_budget_does_not_create_fake_values() {
        let input = r#"{"x":"first","\u0078":[1,2],"kind":"tool"}"#;
        let result = extract_top_level(
            || chunks(input, 1).into_iter(),
            input.len() as u64,
            &["x", "kind"],
            4096,
        )
        .unwrap();
        assert_eq!(result.fields.len(), 2);
        assert!(result.fields[0].duplicate_known_key);
        assert_eq!(result.fields[0].value, ExtractedValue::Deferred);
        let input = format!("{{\"x\":\"{}\",\"kind\":\"tool\"}}", "x".repeat(10000));
        let result = extract_top_level(
            || chunks(&input, 32768).into_iter(),
            input.len() as u64,
            &["x", "kind"],
            4096,
        )
        .unwrap();
        assert_eq!(result.fields[0].value, ExtractedValue::Deferred);
        assert_eq!(
            result.fields[1].value,
            ExtractedValue::Complete(ScalarValue::String("tool".into()))
        );
        let mut calls = 0;
        assert!(extract_top_level(
            || {
                calls += 1;
                chunks(if calls == 1 { "{\"x\":1}" } else { "{\"x\":2}" }, 3).into_iter()
            },
            7,
            &["x"],
            4096
        )
        .unwrap_err()
        .to_string()
        .contains("changed"));
        assert!(extract_top_level(|| chunks("null", 1).into_iter(), 4, &["x", "x"], 4096).is_err());
        assert!(extract_top_level(|| chunks("null", 1).into_iter(), 4, &["x"], 0).is_err());
    }
    #[test]
    fn unicode_keys_budget_edges_and_reopened_source_errors_are_explicit() {
        let input = r#"{"\uD83D\uDE00":"a","😀":"b","nil":null,"empty":""}"#;
        let result = extract_top_level(
            || chunks(input, 1).into_iter(),
            input.len() as u64,
            &["😀", "nil", "empty"],
            4096,
        )
        .unwrap();
        assert!(result.fields[0].duplicate_known_key);
        assert_eq!(result.fields[0].value, ExtractedValue::Deferred);
        assert_eq!(
            result.fields[1].value,
            ExtractedValue::Complete(ScalarValue::Null)
        );
        assert_eq!(
            result.fields[2].value,
            ExtractedValue::Complete(ScalarValue::String(String::new()))
        );
        let input = r#"{"x":"a"}"#;
        let metadata =
            std::mem::size_of::<FieldToken>() + std::mem::size_of::<TokenProjection>() + 64;
        let deferred = extract_top_level(
            || chunks(input, 1).into_iter(),
            input.len() as u64,
            &["x"],
            metadata + 2,
        )
        .unwrap();
        assert_eq!(deferred.fields[0].value, ExtractedValue::Deferred);
        let complete = extract_top_level(
            || chunks(input, 1).into_iter(),
            input.len() as u64,
            &["x"],
            metadata + 3,
        )
        .unwrap();
        assert_eq!(
            complete.fields[0].value,
            ExtractedValue::Complete(ScalarValue::String("a".into()))
        );
        // Raw token bytes, including quotes/escapes, must fit the temporary
        // scalar budget; a small decoded value may conservatively be Deferred.
        for mode in 0..4 {
            let mut pass = 0;
            let result = extract_top_level(
                || {
                    pass += 1;
                    if pass == 1 {
                        return chunks(input, 1).into_iter();
                    }
                    match mode {
                        0 => vec![Err("interrupted immutable source".into())],
                        1 => vec![Ok((1, input.to_string()))],
                        2 => vec![Ok((0, input[..input.len() - 1].to_string()))],
                        _ => vec![Ok((0, String::new()))],
                    }
                    .into_iter()
                },
                input.len() as u64,
                &["x"],
                4096,
            );
            assert!(result.is_err(), "mode={mode}");
        }
        let names = vec!["x"; 65];
        assert!(extract_top_level(|| chunks("null", 1).into_iter(), 4, &names, 4096).is_err());
        let name = "x".repeat(257);
        assert!(
            extract_top_level(|| chunks("null", 1).into_iter(), 4, &[name.as_str()], 4096).is_err()
        );
        assert!(extract_top_level(|| chunks("null", 1).into_iter(), 4, &[], 32769).is_err());
    }
}
