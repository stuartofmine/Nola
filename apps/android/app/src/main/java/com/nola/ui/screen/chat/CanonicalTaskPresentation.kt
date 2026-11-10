package com.nola.ui.screen.chat

import com.nola.core.AgentCoreTaskHandle
import com.nola.domain.projection.ConversationProjectionV1

/** Task placement comes from the canonical publication, including after cold replay.
 * The task handle supplies current state; it cannot attach itself to another message. */
internal fun canonicalTaskHandles(
    coreSessionId: String?,
    parts: List<ConversationProjectionV1.Part>,
    handles: Map<String, AgentCoreTaskHandle>,
): List<AgentCoreTaskHandle> = parts.filterIsInstance<ConversationProjectionV1.Part.Task>()
    .mapNotNull { handles[it.taskId]?.takeIf { task -> task.ownerSessionId == coreSessionId } }
    .distinctBy { it.id }
