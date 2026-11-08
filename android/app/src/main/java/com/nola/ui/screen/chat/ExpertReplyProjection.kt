// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import com.nola.domain.model.ChatRole
import com.nola.domain.model.ConversationMessage

/** Read the existing journal presentation without introducing a second reducer. */
internal fun ChatUiState.exactExpertAssistant(contextId: String, runId: String): ConversationMessage? {
    if (conversationId != contextId) return null
    return timelineMessages().singleOrNull {
        it.id == "$runId:assistant" && it.conversationId == contextId && it.role == ChatRole.assistant
    }
}
