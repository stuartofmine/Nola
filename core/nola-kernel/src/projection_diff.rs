// SPDX-License-Identifier: Apache-2.0
//! Pure canonical projection changes; Application owns event identity and persistence.

use nola_domain::{ConversationProjection, ConversationProjectionMutation, MessagePartView};
use std::collections::{BTreeMap, BTreeSet};

/// Computes ordered mutations for two snapshots of the same conversation.
/// Inputs must describe the same projection scope and history window.
pub fn conversation_projection_diff(
    before: Option<&ConversationProjection>,
    after: &ConversationProjection,
) -> Vec<ConversationProjectionMutation> {
    let mut mutations = Vec::new();
    if before.map_or(true, |projection| projection.title != after.title) {
        mutations.push(ConversationProjectionMutation::SetTitle {
            title: after.title.clone(),
            revision: after.revision,
        });
    }
    let previous_messages = before
        .map(|projection| {
            projection
                .messages
                .iter()
                .map(|message| (message.message_id.0.as_str(), message))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    for message in &after.messages {
        match previous_messages
            .get(message.message_id.0.as_str())
            .copied()
        {
            None => mutations.push(ConversationProjectionMutation::UpsertMessage {
                message: message.clone(),
            }),
            Some(previous) if previous == message => {}
            Some(previous) => {
                if let Some(text_mutations) = text_part_mutations(previous, message) {
                    mutations.extend(text_mutations);
                } else {
                    mutations.push(ConversationProjectionMutation::UpsertMessage {
                        message: message.clone(),
                    });
                }
            }
        }
    }
    if let Some(before) = before {
        let current = after
            .messages
            .iter()
            .map(|message| message.message_id.0.as_str())
            .collect::<BTreeSet<_>>();
        for message in &before.messages {
            if !current.contains(message.message_id.0.as_str()) {
                mutations.push(ConversationProjectionMutation::RemoveMessage {
                    message_id: message.message_id.clone(),
                    revision: after.revision,
                });
            }
        }
    }
    let active_run_changed =
        before.map_or(true, |projection| projection.active_run != after.active_run);
    let has_run_identity = after.active_run.is_some()
        || before.is_some_and(|projection| projection.active_run.is_some());
    if active_run_changed && has_run_identity {
        mutations.push(ConversationProjectionMutation::SetActiveRun {
            active_run: after.active_run.clone(),
        });
    }
    if before.map_or(true, |projection| projection.surface != after.surface) {
        mutations.push(ConversationProjectionMutation::ReplaceSurface {
            surface: after.surface.clone(),
        });
    }
    if before.map_or(true, |projection| {
        projection.context_usage != after.context_usage
    }) {
        mutations.push(ConversationProjectionMutation::SetContextUsage {
            context_usage: after.context_usage.clone(),
        });
    }
    mutations
}

/// Emits compact stream mutations only when the message envelope and every
/// non-text Part are unchanged. Any structural/state transition falls back to
/// an authoritative message upsert, while ordinary model deltas remain O(n).
pub fn text_part_mutations(
    before: &nola_domain::MessageView,
    after: &nola_domain::MessageView,
) -> Option<Vec<ConversationProjectionMutation>> {
    // Message revision follows Part revision during a stream. Normalize it so
    // it does not turn an otherwise compact text delta into a full upsert.
    // Compare borrowed envelope fields. Cloning MessageView first also clones
    // all Markdown and tool payloads, even though they are immediately cleared.
    let nola_domain::MessageView {
        message_id,
        role,
        state,
        revision: _,
        created_at_ms,
        parts,
    } = before;
    if message_id != &after.message_id
        || role != &after.role
        || state != &after.state
        || created_at_ms != &after.created_at_ms
        || parts.len() != after.parts.len()
    {
        return None;
    }

    let mut mutations = Vec::new();
    for (previous_part, current_part) in before.parts.iter().zip(&after.parts) {
        if previous_part == current_part {
            continue;
        }
        match (previous_part, current_part) {
            (
                MessagePartView::Text {
                    part_id: previous_part_id,
                    revision: previous_revision,
                    markdown: previous_markdown,
                    is_final: previous_is_final,
                    review: previous_review,
                },
                MessagePartView::Text {
                    part_id,
                    revision,
                    markdown,
                    is_final,
                    review,
                },
            ) if previous_part_id == part_id
                && revision > previous_revision
                && previous_review == review => {
                // Aggregate revisions can advance for unrelated conversation
                // state. Republishing an already-final document makes every
                // client rebuild the same Markdown tree.
                if markdown == previous_markdown && is_final == previous_is_final {
                    continue;
                } else if *is_final {
                    mutations.push(ConversationProjectionMutation::CompleteTextPart {
                        message_id: after.message_id.clone(),
                        part_id: part_id.clone(),
                        revision: *revision,
                        markdown: markdown.clone(),
                    });
                } else if !*previous_is_final {
                    let delta = markdown.strip_prefix(previous_markdown)?;
                    if !delta.is_empty() {
                        mutations.push(ConversationProjectionMutation::AppendTextPreview {
                            message_id: after.message_id.clone(),
                            part_id: part_id.clone(),
                            revision: *revision,
                            delta: delta.to_string(),
                        });
                    }
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    }
    // Text events carry the Part revision, not an independent message revision.
    // Only use them if the consumer can reconstruct the exact envelope; do not
    // silently discard a separately advancing message revision.
    let reconstructed_revision =
        mutations
            .iter()
            .fold(before.revision, |current, mutation| match mutation {
                ConversationProjectionMutation::AppendTextPreview { revision, .. }
                | ConversationProjectionMutation::CompleteTextPart { revision, .. } => {
                    current.max(*revision)
                }
                _ => current,
            });
    if mutations.is_empty() {
        // Only the aggregate revision changed; visible text and lifecycle did
        // not, so the correct projection delta is empty.
        Some(mutations)
    } else {
        (reconstructed_revision == after.revision).then_some(mutations)
    }
}
