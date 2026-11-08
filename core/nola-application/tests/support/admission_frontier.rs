// SPDX-License-Identifier: Apache-2.0
//! Caller-owned, bounded admission read for existing-owner test scenarios.
//! The caller freezes this value before constructing a command; Begin never
//! fills or refreshes the caller's preconditions.
pub fn read(store: &nola_store::NolaStore, conversation: &str) -> (Option<i64>, Option<String>) {
    match store
        .load_conversation_message_window(conversation, None, 1, 1)
        .unwrap()
    {
        Some(window) => (
            Some(window.conversation.configuration_revision),
            window
                .conversation
                .messages
                .last()
                .map(|message| message.id.clone()),
        ),
        None => (None, None),
    }
}
