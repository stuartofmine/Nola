// SPDX-License-Identifier: Apache-2.0
//! Owner-bound visible Part planning with bounded publication frames.
//! No Store writes, transport, synthetic ownership guesses or authority.
use crate::public_part_semantics::{PublicPart, PublicTextPart};
use nola_domain::assistant_journal::{
    AssistantJournalIdentity, AssistantTextChunkV1, ASSISTANT_CHUNK_MAX_BYTES,
    ASSISTANT_JOURNAL_VERSION,
};
use nola_domain::assistant_journal_operations::*;
use nola_domain::TextReviewState;
#[cfg(test)]
use nola_agent_engine::agent::parts::AgentPart;
use nola_agent_engine::agent::parts::PartUpdate;
use nola_store_port::assistant_journal::{AssistantJournalPartState, AssistantManifestPage};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAX_SUPPLIED_FRAME_BYTES: usize = 256 * 1024;
const MAX_SUPPLIED_FRAME_CHUNKS: usize = 128;
// JSON escaping can make a valid 128 KiB canonical batch several times larger.
// Keep its wire form within 128 KiB too, leaving the 256 KiB transport envelope
// room for a record header, content proofs and one original 64 KiB base64 chunk.
const MAX_BATCH_JSON_BYTES: usize = 128 * 1024;
// Each full immutable UTF-8 chunk is at most 64 KiB. An Append operation
// itself must fit in one frame, not merely in the logical transition.
const MAX_APPEND_OPERATION_CHUNKS: usize = MAX_SUPPLIED_FRAME_BYTES / ASSISTANT_CHUNK_MAX_BYTES;

#[path = "assistant_journal_planner/content_match.rs"]
mod content_match;
#[path = "assistant_journal_planner/synthetic_parts.rs"]
mod synthetic_parts;
#[path = "assistant_journal_planner/tool_parts.rs"]
mod tool_parts;
#[path = "assistant_journal_planner/visible_parts.rs"]
mod visible_parts;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannerError {
    NotYetSupported(&'static str),
    Invalid(String),
    OwnerMismatch,
    MissingPart,
    RetiredPart,
    Budget,
}
impl std::fmt::Display for PlannerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for PlannerError {}
impl From<OperationError> for PlannerError {
    fn from(error: OperationError) -> Self {
        Self::Invalid(error.to_string())
    }
}
type Result<T> = std::result::Result<T, PlannerError>;

pub trait JournalPlanningReader {
    /// Narrow lookup inside the eventual commit UoW. Retirement and absence are
    /// separate facts; callers must never treat a tombstone as a new slot.
    fn load_head(
        &self,
        identity: &AssistantJournalIdentity,
        part_id: &str,
    ) -> Result<AssistantJournalPartState>;
    /// Exact published manifest boundaries; implementations must not fabricate
    /// a fixed rechunking of the candidate text or silently return an empty page.
    fn load_manifest_page(
        &self,
        identity: &AssistantJournalIdentity,
        part_id: &str,
        content: &JournalContentRef,
        after_chunk: u64,
        max_records: usize,
    ) -> Result<AssistantManifestPage>;
}

#[derive(Debug)]
pub struct PrepareJournalContent {
    pub part_id: String,
    pub previous: JournalContentRef,
    pub resulting: JournalContentRef,
    pub chunks: Vec<AssistantTextChunkV1>,
}
#[derive(Debug)]
pub struct PreparedAssistantJournal {
    pub batch: AssistantOperationBatchV1,
    pub supplied_chunks: Vec<AssistantTextChunkV1>,
    pub preparations: Vec<PrepareJournalContent>,
}
#[derive(Debug)]
pub enum VisibleJournalPlan {
    Unchanged,
    Changed(Vec<PreparedAssistantJournal>),
}

pub struct AssistantJournalPlanner<'a, R: JournalPlanningReader + ?Sized> {
    reader: &'a R,
    identity: AssistantJournalIdentity,
    heads: BTreeMap<String, Option<JournalPartHead>>,
    retired: BTreeSet<String>,
    operations: Vec<AssistantJournalOperationV1>,
    supplied_chunks: Vec<AssistantTextChunkV1>,
    preparations: Vec<PrepareJournalContent>,
}
impl<'a, R: JournalPlanningReader + ?Sized> AssistantJournalPlanner<'a, R> {
    pub fn new(reader: &'a R, identity: &AssistantJournalIdentity) -> Result<Self> {
        identity
            .validate()
            .map_err(|e| PlannerError::Invalid(e.to_string()))?;
        Ok(Self {
            reader,
            identity: identity.clone(),
            heads: BTreeMap::new(),
            retired: BTreeSet::new(),
            operations: vec![],
            supplied_chunks: vec![],
            preparations: vec![],
        })
    }

    fn head(&mut self, part_id: &str) -> Result<Option<JournalPartHead>> {
        if self.retired.contains(part_id) {
            return Err(PlannerError::RetiredPart);
        }
        if !self.heads.contains_key(part_id) {
            let head = match self.reader.load_head(&self.identity, part_id)? {
                AssistantJournalPartState::NeverDeclared => None,
                AssistantJournalPartState::Live(head) => Some(head),
                AssistantJournalPartState::Retired => {
                    self.retired.insert(part_id.into());
                    return Err(PlannerError::RetiredPart);
                }
            };
            if head
                .as_ref()
                .is_some_and(|h| h.identity != self.identity || h.part_id != part_id)
            {
                return Err(PlannerError::OwnerMismatch);
            }
            self.heads.insert(part_id.to_owned(), head);
        }
        Ok(self.heads.get(part_id).cloned().flatten())
    }

    pub fn begin_text(&mut self, part_id: &str, order: JournalOrderKey) -> Result<()> {
        if let Some(head) = self.head(part_id)? {
            text_metadata(&head)?;
            return Ok(());
        }
        self.declare(
            part_id,
            order,
            JournalTextMetadata {
                is_final: false,
                visibility: JournalVisibility::Public,
                review: None,
            },
            "",
        )
    }

    #[cfg(test)]
    pub fn part_update(
        &mut self,
        update: &PartUpdate,
        canonical_part: &AgentPart,
        order: JournalOrderKey,
    ) -> Result<()> {
        self.part_update_public(update, PublicPart::from_core(canonical_part), order)
    }

    pub(crate) fn part_update_view(
        &mut self,
        update: &PartUpdate,
        canonical: &crate::canonical_part_projection::CanonicalPartProjection<'_>,
        order: JournalOrderKey,
    ) -> Result<()> {
        if canonical.run_id() != self.identity.run_id {
            return Err(PlannerError::OwnerMismatch);
        }
        self.part_update_public(update, canonical.public_part(), order)
    }

    fn part_update_public(
        &mut self,
        update: &PartUpdate,
        public_part: PublicPart<'_>,
        order: JournalOrderKey,
    ) -> Result<()> {
        let identity = public_part.identity();
        if identity.message_id != self.identity.assistant_message_id
            || update.part_id() != identity.part_id
        {
            return Err(PlannerError::OwnerMismatch);
        }
        let PublicPart::Text(part) = public_part else {
            return self.plan_non_text(update, public_part, order);
        };
        let metadata = visible_metadata(&part);
        match update {
            PartUpdate::Created { .. } => {
                match self.head(part.identity.part_id)? {
                    None => {
                        self.declare(part.identity.part_id, order, metadata, visible_text(&part))
                    }
                    Some(head) => {
                        // Admission creates the placeholder before Core's Created event.
                        // Only that empty same-owner placeholder may be materialized here.
                        if head.bodies.iter().any(|body| body.utf8_bytes.0 != 0) {
                            return Err(PlannerError::Invalid(
                                "duplicate nonempty Text creation".into(),
                            ));
                        }
                        if visible_text(&part).is_empty()
                            && text_metadata(&head)?.visibility == metadata.visibility
                        {
                            self.update_metadata(head, metadata)
                        } else {
                            self.replace(head, metadata, visible_text(&part))
                        }
                    }
                }
            }
            PartUpdate::TextDelta { delta, .. } | PartUpdate::TextCommittedDelta { delta, .. } => {
                let head = self
                    .head(part.identity.part_id)?
                    .ok_or(PlannerError::MissingPart)?;
                let previous = text_metadata(&head)?;
                if previous.visibility != metadata.visibility {
                    return Err(PlannerError::Invalid(
                        "Text visibility changed without review boundary".into(),
                    ));
                }
                if metadata.visibility == JournalVisibility::Withheld {
                    return self.update_metadata(head, metadata);
                }
                let body = markdown(&head)?;
                if body.utf8_bytes.0.checked_add(delta.len() as u64) != Some(part.text.len() as u64)
                    || !part.text.ends_with(delta)
                {
                    return Err(PlannerError::Invalid(
                        "Text delta disagrees with committed part length/suffix".into(),
                    ));
                }
                if !delta.is_empty() {
                    if previous.is_final {
                        return Err(PlannerError::Invalid("append after final Text".into()));
                    }
                    self.append(head, delta)?;
                }
                let head = self
                    .head(part.identity.part_id)?
                    .ok_or(PlannerError::MissingPart)?;
                self.update_metadata(head, metadata)
            }
            PartUpdate::TextReplaced {
                text, is_complete, ..
            } => {
                if text != part.text || *is_complete != part.is_complete {
                    return Err(PlannerError::Invalid(
                        "Text replacement disagrees with committed part".into(),
                    ));
                }
                let head = self
                    .head(part.identity.part_id)?
                    .ok_or(PlannerError::MissingPart)?;
                if metadata.visibility == JournalVisibility::Withheld
                    && text_metadata(&head)?.visibility == JournalVisibility::Withheld
                {
                    self.update_metadata(head, metadata)
                } else {
                    self.replace(head, metadata, visible_text(&part))
                }
            }
            PartUpdate::TextReviewChanged { state, .. } => {
                if part.review != Some(*state) {
                    return Err(PlannerError::Invalid(
                        "review update disagrees with committed part".into(),
                    ));
                }
                let head = self
                    .head(part.identity.part_id)?
                    .ok_or(PlannerError::MissingPart)?;
                if text_metadata(&head)?.visibility == metadata.visibility {
                    self.update_metadata(head, metadata)
                } else {
                    self.replace(head, metadata, visible_text(&part))
                }
            }
            PartUpdate::Completed { .. } => {
                let head = self
                    .head(part.identity.part_id)?
                    .ok_or(PlannerError::MissingPart)?;
                if text_metadata(&head)?.visibility != metadata.visibility {
                    return Err(PlannerError::Invalid(
                        "completion changes review visibility".into(),
                    ));
                }
                self.update_metadata(head, metadata)
            }
            _ => Err(PlannerError::NotYetSupported("non-text update")),
        }
    }

    /// Canonical-only terminal metadata, with no cumulative body fetch. Core
    /// Part events handle ordinary closes; this covers placeholder/Run closes.
    pub(crate) fn close_admitted_part(
        &mut self, part_id: &str, state: &str, order: JournalOrderKey,
    ) -> Result<()> {
        if self.head(part_id)?.is_none() {
            // A failed/cancelled run can close its admitted text without ever
            // emitting content. Materialize it only now, so the exact terminal
            // seal still has a durable frontier without stealing earlier order.
            self.begin_text(part_id, order)?;
        }
        self.close_public_part(part_id, state)
    }

    pub(crate) fn close_public_part(&mut self, part_id: &str, state: &str) -> Result<()> {
        let head = self.head(part_id)?.ok_or(PlannerError::MissingPart)?;
        let activity = match state {
            "sealed" => nola_domain::ActivityState::Sealed,
            "completed" => nola_domain::ActivityState::Completed,
            "cancelled" => nola_domain::ActivityState::Cancelled,
            "failed" => nola_domain::ActivityState::Failed,
            _ => return Err(PlannerError::Invalid("nonterminal synthetic close".into())),
        };
        match head.metadata.clone() {
            JournalPartMetadata::Text(mut metadata) => {
                metadata.is_final = true;
                self.update_metadata(head, metadata)
            }
            JournalPartMetadata::Reasoning(mut metadata) => {
                metadata.state = activity;
                self.update_visible_metadata(head, JournalPartMetadata::Reasoning(metadata))
            }
            _ => Err(PlannerError::Invalid(
                "synthetic close has unsupported Part kind".into(),
            )),
        }
    }

    fn bodies(
        &mut self,
        part_id: &str,
        generation: JournalNumber,
        metadata: &JournalTextMetadata,
        text: &str,
    ) -> Result<Vec<JournalContentRef>> {
        if metadata.visibility == JournalVisibility::Withheld {
            if !text.is_empty() {
                return Err(PlannerError::Invalid(
                    "withheld body must not be supplied".into(),
                ));
            }
            return Ok(vec![]);
        }
        Ok(vec![self.prepare_body(
            part_id,
            generation,
            JournalContentField::Markdown,
            text,
        )?])
    }

    fn prepare_body(
        &mut self,
        part_id: &str,
        generation: JournalNumber,
        field: JournalContentField,
        text: &str,
    ) -> Result<JournalContentRef> {
        let previous = empty_journal_content(&self.identity, part_id, field, generation)?;
        let chunks = chunk_text(text)?;
        let mut resulting = previous.clone();
        // Store preparation accepts at most 256 KiB per call; each continuation
        // is an immutable manifest extension, not a new visible generation.
        for group in chunks.chunks(4) {
            let previous = resulting;
            resulting = append_journal_content(&previous, &chunk_refs(group))?;
            self.preparations.push(PrepareJournalContent {
                part_id: part_id.to_owned(),
                previous,
                resulting: resulting.clone(),
                chunks: group.to_vec(),
            });
        }
        Ok(resulting)
    }

    fn declare(
        &mut self,
        part_id: &str,
        order_key: JournalOrderKey,
        metadata: JournalTextMetadata,
        text: &str,
    ) -> Result<()> {
        let bodies = self.bodies(part_id, JournalNumber(1), &metadata, text)?;
        let head = JournalPartHead {
            identity: self.identity.clone(),
            part_id: part_id.to_owned(),
            generation: JournalNumber(1),
            revision: JournalNumber(1),
            metadata_revision: JournalNumber(1),
            order_key,
            metadata: JournalPartMetadata::Text(metadata),
            bodies,
        };
        self.operations
            .push(AssistantJournalOperationV1::Declare(DeclareJournalPart {
                part: head.clone(),
            }));
        self.heads.insert(part_id.to_owned(), Some(head));
        Ok(())
    }

    fn replace(
        &mut self,
        head: JournalPartHead,
        metadata: JournalTextMetadata,
        text: &str,
    ) -> Result<()> {
        let generation = increment(head.generation)?;
        let bodies = self.bodies(&head.part_id, increment(head.revision)?, &metadata, text)?;
        let mut replacement = head.clone();
        replacement.generation = generation;
        replacement.revision = increment(head.revision)?;
        replacement.metadata_revision = JournalNumber(1);
        replacement.metadata = JournalPartMetadata::Text(metadata);
        replacement.bodies = bodies;
        self.operations
            .push(AssistantJournalOperationV1::Replace(ReplaceJournalPart {
                part_id: head.part_id.clone(),
                expected_generation: head.generation,
                expected_revision: head.revision,
                part: replacement.clone(),
            }));
        self.heads.insert(head.part_id, Some(replacement));
        Ok(())
    }

    fn append(&mut self, mut head: JournalPartHead, delta: &str) -> Result<()> {
        let chunks = chunk_text(delta)?;
        for group in chunks.chunks(MAX_APPEND_OPERATION_CHUNKS) {
            let expected = markdown(&head)?.clone();
            let refs = chunk_refs(group);
            let resulting = append_journal_content(&expected, &refs)?;
            self.operations
                .push(AssistantJournalOperationV1::Append(AppendJournalText {
                    part_id: head.part_id.clone(),
                    expected_generation: head.generation,
                    expected_revision: head.revision,
                    expected,
                    chunks: refs,
                    resulting: resulting.clone(),
                }));
            head.revision = increment(head.revision)?;
            head.bodies = vec![resulting];
        }
        self.supplied_chunks.extend(chunks);
        self.heads.insert(head.part_id.clone(), Some(head));
        Ok(())
    }

    fn update_metadata(
        &mut self,
        mut head: JournalPartHead,
        metadata: JournalTextMetadata,
    ) -> Result<()> {
        if text_metadata(&head)? == &metadata {
            return Ok(());
        }
        self.operations
            .push(AssistantJournalOperationV1::Update(UpdateJournalMetadata {
                part_id: head.part_id.clone(),
                expected_generation: head.generation,
                expected_revision: head.revision,
                metadata: JournalPartMetadata::Text(metadata.clone()),
            }));
        head.revision = increment(head.revision)?;
        head.metadata_revision = increment(head.metadata_revision)?;
        head.metadata = JournalPartMetadata::Text(metadata);
        self.heads.insert(head.part_id.clone(), Some(head));
        Ok(())
    }

    pub fn finish(self) -> Result<VisibleJournalPlan> {
        if self.operations.is_empty() {
            return Ok(VisibleJournalPlan::Unchanged);
        }
        let mut chunks = VecDeque::from(self.supplied_chunks);
        let mut frames = Vec::new();
        let mut batch = AssistantOperationBatchV1 {
            protocol_version: ASSISTANT_JOURNAL_VERSION,
            identity: self.identity.clone(),
            operations: Vec::new(),
        };
        let mut supplied = Vec::new();
        let mut supplied_bytes = 0usize;
        for operation in self.operations {
            let mut introduced = Vec::new();
            if let AssistantJournalOperationV1::Append(append) = &operation {
                for reference in &append.chunks {
                    let chunk = chunks.pop_front().ok_or_else(|| {
                        PlannerError::Invalid("append has no supplied chunk".into())
                    })?;
                    if reference.digest != chunk.digest()
                        || reference.utf8_bytes.0 != chunk.byte_length() as u64
                    {
                        return Err(PlannerError::Invalid(
                            "append/supplied chunk order differs".into(),
                        ));
                    }
                    introduced.push(chunk);
                }
            }
            let introduced_bytes = introduced
                .iter()
                .map(AssistantTextChunkV1::byte_length)
                .sum::<usize>();
            batch.operations.push(operation);
            let encoding = batch.canonical_bytes();
            let content_budget = supplied.len() + introduced.len() > MAX_SUPPLIED_FRAME_CHUNKS
                || supplied_bytes + introduced_bytes > MAX_SUPPLIED_FRAME_BYTES;
            let wire_budget = !batch_fits_json_budget(&batch)?;
            if content_budget || wire_budget || matches!(&encoding, Err(OperationError::Budget)) {
                let operation = batch.operations.pop().expect("just appended operation");
                if batch.operations.is_empty() {
                    return Err(PlannerError::Budget);
                }
                // Retain the changed-head overlay across frames. Splitting is
                // only a bounded publication envelope, never a new generation.
                frames.push(PreparedAssistantJournal {
                    batch: std::mem::replace(
                        &mut batch,
                        AssistantOperationBatchV1 {
                            protocol_version: ASSISTANT_JOURNAL_VERSION,
                            identity: self.identity.clone(),
                            operations: vec![operation],
                        },
                    ),
                    supplied_chunks: std::mem::take(&mut supplied),
                    preparations: Vec::new(),
                });
                supplied_bytes = 0;
                batch.canonical_bytes()?;
                if introduced.len() > MAX_SUPPLIED_FRAME_CHUNKS
                    || introduced_bytes > MAX_SUPPLIED_FRAME_BYTES
                    || !batch_fits_json_budget(&batch)?
                {
                    return Err(PlannerError::Budget);
                }
            } else {
                encoding?;
            }
            supplied_bytes += introduced_bytes;
            supplied.extend(introduced);
        }
        if !chunks.is_empty() {
            return Err(PlannerError::Invalid(
                "unused supplied append chunks".into(),
            ));
        }
        frames.push(PreparedAssistantJournal {
            batch,
            supplied_chunks: supplied,
            preparations: Vec::new(),
        });
        // Preparations precede publication inside the same outer write UoW.
        // They remain invisible and may be used by any subsequent frame.
        frames[0].preparations = self.preparations;
        Ok(VisibleJournalPlan::Changed(frames))
    }
}

fn batch_fits_json_budget(batch: &AssistantOperationBatchV1) -> Result<bool> {
    struct BudgetWriter {
        used: usize,
        exceeded: bool,
    }
    impl std::io::Write for BudgetWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_BATCH_JSON_BYTES - self.used {
                self.exceeded = true;
                return Err(std::io::Error::other("journal JSON budget"));
            }
            self.used += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = BudgetWriter {
        used: 0,
        exceeded: false,
    };
    match serde_json::to_writer(&mut writer, batch) {
        Ok(()) => Ok(true),
        Err(_) if writer.exceeded => Ok(false),
        Err(error) => Err(PlannerError::Invalid(format!(
            "encode journal batch: {error}"
        ))),
    }
}

fn increment(value: JournalNumber) -> Result<JournalNumber> {
    value
        .0
        .checked_add(1)
        .map(JournalNumber)
        .ok_or_else(|| PlannerError::Invalid("journal revision overflow".into()))
}
fn visible_metadata(part: &PublicTextPart<'_>) -> JournalTextMetadata {
    JournalTextMetadata {
        is_final: part.is_complete,
        review: part.review,
        visibility: if part.review.is_none() || part.review == Some(TextReviewState::Approved) {
            JournalVisibility::Public
        } else {
            JournalVisibility::Withheld
        },
    }
}
fn visible_text<'a>(part: &PublicTextPart<'a>) -> &'a str {
    if visible_metadata(part).visibility == JournalVisibility::Public {
        &part.text
    } else {
        ""
    }
}
fn text_metadata(head: &JournalPartHead) -> Result<&JournalTextMetadata> {
    match &head.metadata {
        JournalPartMetadata::Text(value) => Ok(value),
        _ => Err(PlannerError::NotYetSupported("non-text head")),
    }
}
fn markdown(head: &JournalPartHead) -> Result<&JournalContentRef> {
    head.bodies
        .iter()
        .find(|body| body.field == JournalContentField::Markdown)
        .ok_or_else(|| PlannerError::Invalid("public Text has no Markdown content ref".into()))
}
fn chunk_text(mut text: &str) -> Result<Vec<AssistantTextChunkV1>> {
    let mut chunks = Vec::new();
    while !text.is_empty() {
        let mut end = text.len().min(ASSISTANT_CHUNK_MAX_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        chunks.push(
            AssistantTextChunkV1::new(text[..end].to_owned())
                .map_err(|e| PlannerError::Invalid(e.to_string()))?,
        );
        text = &text[end..];
    }
    Ok(chunks)
}
fn chunk_refs(chunks: &[AssistantTextChunkV1]) -> Vec<JournalChunkRef> {
    chunks
        .iter()
        .map(|chunk| JournalChunkRef {
            digest: chunk.digest(),
            utf8_bytes: JournalNumber(chunk.byte_length() as u64),
        })
        .collect()
}

#[cfg(test)]
#[path = "assistant_journal_planner_tests.rs"]
mod tests;
