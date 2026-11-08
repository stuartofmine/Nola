// SPDX-License-Identifier: Apache-2.0
//! JSON hydration and optional review projection. Original bytes and display
//! rewrites remain distinct; neither a projection nor a deferral owns a digest.
use crate::canonical_field_reader::{CanonicalFieldReader, CanonicalJsonRead};
use crate::semantic_json_projection::{self, TokenKind};
use crate::{StoreError, StoreResult};

pub(super) enum JsonProjection {
    Absent,
    Original(String),
    Review(String),
    Deferred,
}

pub(super) fn hydrate(
    field: &CanonicalFieldReader<'_>,
    part_kind: &str,
    text_preview: &str,
    maximum_utf8_bytes: Option<usize>,
) -> StoreResult<JsonProjection> {
    let total = match field.read_json(maximum_utf8_bytes)? {
        CanonicalJsonRead::Absent => return Ok(JsonProjection::Absent),
        CanonicalJsonRead::Complete(raw) => return Ok(JsonProjection::Original(raw)),
        CanonicalJsonRead::Deferred { total_utf8_bytes } => total_utf8_bytes,
    };
    let limit = maximum_utf8_bytes.ok_or_else(|| invalid("unbounded JSON was deferred"))?;
    if part_kind != "text" {
        return Ok(JsonProjection::Deferred);
    }
    // Only spans are needed: decoding an opaque number/review through Value
    // would lose its original lexical representation and waste output budget.
    let projection = semantic_json_projection::extract_top_level_with_scalar_fields(
        || chunks(field, total),
        total,
        &["review", "text"],
        &[false, false],
        1024,
    )?;
    if projection
        .fields
        .iter()
        .any(|field| field.duplicate_known_key)
    {
        return Err(invalid("ambiguous review projection field"));
    }
    let review = projection
        .fields
        .iter()
        .find(|field| field.field_index == 0);
    let text = projection
        .fields
        .iter()
        .find(|field| field.field_index == 1);
    let (Some(review), Some(text)) = (review, text) else {
        return Ok(JsonProjection::Deferred);
    };
    if review.span.kind == TokenKind::Null || text_preview.len() > limit {
        return Ok(JsonProjection::Deferred);
    }
    let unchanged = total - (text.span.end_bytes - text.span.start_bytes);
    if unchanged > limit as u64 {
        return Ok(JsonProjection::Deferred);
    }
    // Encode just the replacement scalar, never the surrounding raw object.
    let replacement = serde_json::to_string(text_preview)?;
    let projected_bytes = unchanged.saturating_add(replacement.len() as u64);
    if projected_bytes > limit as u64 {
        return Ok(JsonProjection::Deferred);
    }
    let mut output = String::with_capacity(projected_bytes as usize);
    append_span(field, 0, text.span.start_bytes, &mut output)?;
    output.push_str(&replacement);
    append_span(field, text.span.end_bytes, total, &mut output)?;
    if output.len() as u64 != projected_bytes {
        return Err(invalid(
            "review projection length differs from original spans",
        ));
    }
    Ok(JsonProjection::Review(output))
}

fn chunks<'a>(
    field: &'a CanonicalFieldReader<'_>,
    total: u64,
) -> impl Iterator<Item = Result<(u64, String), String>> + 'a {
    let mut offset = 0;
    std::iter::from_fn(move || {
        if offset >= total {
            return None;
        }
        let start = offset;
        match field.read_range(start, 32768) {
            Ok(range) if range.next_offset_bytes > start => {
                offset = range.next_offset_bytes;
                Some(Ok((start, range.text)))
            }
            Ok(_) => {
                offset = total;
                Some(Err("JSON projection range did not advance".into()))
            }
            Err(error) => {
                offset = total;
                Some(Err(error.to_string()))
            }
        }
    })
}

fn append_span(
    field: &CanonicalFieldReader<'_>,
    mut offset: u64,
    end: u64,
    output: &mut String,
) -> StoreResult<()> {
    while offset < end {
        let range = field.read_range(offset, (end - offset).clamp(4, 32768) as usize)?;
        let count = range.text.len().min((end - offset) as usize);
        if count == 0 || !range.text.is_char_boundary(count) {
            return Err(invalid("invalid original JSON projection span"));
        }
        output.push_str(&range.text[..count]);
        offset += count as u64;
    }
    Ok(())
}

fn invalid(message: &str) -> StoreError {
    StoreError::Invariant(message.into())
}
