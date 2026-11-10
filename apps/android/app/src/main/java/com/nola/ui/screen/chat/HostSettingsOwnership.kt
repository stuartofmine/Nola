package com.nola.ui.screen.chat

import com.nola.llm.provider.WebAgentProvider

/** A capability refresh belongs to a selection AND its frozen connection.
 * No credentials are logged or persisted by this in-memory admission gate. */
internal class HostSettingsOwnership {
    class Request internal constructor(
        internal val sequence: Long,
        val conversationId: String?,
        val selectionGeneration: Long,
        val configuration: WebAgentProvider.Config,
    )

    private var sequence = 0L

    fun begin(conversationId: String?, selectionGeneration: Long, configuration: WebAgentProvider.Config): Request =
        Request(++sequence, conversationId, selectionGeneration, configuration.copy())

    fun accepts(request: Request, conversationId: String?, selectionGeneration: Long,
                configuration: WebAgentProvider.Config?): Boolean =
        request.sequence == sequence && request.conversationId == conversationId &&
            request.selectionGeneration == selectionGeneration && request.configuration == configuration
}
