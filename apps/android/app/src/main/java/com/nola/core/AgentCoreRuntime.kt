@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)

package com.nola.core

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

internal object AgentCoreNative {
    private val loaded: Boolean = runCatching {
        System.loadLibrary("nola_core_jni")
    }.isSuccess

    val isIntelligenceAvailable: Boolean
        get() = loaded && runCatching { nativeIntelligenceAbiVersion() == 1 }.getOrDefault(false)

    val isToolPolicyAvailable: Boolean
        get() = loaded && runCatching { nativeToolPolicyAbiVersion() == 1 }.getOrDefault(false)

    val isPromptComposerAvailable: Boolean
        get() = loaded && runCatching { nativePromptComposerAbiVersion() == 1 }.getOrDefault(false)

    val isMemorySelectionAvailable: Boolean
        get() = loaded && runCatching { nativeMemorySelectionAbiVersion() == 1 }.getOrDefault(false)

    val isLifecycleHookRegistryAvailable: Boolean
        get() = loaded && runCatching { nativeLifecycleHookAbiVersion() == 1 }.getOrDefault(false)

    val isApplicationAvailable: Boolean
        get() = loaded && runCatching {
            supportsApplicationContract(nativeApplicationAbiVersion(), nativeApplicationProtocolVersion())
        }.getOrDefault(false)

    // Match the single shipped Application command contract, never a raw Core ABI.
    internal fun supportsApplicationContract(abi: Int, protocol: Int): Boolean = abi == 1 && protocol == 1

    external fun nativeMobileMcpRetain(handle: Long): Long
    external fun nativeMobileMcpHost(handle: Long, input: ByteArray): ByteArray?
    external fun nativeIntelligenceAbiVersion(): Int
    external fun nativeSoftwareUpdate(input: ByteArray): ByteArray?
    external fun nativeContextPlan(input: ByteArray): ByteArray?
    external fun nativeMemorySelectionAbiVersion(): Int
    external fun nativeMemorySelect(input: ByteArray): ByteArray?
    external fun nativeDataSourceAbiVersion(): Int
    external fun nativeDataSourceResolve(input: ByteArray): ByteArray?
    external fun nativeDataSourceSealContract(input: ByteArray): ByteArray?
    external fun nativeResourceAccessAbiVersion(): Int
    external fun nativeResourceAccessEvaluate(input: ByteArray): ByteArray?
    external fun nativeLifecycleHookAbiVersion(): Int
    external fun nativeLifecycleHookBuildRegistry(input: ByteArray): ByteArray?
    external fun nativeLifecycleHookPlan(input: ByteArray): ByteArray?
    external fun nativeLifecycleHookProjectExecution(input: ByteArray): ByteArray?
    external fun nativePromptCacheAbiVersion(): Int
    external fun nativePromptCacheDiagnose(input: ByteArray): ByteArray?
    external fun nativeToolPolicyAbiVersion(): Int
    external fun nativeToolPolicy(input: ByteArray): ByteArray?
    external fun nativePromptComposerAbiVersion(): Int
    external fun nativeComposeAgentPrompt(input: ByteArray): ByteArray?
    external fun nativeParseAgentModelOutput(input: ByteArray): ByteArray?
    external fun nativeStitchAgentModelContinuation(input: ByteArray): ByteArray?
    external fun nativeIntelligenceIngest(input: ByteArray): ByteArray?
    external fun nativeIntelligenceRecall(input: ByteArray): ByteArray?
    external fun nativeIntelligenceSnapshot(): ByteArray?
    external fun nativeIntelligenceClear(): Boolean
    external fun nativeApplicationAbiVersion(): Int
    external fun nativeApplicationOpen(path: ByteArray): Long
    external fun nativeApplicationExecute(handle: Long, command: ByteArray): ByteArray?
    external fun nativeApplicationExpertHostOperation(handle: Long, operation: ByteArray): ByteArray?
    external fun nativeApplicationAttestEditedResendHostPolicy(handle: Long, target: ByteArray, config: ByteArray): ByteArray?
    external fun nativeApplicationProtocolVersion(): Int
    external fun nativeApplicationSubmit(handle: Long, envelope: ByteArray): ByteArray?
    external fun nativeChildPumpStart(application: Long, input: ByteArray): Long
    external fun nativeChildPumpPoll(handle: Long, waitMs: Int): ByteArray?
    external fun nativeChildPumpRespond(handle: Long, input: ByteArray): ByteArray?
    external fun nativeChildPumpClose(handle: Long)
    external fun nativeChildPumpFree(handle: Long)
    external fun nativeApplicationFree(handle: Long)
}

interface AgentCoreTransitionRuntime {
    suspend fun submit(submission: JsonObject): JsonObject
    suspend fun snapshot(): JsonObject
    suspend fun pendingHostRequests(): JsonObject
}

/** Pure evaluation only: no execution handle or mutable reducer authority. */
object AgentCoreToolPolicyEvaluator {
    fun evaluate(input: AgentCoreToolPolicyInput): AgentCoreToolPolicyOutput? {
        if (!AgentCoreNative.isToolPolicyAvailable) return null
        val bytes = AgentCoreNative.nativeToolPolicy(AgentCoreToolPolicyCodec.encode(input))
            ?: return null
        return AgentCoreToolPolicyCodec.decode(bytes)
    }
}

class AgentCoreProtocolException(
    val code: String,
    val category: String,
    override val message: String,
) : IllegalStateException(message)

internal fun JsonObject.agentCoreTransitionOrThrow(): JsonObject {
    val accepted = (this["ok"] as? JsonPrimitive)
        ?.content
        ?.toBooleanStrictOrNull()
    if (accepted != true) {
        val message = (this["error"] as? JsonPrimitive)?.content
            ?: throw AgentCoreProtocolException(
                code = "invalid_error_envelope",
                category = "protocol_violation",
                message = "Agent Core returned an invalid error envelope",
            )
        val code = (this["error_code"] as? JsonPrimitive)?.content
            ?: throw AgentCoreProtocolException(
                code = "missing_error_code",
                category = "protocol_violation",
                message = "Agent Core returned no error code",
            )
        val category = (this["error_category"] as? JsonPrimitive)?.content
            ?: throw AgentCoreProtocolException(
                code = "missing_error_category",
                category = "protocol_violation",
                message = "Agent Core returned no error category",
            )
        throw AgentCoreProtocolException(code, category, message)
    }
    return this["transition"] as? JsonObject
        ?: throw AgentCoreProtocolException(
            code = "missing_transition",
            category = "protocol_violation",
            message = "Agent Core returned no transition",
        )
}

@Serializable
data class AgentCoreRuntimeConfig(
    val platform: String = "android",
    @SerialName("execution_mode") val executionMode: String = "device_local",
    @SerialName("host_id") val hostId: String? = null,
    val model: String? = null,
    @SerialName("prepared_host_invocation") val preparedHostInvocation: JsonObject? = null,
    @SerialName("stable_prompt_digest") val stablePromptDigest: String? = null,
    @SerialName("prompt_fragment_ids") val promptFragmentIds: List<String> = emptyList(),
    @SerialName("stable_prompt_fragment_digests") val stablePromptFragmentDigests: Map<String, String> = emptyMap(),
    @SerialName("max_iterations") val maxIterations: Int = 0,
    @SerialName("context_window_tokens") val contextWindowTokens: Int? = null,
    @SerialName("reserved_output_tokens") val reservedOutputTokens: Int = 16_384,
    @SerialName("granted_capabilities") val grantedCapabilities: List<String> = listOf("network")
)

/**
 * Canonical policy used by both the foreground Turn and its deferred worker.
 * Keeping one typed builder prevents cold recovery from restoring the same
 * checkpoint with a different policy and makes Turn completion enqueue work.
 */
internal fun automaticMemoryPolicyConfig(): JsonObject = buildJsonObject {
    put("long_term_enabled", true)
    put("automatic_extraction_enabled", true)
    put("search_limit", 20)
    put("minimum_confidence", 0.5)
    put("maximum_age_days", JsonNull)
    put("allowed_sensitivities", JsonArray(emptyList()))
}

@Serializable
data class AgentCoreMessage(
    val role: String,
    val content: String,
    val name: String? = null,
    @SerialName("tool_call_id") val toolCallId: String? = null,
    @SerialName("tool_calls") val toolCalls: List<JsonObject> = emptyList()
)

@Serializable
data class AgentCoreTokenUsage(
    @SerialName("prompt_tokens") val promptTokens: Int? = null,
    @SerialName("cached_input_tokens") val cachedInputTokens: Int? = null,
    @SerialName("cache_write_input_tokens") val cacheWriteInputTokens: Int? = null,
    @SerialName("completion_tokens") val completionTokens: Int? = null,
    @SerialName("total_tokens") val totalTokens: Int? = null,
    @SerialName("reasoning_tokens") val reasoningTokens: Int? = null,
)

internal fun AgentCoreTokenUsage.toCoreUsageJson(): JsonObject = buildJsonObject {
    put("input_tokens", promptTokens)
    put("cached_input_tokens", cachedInputTokens)
    put("cache_write_input_tokens", cacheWriteInputTokens)
    put("output_tokens", completionTokens)
    put("reasoning_tokens", reasoningTokens)
    put("total_tokens", totalTokens)
    put("request_count", 1)
    // Budget lower bound is not a complete usage total or a user-facing count.
    put("known_token_lower_bound", totalTokens ?: ((promptTokens ?: 0).toLong() + (completionTokens ?: 0).toLong()))
}

@Serializable
data class AgentCorePromptCacheDiagnostic(
    val schemaVersion: Int = 1,
    val status: String,
    val identityStable: Boolean = false,
    val identitySha256: String? = null,
    val reasons: List<String> = emptyList(),
    val fragmentChanges: List<AgentCorePromptCacheFragmentChange> = emptyList(),
    val cacheReadInputTokens: Int? = null,
    val cacheWriteInputTokens: Int? = null,
)

@Serializable
data class AgentCorePromptCacheFragmentChange(
    val fragmentId: String,
    val kind: String,
)

@Serializable
data class AgentCoreSnapshot(
    val platform: String,
    @SerialName("execution_mode") val executionMode: String = "device_local",
    @SerialName("host_id") val hostId: String? = null,
    val phase: String,
    @SerialName("session_id") val sessionId: String? = null,
    @SerialName("turn_id") val turnId: String? = null,
    val iteration: Int = 0,
    @SerialName("message_count") val messageCount: Int = 0,
    @SerialName("estimated_context_tokens") val estimatedContextTokens: Int = 0,
    @SerialName("context_window_tokens") val contextWindowTokens: Int? = null,
    @SerialName("context_usage_percent") val contextUsagePercent: Double? = null,
    @SerialName("pending_tool_count") val pendingToolCount: Int = 0,
    @SerialName("session_closing") val sessionClosing: Boolean = false,
    @SerialName("session_closed") val sessionClosed: Boolean = false,
    @SerialName("awaiting_permission") val awaitingPermission: Boolean = false,
    @SerialName("awaiting_host_call") val awaitingHostCall: Boolean = false,
    val usage: AgentCoreTokenUsage = AgentCoreTokenUsage(),
    @SerialName("prompt_cache_diagnostic")
    val promptCacheDiagnostic: AgentCorePromptCacheDiagnostic? = null,
    val permissions: AgentCorePermissionProjection? = null,
)

@Serializable
data class AgentCorePermissionProjection(
    val revision: Long = 0,
    val pending: List<AgentCorePendingPermissionRequest> = emptyList(),
    @SerialName("active_policy") val activePolicy: AgentActivePermissionPolicy? = null,
)

@Serializable
data class AgentActivePermissionPolicy(
    @SerialName("run_id") val runId: String,
    @SerialName("configuration_revision") val configurationRevision: Long,
    val profile: String,
) {
    init {
        require(runId.isNotBlank() && configurationRevision > 0 && profile in setOf("cautious", "allow_all")) {
            "Invalid active permission policy"
        }
    }
}

@Serializable
data class AgentPermissionInstruction(
    @SerialName("permission_request_id") val permissionRequestId: String,
    val instruction: String,
) {
    init {
        require(permissionRequestId.isNotBlank() && instruction.isNotBlank()) {
            "Invalid permission instruction metadata"
        }
    }
}

@Serializable
data class AgentCorePendingPermissionRequest(
    @SerialName("allow_conversation") val allowConversation: Boolean,
    @SerialName("request_id") val requestId: String,
    @SerialName("turn_id") val turnId: String,
    @SerialName("call_id") val callId: String,
    @SerialName("capability_id") val capabilityId: String,
    val resource: String,
    val arguments: JsonElement,
    @SerialName("permission_domain") val permissionDomain: String,
    val rationale: String,
    val evaluation: JsonElement,
    @SerialName("permission_instructions") val permissionInstructions: List<AgentPermissionInstruction> = emptyList(),
)

@Serializable
data class AgentCoreDispatchResponse(
    val ok: Boolean,
    val events: List<JsonObject> = emptyList(),
    val snapshot: AgentCoreSnapshot,
    val error: String? = null
)

@Serializable
data class AgentCoreContextPlanInput(
    @SerialName("context_window_tokens") val contextWindowTokens: Int?,
    @SerialName("token_threshold") val tokenThreshold: Int? = null,
    @SerialName("prompt_token_limit") val promptTokenLimit: Int? = null,
    @SerialName("reserved_output_tokens") val reservedOutputTokens: Int = 16_384,
    @SerialName("estimated_tokens") val estimatedTokens: Int,
    @SerialName("message_count") val messageCount: Int,
    @SerialName("threshold_percent") val thresholdPercent: Int = 60,
    @SerialName("keep_recent_messages") val keepRecentMessages: Int = 8,
    @SerialName("min_messages_before_compaction") val minMessagesBeforeCompaction: Int = 4,
    @SerialName("adaptive_long_history") val adaptiveLongHistory: Boolean = true
)

@Serializable
data class AgentCoreContextPlan(
    @SerialName("should_compact") val shouldCompact: Boolean,
    @SerialName("hard_limit_reached") val hardLimitReached: Boolean,
    @SerialName("trigger_tokens") val triggerTokens: Int,
    @SerialName("prompt_token_limit") val promptTokenLimit: Int,
    @SerialName("target_tokens") val targetTokens: Int,
    @SerialName("keep_recent_messages") val keepRecentMessages: Int,
    @SerialName("archive_message_count") val archiveMessageCount: Int,
    @SerialName("usage_percent") val usagePercent: Double? = null,
    val reason: String
)
