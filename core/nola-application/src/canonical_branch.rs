// SPDX-License-Identifier: Apache-2.0
//! Bootstrap model context from canonical history without a cloned checkpoint.
use super::*;
use nola_store_port::conversation_history::CanonicalHistoryRepository;

pub(crate) fn restore_canonical_context<S: CanonicalHistoryRepository + ?Sized>(
    store: &S,
    conversation_id: &str,
    core: &mut AgentCore,
    prompt_token_limit: usize,
) -> Result<(), String> {
    restore_canonical_context_before(store, conversation_id, core, prompt_token_limit, None)
}

/// A revert restores a bounded prefix ending at its real canonical boundary.
/// This is the same selection/adapter path used by ordinary turn bootstrap.
pub(crate) fn restore_canonical_context_before<S: CanonicalHistoryRepository + ?Sized>(
    store: &S,
    conversation_id: &str,
    core: &mut AgentCore,
    prompt_token_limit: usize,
    before_ordinal: Option<i64>,
) -> Result<(), String> {
    // Budget actual selected UTF-8 bytes conservatively; the runtime estimates
    // tokens for exactly these items. Original storage length is separate fact.
    let budget = (prompt_token_limit / 4).clamp(128, 32768);
    let Some(window) = store
        .load_conversation_message_window(conversation_id, before_ordinal, 32, budget)
        .map_err(|error| format!("load canonical context window failed: {error}"))?
    else {
        return Ok(());
    };
    if window.conversation.messages.is_empty() {
        return Ok(());
    }
    let (mut items, mut ownership, _) =
        durable_conversation_items(store, &window.conversation, None)?;
    if window.has_more || !window.deferred_text_parts.is_empty() {
        let mut notice = format!("Canonical history context selection for conversation {conversation_id}. Included {} of {} messages. This is a bounded context excerpt, not the complete transcript. Omitted originals remain available through conversation recall and original UTF-8 range reads. ",window.conversation.messages.len(),window.total_message_count);
        for part in window.deferred_text_parts.iter().take(8) {
            notice.push_str(&format!(
                "Message {}, Part {}: selected original bytes 0..{} of {}. ",
                part.message_id, part.part_id, part.preview_utf8_bytes, part.total_utf8_bytes
            ));
        }
        if window.deferred_text_parts.len() > 8 {
            notice.push_str(&format!(
                "{} additional Parts are also excerpted.",
                window.deferred_text_parts.len() - 8
            ));
        }
        let owner = window
            .conversation
            .messages
            .first()
            .expect("nonempty canonical window");
        let item_id = crate::historical_part_projection::context_item_id(
            conversation_id,
            &owner.id,
            None,
            "selection-notice",
        );
        ownership.insert(
            item_id.clone(),
            AgentItemOwnership {
                source: Default::default(),
                turn_id: format!("{conversation_id}:canonical-context"),
                message_id: owner.id.clone(),
            },
        );
        items.insert(
            0,
            AgentItem::Message {
                id: ItemId::from_string(item_id),
                role: AgentRole::System,
                content: notice,
            },
        );
    }
    crate::historical_part_projection::bound_context(
        conversation_id,
        &mut items,
        &mut ownership,
        32768,
    )?;
    // Rebase has no time-dependent Host Effect; use a stable logical time.
    core.submit_at(
        AgentSubmission::RebaseConversation {
            items,
            item_ownership: ownership,
        },
        0,
    )
    .map_err(|error| format!("restore canonical context failed: {error}"))?;
    Ok(())
}
