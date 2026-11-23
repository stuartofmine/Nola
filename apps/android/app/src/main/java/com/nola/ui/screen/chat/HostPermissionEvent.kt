package com.nola.ui.screen.chat

import com.nola.data.remote.dto.HOST_PERMISSION_PROTOCOL_VERSION
import com.nola.data.remote.sse.SseEvent
import com.nola.domain.model.HostAgentPermissionRequest
import com.nola.domain.model.ProcessingState

/** Actual remote permission branch: invalid or obsolete stream events cannot replace pending UI. */
internal fun acceptHostPermissionEvent(
    state: ChatUiState,
    event: SseEvent.PermissionRequest,
    streamConversationId: String,
    expectedHostId: String?,
    expectedClientId: String?,
    nowSeconds: Long,
): ChatUiState? {
    if (expectedHostId.isNullOrBlank() || expectedClientId.isNullOrBlank() ||
        state.conversationId != streamConversationId ||
        event.hostId != expectedHostId || event.clientId != expectedClientId ||
        event.sessionId != streamConversationId ||
        event.protocolVersion != HOST_PERMISSION_PROTOCOL_VERSION ||
        event.id.isBlank() || event.nonce.isNullOrBlank() ||
        event.expiresAt == null || event.expiresAt < nowSeconds
    ) return null
    val existingQueue = state.hostPermissionQueue
    // A replayed predecessor ask cannot replace or duplicate the successor carrying its instruction.
    if (existingQueue.any { queued -> queued.permissionInstructions.any { it.permissionRequestId == event.id } }) return null
    val request = HostAgentPermissionRequest(
        allowConversation = event.allowConversation,
        id = event.id, toolName = event.toolName, args = event.args.mapValues { it.value },
        match = event.match, reason = event.reason, permissionInstructions = event.permissionInstructions,
        message = event.message, nonce = event.nonce, hostId = event.hostId,
        safeReviewText = event.safeReviewText,
        clientId = event.clientId, sessionId = event.sessionId,
        taskId = event.taskId,
        expiresAt = event.expiresAt, protocolVersion = event.protocolVersion,
    )
    // SSE recovery may replay an already queued or visible challenge. Preserve
    // the current object and interaction state so Compose does not reopen the
    // prompt or repeat its transition animation.
    if (existingQueue.any { it == request }) return state
    val nextQueue = if (existingQueue.any { it.id == request.id }) {
        existingQueue.map { if (it.id == request.id) request else it }
    } else {
        existingQueue + request
    }
    return state.copy(
        toolEvents = transitionHostToolPermission(state.toolEvents, request, "permission_required"),
        pendingPermissionRequests = nextQueue,
        permissionRequest = nextQueue.firstOrNull(),
        processingState = ProcessingState.Streaming,
        thinkingFinished = true,
    )
}
