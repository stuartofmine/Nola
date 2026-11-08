package com.nola.core

import com.nola.data.remote.sse.SseEvent
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.json.Json
import org.junit.Assert.*
import org.junit.Test
import org.nola.app.sse.SseEventDto

class PermissionInstructionContractTest {
    @Test fun strictPendingProjectionDecodesGuidanceWithoutChangingArguments() {
        val wire = """{"revision":2,"pending":[{"allow_conversation":false,"request_id":"new","turn_id":"turn","call_id":"call","capability_id":"write","resource":"/original","arguments":{"path":"/original"},"permission_domain":"files","rationale":"confirm","evaluation":{},"permission_instructions":[{"permission_request_id":"old","instruction":"Preview first."}]}]}"""
        val pending = Json.decodeFromString<AgentCorePermissionProjection>(wire).pending.single()
        assertEquals("new", pending.requestId)
        assertFalse(pending.allowConversation)
        assertTrue(runCatching { Json.decodeFromString<AgentCorePermissionProjection>(wire.replace("\"allow_conversation\":false,", "")) }.isFailure)
        assertEquals("old", pending.permissionInstructions.single().permissionRequestId)
        assertEquals("/original", pending.resource)
        assertFalse(pending.arguments.toString().contains("Preview"))
        val legacy = wire.replace(",\"permission_instructions\":[{\"permission_request_id\":\"old\",\"instruction\":\"Preview first.\"}]", "")
        assertTrue(Json.decodeFromString<AgentCorePermissionProjection>(legacy).pending.single().permissionInstructions.isEmpty())
    }

    @Test fun productionSseMappingPreservesTypedGuidanceAndRejectsMalformedRecords() {
        val wire = """{"type":"permission_request", "allow_conversation": true,"id":"new","tool_name":"write","args":{"path":"/original"},"permission_instructions":[{"permission_request_id":"old","instruction":"Preview first."}]}"""
        val dto = Json.decodeFromString<SseEventDto>(wire)
        val trace = com.nola.agent.trace.AgentTraceEvent(
            traceId = "trace", runId = "run", conversationId = "conversation",
            turnId = "turn", eventId = "event", sequence = 1,
            emittedAt = "2026-09-08T00:00:00Z", kind = "tool.permission.requested",
        )
        val event = dto.copy(trace = trace).toDomain() as SseEvent.PermissionRequest
        assertTrue(dto.copy(type = "other", trace = trace).toDomain() is SseEvent.Trace)
        assertEquals("old", event.permissionInstructions.single().permissionRequestId)
        assertEquals("/original", event.args["path"])
        for ((bad, good) in listOf("42" to "\"Preview first.\"", "\" \"" to "\"Preview first.\"", "null" to "[{\"permission_request_id\":\"old\",\"instruction\":\"Preview first.\"}]")) {
            assertTrue(runCatching { Json.decodeFromString<SseEventDto>(wire.replace(good, bad)) }.isFailure)
        }
    }
}
