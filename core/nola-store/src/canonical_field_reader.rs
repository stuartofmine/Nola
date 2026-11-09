// SPDX-License-Identifier: Apache-2.0
//! Function-local, fixed-snapshot canonical original-field reads. Not a portable
//! handle, public-journal reference conversion, or authority to execute content.
use crate::canonical_text_store::CanonicalTextRange;
use crate::published_part_body::{self, ResolvedPartField};
use crate::{Connection, StoreError, StoreResult};
use nola_domain::{CanonicalBodyReference, PartBodyField};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CanonicalFieldBacking {
    Absent,
    Inline { total_utf8_bytes: u64 },
    Sealed(CanonicalBodyReference),
    Chunked(nola_domain::CanonicalStreamHead),
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CanonicalJsonRead {
    Absent,
    Complete(String),
    /// Owner/reference binding is validated, not payload availability. No
    /// chunks are scanned merely to report that the budget is insufficient.
    Deferred {
        total_utf8_bytes: u64,
    },
}

/// Borrowed only within a repository call. Holding this value across HTTP or
/// exposing it through StorePort is intentionally unsupported.
pub(super) struct CanonicalFieldReader<'c> {
    connection: &'c Connection,
    conversation_id: String,
    message_id: String,
    part_id: String,
    field: PartBodyField,
    backing: CanonicalFieldBacking,
    _snapshot: Option<rusqlite::Transaction<'c>>,
}

impl<'c> CanonicalFieldReader<'c> {
    pub(super) fn open(
        connection: &'c Connection,
        conversation: &str,
        message: &str,
        part: &str,
        field: PartBodyField,
    ) -> StoreResult<Self> {
        Self::open_mode(connection, conversation, message, part, field, false)
    }

    pub(super) fn open_disclosure_source(
        connection: &'c Connection,
        conversation: &str,
        message: &str,
        part: &str,
    ) -> StoreResult<Self> {
        Self::open_mode(
            connection,
            conversation,
            message,
            part,
            PartBodyField::ContentJson,
            true,
        )
    }

    fn open_mode(
        connection: &'c Connection,
        conversation: &str,
        message: &str,
        part: &str,
        field: PartBodyField,
        disclosure: bool,
    ) -> StoreResult<Self> {
        let snapshot = crate::read_snapshot::pin(connection)?;
        let resolved = if disclosure {
            published_part_body::resolve_disclosure_field(connection, conversation, message, part)?
        } else {
            published_part_body::resolve_field(connection, conversation, message, part, field)?
        };
        let backing = match resolved {
            ResolvedPartField::Absent => CanonicalFieldBacking::Absent,
            ResolvedPartField::Inline { total_utf8_bytes } => {
                CanonicalFieldBacking::Inline { total_utf8_bytes }
            }
            ResolvedPartField::Referenced(reference) => CanonicalFieldBacking::Sealed(reference),
            ResolvedPartField::Chunked(head) => CanonicalFieldBacking::Chunked(head),
        };
        Ok(Self {
            connection,
            conversation_id: conversation.into(),
            message_id: message.into(),
            part_id: part.into(),
            field,
            backing,
            _snapshot: snapshot,
        })
    }

    pub(super) fn backing(&self) -> &CanonicalFieldBacking {
        &self.backing
    }
    pub(super) fn total_utf8_bytes(&self) -> Option<u64> {
        match &self.backing {
            CanonicalFieldBacking::Absent => None,
            CanonicalFieldBacking::Inline { total_utf8_bytes } => Some(*total_utf8_bytes),
            CanonicalFieldBacking::Sealed(reference) => Some(reference.total_utf8_bytes()),
            CanonicalFieldBacking::Chunked(head) => Some(head.total_utf8_bytes),
        }
    }
    pub(super) fn sealed_reference(&self) -> Option<&CanonicalBodyReference> {
        match &self.backing {
            CanonicalFieldBacking::Sealed(reference) => Some(reference),
            _ => None,
        }
    }
    pub(super) fn read_range(&self, offset: u64, limit: usize) -> StoreResult<CanonicalTextRange> {
        // The exact immutable owner/reference was validated when this reader
        // pinned its snapshot. In particular, write-side disclosure derivation
        // must not accidentally re-enter product finality/visibility gates.
        if let CanonicalFieldBacking::Sealed(reference) = &self.backing {
            return crate::canonical_text_store::read_range_from_connection(
                self.connection,
                reference,
                offset,
                limit,
            );
        }
        let expected = self
            .total_utf8_bytes()
            .ok_or_else(|| invalid("canonical field is absent"))?;
        let (total, range) = published_part_body::field_range(
            self.connection,
            &self.conversation_id,
            &self.message_id,
            &self.part_id,
            self.field,
            offset,
            limit,
        )?;
        if total != expected {
            return Err(invalid("canonical field changed inside fixed read view"));
        }
        Ok(range)
    }

    /// Read original JSON bytes without interpreting or re-encoding them.
    /// None is explicitly unbounded; Some(0) is a real zero-byte budget.
    pub(super) fn read_json(
        &self,
        max_utf8_bytes: Option<usize>,
    ) -> StoreResult<CanonicalJsonRead> {
        if self.field != PartBodyField::ContentJson {
            return Err(invalid("JSON hydration cannot reinterpret text field"));
        }
        let Some(total) = self.total_utf8_bytes() else {
            return Ok(CanonicalJsonRead::Absent);
        };
        if max_utf8_bytes.is_some_and(|limit| total > limit as u64) {
            return Ok(CanonicalJsonRead::Deferred {
                total_utf8_bytes: total,
            });
        }
        let length = usize::try_from(total)
            .map_err(|_| invalid("original JSON exceeds platform allocation range"))?;
        if matches!(self.backing, CanonicalFieldBacking::Inline { .. }) {
            // One SQLite value read after byte-length admission; repeated
            // substr(BLOB) range reads would repeatedly copy the inline body.
            let original: String = self.connection.query_row(
                "SELECT content_json FROM message_parts WHERE id=?1 AND message_id=?2",
                crate::params![self.part_id, self.message_id],
                |row| row.get(0),
            )?;
            if original.len() != length {
                return Err(invalid("original JSON changed inside fixed read view"));
            }
            return Ok(CanonicalJsonRead::Complete(original));
        }
        let mut original = String::new();
        original
            .try_reserve_exact(length)
            .map_err(|_| invalid("original JSON allocation unavailable"))?;
        let mut offset = 0;
        while offset < total {
            let range = self.read_range(offset, (total - offset).clamp(4, 32768) as usize)?;
            if range.next_offset_bytes <= offset
                || range.next_offset_bytes > total
                || range.next_offset_bytes - offset != range.text.len() as u64
            {
                return Err(invalid("original JSON range did not advance consistently"));
            }
            original.push_str(&range.text);
            offset = range.next_offset_bytes;
        }
        if original.len() != length {
            return Err(invalid("original JSON length disagrees with source"));
        }
        Ok(CanonicalJsonRead::Complete(original))
    }

    /// Extract bounded scalar metadata without hydrating a full JSON String.
    /// Inline JSON is borrowed from one SQLite row while both visitor passes
    /// run. Repeated inline substr queries would copy the full value per chunk.
    /// Immutable storage is reopened as bounded ranges. Both paths still scan
    /// the complete JSON twice for syntax/digest and selected-field extraction.
    pub(super) fn extract_json_scalar_fields(
        &self,
        known_fields: &[&str],
        maximum_output_bytes: usize,
    ) -> StoreResult<Option<crate::semantic_json_projection::TokenProjection>> {
        if self.field != PartBodyField::ContentJson {
            return Err(invalid(
                "JSON scalar extraction cannot reinterpret text field",
            ));
        }
        let Some(total) = self.total_utf8_bytes() else {
            return Ok(None);
        };
        if matches!(self.backing, CanonicalFieldBacking::Inline { .. }) {
            let mut statement = self
                .connection
                .prepare("SELECT content_json FROM message_parts WHERE id=?1 AND message_id=?2")?;
            let mut rows = statement.query(crate::params![self.part_id, self.message_id])?;
            let row = rows
                .next()?
                .ok_or_else(|| invalid("original JSON owner disappeared"))?;
            let raw = row
                .get_ref(0)?
                .as_str()
                .map_err(|_| invalid("original JSON is not UTF-8 text"))?;
            if raw.len() as u64 != total {
                return Err(invalid("original JSON changed inside fixed read view"));
            }
            return crate::semantic_json_projection::extract_top_level(
                || borrowed_json_chunks(raw),
                total,
                known_fields,
                maximum_output_bytes,
            )
            .map(Some);
        }
        crate::semantic_json_projection::extract_top_level(
            || {
                let mut offset = 0;
                std::iter::from_fn(move || {
                    if offset >= total {
                        return None;
                    }
                    let start = offset;
                    match self.read_range(start, 32768) {
                        Ok(range) if range.next_offset_bytes > start => {
                            offset = range.next_offset_bytes;
                            Some(Ok((start, range.text)))
                        }
                        Ok(_) => {
                            offset = total;
                            Some(Err("JSON scalar source range did not advance".into()))
                        }
                        Err(error) => {
                            offset = total;
                            Some(Err(error.to_string()))
                        }
                    }
                })
            },
            total,
            known_fields,
            maximum_output_bytes,
        )
        .map(Some)
    }
    /// Existing tail policy, in characters rather than bytes. Unbounded reads
    /// are explicit (tail_characters == 0); streaming callers use read_range.
    pub(super) fn read_text(&self, tail_characters: usize) -> StoreResult<String> {
        if self.field != PartBodyField::ContentText {
            return Err(invalid("text projection cannot reinterpret JSON field"));
        }
        if tail_characters > 0 {
            if let CanonicalFieldBacking::Chunked(head) = &self.backing {
                return crate::canonical_stream_read::tail(self.connection, head, tail_characters);
            }
        }
        if let CanonicalFieldBacking::Inline { total_utf8_bytes } = self.backing {
            // A Unicode suffix needs at most four bytes per requested scalar.
            // Avoid SQL length/substr, which materializes the complete large TEXT.
            let bytes_needed = if tail_characters == 0 { total_utf8_bytes }
                else { total_utf8_bytes.min(tail_characters.saturating_mul(4) as u64) };
            let offset = total_utf8_bytes - bytes_needed;
            let bytes = published_part_body::inline_bytes(self.connection, &self.part_id,
                self.field, offset, bytes_needed as usize)?;
            let skip = bytes.iter().take_while(|byte| **byte & 0xc0 == 0x80).count();
            if skip > 3 || (offset == 0 && skip > 0) { return Err(invalid("invalid original UTF-8 suffix")); }
            let text = std::str::from_utf8(&bytes[skip..]).map_err(|_| invalid("invalid original UTF-8 text"))?;
            let start = if tail_characters == 0 { 0 }
                else { text.char_indices().rev().nth(tail_characters - 1).map(|(index, _)| index).unwrap_or(0) };
            let suffix = &text[start..];
            return Ok(if offset > 0 || skip > 0 || start > 0 { format!("…{suffix}") }
                else { suffix.to_owned() });
        }
        let total = self
            .total_utf8_bytes()
            .ok_or_else(|| invalid("canonical text is absent"))?;
        let (mut result, mut offset, mut truncated) = (String::new(), 0, false);
        while offset < total {
            let range = self.read_range(offset, 32768)?;
            if range.next_offset_bytes <= offset {
                return Err(invalid("canonical range failed to advance"));
            }
            result.push_str(&range.text);
            offset = range.next_offset_bytes;
            if tail_characters > 0 {
                if let Some((start, _)) = result.char_indices().rev().nth(tail_characters - 1) {
                    if start > 0 {
                        result.drain(..start);
                        truncated = true;
                    }
                }
            }
        }
        if truncated {
            result.insert(0, '…');
        }
        Ok(result)
    }

    /// A bounded preview of the original field, never an unbounded read followed
    /// by truncation. Prefix limits are Unicode scalars, like SQLite substr.
    pub(super) fn read_text_prefix(&self, characters: usize) -> StoreResult<String> {
        if self.field != PartBodyField::ContentText {
            return Err(invalid("text prefix cannot reinterpret JSON field"));
        }
        let total = self
            .total_utf8_bytes()
            .ok_or_else(|| invalid("canonical text is absent"))?;
        let mut result = String::new();
        let mut offset = 0;
        let mut remaining = characters;
        while remaining > 0 && offset < total {
            let limit = remaining.saturating_mul(4).clamp(4, 32768);
            let range = self.read_range(offset, limit)?;
            if range.next_offset_bytes <= offset {
                return Err(invalid("canonical prefix failed to advance"));
            }
            for character in range.text.chars().take(remaining) {
                result.push(character);
                remaining -= 1;
            }
            offset = range.next_offset_bytes;
        }
        Ok(result)
    }

    /// Logical text search, independent of field backing. The caller installs
    /// the borrowed nola_literal_position SQLite function once per search.
    pub(super) fn find_literal(&self, query: &str) -> StoreResult<Option<(u64, String)>> {
        if self.field != PartBodyField::ContentText
            || query.trim().is_empty()
            || query.chars().count() > 500
        {
            return Err(invalid("invalid canonical text search"));
        }
        if matches!(self.backing, CanonicalFieldBacking::Inline { .. }) {
            use rusqlite::OptionalExtension;
            // Materialize only one scalar position, never the whole body or
            // conversation. The matcher runs once for this candidate.
            return Ok(self
                .connection
                .query_row(
                    "WITH hit AS MATERIALIZED (
                    SELECT nola_literal_position(content_text,?2) AS position
                    FROM message_parts WHERE id=?1 AND message_id=?3)
                 SELECT MAX(1,h.position-240)-1,
                    substr(p.content_text,MAX(1,h.position-240),1600)
                 FROM hit h JOIN message_parts p ON p.id=?1 WHERE h.position>0",
                    crate::params![self.part_id, query, self.message_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?);
        }
        let total = self
            .total_utf8_bytes()
            .ok_or_else(|| invalid("canonical text search field is absent"))?;
        Ok(
            crate::canonical_text_search::search_literal_bytes_with_reader(
                total,
                query,
                |offset, size| self.read_range(offset, size),
            )?
            .map(|hit| (hit.start_character, hit.snippet)),
        )
    }
}

pub(super) fn borrowed_json_chunks(
    raw: &str,
) -> impl Iterator<Item = Result<(u64, String), String>> + '_ {
    let mut offset = 0;
    std::iter::from_fn(move || {
        if offset == raw.len() {
            return None;
        }
        let start = offset;
        let mut end = raw.len().min(start + 32768);
        while !raw.is_char_boundary(end) {
            end -= 1;
        }
        offset = end;
        Some(Ok((start as u64, raw[start..end].to_owned())))
    })
}
fn invalid(message: &str) -> StoreError {
    StoreError::Invariant(message.into())
}

#[cfg(test)]
#[path = "canonical_field_reader_tests.rs"]
mod tests;
