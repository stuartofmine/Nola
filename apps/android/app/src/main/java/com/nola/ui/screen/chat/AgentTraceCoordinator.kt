package com.nola.ui.screen.chat

import com.nola.agent.trace.AgentTraceEvent
import com.nola.agent.trace.AgentTraceProjection
import com.nola.agent.trace.AgentTraceSession
import com.nola.data.repository.AgentTraceRepository
import java.util.UUID

internal data class AgentTraceBinding(
    val session: AgentTraceSession,
    val turnId: String,
)

/**
 * Owns the active trace session and durable event reduction for one assistant turn.
 *
 * Input: conversation/message identity plus trace events.
 * Output: a stable turn identity and an updated projection when an event was applied.
 */
internal class AgentTraceCoordinator(
    private val repository: AgentTraceRepository,
) {
    private var session: AgentTraceSession? = null
    private var turnId: String? = null

    fun ensure(
        conversationId: String,
        activeTurnId: String?,
        messageId: String?,
    ): AgentTraceBinding {
        session?.let { active ->
            return AgentTraceBinding(active, checkNotNull(turnId))
        }
        val resolvedTurnId = activeTurnId ?: "android-turn-${UUID.randomUUID()}"
        val created = AgentTraceSession(
            traceId = "android-${UUID.randomUUID()}",
            runId = "android-run-${UUID.randomUUID()}",
            conversationId = conversationId,
            messageId = messageId,
            turnId = resolvedTurnId,
        )
        session = created
        turnId = resolvedTurnId
        return AgentTraceBinding(created, resolvedTurnId)
    }

    suspend fun accept(
        event: AgentTraceEvent,
        activeTurnId: String?,
        messageId: String?,
    ): AgentTraceProjection? {
        val active = ensure(event.conversationId, activeTurnId, messageId).session
        val reduction = active.accept(event)
        repository.append(event)
        return reduction.projection.takeIf { reduction.applied }
    }

    fun permissionResolved(requestId: String, toolName: String, allow: Boolean): AgentTraceEvent? =
        session?.permissionResolved(requestId, toolName, allow)

    fun reset() {
        session = null
        turnId = null
    }
}
