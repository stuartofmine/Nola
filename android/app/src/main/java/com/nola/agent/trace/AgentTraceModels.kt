package com.nola.agent.trace

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject

const val AGENT_TRACE_SCHEMA_VERSION = 1

@Serializable
data class AgentTraceEvent(
    @SerialName("schema_version") val schemaVersion: Int = AGENT_TRACE_SCHEMA_VERSION,
    @SerialName("trace_id") val traceId: String,
    @SerialName("run_id") val runId: String,
    @SerialName("conversation_id") val conversationId: String,
    @SerialName("message_id") val messageId: String? = null,
    @SerialName("turn_id") val turnId: String,
    @SerialName("event_id") val eventId: String,
    val sequence: Long,
    @SerialName("emitted_at") val emittedAt: String,
    val kind: String,
    @SerialName("span_id") val spanId: String? = null,
    @SerialName("parent_span_id") val parentSpanId: String? = null,
    val status: String = "running",
    val visibility: String = "summary",
    val payload: AgentTracePayload = AgentTracePayload(),
) {
    init {
        require(kind !in setOf("assistant.delta", "assistant.reset", "thinking.delta")) {
            "Assistant content belongs to the committed journal, not trace"
        }
    }
}

@Serializable
data class AgentTracePayload(
    val summary: String? = null,
    val detail: String? = null,
    val phase: String? = null,
    val reason: String? = null,
    @SerialName("tool_name") val toolName: String? = null,
    @SerialName("call_id") val callId: String? = null,
    @SerialName("permission_request_id") val permissionRequestId: String? = null,
    @SerialName("permission_scope") val permissionScope: String? = null,
    @SerialName("permission_decision") val permissionDecision: String? = null,
    val code: String? = null,
    val arguments: JsonObject? = null,
    val resources: List<AgentTraceResource> = emptyList(),
    val result: JsonElement? = null,
    val progress: Double? = null,
    val usage: AgentTraceUsage? = null,
    @SerialName("prompt_cache_diagnostic")
    val promptCacheDiagnostic: AgentTracePromptCacheDiagnostic? = null,
    val validation: AgentTraceValidation? = null,
    val memories: List<AgentTraceMemoryUsage> = emptyList(),
)

@Serializable
data class AgentTraceMemoryUsage(
    @SerialName("record_id") val recordId: String,
    @SerialName("component_id") val componentId: String,
    val scope: JsonElement? = null,
    val namespace: JsonElement? = null,
    @SerialName("provenance_kind") val provenanceKind: String? = null,
    val source: String? = null,
    val title: String? = null,
    @SerialName("observed_at") val observedAt: String? = null,
    @SerialName("requires_current_state_verification")
    val requiresCurrentStateVerification: Boolean = false,
    val truncated: Boolean = false,
)

@Serializable
data class AgentTraceResource(
    val uri: String,
    val operation: String,
    @SerialName("display_name") val displayName: String? = null,
    @SerialName("mime_type") val mimeType: String? = null,
)

@Serializable
data class AgentTraceUsage(
    @SerialName("used_tokens") val usedTokens: Int = 0,
    @SerialName("context_window_tokens") val contextWindowTokens: Int = 0,
    @SerialName("prompt_token_limit") val promptTokenLimit: Int? = null,
    @SerialName("cached_input_tokens") val cachedInputTokens: Int = 0,
    @SerialName("cached_input_tokens_reported") val cachedInputTokensReported: Boolean = false,
    @SerialName("cache_write_input_tokens") val cacheWriteInputTokens: Int = 0,
    @SerialName("cache_write_input_tokens_reported")
    val cacheWriteInputTokensReported: Boolean = false,
)

@Serializable
data class AgentTracePromptCacheDiagnostic(
    val schemaVersion: Int = 1,
    val status: String = "unknown",
    val identityStable: Boolean = false,
    val identitySha256: String? = null,
    val reasons: List<String> = emptyList(),
    val cacheReadInputTokens: Int? = null,
    val cacheWriteInputTokens: Int? = null,
)

@Serializable
data class AgentTraceValidation(
    val status: String = "unknown",
    val level: String? = null,
    val summary: String? = null,
    @SerialName("next_action") val nextAction: String? = null,
)

data class AgentTraceToolProjection(
    val traceId: String,
    val spanId: String,
    val callId: String,
    val toolName: String,
    val status: String,
    val summary: String? = null,
    val progress: Double? = null,
    val arguments: JsonObject = JsonObject(emptyMap()),
    val resources: List<AgentTraceResource> = emptyList(),
    val result: JsonElement? = null,
) {
    val stableId: String get() = "$traceId:$spanId"
}

data class AgentTraceProjection(
    val traceId: String? = null,
    val runId: String? = null,
    val conversationId: String? = null,
    val messageId: String? = null,
    val turnId: String? = null,
    val phase: String? = null,
    val status: String = "idle",
    val tools: List<AgentTraceToolProjection> = emptyList(),
    val memories: List<AgentTraceMemoryUsage> = emptyList(),
    val usage: AgentTraceUsage? = null,
    val promptCacheDiagnostic: AgentTracePromptCacheDiagnostic? = null,
    val validation: AgentTraceValidation? = null,
    val terminalReason: String? = null,
    val lastSequence: Long = 0,
    val missingSequences: Set<Long> = emptySet(),
    val seenEventIds: Set<String> = emptySet(),
    val ignoredDuplicateCount: Int = 0,
    val ignoredOutOfOrderCount: Int = 0,
) {
    val isTerminal: Boolean
        get() = status in setOf("succeeded", "failed", "cancelled")

    val currentTool: AgentTraceToolProjection?
        get() = tools.lastOrNull {
            it.status in setOf(
                "queued",
                "running",
                "waiting",
                "permission_required",
                "suspended",
                "retry_waiting",
            )
        }
}

data class AgentTraceReduction(
    val projection: AgentTraceProjection,
    val applied: Boolean,
    val duplicate: Boolean = false,
    val outOfOrder: Boolean = false,
)
