// SPDX-License-Identifier: Apache-2.0
//! Canonical Part updates shared by submission and Host-response persistence planners.
use nola_domain::PartMutation;
use nola_domain::{
    CanonicalStreamHead, CanonicalStreamOperation, CanonicalStreamOwner, CanonicalTextState,
};
use nola_agent_engine::agent::parts::{AgentPart, PartUpdate, ToolPartState};
use sha2::{Digest, Sha256};

use crate::public_part_semantics::PublicPart;

/// Validated identity remapping without copying the cumulative Part body.
pub(crate) struct CanonicalPartProjection<'a> {
    part: &'a AgentPart,
    run_id: &'a str,
    message_id: &'a str,
}

impl<'a> CanonicalPartProjection<'a> {
    pub(crate) fn new(
        part: &'a AgentPart,
        run_id: &'a str,
        initial_message_id: Option<&'a str>,
    ) -> Result<Self, String> {
        let message_id = if part.message_id() == format!("{run_id}:assistant") {
            initial_message_id.ok_or_else(|| {
                format!("conversation Run {run_id} has no assistant Message owner")
            })?
        } else {
            part.message_id()
        };
        Ok(Self {
            part,
            run_id,
            message_id,
        })
    }

    pub(crate) fn part(&self) -> &'a AgentPart {
        self.part
    }
    pub(crate) fn message_id(&self) -> &'a str {
        self.message_id
    }
    pub(crate) fn run_id(&self) -> &'a str {
        self.run_id
    }

    pub(crate) fn public_part(&self) -> PublicPart<'a> {
        PublicPart::from_canonical(self)
    }

    fn materialized(&self) -> std::borrow::Cow<'a, AgentPart> {
        if self.message_id == self.part.message_id() {
            std::borrow::Cow::Borrowed(self.part)
        } else {
            std::borrow::Cow::Owned(self.part.clone().with_message_id(self.message_id))
        }
    }

    pub(crate) fn initial_parts(
        &self,
        update: &PartUpdate,
        at: i64,
    ) -> Result<Vec<nola_domain::MessagePartInput>, String> {
        crate::initial_parts_for_transition_message(update, &self.materialized(), at)
    }
}

/// One planner per committed transition; only affected heads are loaded/cached.
#[derive(Default)]
pub(crate) struct CanonicalPartProjectionPlanner {
    states: std::collections::BTreeMap<String, (CanonicalStreamOwner, CanonicalTextState)>,
}

impl CanonicalPartProjectionPlanner {
    fn state(
        &mut self,
        owner: &CanonicalStreamOwner,
        load: &mut impl FnMut(&CanonicalStreamOwner) -> Result<CanonicalTextState, String>,
    ) -> Result<CanonicalTextState, String> {
        if let Some((saved_owner, state)) = self.states.get(&owner.part_id) {
            if saved_owner != owner {
                return Err("canonical Text owner changed within transition".into());
            }
            return Ok(state.clone());
        }
        let state = load(owner)?;
        self.states
            .insert(owner.part_id.clone(), (owner.clone(), state.clone()));
        Ok(state)
    }

    fn emit(
        &mut self,
        owner: &CanonicalStreamOwner,
        current: Option<&CanonicalStreamHead>,
        operation: CanonicalStreamOperation,
        at: i64,
        mutations: &mut Vec<PartMutation>,
    ) -> Result<CanonicalStreamHead, String> {
        let head = nola_domain::apply_canonical_stream(current, &operation)
            .map_err(|error| format!("canonical Text planning failed: {error:?}"))?;
        mutations.push(PartMutation::Stream {
            operation,
            updated_at_ms: at,
        });
        self.states.insert(
            owner.part_id.clone(),
            (owner.clone(), CanonicalTextState::Stream(head.clone())),
        );
        Ok(head)
    }

    fn append(
        &mut self,
        owner: &CanonicalStreamOwner,
        mut head: CanonicalStreamHead,
        text: &str,
        at: i64,
        mutations: &mut Vec<PartMutation>,
    ) -> Result<CanonicalStreamHead, String> {
        let mut remaining = text;
        while !remaining.is_empty() {
            let mut length = remaining
                .len()
                .min(nola_domain::CANONICAL_BODY_CHUNK_MAX_BYTES as usize);
            while !remaining.is_char_boundary(length) {
                length -= 1;
            }
            let operation = CanonicalStreamOperation::Append {
                owner: owner.clone(),
                expected_head: head.clone(),
                delta: remaining[..length].into(),
            };
            head = self.emit(owner, Some(&head), operation, at, mutations)?;
            remaining = &remaining[length..];
        }
        Ok(head)
    }

    pub(crate) fn close_if_stream(
        &mut self,
        owner: &CanonicalStreamOwner,
        at: i64,
        mut load: impl FnMut(&CanonicalStreamOwner) -> Result<CanonicalTextState, String>,
    ) -> Result<Vec<PartMutation>, String> {
        let mut mutations = Vec::new();
        if let CanonicalTextState::Stream(head) = self.state(owner, &mut load)? {
            if !head.closed {
                self.emit(
                    owner,
                    Some(&head),
                    CanonicalStreamOperation::Close {
                        owner: owner.clone(),
                        expected_head: head.clone(),
                    },
                    at,
                    &mut mutations,
                )?;
            }
        }
        Ok(mutations)
    }

    #[cfg(test)]
    pub(crate) fn plan(
        &mut self,
        owner: &CanonicalStreamOwner,
        update: &PartUpdate,
        part: &AgentPart,
        ordinal: i64,
        at: i64,
        load: impl FnMut(&CanonicalStreamOwner) -> Result<CanonicalTextState, String>,
    ) -> Result<Vec<PartMutation>, String> {
        let view = CanonicalPartProjection::new(part, &owner.run_id, Some(&owner.message_id))?;
        self.plan_view(owner, update, &view, ordinal, at, load)
    }

    pub(crate) fn plan_view(
        &mut self,
        owner: &CanonicalStreamOwner,
        update: &PartUpdate,
        view: &CanonicalPartProjection<'_>,
        ordinal: i64,
        at: i64,
        mut load: impl FnMut(&CanonicalStreamOwner) -> Result<CanonicalTextState, String>,
    ) -> Result<Vec<PartMutation>, String> {
        let part = view.part();
        if owner.part_id != part.id()
            || owner.message_id != view.message_id
            || owner.run_id != view.run_id
        {
            return Err("canonical Text projection owner mismatch".into());
        }
        let AgentPart::Text(text) = part else {
            return part_mutations_for_update(update, &view.materialized(), ordinal, at);
        };
        let state = self.state(owner, &mut load)?;
        let missing = matches!(&state, CanonicalTextState::Missing);
        let promote = matches!(&state, CanonicalTextState::Missing)
            || matches!(&state, CanonicalTextState::Existing { kind, state, total_utf8_bytes }
                if kind == "text" && state == "open" && *total_utf8_bytes == 0);
        if !promote && !matches!(&state, CanonicalTextState::Stream(_)) {
            return part_mutations_for_update(update, &view.materialized(), ordinal, at);
        }
        if let PartUpdate::TextReviewChanged {
            part_id,
            state: review,
        } = update
        {
            if part_id != &text.id || text.review != Some(*review) || !text.is_complete {
                return Err("canonical Text review projection mismatch".into());
            }
            let CanonicalTextState::Stream(_) = &state else {
                return Err("canonical Text review has no previously declared stream".into());
            };
        }
        let mut mutations = Vec::new();
        let mut head = if let CanonicalTextState::Stream(head) = state {
            head
        } else {
            let metadata = text_metadata(text, false).with_message_id(view.message_id);
            mutations.push(part_upsert(&metadata, ordinal, at)?);
            self.emit(
                owner,
                None,
                CanonicalStreamOperation::Declare {
                    owner: owner.clone(),
                },
                at,
                &mut mutations,
            )?
        };
        match update {
            PartUpdate::TextDelta { part_id, delta }
            | PartUpdate::TextCommittedDelta { part_id, delta } => {
                if part_id != &text.id {
                    return Err("canonical Text delta Part mismatch".into());
                }
                let prefix = text
                    .text
                    .strip_suffix(delta)
                    .ok_or("canonical Text delta is not projected suffix")?;
                if missing && !prefix.is_empty() {
                    head = self.append(owner, head, prefix, at, &mut mutations)?;
                }
                if prefix.len() as u64 != head.total_utf8_bytes {
                    return Err("canonical Text delta prefix length mismatch".into());
                }
                head = self.append(owner, head, delta, at, &mut mutations)?;
            }
            PartUpdate::TextReplaced {
                part_id,
                text: replacement,
                ..
            } => {
                if part_id != &text.id || replacement != &text.text {
                    return Err("canonical Text replacement mismatch".into());
                }
                if !promote {
                    head = self.emit(
                        owner,
                        Some(&head),
                        CanonicalStreamOperation::Replace {
                            owner: owner.clone(),
                            expected_head: head.clone(),
                            text: String::new(),
                        },
                        at,
                        &mut mutations,
                    )?;
                }
                head = self.append(owner, head, replacement, at, &mut mutations)?;
            }
            PartUpdate::Created { .. } if promote => {
                head = self.append(owner, head, &text.text, at, &mut mutations)?;
            }
            PartUpdate::Completed { part_id } => {
                if part_id != &text.id {
                    return Err("canonical Text completion Part mismatch".into());
                }
                if promote {
                    head = self.append(owner, head, &text.text, at, &mut mutations)?;
                }
            }
            PartUpdate::TextReviewChanged { .. } => {}
            _ => return Err("unsupported update for canonical Text stream".into()),
        }
        if head.total_utf8_bytes != text.text.len() as u64 {
            return Err("canonical Text projected length mismatch".into());
        }
        // Core can complete a Text through creation, replacement, or review
        // without emitting a separate Completed event. Close exactly here;
        // subsequent review changes only update bounded metadata.
        let explicit_completion = matches!(update, PartUpdate::Completed { .. });
        if explicit_completion || (text.is_complete && !head.closed) {
            if !head.closed {
                self.emit(
                    owner,
                    Some(&head),
                    CanonicalStreamOperation::Close {
                        owner: owner.clone(),
                        expected_head: head.clone(),
                    },
                    at,
                    &mut mutations,
                )?;
            }
            mutations.extend(part_mutations_for_update(
                &PartUpdate::Completed {
                    part_id: text.id.clone(),
                },
                &view.materialized(),
                ordinal,
                at,
            )?);
            return Ok(mutations);
        }
        let metadata = text_metadata(text, text.is_complete).with_message_id(view.message_id);
        mutations.push(PartMutation::Metadata {
            part_id: text.id.clone(),
            state: if head.closed { "completed" } else { "open" }.into(),
            content_json: Some(
                serde_json::to_string(&metadata).map_err(|error| error.to_string())?,
            ),
            updated_at_ms: at,
        });
        Ok(mutations)
    }
}

fn text_metadata(
    text: &nola_agent_engine::agent::parts::TextPart,
    is_complete: bool,
) -> AgentPart {
    AgentPart::Text(nola_agent_engine::agent::parts::TextPart {
        id: text.id.clone(),
        ownership: text.ownership.clone(),
        review: text.review.clone(),
        text: String::new(),
        is_complete,
    })
}

#[cfg(test)]
#[path = "canonical_part_semantics_tests.rs"]
mod semantic_tests;

pub(crate) fn part_mutations_for_update(
    update: &PartUpdate,
    part: &AgentPart,
    ordinal: i64,
    now_ms: i64,
) -> Result<Vec<PartMutation>, String> {
    match update {
        PartUpdate::TextDelta { part_id, delta }
        | PartUpdate::TextCommittedDelta { part_id, delta } => {
            if delta.is_empty() {
                return Ok(Vec::new());
            }
            let AgentPart::Text(text_part) = part else {
                return Err(format!("Text delta {part_id} does not project a Text Part"));
            };
            if text_part.id != *part_id {
                return Err(format!(
                    "Text delta Part mismatch: update={part_id}, projection={}",
                    text_part.id
                ));
            }
            let prefix = text_part.text.strip_suffix(delta).ok_or_else(|| {
                format!("Text delta for Part {part_id} is not the projected text suffix")
            })?;
            Ok(vec![PartMutation::AppendText {
                part_id: part_id.clone(),
                expected_content_digest: format!("{:x}", Sha256::digest(prefix.as_bytes())),
                text: delta.clone(),
                updated_at_ms: now_ms,
            }])
        }
        PartUpdate::Completed { part_id } => {
            if part.id() != part_id {
                return Err(format!(
                    "Completed Part mismatch: update={part_id}, projection={}",
                    part.id()
                ));
            }
            let canonical = part_upsert(part, ordinal, now_ms)?;
            let content_text = match &canonical {
                PartMutation::Upsert { content_text, .. } => content_text,
                _ => unreachable!("part_upsert always creates an Upsert mutation"),
            };
            Ok(vec![PartMutation::Close {
                part_id: part_id.clone(),
                state: "completed".to_string(),
                content_json: Some(
                    serde_json::to_string(part)
                        .map_err(|error| format!("encode completed Part failed: {error}"))?,
                ),
                content_digest: Some(format!("{:x}", Sha256::digest(content_text.as_bytes()))),
                updated_at_ms: now_ms,
            }])
        }
        _ => Ok(vec![part_upsert(part, ordinal, now_ms)?]),
    }
}

pub(crate) fn part_upsert(
    part: &AgentPart,
    ordinal: i64,
    now_ms: i64,
) -> Result<PartMutation, String> {
    // A canonical Part is an original, not the diagnostic event preview.
    // Store's immutable JSON field reader applies bounded selection later;
    // changing a structured result here would destroy its only durable source
    // when the Host has not supplied an external full-output reference.
    let (kind, state, content_text) = match part {
        AgentPart::Text(value) => (
            "text",
            if value.is_complete {
                "completed"
            } else {
                "open"
            },
            value.text.clone(),
        ),
        AgentPart::Reasoning(value) => (
            "reasoning",
            if value.is_complete {
                "completed"
            } else {
                "open"
            },
            value.detail.clone(),
        ),
        AgentPart::Tool(value) => (
            "tool",
            match value.state {
                ToolPartState::Pending => "pending",
                ToolPartState::WaitingForPermission => "waiting_for_permission",
                ToolPartState::WaitingForInput => "waiting_input",
                ToolPartState::Running => "running",
                ToolPartState::Completed => "completed",
                ToolPartState::Failed => "failed",
                ToolPartState::Cancelled => "cancelled",
                ToolPartState::Sealed => "sealed",
            },
            persisted_tool_content_text(value),
        ),
        AgentPart::Source(value) => ("source", "completed", value.title.clone()),
        AgentPart::Citation(value) => ("citation", "completed", value.title.clone()),
        AgentPart::Artifact(value) => ("artifact", "completed", value.artifact.name.clone()),
        AgentPart::Step(value) => ("step", "completed", value.title.clone()),
        AgentPart::Compaction(value) => ("compaction", "completed", value.summary.clone()),
    };
    Ok(PartMutation::Upsert {
        part_id: part.id().to_string(),
        kind: kind.to_string(),
        ordinal,
        state: state.to_string(),
        content_text,
        content_json: Some(
            serde_json::to_string(part)
                .map_err(|error| format!("encode canonical Part failed: {error}"))?,
        ),
        content_digest: None,
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
    })
}

fn persisted_tool_content_text(
    value: &nola_agent_engine::agent::parts::ToolPart,
) -> String {
    if let Some(preview) = value
        .output_preview
        .as_deref()
        .filter(|preview| !preview.trim().is_empty())
    {
        return preview.to_string();
    }
    if let Some(summary) = value
        .summary
        .as_deref()
        .filter(|summary| !summary.trim().is_empty())
    {
        return summary.to_string();
    }
    if matches!(value.capability_id.as_str(), "bash" | "bash_command_review") {
        if let Some(command) = value
            .input
            .get("command")
            .or_else(|| value.input.get("cmd"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|command| !command.is_empty())
        {
            return command.chars().take(2_048).collect();
        }
    }
    value.title.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_text_view_borrows_body_and_remaps_only_canonical_identity() {
        let mut part = text(&"x".repeat(8 * 1024 * 1024));
        let view = CanonicalPartProjection::new(&part, "turn", Some("canonical-message")).unwrap();
        let AgentPart::Text(original) = &part else {
            panic!("Text")
        };
        let AgentPart::Text(borrowed) = view.part() else {
            panic!("Text")
        };
        assert!(std::ptr::eq(view.part(), &part));
        assert_eq!(borrowed.text.as_ptr(), original.text.as_ptr());
        assert_eq!(view.message_id(), "canonical-message");
        assert_eq!(borrowed.ownership.message_id, "turn:assistant");
        let PublicPart::Text(public) = view.public_part() else {
            panic!("public Text")
        };
        assert_eq!(public.text.as_ptr(), original.text.as_ptr());
        assert_eq!(public.identity.part_id.as_ptr(), original.id.as_ptr());
        assert_eq!(public.identity.message_id, "canonical-message");
        assert_eq!(
            public.identity.message_id.as_ptr(),
            view.message_id().as_ptr()
        );
        assert_eq!(
            public.identity.turn_id.as_ptr(),
            original.ownership.turn_id.as_ptr()
        );
        assert_eq!(public.review, original.review);
        assert!(CanonicalPartProjection::new(&part, "turn", None).is_err());
        let mut canonical_owner = owner();
        canonical_owner.run_id = "turn".into();
        canonical_owner.message_id = "canonical-message".into();
        let mut wrong_owner = canonical_owner.clone();
        wrong_owner.message_id = "foreign-message".into();
        let update = PartUpdate::Created { part: part.clone() };
        let mut planner = CanonicalPartProjectionPlanner::default();
        assert!(planner
            .plan_view(&wrong_owner, &update, &view, 1, 1, |_| panic!(
                "must reject before read"
            ))
            .is_err());
        let mutations = planner
            .plan_view(&canonical_owner, &update, &view, 1, 1, |_| {
                Ok(CanonicalTextState::Missing)
            })
            .unwrap();
        let Some(PartMutation::Metadata {
            content_json: Some(encoded),
            ..
        }) = mutations.last()
        else {
            panic!("bounded metadata")
        };
        assert!(encoded.len() < 512);
        let AgentPart::Text(metadata) = serde_json::from_str::<AgentPart>(encoded).unwrap() else {
            panic!("Text")
        };
        assert_eq!(metadata.ownership.message_id, "canonical-message");
        assert!(metadata.text.is_empty());
        if let AgentPart::Text(value) = &mut part {
            value.is_complete = true;
        }
        let view = CanonicalPartProjection::new(&part, "turn", Some("canonical-message")).unwrap();
        let mutations = planner
            .plan_view(
                &canonical_owner,
                &PartUpdate::Completed {
                    part_id: "text".into(),
                },
                &view,
                1,
                2,
                |_| panic!("cached"),
            )
            .unwrap();
        let expected = part.clone().with_message_id("canonical-message");
        assert_terminal_mutations(&mutations, &expected);
    }
    use nola_agent_engine::agent::parts::{
        PartOwnership, ReasoningPart, TextPart, ToolPart,
    };

    #[test]
    fn canonical_public_tool_borrows_large_json_and_metadata_without_private_fields() {
        let canonical_message = String::from("canonical-tool-message");
        let part = AgentPart::Tool(ToolPart {
            execution_source: Default::default(),
            id: "tool".into(),
            ownership: PartOwnership::for_turn("turn"),
            call_id: "call".into(),
            capability_id: "read_file".into(),
            title: "工具🙂".repeat(16_000),
            summary: Some(String::new()),
            origin: nola_domain::HostCapabilityOrigin {
                provider_title: Some("提供方🙂".repeat(16_000)),
                ..Default::default()
            },
            state: ToolPartState::Completed,
            input: serde_json::json!({"payload": "输入🙂".repeat(1_024_000)}),
            raw_input: "PRIVATE_RAW_INPUT".into(),
            permission: Some(nola_domain::ToolPermissionDisplay {
                request_id: "request".into(),
                permission_domain: "filesystem".into(),
                rationale: "只读🙂".repeat(16_000),
                decision: Some("approved".into()),
            }),
            output_preview: Some("preview".into()),
            structured_output: Some(serde_json::json!({
                "result": "输出🙂".repeat(256_000),
                "nullable": null
            })),
            full_output_ref: Some("/PRIVATE_OUTPUT_PATH".into()),
            artifacts: vec![nola_domain::ArtifactReference {
                uri: "opaque-output".into(),
                name: "输出🙂".into(),
                mime_type: Some("text/plain".into()),
                operation: Some("create".into()),
            }],
            started_at_ms: Some(0),
            finished_at_ms: Some(u64::MAX),
        });
        let AgentPart::Tool(original) = &part else {
            panic!("Tool")
        };
        let view = CanonicalPartProjection::new(&part, "turn", Some(&canonical_message)).unwrap();
        let PublicPart::Tool(public) = view.public_part() else {
            panic!("public Tool")
        };
        assert_eq!(public.identity.message_id, canonical_message);
        assert_eq!(
            public.identity.message_id.as_ptr(),
            canonical_message.as_ptr()
        );
        assert_eq!(original.ownership.message_id, "turn:assistant");
        assert_eq!(public.identity.part_id.as_ptr(), original.id.as_ptr());
        assert_eq!(
            public.identity.turn_id.as_ptr(),
            original.ownership.turn_id.as_ptr()
        );
        assert!(std::ptr::eq(public.input, &original.input));
        assert!(std::ptr::eq(
            public.structured_output.unwrap(),
            original.structured_output.as_ref().unwrap()
        ));
        assert_eq!(public.title.as_ptr(), original.title.as_ptr());
        assert_eq!(
            public.summary.unwrap().as_ptr(),
            original.summary.as_ref().unwrap().as_ptr()
        );
        assert!(std::ptr::eq(public.origin, &original.origin));
        assert!(std::ptr::eq(
            public.permission.unwrap(),
            original.permission.as_ref().unwrap()
        ));
        assert_eq!(public.artifacts.as_ptr(), original.artifacts.as_ptr());
        assert_eq!(public.summary, Some(""));
        assert_eq!(public.output_preview, Some("preview"));
        assert!(public.has_full_output);
        assert_eq!(public.finished_at_ms, Some(u64::MAX));
    }

    #[test]
    fn successor_public_identity_stays_borrowed_and_foreign_planner_owner_is_rejected() {
        let part = AgentPart::Text(TextPart {
            id: "successor-text".into(),
            ownership: PartOwnership {
                message_id: "successor-message".into(),
                turn_id: "successor-turn".into(),
            },
            text: "后续🙂".into(),
            is_complete: false,
            review: Some(nola_domain::TextReviewState::Pending),
        });
        let AgentPart::Text(original) = &part else {
            panic!("Text")
        };
        // The initial-message remap is neither applied to a successor nor a
        // substitute for the downstream exact-owner gate.
        for initial_message in [Some("initial-canonical-message"), None] {
            let view = CanonicalPartProjection::new(&part, "turn", initial_message).unwrap();
            let PublicPart::Text(public) = view.public_part() else {
                panic!("public Text")
            };
            assert_eq!(public.identity.message_id, "successor-message");
            assert_eq!(
                public.identity.message_id.as_ptr(),
                original.ownership.message_id.as_ptr()
            );
            assert_eq!(
                public.identity.turn_id.as_ptr(),
                original.ownership.turn_id.as_ptr()
            );
            assert_eq!(public.text.as_ptr(), original.text.as_ptr());
            assert_eq!(public.review, Some(nola_domain::TextReviewState::Pending));
            let wrong_owner = CanonicalStreamOwner {
                run_id: "turn".into(),
                message_id: "initial-canonical-message".into(),
                part_id: "successor-text".into(),
                ..owner()
            };
            assert!(CanonicalPartProjectionPlanner::default()
                .plan_view(
                    &wrong_owner,
                    &PartUpdate::Created { part: part.clone() },
                    &view,
                    1,
                    1,
                    |_| panic!("foreign owner must be rejected before read"),
                )
                .is_err());
        }
    }

    fn text(value: &str) -> AgentPart {
        AgentPart::Text(TextPart {
            id: "text".into(),
            ownership: PartOwnership::for_turn("turn"),
            text: value.into(),
            is_complete: false,
            review: None,
        })
    }

    fn owner() -> CanonicalStreamOwner {
        CanonicalStreamOwner {
            conversation_id: "conversation".into(),
            run_id: "run".into(),
            message_id: "turn:assistant".into(),
            part_id: "text".into(),
            field: nola_domain::PartBodyField::ContentText,
        }
    }

    #[test]
    fn stream_planner_loads_once_and_advances_multiple_utf8_deltas_without_body_upsert() {
        let mut planner = CanonicalPartProjectionPlanner::default();
        let calls = std::cell::Cell::new(0);
        let load = |_: &CanonicalStreamOwner| {
            calls.set(calls.get() + 1);
            Ok(CanonicalTextState::Missing)
        };
        let initial = "🙂".repeat(9000);
        let part = text(&initial);
        let first = planner
            .plan(
                &owner(),
                &PartUpdate::Created { part: part.clone() },
                &part,
                1,
                2,
                load,
            )
            .unwrap();
        let payloads: Vec<_> = first
            .iter()
            .filter_map(|mutation| match mutation {
                PartMutation::Stream {
                    operation: CanonicalStreamOperation::Append { delta, .. },
                    ..
                } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(payloads.concat(), initial);
        assert_eq!(payloads.len(), 2);
        assert!(payloads.iter().all(|value| value.len() <= 32768));
        assert!(first.iter().all(|mutation| !matches!(mutation, PartMutation::Upsert {content_text, ..} if !content_text.is_empty())));
        let mut all = initial;
        for delta in ["尾", "🙂last"] {
            all.push_str(delta);
            let updates = planner
                .plan(
                    &owner(),
                    &PartUpdate::TextDelta {
                        part_id: "text".into(),
                        delta: delta.into(),
                    },
                    &text(&all),
                    2,
                    3,
                    |_| {
                        calls.set(calls.get() + 1);
                        Ok(CanonicalTextState::Missing)
                    },
                )
                .unwrap();
            assert!(!updates.iter().any(|mutation| matches!(
                mutation,
                PartMutation::Upsert { .. } | PartMutation::AppendText { .. }
            )));
            for mutation in updates {
                if let PartMutation::Metadata { content_json, .. } = mutation {
                    let metadata: AgentPart =
                        serde_json::from_str(content_json.as_deref().unwrap()).unwrap();
                    assert!(matches!(metadata, AgentPart::Text(value) if value.text.is_empty()));
                }
            }
        }
        assert_eq!(calls.get(), 1);
        let completed = planner
            .plan(
                &owner(),
                &PartUpdate::Completed {
                    part_id: "text".into(),
                },
                &text(&all),
                3,
                4,
                |_| panic!("cached head must be reused"),
            )
            .unwrap();
        assert!(matches!(
            &completed[0],
            PartMutation::Stream {
                operation: CanonicalStreamOperation::Close { .. },
                ..
            }
        ));
        assert!(matches!(&completed[1], PartMutation::Close { .. }));
    }

    #[test]
    fn stream_planner_replaces_generation_and_rejects_cross_owner_overlay() {
        let mut planner = CanonicalPartProjectionPlanner::default();
        let initial = text("old");
        planner
            .plan(
                &owner(),
                &PartUpdate::Created {
                    part: initial.clone(),
                },
                &initial,
                1,
                1,
                |_| {
                    Ok(CanonicalTextState::Existing {
                        kind: "text".into(),
                        state: "open".into(),
                        total_utf8_bytes: 0,
                    })
                },
            )
            .unwrap();
        let replaced = planner
            .plan(
                &owner(),
                &PartUpdate::TextReplaced {
                    part_id: "text".into(),
                    text: "new🙂".into(),
                    is_complete: false,
                },
                &text("new🙂"),
                1,
                2,
                |_| panic!("cached"),
            )
            .unwrap();
        assert!(
            matches!(&replaced[0], PartMutation::Stream {operation:CanonicalStreamOperation::Replace { text, .. },..} if text.is_empty())
        );
        let CanonicalTextState::Stream(head) = &planner.states["text"].1 else {
            panic!("stream")
        };
        assert_eq!(head.generation, 2);
        assert_eq!(head.total_utf8_bytes, "new🙂".len() as u64);
        let mut wrong = owner();
        wrong.run_id = "other".into();
        assert!(planner
            .close_if_stream(&wrong, 3, |_| panic!("must reject cached identity"))
            .is_err());
    }

    #[test]
    fn existing_nonempty_original_remains_legacy_without_forged_declare() {
        let mut planner = CanonicalPartProjectionPlanner::default();
        let result = planner
            .plan(
                &owner(),
                &PartUpdate::TextDelta {
                    part_id: "text".into(),
                    delta: "尾".into(),
                },
                &text("existing尾"),
                1,
                2,
                |_| {
                    Ok(CanonicalTextState::Existing {
                        kind: "text".into(),
                        state: "open".into(),
                        total_utf8_bytes: 8,
                    })
                },
            )
            .unwrap();
        assert!(matches!(&result[..], [PartMutation::AppendText { .. }]));
    }

    #[test]
    fn synthetic_close_is_ordered_once_and_does_not_create_legacy_streams() {
        let mut planner = CanonicalPartProjectionPlanner::default();
        let part = text("initial");
        planner
            .plan(
                &owner(),
                &PartUpdate::Created { part: part.clone() },
                &part,
                1,
                1,
                |_| Ok(CanonicalTextState::Missing),
            )
            .unwrap();
        let closed = planner
            .close_if_stream(&owner(), 2, |_| panic!("cached"))
            .unwrap();
        assert!(matches!(
            &closed[..],
            [PartMutation::Stream {
                operation: CanonicalStreamOperation::Close { .. },
                ..
            }]
        ));
        assert!(planner
            .close_if_stream(&owner(), 3, |_| panic!("cached"))
            .unwrap()
            .is_empty());
        let mut legacy = CanonicalPartProjectionPlanner::default();
        assert!(legacy
            .close_if_stream(&owner(), 3, |_| Ok(CanonicalTextState::Existing {
                kind: "text".into(),
                state: "open".into(),
                total_utf8_bytes: 10,
            }))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn review_closes_open_stream_and_only_updates_metadata_when_already_closed() {
        use nola_agent_engine::agent::parts::TextReviewState;
        for closed in [false, true] {
            for review in [
                TextReviewState::Approved,
                TextReviewState::Rejected,
                TextReviewState::Interrupted,
            ] {
                let mut planner = CanonicalPartProjectionPlanner::default();
                let mut part = text(&"正文🙂".repeat(20_000));
                if let AgentPart::Text(value) = &mut part {
                    value.review = Some(TextReviewState::Pending);
                }
                planner
                    .plan(
                        &owner(),
                        &PartUpdate::Created { part: part.clone() },
                        &part,
                        1,
                        1,
                        |_| Ok(CanonicalTextState::Missing),
                    )
                    .unwrap();
                if closed {
                    planner
                        .close_if_stream(&owner(), 2, |_| panic!("cached"))
                        .unwrap();
                }
                let before = planner.states["text"].1.clone();
                if let AgentPart::Text(value) = &mut part {
                    value.review = Some(review);
                    value.is_complete = true;
                }
                let mutations = planner
                    .plan(
                        &owner(),
                        &PartUpdate::TextReviewChanged {
                            part_id: "text".into(),
                            state: review,
                        },
                        &part,
                        1,
                        3,
                        |_| panic!("cached"),
                    )
                    .unwrap();
                if !closed {
                    assert_terminal_mutations(&mutations, &part);
                    let CanonicalTextState::Stream(head) = &planner.states["text"].1 else {
                        panic!("stream head")
                    };
                    assert!(head.closed);
                    assert!(planner
                        .close_if_stream(&owner(), 4, |_| panic!("cached"))
                        .unwrap()
                        .is_empty());
                    continue;
                }
                let [PartMutation::Metadata {
                    state,
                    content_json,
                    ..
                }] = mutations.as_slice()
                else {
                    panic!("review must not rewrite/close body")
                };
                assert_eq!(state, "completed");
                let encoded = content_json.as_deref().unwrap();
                assert!(encoded.len() < 512);
                let AgentPart::Text(metadata) = serde_json::from_str::<AgentPart>(encoded).unwrap()
                else {
                    panic!("Text metadata")
                };
                assert!(metadata.text.is_empty());
                assert!(metadata.is_complete);
                assert_eq!(metadata.review, Some(review));
                assert_eq!(planner.states["text"].1, before);
            }
        }
    }

    fn assert_terminal_mutations(mutations: &[PartMutation], part: &AgentPart) {
        let [PartMutation::Stream {
            operation: CanonicalStreamOperation::Close { .. },
            ..
        }, PartMutation::Close {
            state,
            content_json,
            content_digest,
            ..
        }] = &mutations[mutations.len() - 2..]
        else {
            panic!("stream close followed by final Part close")
        };
        assert_eq!(state, "completed");
        assert_eq!(
            serde_json::from_str::<AgentPart>(content_json.as_deref().unwrap()).unwrap(),
            *part
        );
        let AgentPart::Text(text) = part else {
            panic!("Text")
        };
        assert_eq!(
            content_digest.as_deref(),
            Some(format!("{:x}", Sha256::digest(text.text.as_bytes())).as_str())
        );
        assert!(!mutations
            .iter()
            .any(|mutation| matches!(mutation, PartMutation::Metadata { .. })));
    }

    #[test]
    fn creation_and_replacement_complete_stream_without_completed_event() {
        for replacement in [false, true] {
            for body in ["", "最终🙂"] {
                let mut planner = CanonicalPartProjectionPlanner::default();
                if replacement {
                    let initial = text("discarded body");
                    planner
                        .plan(
                            &owner(),
                            &PartUpdate::Created {
                                part: initial.clone(),
                            },
                            &initial,
                            1,
                            1,
                            |_| Ok(CanonicalTextState::Missing),
                        )
                        .unwrap();
                }
                let mut part = text(body);
                if let AgentPart::Text(value) = &mut part {
                    value.is_complete = true;
                }
                let update = if replacement {
                    PartUpdate::TextReplaced {
                        part_id: "text".into(),
                        text: body.into(),
                        is_complete: true,
                    }
                } else {
                    PartUpdate::Created { part: part.clone() }
                };
                let mutations = planner
                    .plan(&owner(), &update, &part, 1, 2, |_| {
                        Ok(CanonicalTextState::Missing)
                    })
                    .unwrap();
                assert_terminal_mutations(&mutations, &part);
                let CanonicalTextState::Stream(head) = &planner.states["text"].1 else {
                    panic!("stream")
                };
                assert!(head.closed);
                assert_eq!(head.total_utf8_bytes, body.len() as u64);
                assert!(planner
                    .close_if_stream(&owner(), 3, |_| panic!("cached"))
                    .unwrap()
                    .is_empty());
            }
        }
    }

    #[test]
    fn text_delta_hashes_exact_prefix_and_rejects_wrong_part_or_suffix() {
        for update in [
            PartUpdate::TextDelta {
                part_id: "text".into(),
                delta: "🙂尾".into(),
            },
            PartUpdate::TextCommittedDelta {
                part_id: "text".into(),
                delta: "🙂尾".into(),
            },
        ] {
            let mutations = part_mutations_for_update(&update, &text("前文🙂尾"), 3, 42).unwrap();
            let [PartMutation::AppendText {
                part_id,
                expected_content_digest,
                text: delta,
                updated_at_ms,
            }] = mutations.as_slice()
            else {
                panic!("append mutation")
            };
            assert_eq!(part_id, "text");
            assert_eq!(delta, "🙂尾");
            assert_eq!(
                expected_content_digest,
                &format!("{:x}", Sha256::digest("前文".as_bytes()))
            );
            assert_eq!(*updated_at_ms, 42);
            assert!(part_mutations_for_update(&update, &text("not the suffix"), 3, 42).is_err());
        }
        assert!(part_mutations_for_update(
            &PartUpdate::TextDelta {
                part_id: "other".into(),
                delta: "尾".into()
            },
            &text("前尾"),
            1,
            1
        )
        .is_err());
    }

    #[test]
    fn completed_digest_and_json_describe_the_projected_part() {
        let mut part = text("最终🙂");
        if let AgentPart::Text(value) = &mut part {
            value.is_complete = true;
        }
        let mutations = part_mutations_for_update(
            &PartUpdate::Completed {
                part_id: "text".into(),
            },
            &part,
            1,
            7,
        )
        .unwrap();
        let [PartMutation::Close {
            state,
            content_json,
            content_digest,
            updated_at_ms,
            ..
        }] = mutations.as_slice()
        else {
            panic!("close mutation")
        };
        assert_eq!(state, "completed");
        assert_eq!(*updated_at_ms, 7);
        assert_eq!(
            content_digest.as_deref(),
            Some(format!("{:x}", Sha256::digest("最终🙂".as_bytes())).as_str())
        );
        assert_eq!(
            serde_json::from_str::<AgentPart>(content_json.as_deref().unwrap()).unwrap(),
            part
        );
        assert!(part_mutations_for_update(
            &PartUpdate::Completed {
                part_id: "other".into()
            },
            &part,
            1,
            7
        )
        .is_err());
    }

    #[test]
    fn nontext_creation_preserves_upsert_metadata_and_content() {
        let part = AgentPart::Reasoning(ReasoningPart {
            id: "reason".into(),
            ownership: PartOwnership::for_turn("turn"),
            title: "Analysis".into(),
            detail: "Exact detail🙂".into(),
            is_complete: true,
        });
        let mutations =
            part_mutations_for_update(&PartUpdate::Created { part: part.clone() }, &part, 4, 11)
                .unwrap();
        let [PartMutation::Upsert {
            part_id,
            kind,
            ordinal,
            state,
            content_text,
            content_json,
            content_digest,
            created_at_ms,
            updated_at_ms,
        }] = mutations.as_slice()
        else {
            panic!("upsert mutation")
        };
        assert_eq!(
            (part_id.as_str(), kind.as_str(), *ordinal, state.as_str()),
            ("reason", "reasoning", 4, "completed")
        );
        assert_eq!(content_text, "Exact detail🙂");
        assert_eq!((*created_at_ms, *updated_at_ms), (11, 11));
        assert!(content_digest.is_none());
        assert_eq!(
            serde_json::from_str::<AgentPart>(content_json.as_deref().unwrap()).unwrap(),
            part
        );
        assert!(part_mutations_for_update(
            &PartUpdate::TextDelta {
                part_id: "reason".into(),
                delta: "🙂".into()
            },
            &part,
            4,
            11
        )
        .is_err());
    }
}
