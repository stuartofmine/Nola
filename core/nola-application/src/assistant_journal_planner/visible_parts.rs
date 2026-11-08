// SPDX-License-Identifier: Apache-2.0
//! Shared visible-field operations. Each intermediate head remains valid;
//! immutable bodies are never hydrated by the planning reader.
use super::*;
use crate::public_part_semantics::{PublicPart, PublicReasoningPart, PublicStepPart};
use nola_domain::assistant_journal_content::encode_assistant_source_evidence_content;
use nola_domain::ActivityState;
use nola_agent_engine::agent::parts::StepStatus;

const INLINE_TEXT_BYTES: usize = 1024;

impl<R: JournalPlanningReader + ?Sized> AssistantJournalPlanner<'_, R> {
    pub(super) fn plan_non_text(
        &mut self,
        update: &PartUpdate,
        part: PublicPart<'_>,
        order: JournalOrderKey,
    ) -> Result<()> {
        match part {
            PublicPart::Reasoning(part) => self.plan_reasoning(update, part, order),
            PublicPart::Tool(part) => self.plan_tool(update, part, order),
            PublicPart::Step(part) => self.plan_step(update, part, order),
            PublicPart::Source(part) => {
                if !matches!(update, PartUpdate::Created { .. }) {
                    return self.static_completion(update, part.identity.part_id, |metadata| {
                        matches!(metadata, JournalPartMetadata::Source(_))
                    });
                }
                self.require_undeclared(part.identity.part_id)?;
                let mut bodies = vec![];
                let title = self.text_value(
                    part.identity.part_id,
                    JournalContentField::Title,
                    part.title,
                    &mut bodies,
                )?;
                let uri = self.text_value(
                    part.identity.part_id,
                    JournalContentField::Uri,
                    part.uri,
                    &mut bodies,
                )?;
                let mime_type = part
                    .mime_type
                    .map(|value| {
                        self.text_value(
                            part.identity.part_id,
                            JournalContentField::MimeType,
                            value,
                            &mut bodies,
                        )
                    })
                    .transpose()?;
                let evidence = if let Some(evidence) = part.evidence {
                    bodies.push(
                        self.prepare_body(
                            part.identity.part_id,
                            JournalNumber(1),
                            JournalContentField::SourceEvidence,
                            &encode_assistant_source_evidence_content(evidence)
                                .map_err(PlannerError::Invalid)?,
                        )?,
                    );
                    Some(JournalContentField::SourceEvidence)
                } else {
                    None
                };
                self.declare_visible(
                    part.identity.part_id,
                    order,
                    JournalPartMetadata::Source(JournalSourceMetadata {
                        source_id: part.source_id.to_owned(),
                        title,
                        uri,
                        mime_type,
                        evidence,
                    }),
                    bodies,
                )
            }
            PublicPart::Citation(part) => {
                if !matches!(update, PartUpdate::Created { .. }) {
                    return self.static_completion(update, part.identity.part_id, |metadata| {
                        matches!(metadata, JournalPartMetadata::Citation(_))
                    });
                }
                self.require_undeclared(part.identity.part_id)?;
                let mut bodies = vec![];
                let title = self.text_value(
                    part.identity.part_id,
                    JournalContentField::Title,
                    part.title,
                    &mut bodies,
                )?;
                let uri = self.text_value(
                    part.identity.part_id,
                    JournalContentField::Uri,
                    part.uri,
                    &mut bodies,
                )?;
                let mime_type = part
                    .mime_type
                    .map(|value| {
                        self.text_value(
                            part.identity.part_id,
                            JournalContentField::MimeType,
                            value,
                            &mut bodies,
                        )
                    })
                    .transpose()?;
                let location = part
                    .location
                    .map(|location| {
                        Ok::<_, PlannerError>(JournalCitationLocation {
                            page: location.page.map(JournalNumber),
                            line_start: location.line_start.map(JournalNumber),
                            line_end: location.line_end.map(JournalNumber),
                            section: location
                                .section
                                .as_deref()
                                .map(|section| {
                                    self.text_value(
                                        part.identity.part_id,
                                        JournalContentField::CitationSection,
                                        section,
                                        &mut bodies,
                                    )
                                })
                                .transpose()?,
                        })
                    })
                    .transpose()?;
                self.declare_visible(
                    part.identity.part_id,
                    order,
                    JournalPartMetadata::Citation(JournalCitationMetadata {
                        citation_id: part.citation_id.to_owned(),
                        source_id: part.source_id.to_owned(),
                        title,
                        uri,
                        mime_type,
                        origin_tool_call_id: part.origin_tool_call_id.to_owned(),
                        location,
                        byte_range: part.range.map(|range| JournalRange {
                            start: JournalNumber(range.start),
                            end: JournalNumber(range.end),
                        }),
                    }),
                    bodies,
                )
            }
            PublicPart::Attachment(part) => {
                if !matches!(update, PartUpdate::Created { .. }) {
                    return self.static_completion(update, part.identity.part_id, |metadata| {
                        matches!(metadata, JournalPartMetadata::Attachment(_))
                    });
                }
                self.require_undeclared(part.identity.part_id)?;
                let mut bodies = vec![];
                let name = self.text_value(
                    part.identity.part_id,
                    JournalContentField::Name,
                    part.name,
                    &mut bodies,
                )?;
                let uri = self.text_value(
                    part.identity.part_id,
                    JournalContentField::Uri,
                    part.uri,
                    &mut bodies,
                )?;
                let mime_type = part
                    .mime_type
                    .map(|value| {
                        self.text_value(
                            part.identity.part_id,
                            JournalContentField::MimeType,
                            value,
                            &mut bodies,
                        )
                    })
                    .transpose()?;
                let operation = part
                    .operation
                    .map(|value| {
                        self.text_value(
                            part.identity.part_id,
                            JournalContentField::Operation,
                            value,
                            &mut bodies,
                        )
                    })
                    .transpose()?;
                self.declare_visible(
                    part.identity.part_id,
                    order,
                    JournalPartMetadata::Attachment(JournalAttachmentMetadata {
                        attachment_id: part.identity.part_id.to_owned(),
                        name,
                        mime_type,
                        uri,
                        operation,
                    }),
                    bodies,
                )
            }
            PublicPart::Compaction(_) => {
                Err(PlannerError::NotYetSupported("Compaction journal adapter"))
            }
            PublicPart::Text(_) => Err(PlannerError::Invalid(
                "Text entered non-text planner".into(),
            )),
        }
    }

    fn require_undeclared(&mut self, id: &str) -> Result<()> {
        if self.head(id)?.is_some() {
            return Err(PlannerError::Invalid(
                "duplicate visible Part creation".into(),
            ));
        }
        Ok(())
    }

    fn static_completion(
        &mut self,
        update: &PartUpdate,
        id: &str,
        expected_kind: fn(&JournalPartMetadata) -> bool,
    ) -> Result<()> {
        if !matches!(update, PartUpdate::Completed { .. }) {
            return Err(PlannerError::NotYetSupported(
                "update on static visible Part",
            ));
        }
        let head = self.head(id)?.ok_or(PlannerError::MissingPart)?;
        if !expected_kind(&head.metadata) {
            return Err(PlannerError::Invalid(
                "static completion has a different Part kind".into(),
            ));
        }
        Ok(())
    }

    fn plan_step(
        &mut self,
        update: &PartUpdate,
        part: PublicStepPart<'_>,
        order: JournalOrderKey,
    ) -> Result<()> {
        let state = match part.status {
            StepStatus::Started => ActivityState::Running,
            StepStatus::Completed => ActivityState::Completed,
            StepStatus::Failed => ActivityState::Failed,
            StepStatus::Cancelled => ActivityState::Cancelled,
        };
        if matches!(update, PartUpdate::Created { .. }) {
            self.require_undeclared(part.identity.part_id)?;
            let mut bodies = vec![];
            let title = self.text_value(
                part.identity.part_id,
                JournalContentField::Title,
                part.title,
                &mut bodies,
            )?;
            return self.declare_visible(
                part.identity.part_id,
                order,
                JournalPartMetadata::Step(JournalStepMetadata { title, state }),
                bodies,
            );
        }
        if !matches!(update, PartUpdate::Completed { .. }) {
            return Err(PlannerError::NotYetSupported("non-completion Step update"));
        }
        let head = self
            .head(part.identity.part_id)?
            .ok_or(PlannerError::MissingPart)?;
        let JournalPartMetadata::Step(mut metadata) = head.metadata.clone() else {
            return Err(PlannerError::Invalid(
                "Step update has a different Part kind".into(),
            ));
        };
        if !self.text_value_matches(&head, &metadata.title, part.title)? {
            return Err(PlannerError::Invalid(
                "Step completion changed its title".into(),
            ));
        }
        metadata.state = state;
        self.update_visible_metadata(head, JournalPartMetadata::Step(metadata))
    }

    fn plan_reasoning(
        &mut self,
        update: &PartUpdate,
        part: PublicReasoningPart<'_>,
        order: JournalOrderKey,
    ) -> Result<()> {
        let state = if part.is_complete {
            ActivityState::Completed
        } else {
            ActivityState::Running
        };
        if matches!(update, PartUpdate::Created { .. }) {
            self.require_undeclared(part.identity.part_id)?;
            let mut bodies = vec![self.prepare_body(
                part.identity.part_id,
                JournalNumber(1),
                JournalContentField::ReasoningSummary,
                part.detail,
            )?];
            let title = self.text_value(
                part.identity.part_id,
                JournalContentField::Title,
                part.title,
                &mut bodies,
            )?;
            return self.declare_visible(
                part.identity.part_id,
                order,
                JournalPartMetadata::Reasoning(JournalReasoningMetadata { title, state }),
                bodies,
            );
        }
        let mut head = self
            .head(part.identity.part_id)?
            .ok_or(PlannerError::MissingPart)?;
        let JournalPartMetadata::Reasoning(mut metadata) = head.metadata.clone() else {
            return Err(PlannerError::Invalid(
                "Reasoning update has a different Part kind".into(),
            ));
        };
        let mut removed = vec![];
        match update {
            PartUpdate::ReasoningDelta {
                title,
                detail_delta,
                ..
            } => {
                if matches!(
                    metadata.state,
                    ActivityState::Completed | ActivityState::Cancelled | ActivityState::Failed
                ) {
                    return Err(PlannerError::Invalid(
                        "append after terminal Reasoning".into(),
                    ));
                }
                if let Some(title) = title {
                    if title != part.title {
                        return Err(PlannerError::Invalid(
                            "Reasoning title disagrees with committed Part".into(),
                        ));
                    }
                    metadata.title = self.updated_text_value(
                        &mut head,
                        JournalContentField::Title,
                        part.title,
                        &mut removed,
                    )?;
                } else if !self.text_value_matches(&head, &metadata.title, part.title)? {
                    return Err(PlannerError::Invalid(
                        "Reasoning title changed without title update".into(),
                    ));
                }
                let body = head
                    .bodies
                    .iter()
                    .find(|body| body.field == JournalContentField::ReasoningSummary)
                    .ok_or(PlannerError::MissingPart)?;
                if body.utf8_bytes.0.checked_add(detail_delta.len() as u64)
                    != Some(part.detail.len() as u64)
                    || !part.detail.ends_with(detail_delta)
                {
                    return Err(PlannerError::Invalid(
                        "Reasoning delta disagrees with committed Part".into(),
                    ));
                }
                self.append_visible_field(
                    &mut head,
                    JournalContentField::ReasoningSummary,
                    detail_delta,
                )?;
            }
            PartUpdate::ReasoningReplaced {
                title,
                detail,
                is_complete,
                ..
            } => {
                if title != part.title || detail != part.detail || *is_complete != part.is_complete
                {
                    return Err(PlannerError::Invalid(
                        "Reasoning replacement disagrees with committed Part".into(),
                    ));
                }
                metadata.title = self.updated_text_value(
                    &mut head,
                    JournalContentField::Title,
                    part.title,
                    &mut removed,
                )?;
                self.replace_visible_field(
                    &mut head,
                    JournalContentField::ReasoningSummary,
                    Some(part.detail),
                )?;
            }
            PartUpdate::Completed { .. } => {
                if !self.text_value_matches(&head, &metadata.title, part.title)? {
                    return Err(PlannerError::Invalid(
                        "Reasoning completion changed its title".into(),
                    ));
                }
            }
            _ => {
                return Err(PlannerError::NotYetSupported(
                    "non-reasoning update on Reasoning",
                ))
            }
        }
        metadata.state = state;
        self.update_visible_metadata(head, JournalPartMetadata::Reasoning(metadata))?;
        let mut head = self
            .head(part.identity.part_id)?
            .ok_or(PlannerError::MissingPart)?;
        for field in removed {
            self.replace_visible_field(&mut head, field, None)?;
        }
        Ok(())
    }

    pub(super) fn text_value(
        &mut self,
        id: &str,
        field: JournalContentField,
        text: &str,
        bodies: &mut Vec<JournalContentRef>,
    ) -> Result<JournalTextValue> {
        if text.len() <= INLINE_TEXT_BYTES {
            return Ok(JournalTextValue::Inline(text.to_owned()));
        }
        bodies.push(self.prepare_body(id, JournalNumber(1), field, text)?);
        Ok(JournalTextValue::Field(field))
    }

    /// Add new dependencies first. Removals must wait until the metadata update
    /// has released its old references, because Domain validates every op.
    pub(super) fn updated_text_value(
        &mut self,
        head: &mut JournalPartHead,
        field: JournalContentField,
        text: &str,
        removed: &mut Vec<JournalContentField>,
    ) -> Result<JournalTextValue> {
        if text.len() <= INLINE_TEXT_BYTES {
            removed.push(field);
            Ok(JournalTextValue::Inline(text.to_owned()))
        } else {
            self.replace_visible_field(head, field, Some(text))?;
            Ok(JournalTextValue::Field(field))
        }
    }

    pub(super) fn updated_optional_text_value(
        &mut self,
        head: &mut JournalPartHead,
        field: JournalContentField,
        text: Option<&str>,
        removed: &mut Vec<JournalContentField>,
    ) -> Result<Option<JournalTextValue>> {
        match text {
            Some(text) => self
                .updated_text_value(head, field, text, removed)
                .map(Some),
            None => {
                removed.push(field);
                Ok(None)
            }
        }
    }

    pub(super) fn text_value_matches(
        &self,
        head: &JournalPartHead,
        value: &JournalTextValue,
        text: &str,
    ) -> Result<bool> {
        match value {
            JournalTextValue::Inline(value) => Ok(value == text),
            JournalTextValue::Field(field) => head
                .bodies
                .iter()
                .find(|body| body.field == *field)
                .map(|body| self.content_matches(&head.part_id, body, text))
                .transpose()
                .map(|matched| matched.unwrap_or(false)),
        }
    }

    pub(super) fn declare_visible(
        &mut self,
        id: &str,
        order_key: JournalOrderKey,
        metadata: JournalPartMetadata,
        bodies: Vec<JournalContentRef>,
    ) -> Result<()> {
        self.require_undeclared(id)?;
        let head = JournalPartHead {
            identity: self.identity.clone(),
            part_id: id.to_owned(),
            generation: JournalNumber(1),
            revision: JournalNumber(1),
            metadata_revision: JournalNumber(1),
            order_key,
            metadata,
            bodies,
        };
        self.operations
            .push(AssistantJournalOperationV1::Declare(DeclareJournalPart {
                part: head.clone(),
            }));
        self.heads.insert(id.to_owned(), Some(head));
        Ok(())
    }

    pub(super) fn update_visible_metadata(
        &mut self,
        mut head: JournalPartHead,
        metadata: JournalPartMetadata,
    ) -> Result<()> {
        if head.metadata == metadata {
            self.heads.insert(head.part_id.clone(), Some(head));
            return Ok(());
        }
        self.operations
            .push(AssistantJournalOperationV1::Update(UpdateJournalMetadata {
                part_id: head.part_id.clone(),
                expected_generation: head.generation,
                expected_revision: head.revision,
                metadata: metadata.clone(),
            }));
        head.revision = increment(head.revision)?;
        head.metadata_revision = increment(head.metadata_revision)?;
        head.metadata = metadata;
        self.heads.insert(head.part_id.clone(), Some(head));
        Ok(())
    }

    pub(super) fn replace_visible_field(
        &mut self,
        head: &mut JournalPartHead,
        field: JournalContentField,
        text: Option<&str>,
    ) -> Result<()> {
        let expected = head.bodies.iter().find(|body| body.field == field).cloned();
        if matches!((&expected, text), (None, None)) {
            return Ok(());
        }
        if let (Some(expected), Some(text)) = (&expected, text) {
            if self.content_matches(&head.part_id, expected, text)? {
                return Ok(());
            }
        }
        let resulting = text
            .map(|text| self.prepare_body(&head.part_id, increment(head.revision)?, field, text))
            .transpose()?;
        self.operations
            .push(AssistantJournalOperationV1::ReplaceContent(
                ReplaceJournalContent {
                    part_id: head.part_id.clone(),
                    expected_generation: head.generation,
                    expected_revision: head.revision,
                    field,
                    expected,
                    resulting: resulting.clone(),
                },
            ));
        head.bodies.retain(|body| body.field != field);
        if let Some(resulting) = resulting {
            head.bodies.push(resulting);
        }
        head.revision = increment(head.revision)?;
        self.heads.insert(head.part_id.clone(), Some(head.clone()));
        Ok(())
    }

    pub(super) fn append_visible_field(
        &mut self,
        head: &mut JournalPartHead,
        field: JournalContentField,
        text: &str,
    ) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        let chunks = chunk_text(text)?;
        for group in chunks.chunks(MAX_APPEND_OPERATION_CHUNKS) {
            let expected = head
                .bodies
                .iter()
                .find(|body| body.field == field)
                .cloned()
                .ok_or(PlannerError::MissingPart)?;
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
            head.bodies.retain(|body| body.field != field);
            head.bodies.push(resulting);
            head.revision = increment(head.revision)?;
        }
        self.supplied_chunks.extend(chunks);
        self.heads.insert(head.part_id.clone(), Some(head.clone()));
        Ok(())
    }
}
