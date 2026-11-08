// SPDX-License-Identifier: Apache-2.0
//! Changed-item Task, Skill and failed-Attempt adapters. The caller supplies
//! independently proven ownership and a permanent Store-reserved order.

use super::*;
use crate::assistant_synthetic_semantics::{
    PublicModelAttemptPart, PublicSkillPart, PublicTaskPart,
};

impl<R: JournalPlanningReader + ?Sized> AssistantJournalPlanner<'_, R> {
    pub(crate) fn task_binding_update(&mut self, task_id: &str, state: nola_domain::ActivityState, available: bool) -> Result<()> {
        let Some(head) = self.head(&format!("task:{task_id}"))? else { return Ok(()) };
        let JournalPartMetadata::TaskControl(mut metadata) = head.metadata.clone() else {
            return Err(PlannerError::OwnerMismatch);
        };
        if metadata.task.task_id != task_id { return Err(PlannerError::OwnerMismatch) }
        metadata.task.state = state;
        metadata.supports_guidance = available;
        self.update_visible_metadata(head, JournalPartMetadata::TaskControl(metadata))
    }

    pub(crate) fn task_update(
        &mut self,
        part: &PublicTaskPart<'_>,
        order: JournalOrderKey,
    ) -> Result<()> {
        let id = part.part_id();
        let metadata = JournalPartMetadata::TaskControl(JournalTaskControlMetadata { task: JournalTaskMetadata {
            task_id: part.task_id.into(),
            display_key: part.display_key.into(),
            state: part.state.clone(),
            progress: part
                .progress
                .map(JournalProgress::from_fraction)
                .transpose()?,
            cancel_requested: part.cancel_requested,
        }, supports_guidance: part.supports_guidance, supports_cancellation: true });
        match self.head(&id)? {
            None => {
                let mut bodies = Vec::new();
                for (field, text) in [
                    (JournalContentField::TaskCurrentStep, part.current_step),
                    (JournalContentField::ResultReference, part.result_ref),
                ] {
                    if let Some(text) = text {
                        bodies.push(self.prepare_body(&id, JournalNumber(1), field, text)?);
                    }
                }
                self.declare_visible(&id, order, metadata, bodies)
            }
            Some(mut head) => {
                let previous_task_id = match &head.metadata {
                    JournalPartMetadata::Task(previous) => Some(previous.task_id.as_str()),
                    JournalPartMetadata::TaskControl(previous) => Some(previous.task.task_id.as_str()),
                    _ => None,
                };
                if previous_task_id != Some(part.task_id) {
                    return Err(PlannerError::OwnerMismatch);
                }
                if matches!(head.metadata, JournalPartMetadata::Task(_)) {
                    let previous_generation = head.generation;
                    let previous_revision = head.revision;
                    head.generation = increment(head.generation)?;
                    head.revision = increment(head.revision)?;
                    head.metadata_revision = JournalNumber(1);
                    head.metadata = metadata.clone();
                    self.operations.push(AssistantJournalOperationV1::Replace(ReplaceJournalPart {
                        part_id: head.part_id.clone(), expected_generation: previous_generation,
                        expected_revision: previous_revision, part: head.clone(),
                    }));
                    self.heads.insert(head.part_id.clone(), Some(head.clone()));
                }
                self.replace_visible_field(
                    &mut head,
                    JournalContentField::TaskCurrentStep,
                    part.current_step,
                )?;
                self.replace_visible_field(
                    &mut head,
                    JournalContentField::ResultReference,
                    part.result_ref,
                )?;
                self.update_visible_metadata(head, metadata)
            }
        }
    }

    pub(crate) fn skill_update(
        &mut self,
        part: &PublicSkillPart<'_>,
        order: JournalOrderKey,
    ) -> Result<()> {
        let id = part.part_id();
        let mode = match part.mode {
            "inline" => JournalSkillMode::Inline,
            "fork" => JournalSkillMode::Fork,
            _ => return Err(PlannerError::Invalid("invalid public Skill mode".into())),
        };
        let metadata = |display_name| {
            JournalPartMetadata::Skill(JournalSkillMetadata {
                run_id: part.run_id.into(),
                skill_id: part.skill_id.into(),
                display_name,
                mode,
                state: part.state.clone(),
                model_turns: JournalNumber(u64::from(part.model_turns)),
                tool_calls: JournalNumber(u64::from(part.tool_calls)),
                elapsed_millis: JournalNumber(part.elapsed_millis),
            })
        };
        match self.head(&id)? {
            None => {
                let mut bodies = Vec::new();
                let display_name = self.text_value(
                    &id,
                    JournalContentField::Title,
                    part.display_name,
                    &mut bodies,
                )?;
                for (field, text) in [
                    (JournalContentField::ResultReference, part.result_ref),
                    (JournalContentField::FailureMessage, part.failure),
                ] {
                    if let Some(text) = text {
                        bodies.push(self.prepare_body(&id, JournalNumber(1), field, text)?);
                    }
                }
                self.declare_visible(&id, order, metadata(display_name), bodies)
            }
            Some(mut head) => {
                if !matches!(&head.metadata,
                    JournalPartMetadata::Skill(previous)
                        if previous.run_id == part.run_id && previous.skill_id == part.skill_id)
                {
                    return Err(PlannerError::OwnerMismatch);
                }
                let mut removed = Vec::new();
                let display_name = self.updated_text_value(
                    &mut head,
                    JournalContentField::Title,
                    part.display_name,
                    &mut removed,
                )?;
                self.replace_visible_field(
                    &mut head,
                    JournalContentField::ResultReference,
                    part.result_ref,
                )?;
                self.replace_visible_field(
                    &mut head,
                    JournalContentField::FailureMessage,
                    part.failure,
                )?;
                // Release metadata references before deleting their fields.
                self.update_visible_metadata(head, metadata(display_name))?;
                let mut head = self.head(&id)?.ok_or(PlannerError::MissingPart)?;
                for field in removed {
                    self.replace_visible_field(&mut head, field, None)?;
                }
                Ok(())
            }
        }
    }

    pub(crate) fn model_attempt_update(
        &mut self,
        part: &PublicModelAttemptPart<'_>,
        order: JournalOrderKey,
    ) -> Result<()> {
        let id = part.part_id();
        let metadata = JournalPartMetadata::ModelAttempt(JournalModelAttemptMetadata {
            attempt_id: part.attempt_id.into(),
            state: part.state.clone(),
            provider: part.provider.map(str::to_owned),
            model: part.model.map(str::to_owned),
            failure_code: part.failure.code.clone(),
            retryable: part.failure.retryable,
            http_status: part
                .failure
                .http_status
                .map(|value| JournalNumber(u64::from(value))),
            fallback_attempt_id: part.fallback_attempt_id.map(str::to_owned),
            fallback_provider: part.fallback_provider.map(str::to_owned),
            fallback_model: part.fallback_model.map(str::to_owned),
        });
        match self.head(&id)? {
            None => {
                let failure = self.prepare_body(
                    &id,
                    JournalNumber(1),
                    JournalContentField::FailureMessage,
                    &part.failure.message,
                )?;
                self.declare_visible(&id, order, metadata, vec![failure])
            }
            Some(mut head) => {
                if !matches!(&head.metadata,
                    JournalPartMetadata::ModelAttempt(previous)
                        if previous.attempt_id == part.attempt_id)
                {
                    return Err(PlannerError::OwnerMismatch);
                }
                self.replace_visible_field(
                    &mut head,
                    JournalContentField::FailureMessage,
                    Some(&part.failure.message),
                )?;
                self.update_visible_metadata(head, metadata)
            }
        }
    }

    pub(crate) fn remove_model_attempt(&mut self, attempt_id: &str) -> Result<()> {
        let id = format!("model-attempt:{attempt_id}");
        let head = match self.head(&id) {
            Ok(Some(head)) => head,
            Ok(None) | Err(PlannerError::RetiredPart) => return Ok(()),
            Err(error) => return Err(error),
        };
        if !matches!(&head.metadata,
            JournalPartMetadata::ModelAttempt(previous) if previous.attempt_id == attempt_id)
        {
            return Err(PlannerError::OwnerMismatch);
        }
        self.operations
            .push(AssistantJournalOperationV1::Remove(RemoveJournalPart {
                part_id: id.clone(),
                expected_generation: head.generation,
                expected_revision: head.revision,
            }));
        self.heads.insert(id, None);
        self.retired.insert(format!("model-attempt:{attempt_id}"));
        Ok(())
    }
}
