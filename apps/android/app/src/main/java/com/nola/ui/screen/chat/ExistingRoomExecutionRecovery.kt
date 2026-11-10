// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

/** Existing Core executions recover through the ordinary conversation owner;
 * they must never be admitted again as a new room submission. */
internal suspend fun restoreExistingRoomExecution(
    executionId: String?,
    contextConversationId: String,
    executionExists: suspend (String) -> Boolean,
    loadConversationOwner: (String) -> Unit,
): Boolean {
    if (executionId == null || !executionExists(executionId)) return false
    loadConversationOwner(contextConversationId)
    return true
}
