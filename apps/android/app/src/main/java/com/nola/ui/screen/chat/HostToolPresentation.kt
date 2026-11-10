// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Pure Host tool result reduction and bounded capability activity presentation.

package com.nola.ui.screen.chat

import com.nola.core.AgentCoreCapabilityActivationEnvelope
import com.nola.domain.model.HostAgentPermissionRequest
import com.nola.domain.model.HostAgentToolEvent

internal fun hostToolMetaFlag(meta: Map<String, Any>, key: String): Boolean {
    return when (val value = meta[key]) {
        is Boolean -> value
        is Number -> value.toDouble() == 1.0
        is String -> value.trim().lowercase() in setOf("true", "1", "yes", "y", "on")
        else -> false
    }
}

internal fun hostToolResultStatus(eventError: Boolean, meta: Map<String, Any>): String {
    return when {
        hostToolMetaFlag(meta, "permission_required") -> "permission_required"
        eventError -> "error"
        else -> "done"
    }
}

internal fun hostToolResultError(eventError: Boolean, meta: Map<String, Any>): Boolean {
    return eventError && !hostToolMetaFlag(meta, "permission_required")
}

internal fun hostToolMetaText(meta: Map<String, Any>, key: String): String? {
    return when (val value = meta[key]) {
        is String -> value.trim().takeIf { it.isNotEmpty() }
        is Number -> value.toString()
        is Boolean -> value.toString()
        else -> null
    }
}

internal fun hostToolResultSummary(
    display: String?,
    text: String,
    pageTitle: String?,
    downloadFilename: String?,
    sensitive: Boolean
): String? {
    if (sensitive) return null
    return listOf(display, pageTitle, downloadFilename, text.trim().take(200))
        .firstNotNullOfOrNull { value -> value?.trim()?.takeIf { it.isNotBlank() } }
}

internal fun canResumeHostPermission(
    permissionResumeCount: Int,
    maxPermissionResumes: Int
): Boolean = maxPermissionResumes <= 0 || permissionResumeCount < maxPermissionResumes

internal fun upsertHostToolEvent(
    events: List<HostAgentToolEvent>,
    event: HostAgentToolEvent
): List<HostAgentToolEvent> {
    val index = events.indexOfFirst { it.id == event.id }
    if (index < 0) return events + event
    return events.toMutableList().also { it[index] = event }
}

internal class CapabilityActivationBuffer(
    private val maximumSessions: Int = 64,
    private val maximumEventsPerSession: Int = 32,
) {
    private val activations = linkedMapOf<String, LinkedHashMap<String, AgentCoreCapabilityActivationEnvelope>>()

    fun stage(activation: AgentCoreCapabilityActivationEnvelope): List<HostAgentToolEvent> {
        val session = activations.getOrPut(activation.sessionId) { linkedMapOf() }
        val id = activation.toHostToolEvent().id
        val previous = session[id]
        if (previous == null || previous.cursor < activation.cursor) session[id] = activation
        while (session.size > maximumEventsPerSession) session.remove(session.keys.first())
        while (activations.size > maximumSessions) activations.remove(activations.keys.first())
        return events(activation.sessionId)
    }

    fun events(sessionId: String): List<HostAgentToolEvent> =
        activations[sessionId]?.values?.map { it.toHostToolEvent() }.orEmpty()

    fun clear(sessionId: String) {
        activations.remove(sessionId)
    }
}

internal enum class HostToolActivityState {
    PermissionRequired,
    Running,
    Failed,
    Completed,
}

internal data class HostToolActivityPresentation(
    val state: HostToolActivityState,
    val count: Int,
)

internal fun hostToolActivityPresentation(
    events: List<HostAgentToolEvent>
): HostToolActivityPresentation {
    val permissionCount = events.count { it.status.contains("permission", ignoreCase = true) }
    if (permissionCount > 0) {
        return HostToolActivityPresentation(HostToolActivityState.PermissionRequired, permissionCount)
    }
    val runningCount = events.count { it.status.equals("running", ignoreCase = true) }
    if (runningCount > 0) {
        return HostToolActivityPresentation(HostToolActivityState.Running, runningCount)
    }
    val failedCount = events.count {
        it.error || it.status.equals("failed", ignoreCase = true)
    }
    if (failedCount > 0) {
        return HostToolActivityPresentation(HostToolActivityState.Failed, failedCount)
    }
    return HostToolActivityPresentation(HostToolActivityState.Completed, events.size)
}

internal fun transitionHostToolPermission(
    events: List<HostAgentToolEvent>,
    request: HostAgentPermissionRequest,
    status: String,
    failed: Boolean = false,
): List<HostAgentToolEvent> {
    val existing = events.firstOrNull { it.id == request.id }
    val event = existing?.copy(status = status, error = failed) ?: HostAgentToolEvent(
        id = request.id,
        toolName = request.toolName,
        args = request.args,
        argsPreview = request.args.entries.joinToString(separator = "\n") { (key, value) ->
            "$key: $value"
        },
        status = status,
        error = failed,
    )
    return upsertHostToolEvent(events, event)
}
