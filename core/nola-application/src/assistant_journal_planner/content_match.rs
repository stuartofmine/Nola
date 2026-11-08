// SPDX-License-Identifier: Apache-2.0
//! Compare semantic bytes using the real immutable manifest boundaries. A
//! chained content root is deliberately not a flat whole-body fingerprint.
use super::*;

enum Extension<'a> {
    Prepared(&'a PrepareJournalContent),
    Appended(&'a AppendJournalText),
}
impl Extension<'_> {
    fn previous(&self) -> &JournalContentRef {
        match self {
            Self::Prepared(value) => &value.previous,
            Self::Appended(value) => &value.expected,
        }
    }
    fn resulting(&self) -> &JournalContentRef {
        match self {
            Self::Prepared(value) => &value.resulting,
            Self::Appended(value) => &value.resulting,
        }
    }
}

impl<R: JournalPlanningReader + ?Sized> AssistantJournalPlanner<'_, R> {
    pub(super) fn content_matches(
        &self,
        id: &str,
        expected: &JournalContentRef,
        text: &str,
    ) -> Result<bool> {
        if expected.utf8_bytes.0 != text.len() as u64 {
            return Ok(false);
        }
        // Only transaction-local changed refs are retained here, never the
        // existing cumulative manifest or body. Walk without recursive stacks.
        let mut base = expected.clone();
        let mut suffix = Vec::new();
        loop {
            let extension = self
                .preparations
                .iter()
                .rev()
                .find(|value| value.part_id == id && value.resulting == base)
                .map(Extension::Prepared)
                .or_else(|| {
                    self.operations
                        .iter()
                        .rev()
                        .find_map(|operation| match operation {
                            AssistantJournalOperationV1::Append(value)
                                if value.part_id == id && value.resulting == base =>
                            {
                                Some(Extension::Appended(value))
                            }
                            _ => None,
                        })
                });
            let Some(extension) = extension else { break };
            let previous = extension.previous();
            if previous.field != base.field
                || previous.content_generation != base.content_generation
                || previous.chunk_count.0 >= base.chunk_count.0
                || previous.utf8_bytes.0 >= base.utf8_bytes.0
            {
                return Err(PlannerError::Invalid(
                    "invalid local manifest extension".into(),
                ));
            }
            base = previous.clone();
            suffix.push(extension);
        }
        let mut actual = empty_journal_content(
            &self.identity,
            id,
            expected.field,
            expected.content_generation,
        )?;
        let mut offset = 0;
        let mut matches = true;
        let mut after = 0;
        while after < base.chunk_count.0 {
            let page = self.reader.load_manifest_page(
                &self.identity,
                id,
                &base,
                after,
                MAX_BATCH_OPERATIONS,
            )?;
            let next = after
                .checked_add(page.chunks.len() as u64)
                .ok_or_else(|| PlannerError::Invalid("manifest cursor overflow".into()))?;
            if page.chunks.is_empty()
                || page.chunks.len() > MAX_BATCH_OPERATIONS
                || page.next_chunk != next
                || next > base.chunk_count.0
                || page.complete != (next == base.chunk_count.0)
            {
                return Err(PlannerError::Invalid("invalid manifest page bounds".into()));
            }
            for reference in &page.chunks {
                compare_chunk(&mut actual, reference, text, &mut offset, &mut matches)?;
            }
            after = next;
        }
        if actual != base {
            return Err(PlannerError::Invalid(
                "published manifest disagrees with exact ref".into(),
            ));
        }
        for extension in suffix.into_iter().rev() {
            if extension.previous() != &actual {
                return Err(PlannerError::Invalid(
                    "local manifest prefix mismatch".into(),
                ));
            }
            match &extension {
                Extension::Prepared(value) => {
                    for chunk in &value.chunks {
                        compare_chunk(
                            &mut actual,
                            &JournalChunkRef {
                                digest: chunk.digest(),
                                utf8_bytes: JournalNumber(chunk.byte_length() as u64),
                            },
                            text,
                            &mut offset,
                            &mut matches,
                        )?;
                    }
                }
                Extension::Appended(value) => {
                    for reference in &value.chunks {
                        compare_chunk(&mut actual, reference, text, &mut offset, &mut matches)?;
                    }
                }
            }
            if extension.resulting() != &actual {
                return Err(PlannerError::Invalid(
                    "local manifest result mismatch".into(),
                ));
            }
        }
        Ok(matches && offset == text.len() && actual == *expected)
    }
}

fn compare_chunk(
    actual: &mut JournalContentRef,
    reference: &JournalChunkRef,
    text: &str,
    offset: &mut usize,
    matches: &mut bool,
) -> Result<()> {
    let count = usize::try_from(reference.utf8_bytes.0)
        .map_err(|_| PlannerError::Invalid("manifest chunk length overflow".into()))?;
    if count == 0 || count > ASSISTANT_CHUNK_MAX_BYTES {
        return Err(PlannerError::Invalid(
            "manifest chunk length outside budget".into(),
        ));
    }
    let end = offset
        .checked_add(count)
        .ok_or_else(|| PlannerError::Invalid("manifest byte offset overflow".into()))?;
    if *matches {
        *matches = match text.get(*offset..end) {
            Some(value) => {
                AssistantTextChunkV1::new(value.to_owned())
                    .map_err(|error| PlannerError::Invalid(error.to_string()))?
                    .digest()
                    == reference.digest
            }
            None => false,
        };
    }
    *actual = append_journal_content(actual, std::slice::from_ref(reference))?;
    *offset = end;
    Ok(())
}
