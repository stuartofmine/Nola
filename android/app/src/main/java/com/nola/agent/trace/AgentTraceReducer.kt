package com.nola.agent.trace

import kotlinx.serialization.json.JsonNull

object AgentTraceReducer {
    fun reduce(
        current: AgentTraceProjection,
        event: AgentTraceEvent,
    ): AgentTraceReduction {
        if (event.schemaVersion != AGENT_TRACE_SCHEMA_VERSION) {
            return AgentTraceReduction(
                projection = current.copy(ignoredOutOfOrderCount = current.ignoredOutOfOrderCount + 1),
                applied = false,
                outOfOrder = true,
            )
        }
        if (event.eventId in current.seenEventIds) {
            return AgentTraceReduction(
                projection = current.copy(ignoredDuplicateCount = current.ignoredDuplicateCount + 1),
                applied = false,
                duplicate = true,
            )
        }
        if (current.traceId != null && current.traceId != event.traceId) {
            return reduce(AgentTraceProjection(), event)
        }
        if (event.sequence <= current.lastSequence) {
            return AgentTraceReduction(
                projection = current.copy(
                    seenEventIds = current.seenEventIds + event.eventId,
                    ignoredOutOfOrderCount = current.ignoredOutOfOrderCount + 1,
                ),
                applied = false,
                outOfOrder = true,
            )
        }

        val gaps = if (event.sequence > current.lastSequence + 1) {
            current.missingSequences + ((current.lastSequence + 1) until event.sequence)
        } else {
            current.missingSequences
        }
        var next = current.copy(
            traceId = event.traceId,
            runId = event.runId,
            conversationId = event.conversationId ?: current.conversationId,
            messageId = event.messageId ?: current.messageId,
            turnId = event.turnId,
            lastSequence = event.sequence,
            missingSequences = gaps,
            seenEventIds = current.seenEventIds + event.eventId,
        )

        next = when (event.kind) {
            "run.started" -> next.copy(status = "running", terminalReason = null)
            "phase.changed" -> next.copy(
                phase = event.payload.phase ?: event.payload.summary ?: next.phase,
                status = event.status,
            )
            "context.compacted" -> next.copy(phase = "processing", status = "running")
            "tool.progress" -> if (event.payload.toolName == null && event.payload.callId == null) {
                if (next.phase == "compacting") next else when (event.payload.summary) {
                    "model_retry" -> next.copy(phase = "model_retry", status = event.status)
                    "model_attempt_started" -> next.copy(phase = "processing", status = event.status)
                    else -> next
                }
            } else {
                next.upsertTool(event)
            }
            "tool.started", "tool.permission.requested",
            "tool.permission.resolved", "tool.completed" -> next
                .upsertTool(event)
                .copy(phase = next.phase.takeUnless { it == "model_retry" })
            "usage.snapshot" -> next.copy(
                usage = event.payload.usage ?: next.usage,
                promptCacheDiagnostic = event.payload.promptCacheDiagnostic
                    ?: next.promptCacheDiagnostic,
            )
            "memory.used" -> next.copy(memories = event.payload.memories)
            "validation.updated" -> next.copy(validation = event.payload.validation ?: next.validation)
            "run.completed" -> next.copy(
                status = "succeeded",
                phase = next.phase.takeUnless { it == "compacting" || it == "model_retry" },
                terminalReason = event.payload.reason ?: "completed",
            )
            "run.failed" -> next.copy(
                status = "failed",
                phase = next.phase.takeUnless { it == "compacting" || it == "model_retry" },
                terminalReason = event.payload.reason ?: event.payload.summary ?: "failed",
            )
            "run.cancelled" -> next.copy(
                status = "cancelled",
                phase = next.phase.takeUnless { it == "compacting" || it == "model_retry" },
                terminalReason = event.payload.reason ?: "cancelled",
            )
            else -> next
        }
        return AgentTraceReduction(
            projection = next,
            applied = !current.hasSameVisibleProjection(next),
        )
    }

    fun replay(events: Iterable<AgentTraceEvent>): AgentTraceProjection =
        events
            .sortedWith(compareBy<AgentTraceEvent>({ it.sequence }, { it.emittedAt }, { it.eventId }))
            .fold(AgentTraceProjection()) { projection, event ->
                reduce(projection, event).projection
            }

    private fun AgentTraceProjection.upsertTool(event: AgentTraceEvent): AgentTraceProjection {
        val spanId = event.spanId ?: event.payload.callId
            ?: event.payload.permissionRequestId
            ?: return this
        val existing = tools.firstOrNull { it.spanId == spanId }
        val status = when (event.kind) {
            "tool.permission.requested" -> "permission_required"
            "tool.permission.resolved" -> if (event.status == "failed") "failed" else "running"
            "tool.completed" -> when (event.status) {
                "failed" -> "failed"
                "cancelled" -> "cancelled"
                else -> "succeeded"
            }
            else -> event.status
        }
        val updated = AgentTraceToolProjection(
            traceId = event.traceId,
            spanId = spanId,
            callId = event.payload.callId ?: existing?.callId ?: spanId,
            toolName = event.payload.toolName ?: existing?.toolName ?: "unknown",
            status = status,
            summary = event.payload.summary ?: existing?.summary,
            progress = event.payload.progress ?: existing?.progress,
            arguments = event.payload.arguments ?: existing?.arguments ?: kotlinx.serialization.json.JsonObject(emptyMap()),
            resources = event.payload.resources.ifEmpty { existing?.resources.orEmpty() },
            result = event.payload.result?.takeUnless { it is JsonNull } ?: existing?.result,
        )
        val index = tools.indexOfFirst { it.spanId == spanId }
        val updatedTools = if (index < 0) {
            tools + updated
        } else {
            tools.toMutableList().also { it[index] = updated }
        }
        return copy(tools = updatedTools, status = event.status)
    }

    private fun AgentTraceProjection.hasSameVisibleProjection(other: AgentTraceProjection): Boolean =
        traceId == other.traceId &&
            runId == other.runId &&
            conversationId == other.conversationId &&
            messageId == other.messageId &&
            turnId == other.turnId &&
            phase == other.phase &&
            status == other.status &&
            tools == other.tools &&
            memories == other.memories &&
            usage == other.usage &&
            validation == other.validation &&
            terminalReason == other.terminalReason
}
