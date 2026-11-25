package com.nola.agent.trace

import com.nola.data.remote.sse.SseEvent
import java.time.Instant
import java.util.UUID
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

class AgentTraceSession(
    private var traceId: String,
    private var runId: String,
    private val conversationId: String,
    private val messageId: String?,
    private var turnId: String,
) {
    var projection: AgentTraceProjection = AgentTraceProjection()
        private set

    private var nextSequence = 1L

    fun accept(event: AgentTraceEvent): AgentTraceReduction {
        if (projection.traceId != null && projection.traceId != event.traceId) {
            projection = AgentTraceProjection()
            traceId = event.traceId
            runId = event.runId
            turnId = event.turnId
        }
        nextSequence = maxOf(nextSequence, event.sequence + 1)
        val reduction = AgentTraceReducer.reduce(projection, event)
        projection = reduction.projection
        return reduction
    }

    fun start(): AgentTraceEvent = event(
        kind = "run.started",
        payload = AgentTracePayload(summary = "run_started"),
    )

    fun fromSse(source: SseEvent): AgentTraceEvent? = when (source) {
        is SseEvent.Trace -> source.event
        // Submission ownership is accepted by the admission path, not replayed
        // as activity or a permission challenge.
        is SseEvent.SubmissionReceipt -> null
        is SseEvent.CoreProjectionHint -> null
        is SseEvent.ContextUsageChanged -> null
        is SseEvent.TurnLifecycle -> null
        is SseEvent.TaskProjection -> null
        is SseEvent.TaskStreamCheckpoint -> null
        is SseEvent.AssistantJournal, is SseEvent.AssistantTerminal -> null
        is SseEvent.ExternalSessionStarted -> null
        is SseEvent.V2SessionStarted,
        is SseEvent.V2TextDelta,
        is SseEvent.V2ReasoningDelta,
        is SseEvent.V2TurnCompleted -> null
        is SseEvent.V2Compacted -> event(
            kind = "context.compacted",
            status = "running",
            payload = AgentTracePayload(
                phase = "compacting",
                summary = source.summary.ifBlank { "context_compacted" },
            ),
        )
        is SseEvent.V2UiSurface -> {
            val surfaceId = ((source.snapshot["identity"] as? JsonObject)?.get("surface_id") as? JsonPrimitive)?.content
                ?.takeIf { it.isNotBlank() }
                ?: "ui_surface"
            val summary = (source.snapshot["summary"] as? JsonPrimitive)?.content
                ?.takeIf { it.isNotBlank() }
                ?: (source.snapshot["phase"] as? JsonPrimitive)?.content
                ?: "ui_surface"
            event(
                kind = "tool.progress",
                status = "running",
                spanId = surfaceId,
                payload = AgentTracePayload(
                    summary = summary,
                    toolName = "render_ui_surface",
                    callId = surfaceId,
                    result = source.snapshot,
                ),
            )
        }
        is SseEvent.V2PermissionRequested -> event(
            kind = "tool.permission.requested",
            status = "waiting",
            spanId = source.requestId,
            payload = AgentTracePayload(
                summary = source.reason.ifBlank { source.tool },
                toolName = source.tool,
                callId = source.requestId,
                permissionRequestId = source.requestId,
                arguments = source.arguments as? JsonObject,
            ),
        )
        is SseEvent.V2UserQuestion -> event(
            kind = "phase.changed",
            status = "waiting",
            spanId = source.requestId,
            payload = AgentTracePayload(
                phase = "awaiting_user_input",
                summary = source.question,
                detail = source.options.takeIf { it.isNotEmpty() }?.joinToString(", "),
                callId = source.requestId,
            ),
        )
        is SseEvent.V2ToolStarted -> event(
            kind = "tool.started",
            spanId = source.callId,
            payload = AgentTracePayload(summary = source.name, toolName = source.name, callId = source.callId),
        )
        is SseEvent.V2ToolFinished -> event(
            kind = if (source.isError) "tool.failed" else "tool.completed",
            spanId = source.callId,
            payload = AgentTracePayload(summary = source.summary, toolName = source.name, callId = source.callId),
        )
        is SseEvent.PermissionRequest -> event(
            kind = "tool.permission.requested",
            spanId = source.id,
            payload = AgentTracePayload(
                summary = source.message.ifBlank { source.reason },
                toolName = source.toolName,
                callId = source.id,
                permissionRequestId = source.id,
                arguments = JsonObject(source.args.mapValues { JsonPrimitive(it.value) }),
            ),
        )
        is SseEvent.DeviceToolRequest -> event(
            kind = "tool.permission.requested",
            spanId = source.id,
            payload = AgentTracePayload(
                summary = source.capability,
                toolName = source.toolName,
                callId = source.id,
                permissionRequestId = source.id,
                arguments = source.arguments,
            ),
        )
        is SseEvent.Error -> event(
            kind = "run.failed",
            status = "failed",
            payload = AgentTracePayload(reason = source.message, summary = source.message),
        )
    }

    fun completed(reason: String = "completed"): AgentTraceEvent = event(
        kind = "run.completed",
        status = "succeeded",
        payload = AgentTracePayload(reason = reason),
    )

    fun permissionResolved(
        requestId: String,
        toolName: String,
        allowed: Boolean,
    ): AgentTraceEvent = event(
        kind = "tool.permission.resolved",
        spanId = requestId,
        status = if (allowed) "running" else "failed",
        payload = AgentTracePayload(
            summary = if (allowed) "permission_allowed" else "permission_denied",
            toolName = toolName,
            callId = requestId,
            permissionRequestId = requestId,
        ),
    )

    private fun event(
        kind: String,
        status: String = "running",
        visibility: String = "summary",
        spanId: String? = null,
        payload: AgentTracePayload,
    ): AgentTraceEvent = AgentTraceEvent(
        traceId = traceId,
        runId = runId,
        conversationId = conversationId,
        messageId = messageId,
        turnId = turnId,
        eventId = "android_${nextSequence}_${UUID.randomUUID()}",
        sequence = nextSequence++,
        emittedAt = Instant.now().toString(),
        kind = kind,
        spanId = spanId,
        parentSpanId = turnId,
        status = status,
        visibility = visibility,
        payload = payload,
    )
}

private fun resourcesFor(
    toolName: String,
    arguments: Map<String, String>,
): List<AgentTraceResource> {
    val path = arguments["path"] ?: arguments["file"] ?: arguments["url"] ?: arguments["query"] ?: return emptyList()
    val operation = when {
        toolName.contains("search", ignoreCase = true) -> "search"
        toolName.contains("read", ignoreCase = true) -> "read"
        toolName.contains("write", ignoreCase = true) || toolName.contains("edit", ignoreCase = true) -> "write"
        else -> "open"
    }
    return listOf(AgentTraceResource(uri = path, operation = operation, displayName = path.substringAfterLast('/')))
}
