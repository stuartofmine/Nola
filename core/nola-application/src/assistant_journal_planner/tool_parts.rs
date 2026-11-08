// SPDX-License-Identifier: Apache-2.0
//! Independent Tool facts. Progress never serializes cumulative input/result;
//! provenance and permission are read-only display, not execution authority.
use super::*;
use crate::public_part_semantics::PublicToolPart;
use nola_domain::assistant_journal_content::{
    encode_assistant_artifact_content, encode_assistant_json_content,
    encode_assistant_resource_content,
};
use nola_domain::{ActivityState, MessageId, MessagePartId, ToolCallId, ToolOutputReference};
use nola_agent_engine::agent::parts::ToolPartState;

fn activity(state: ToolPartState) -> ActivityState {
    match state {
        ToolPartState::Pending => ActivityState::Pending,
        ToolPartState::WaitingForPermission => ActivityState::WaitingForPermission,
        ToolPartState::WaitingForInput => ActivityState::WaitingForInput,
        ToolPartState::Running => ActivityState::Running,
        ToolPartState::Completed => ActivityState::Completed,
        ToolPartState::Failed => ActivityState::Failed,
        ToolPartState::Cancelled => ActivityState::Cancelled,
        ToolPartState::Sealed => ActivityState::Sealed,
    }
}
fn structured(value: &serde_json::Value) -> Result<String> {
    encode_assistant_json_content(value).map_err(PlannerError::Invalid)
}

impl<R: JournalPlanningReader + ?Sized> AssistantJournalPlanner<'_, R> {
    pub(super) fn plan_tool(
        &mut self,
        update: &PartUpdate,
        tool: PublicToolPart<'_>,
        order: JournalOrderKey,
    ) -> Result<()> {
        let id = tool.identity.part_id;
        if matches!(update, PartUpdate::Created { .. }) {
            let mut bodies = vec![self.prepare_body(
                id,
                JournalNumber(1),
                JournalContentField::ToolInput,
                &structured(tool.input)?,
            )?];
            if let Some(value) = tool.structured_output {
                bodies.push(self.prepare_body(
                    id,
                    JournalNumber(1),
                    JournalContentField::ToolOutput,
                    &structured(value)?,
                )?);
            }
            if let Some(value) = tool.output_preview {
                bodies.push(self.prepare_body(
                    id,
                    JournalNumber(1),
                    JournalContentField::ToolOutputText,
                    value,
                )?);
            }
            bodies.push(
                self.prepare_body(
                    id,
                    JournalNumber(1),
                    JournalContentField::ToolArtifacts,
                    &encode_assistant_artifact_content(tool.artifacts)
                        .map_err(PlannerError::Invalid)?,
                )?,
            );
            let title = self.text_value(id, JournalContentField::Title, tool.title, &mut bodies)?;
            let summary = tool
                .summary
                .map(|value| {
                    self.text_value(id, JournalContentField::ToolSummary, value, &mut bodies)
                })
                .transpose()?;
            let origin = JournalToolOrigin {
                kind: tool.origin.kind,
                provider_id: tool
                    .origin
                    .provider_id
                    .as_deref()
                    .map(|value| {
                        self.text_value(
                            id,
                            JournalContentField::OriginProviderId,
                            value,
                            &mut bodies,
                        )
                    })
                    .transpose()?,
                provider_title: tool
                    .origin
                    .provider_title
                    .as_deref()
                    .map(|value| {
                        self.text_value(
                            id,
                            JournalContentField::OriginProviderTitle,
                            value,
                            &mut bodies,
                        )
                    })
                    .transpose()?,
                namespace: tool
                    .origin
                    .namespace
                    .as_deref()
                    .map(|value| {
                        self.text_value(
                            id,
                            JournalContentField::OriginNamespace,
                            value,
                            &mut bodies,
                        )
                    })
                    .transpose()?,
            };
            let permission = tool
                .permission
                .map(|permission| {
                    Ok::<_, PlannerError>(JournalToolPermission {
                        request_id: permission.request_id.clone(),
                        permission_domain: self.text_value(
                            id,
                            JournalContentField::PermissionDomain,
                            &permission.permission_domain,
                            &mut bodies,
                        )?,
                        rationale: self.text_value(
                            id,
                            JournalContentField::PermissionRationale,
                            &permission.rationale,
                            &mut bodies,
                        )?,
                        decision: permission
                            .decision
                            .as_deref()
                            .map(|value| {
                                self.text_value(
                                    id,
                                    JournalContentField::PermissionDecision,
                                    value,
                                    &mut bodies,
                                )
                            })
                            .transpose()?,
                    })
                })
                .transpose()?;
            let resources = tool_resources(&tool);
            let affected_resources = if inline_resources(&resources) {
                resources.into()
            } else {
                bodies.push(
                    self.prepare_body(
                        id,
                        JournalNumber(1),
                        JournalContentField::ToolResources,
                        &encode_assistant_resource_content(&resources)
                            .map_err(PlannerError::Invalid)?,
                    )?,
                );
                JournalResourceList::Field(JournalContentField::ToolResources)
            };
            return self.declare_visible(
                id,
                order,
                JournalPartMetadata::Tool(JournalToolMetadata {
                    execution_source: tool.execution_source.clone(),
                    tool_call_id: tool.call_id.to_owned(),
                    capability_id: tool.capability_id.to_owned(),
                    title,
                    summary,
                    origin,
                    permission,
                    state: activity(tool.state),
                    display_key: None,
                    affected_resources,
                    artifacts: JournalContentField::ToolArtifacts,
                    full_output: self.public_tool_output(&tool),
                    started_at_ms: tool.started_at_ms.map(JournalNumber),
                    finished_at_ms: tool.finished_at_ms.map(JournalNumber),
                }),
                bodies,
            );
        }
        let mut head = self.head(id)?.ok_or(PlannerError::MissingPart)?;
        let JournalPartMetadata::Tool(mut metadata) = head.metadata.clone() else {
            return Err(PlannerError::Invalid(
                "Tool update has a different Part kind".into(),
            ));
        };
        if metadata.tool_call_id != tool.call_id || metadata.capability_id != tool.capability_id {
            return Err(PlannerError::OwnerMismatch);
        }
        let mut removed = vec![];
        match update {
            PartUpdate::ToolInputDelta { .. } => return Ok(()),
            PartUpdate::ToolInputResolved { .. } | PartUpdate::ToolInputReplaced { .. } => {
                // Core can bound deeply nested presentation input. The accepted
                // canonical Part, not the raw event's input, is the public fact.
                self.replace_visible_field(
                    &mut head,
                    JournalContentField::ToolInput,
                    Some(&structured(tool.input)?),
                )?;
            }
            PartUpdate::ToolProgress {
                title,
                detail,
                output_delta,
                ..
            } => {
                if let Some(title) = title.as_deref().filter(|title| !title.is_empty()) {
                    if title != tool.title {
                        return Err(PlannerError::Invalid(
                            "Tool progress title disagrees with committed Part".into(),
                        ));
                    }
                    metadata.title = self.updated_text_value(
                        &mut head,
                        JournalContentField::Title,
                        tool.title,
                        &mut removed,
                    )?;
                }
                if let Some(delta) = output_delta.as_deref().filter(|delta| !delta.is_empty()) {
                    let old = head
                        .bodies
                        .iter()
                        .find(|body| body.field == JournalContentField::ToolOutputText);
                    let can_append = old.zip(tool.output_preview).is_some_and(|(old, preview)| {
                        old.utf8_bytes.0.checked_add(delta.len() as u64)
                            == Some(preview.len() as u64)
                            && preview.ends_with(delta)
                    });
                    if can_append {
                        self.append_visible_field(
                            &mut head,
                            JournalContentField::ToolOutputText,
                            delta,
                        )?;
                    } else {
                        self.replace_visible_field(
                            &mut head,
                            JournalContentField::ToolOutputText,
                            tool.output_preview,
                        )?;
                    }
                } else if let Some(detail) = detail.as_deref().filter(|detail| !detail.is_empty()) {
                    // ToolProgress.detail replaces the technical preview, not
                    // the human summary. Nonempty output_delta has precedence.
                    if tool.output_preview != Some(detail) {
                        return Err(PlannerError::Invalid(
                            "Tool progress preview disagrees with committed Part".into(),
                        ));
                    }
                    self.replace_visible_field(
                        &mut head,
                        JournalContentField::ToolOutputText,
                        Some(detail),
                    )?;
                }
            }
            PartUpdate::ToolPermission { permission, .. } => {
                if tool.permission != Some(permission) {
                    return Err(PlannerError::Invalid(
                        "Tool permission disagrees with committed Part".into(),
                    ));
                }
                metadata.permission =
                    Some(self.updated_tool_permission(&mut head, permission, &mut removed)?);
            }
            PartUpdate::ToolState { .. } => {
                metadata.title = self.updated_text_value(
                    &mut head,
                    JournalContentField::Title,
                    tool.title,
                    &mut removed,
                )?;
                metadata.summary = self.updated_optional_text_value(
                    &mut head,
                    JournalContentField::ToolSummary,
                    tool.summary,
                    &mut removed,
                )?;
                metadata.permission = if let Some(permission) = tool.permission {
                    Some(self.updated_tool_permission(&mut head, permission, &mut removed)?)
                } else {
                    removed.extend([
                        JournalContentField::PermissionDomain,
                        JournalContentField::PermissionRationale,
                        JournalContentField::PermissionDecision,
                    ]);
                    None
                };
                self.replace_visible_field(
                    &mut head,
                    JournalContentField::ToolOutput,
                    tool.structured_output
                        .map(structured)
                        .transpose()?
                        .as_deref(),
                )?;
                self.replace_visible_field(
                    &mut head,
                    JournalContentField::ToolOutputText,
                    tool.output_preview,
                )?;
                self.replace_visible_field(
                    &mut head,
                    JournalContentField::ToolArtifacts,
                    Some(
                        &encode_assistant_artifact_content(tool.artifacts)
                            .map_err(PlannerError::Invalid)?,
                    ),
                )?;
                let resources = tool_resources(&tool);
                metadata.affected_resources = if inline_resources(&resources) {
                    removed.push(JournalContentField::ToolResources);
                    resources.into()
                } else {
                    self.replace_visible_field(
                        &mut head,
                        JournalContentField::ToolResources,
                        Some(
                            &encode_assistant_resource_content(&resources)
                                .map_err(PlannerError::Invalid)?,
                        ),
                    )?;
                    JournalResourceList::Field(JournalContentField::ToolResources)
                };
                metadata.full_output = self.public_tool_output(&tool);
            }
            PartUpdate::Completed { .. } => {}
            _ => return Err(PlannerError::NotYetSupported("non-tool update on Tool")),
        }
        metadata.state = activity(tool.state);
        metadata.started_at_ms = tool.started_at_ms.map(JournalNumber);
        metadata.finished_at_ms = tool.finished_at_ms.map(JournalNumber);
        self.update_visible_metadata(head, JournalPartMetadata::Tool(metadata))?;
        let mut head = self.head(id)?.ok_or(PlannerError::MissingPart)?;
        removed.sort();
        removed.dedup();
        for field in removed {
            self.replace_visible_field(&mut head, field, None)?;
        }
        Ok(())
    }

    fn updated_tool_permission(
        &mut self,
        head: &mut JournalPartHead,
        permission: &nola_domain::ToolPermissionDisplay,
        removed: &mut Vec<JournalContentField>,
    ) -> Result<JournalToolPermission> {
        Ok(JournalToolPermission {
            request_id: permission.request_id.clone(),
            permission_domain: self.updated_text_value(
                head,
                JournalContentField::PermissionDomain,
                &permission.permission_domain,
                removed,
            )?,
            rationale: self.updated_text_value(
                head,
                JournalContentField::PermissionRationale,
                &permission.rationale,
                removed,
            )?,
            decision: self.updated_optional_text_value(
                head,
                JournalContentField::PermissionDecision,
                permission.decision.as_deref(),
                removed,
            )?,
        })
    }
    fn public_tool_output(&self, tool: &PublicToolPart<'_>) -> Option<ToolOutputReference> {
        tool.has_full_output.then(|| ToolOutputReference {
            message_id: MessageId(self.identity.assistant_message_id.clone()),
            part_id: MessagePartId(tool.identity.part_id.to_owned()),
            tool_call_id: ToolCallId(tool.call_id.to_owned()),
        })
    }
}

// Derived legacy conveniences; names and optional MIME/operation remain in artifacts.
fn tool_resources(tool: &PublicToolPart<'_>) -> Vec<JournalAffectedResource> {
    tool.artifacts
        .iter()
        .map(|artifact| JournalAffectedResource {
            resource_type: artifact.mime_type.clone().unwrap_or_else(|| "file".into()),
            uri: artifact.uri.clone(),
            operation: artifact.operation.clone().unwrap_or_else(|| "read".into()),
        })
        .collect()
}
fn inline_resources(resources: &[JournalAffectedResource]) -> bool {
    resources.len() <= 8
        && resources
            .iter()
            .map(|resource| {
                resource.resource_type.len() + resource.uri.len() + resource.operation.len()
            })
            .sum::<usize>()
            <= 1024
}
