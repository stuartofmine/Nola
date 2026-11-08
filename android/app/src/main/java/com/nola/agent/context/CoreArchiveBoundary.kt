package com.nola.agent.context

import com.nola.core.AgentCoreContextArchiveRequest

/**
 * Resolves Core's archive boundary without allowing the Android host to infer
 * or widen it. Both representations must identify the same ordered messages.
 */
internal fun resolveCoreArchiveIndices(
    activeMessageIds: List<String>,
    request: AgentCoreContextArchiveRequest?,
): List<Int>? {
    if (request == null || request.messageIndices.isEmpty()) return null
    if (request.messageIndices.size != request.messageIds.size) return null
    if (request.messageIndices.distinct().size != request.messageIndices.size) return null
    if (request.messageIds.distinct().size != request.messageIds.size) return null
    if (request.messageIndices != request.messageIndices.sorted()) return null

    val resolvedIds = request.messageIndices.map { index ->
        activeMessageIds.getOrNull(index) ?: return null
    }
    return request.messageIndices.takeIf { resolvedIds == request.messageIds }
}
