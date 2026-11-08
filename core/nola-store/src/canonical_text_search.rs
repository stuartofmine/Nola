// SPDX-License-Identifier: Apache-2.0
//! Literal recall of immutable chunked text with a bounded overlap window.
use crate::{canonical_text_store::CanonicalTextStore, StoreError, StoreResult};
use nola_domain::{CanonicalBodyReference, CANONICAL_BODY_CHUNK_MAX_BYTES};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalTextMatch {
    pub match_offset_bytes: u64,
    pub start_character: u64,
    pub snippet: String,
}

impl CanonicalTextStore {
    /// Application must resolve and authorize the canonical Part reference first.
    /// Match semantics equal inline transcript recall: literal Unicode, ASCII
    /// case folding only. No SQL wildcards, entity extraction or query rewriting.
    /// Returns the first occurrence, like the existing inline search path.
    pub fn search_literal(
        &self,
        reference: &CanonicalBodyReference,
        query: &str,
    ) -> StoreResult<Option<CanonicalTextMatch>> {
        search_literal_with_reader(reference, query, |offset, limit| {
            self.read_range(reference, offset, limit)
        })
    }
}

pub(crate) fn search_literal_with_reader(
    reference: &CanonicalBodyReference,
    query: &str,
    read_range: impl FnMut(u64, usize) -> StoreResult<crate::canonical_text_store::CanonicalTextRange>,
) -> StoreResult<Option<CanonicalTextMatch>> {
    crate::canonical_body::require_text_reference(reference)?;
    search_literal_bytes_with_reader(reference.total_utf8_bytes(), query, read_range)
}

/// Backing-independent bounded search. The caller supplies an already resolved
/// owner and fixed-snapshot byte length; no synthetic sealed reference is made.
pub(crate) fn search_literal_bytes_with_reader(
    total_utf8_bytes: u64,
    query: &str,
    mut read_range: impl FnMut(
        u64,
        usize,
    ) -> StoreResult<crate::canonical_text_store::CanonicalTextRange>,
) -> StoreResult<Option<CanonicalTextMatch>> {
    if query.trim().is_empty() || query.chars().count() > 500 {
        return Err(StoreError::Invariant(
            "invalid canonical text search query".into(),
        ));
    }
    let mut offset = 0_u64;
    let mut characters = 0_u64;
    let mut overlap = String::new();
    // Keep enough bytes for a split match AND 240 preceding Unicode scalars.
    let overlap_budget = query.len().saturating_sub(1) + 240 * 4;
    while offset < total_utf8_bytes {
        let range = read_range(offset, CANONICAL_BODY_CHUNK_MAX_BYTES as usize)?;
        if range.offset_bytes != offset
            || range.next_offset_bytes <= offset
            || range.next_offset_bytes > total_utf8_bytes
            || range.next_offset_bytes - offset != range.text.len() as u64
            || range.text.len() > CANONICAL_BODY_CHUNK_MAX_BYTES as usize
        {
            return Err(StoreError::Invariant(
                "canonical search range has invalid progress".into(),
            ));
        }
        let window_start_bytes = offset - overlap.len() as u64;
        let window_start_characters = characters - overlap.chars().count() as u64;
        let mut window = overlap;
        window.push_str(&range.text);
        let position = crate::transcript_search::literal_position(&window, query);
        if position > 0 {
            let match_character = (position - 1) as usize;
            let start_character = match_character.saturating_sub(240);
            let start_byte = window
                .char_indices()
                .nth(start_character)
                .map(|(byte, _)| byte)
                .ok_or_else(|| StoreError::Invariant("invalid search excerpt offset".into()))?;
            let match_byte = window
                .char_indices()
                .nth(match_character)
                .map(|(byte, _)| byte)
                .ok_or_else(|| StoreError::Invariant("invalid search match offset".into()))?;
            // A bounded range has room for 1600 scalars even in four-byte
            // UTF-8. Reading from its canonical offset handles later chunks.
            let excerpt = read_range(
                window_start_bytes + start_byte as u64,
                CANONICAL_BODY_CHUNK_MAX_BYTES as usize,
            )?;
            return Ok(Some(CanonicalTextMatch {
                match_offset_bytes: window_start_bytes + match_byte as u64,
                start_character: window_start_characters + start_character as u64,
                snippet: excerpt.text.chars().take(1600).collect(),
            }));
        }
        characters += range.text.chars().count() as u64;
        offset = range.next_offset_bytes;
        let mut retained_start = window.len().saturating_sub(overlap_budget);
        while !window.is_char_boundary(retained_start) {
            retained_start -= 1;
        }
        overlap = window[retained_start..].to_owned();
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn backing_independent_search_preserves_unicode_and_rejects_nonprogress() {
        let text = "前😀Needle后";
        let found =
            search_literal_bytes_with_reader(text.len() as u64, "needle", |offset, limit| {
                let suffix = &text[offset as usize..];
                let mut end = suffix.len().min(limit);
                while !suffix.is_char_boundary(end) {
                    end -= 1;
                }
                Ok(crate::canonical_text_store::CanonicalTextRange {
                    offset_bytes: offset,
                    next_offset_bytes: offset + end as u64,
                    text: suffix[..end].into(),
                    end_of_content: offset + end as u64 == text.len() as u64,
                })
            })
            .unwrap()
            .unwrap();
        assert_eq!(found.match_offset_bytes, "前😀".len() as u64);
        assert_eq!(found.start_character, 0);
        assert_eq!(found.snippet, text);
        assert!(search_literal_bytes_with_reader(4, "x", |offset, _| {
            Ok(crate::canonical_text_store::CanonicalTextRange {
                offset_bytes: offset,
                next_offset_bytes: offset,
                text: String::new(),
                end_of_content: false,
            })
        })
        .is_err());
    }

    fn reference(text: &str, chunks: u32) -> CanonicalBodyReference {
        CanonicalBodyReference::for_field(
            "body".into(),
            "c".into(),
            "m".into(),
            "p".into(),
            nola_domain::PartBodyField::ContentText,
            text.len() as u64,
            chunks,
        )
        .unwrap()
    }

    #[test]
    fn unicode_and_ascii_folded_matches_cross_chunk_boundaries_with_exact_offsets() {
        let chunks = ["开始 中", "😀Ne", "eDle%_\\结束"];
        let text = chunks.concat();
        let reference = reference(&text, chunks.len() as u32);
        let mut store = CanonicalTextStore::open(Path::new(":memory:"), 1000).unwrap();
        let mut offset = 0;
        store
            .put(
                &reference,
                chunks.into_iter().map(|text| {
                    let start = offset;
                    offset += text.len() as u64;
                    Ok((start, text.to_string()))
                }),
            )
            .unwrap();
        for query in ["中😀needle", "needle", "%_\\", "结束"] {
            let found = store.search_literal(&reference, query).unwrap().unwrap();
            let expected = text
                .to_ascii_lowercase()
                .find(&query.to_ascii_lowercase())
                .unwrap();
            assert_eq!(found.match_offset_bytes, expected as u64);
            assert_eq!(found.start_character, 0);
            assert_eq!(found.snippet, text);
        }
        assert!(store
            .search_literal(&reference, "%missing")
            .unwrap()
            .is_none());
        assert!(store.search_literal(&reference, " ").is_err());
        assert!(store.search_literal(&reference, &"x".repeat(501)).is_err());
        let foreign = CanonicalBodyReference::for_field(
            "body".into(),
            "foreign".into(),
            "m".into(),
            "p".into(),
            nola_domain::PartBodyField::ContentText,
            text.len() as u64,
            3,
        )
        .unwrap();
        assert!(store.search_literal(&foreign, "needle").is_err());
    }

    #[test]
    fn ten_million_byte_search_restores_bounded_excerpt_after_restart() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("canonical.db");
        let total = 10_000_000_usize;
        let chunk = CANONICAL_BODY_CHUNK_MAX_BYTES as usize;
        let marker = "CROSS-CHUNK-NEEDLE";
        let marker_start = 300 * chunk - 5;
        let reference = CanonicalBodyReference::for_field(
            "body".into(),
            "c".into(),
            "m".into(),
            "p".into(),
            nola_domain::PartBodyField::ContentText,
            total as u64,
            total.div_ceil(chunk) as u32,
        )
        .unwrap();
        let mut store = CanonicalTextStore::open(&path, total as u64).unwrap();
        store
            .put(
                &reference,
                (0..total).step_by(chunk).map(|start| {
                    let end = (start + chunk).min(total);
                    let mut text = "x".repeat(end - start);
                    let lo = start.max(marker_start);
                    let hi = end.min(marker_start + marker.len());
                    if lo < hi {
                        text.replace_range(
                            lo - start..hi - start,
                            &marker[lo - marker_start..hi - marker_start],
                        );
                    }
                    Ok((start as u64, text))
                }),
            )
            .unwrap();
        drop(store);
        let store = CanonicalTextStore::open_existing(&path, total as u64).unwrap();
        let started = std::time::Instant::now();
        let found = store
            .search_literal(&reference, "cross-chunk-needle")
            .unwrap()
            .unwrap();
        assert_eq!(found.match_offset_bytes, marker_start as u64);
        assert_eq!(found.start_character, (marker_start - 240) as u64);
        assert_eq!(found.snippet.chars().count(), 1600);
        assert_eq!(&found.snippet[240..240 + marker.len()], marker);
        assert!(store
            .search_literal(&reference, "not-present")
            .unwrap()
            .is_none());
        eprintln!(
            "chunked 10M positive and negative recall: {:?}",
            started.elapsed()
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }
}
