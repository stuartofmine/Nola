package org.nola.app.sse // intentionally not com.nola to avoid DTO serialization recursion

import com.nola.agent.trace.AgentTraceEvent
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive

// Place SseEvent DTO in a separate package to avoid name collision with domain model
// These are the raw SSE wire-format events

@Serializable
data class SseEventDto(
    @SerialName("allow_conversation") val allowConversation: Boolean? = null,
    val trace: AgentTraceEvent? = null,
    val type: String? = null,
    val text: String? = null,
    val delta: String? = null,
    val id: String? = null,
    val name: String? = null,
    val args: JsonObject? = null,
    val display: String? = null,
    val error: JsonElement? = null,
    val sensitive: JsonElement? = null,
    val meta: JsonObject? = null,
    @SerialName("tool_name") val toolName: String? = null,
    val match: String? = null,
    val reason: String? = null,
    @SerialName("permission_instructions") val permissionInstructions: List<com.nola.core.AgentPermissionInstruction> = emptyList(),
    val message: String? = null,
    @SerialName("safe_review_text") val safeReviewText: String? = null,
    val nonce: String? = null,
    @SerialName("host_id") val hostId: String? = null,
    @SerialName("client_id") val clientId: String? = null,
    @SerialName("session_id") val sessionId: String? = null,
    @SerialName("submission_id") val submissionId: String? = null,
    @SerialName("backend_id") val backendId: String? = null,
    @SerialName("external_session_id") val externalSessionId: String? = null,
    @SerialName("expires_at") val expiresAt: Long? = null,
    @SerialName("protocol_version") val protocolVersion: Int? = null,
    val capability: String? = null,
    val arguments: JsonObject? = null,
    val conversationId: String? = null,
    val cursor: Long? = null,
    val taskProjection: JsonElement? = null,
    val revision: Long? = null,
    @SerialName("source_host_id") val sourceHostId: String? = null,
) {
    fun toDomain(rawEnvelope: JsonObject? = null): com.nola.data.remote.sse.SseEvent {
        if (type == "context_usage") return com.nola.data.remote.sse.SseEvent.ContextUsageChanged(requireNotNull(rawEnvelope))
        if (type == "submission_receipt") return com.nola.data.remote.sse.SseEvent.SubmissionReceipt(
            requireNotNull(rawEnvelope).getValue("receipt").let { it as? JsonObject ?: error("Invalid submission receipt") })
        // Publication envelopes must not be swallowed by their nested diagnostic trace.
        if (type == "assistant_journal_committed_v1") return com.nola.data.remote.sse.SseEvent.AssistantJournal(requireNotNull(rawEnvelope))
        if (type == "assistant_run_terminal_committed_v1") return com.nola.data.remote.sse.SseEvent.AssistantTerminal(requireNotNull(rawEnvelope))
        trace?.let {
            if (type != "permission_request") return com.nola.data.remote.sse.SseEvent.Trace(it)
        }
        if (type in setOf(
            "turn_started",
            "turn_completed",
            "turn_failed",
            "turn_cancelled"
            ) && rawEnvelope != null
        ) {
            val lifecycle = com.nola.data.remote.sse.SseEvent.TurnLifecycle(rawEnvelope)
            return lifecycle
        }
        return when (type) {
            "permission_request" -> com.nola.data.remote.sse.SseEvent.PermissionRequest(
                allowConversation = requireNotNull(allowConversation) { "Missing permission scope eligibility" },
                id = id ?: "",
                toolName = toolName ?: name ?: "unknown",
                args = args?.mapValues { it.value.asPreviewString() } ?: emptyMap(),
                match = match ?: "",
                reason = reason ?: "",
                permissionInstructions = permissionInstructions,
                message = message ?: "",
                safeReviewText = safeReviewText?.also {
                    require(it.toByteArray(Charsets.UTF_8).size <= 32 * 1024) { "Host permission review exceeds limit" }
                },
                nonce = nonce,
                hostId = hostId,
                clientId = clientId,
                sessionId = sessionId,
                expiresAt = expiresAt,
                protocolVersion = protocolVersion
            )
            "device_tool_request" -> com.nola.data.remote.sse.SseEvent.DeviceToolRequest(
                id = id ?: "",
                nonce = nonce ?: "",
                hostId = hostId ?: "",
                clientId = clientId ?: "",
                sessionId = sessionId ?: "",
                toolName = toolName ?: name ?: "unknown",
                capability = capability ?: "none",
                arguments = arguments ?: JsonObject(emptyMap()),
                args = args?.mapValues { it.value.asPreviewString() } ?: emptyMap(),
                expiresAt = expiresAt ?: 0,
                protocolVersion = protocolVersion ?: 0
            )
            "external_session_started" -> com.nola.data.remote.sse.SseEvent.ExternalSessionStarted(
                backendId = backendId ?: "",
                sessionId = sessionId ?: externalSessionId ?: ""
            )
            "task_projection" -> com.nola.data.remote.sse.SseEvent.TaskProjection(
                conversationId = conversationId.orEmpty(),
                cursor = cursor?.coerceAtLeast(0) ?: 0,
                projection = com.nola.core.decodeAgentCoreTaskProjection(
                    taskProjection,
                    conversationId.orEmpty(),
                ),
            )
            "task_stream_checkpoint" -> com.nola.data.remote.sse.SseEvent.TaskStreamCheckpoint(
                conversationId = conversationId.orEmpty(),
                cursor = cursor?.coerceAtLeast(0) ?: 0,
            )
            "error" -> com.nola.data.remote.sse.SseEvent.Error(
                message = message ?: text ?: "Unknown error"
            )
            else -> throw IllegalArgumentException("Unsupported Agent Host event type: $type")
        }
    }

    private fun JsonElement.asPreviewString(): String = when (this) {
        is JsonPrimitive -> jsonPrimitive.contentOrNull ?: toString()
        else -> toString()
    }

    private fun JsonElement?.asBoolish(): Boolean {
        val raw = (this as? JsonPrimitive)?.contentOrNull?.trim()?.lowercase() ?: return false
        return raw in setOf("true", "1", "yes", "y", "on")
    }
}
