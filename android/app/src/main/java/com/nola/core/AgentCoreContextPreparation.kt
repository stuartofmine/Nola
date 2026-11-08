@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)

package com.nola.core

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject

const val AGENT_CORE_CONTEXT_SCHEMA_VERSION = 2
const val AGENT_CORE_CONTEXT_ABI_VERSION = 2

@Serializable
data class AgentCoreContextPrepareBudget(
    @SerialName("context_window_tokens") val contextWindowTokens: Int? = null,
    @SerialName("prompt_token_limit") val promptTokenLimit: Int? = null,
    @SerialName("token_threshold") val tokenThreshold: Int? = null,
    @SerialName("reserved_output_tokens") val reservedOutputTokens: Int = 16_384,
    @SerialName("threshold_percent") val thresholdPercent: Int = 60,
    @SerialName("keep_recent_messages") val keepRecentMessages: Int = 8,
    @SerialName("min_messages_before_compaction") val minMessagesBeforeCompaction: Int = 4,
    @SerialName("compacted_tool_chars") val compactedToolChars: Int = 1_400,
    @SerialName("compacted_assistant_chars") val compactedAssistantChars: Int = 900,
)

@Serializable
data class AgentCoreContextComponentSource(
    val platform: String? = null,
    val uri: String? = null,
    val title: String? = null,
    @SerialName("mime_type") val mimeType: String? = null,
    @SerialName("observed_at") val observedAt: String? = null,
    val authorization: String? = null,
    @SerialName("maximum_age_minutes") val maximumAgeMinutes: Long? = null,
)

@Serializable
data class AgentCoreContextComponent(
    val kind: String,
    val id: String,
    val content: JsonElement,
    val source: AgentCoreContextComponentSource? = null,
)

@Serializable
data class AgentCoreContextSummaryResult(
    @SerialName("archive_id") val archiveId: String,
    @SerialName("archive_digest") val archiveDigest: String,
    val content: String,
)

@Serializable
data class AgentCoreContextPrepareInput(
    @SerialName("schema_version") val schemaVersion: Int = AGENT_CORE_CONTEXT_SCHEMA_VERSION,
    val mode: String = "normal",
    val platform: String = "android",
    val components: List<AgentCoreContextComponent> = emptyList(),
    val messages: List<JsonObject> = emptyList(),
    val tools: List<JsonElement> = emptyList(),
    val budget: AgentCoreContextPrepareBudget,
    @SerialName("existing_summary") val existingSummary: String? = null,
    @SerialName("summary_result") val summaryResult: AgentCoreContextSummaryResult? = null,
)

@Serializable
data class AgentCoreContextMetrics(
    @SerialName("message_tokens") val messageTokens: Int,
    @SerialName("tool_schema_tokens") val toolSchemaTokens: Int,
    @SerialName("total_prompt_tokens") val totalPromptTokens: Int,
    @SerialName("prompt_token_limit") val promptTokenLimit: Int,
    @SerialName("reserved_output_tokens") val reservedOutputTokens: Int,
    @SerialName("usage_percent") val usagePercent: Double,
    @SerialName("before_tokens") val beforeTokens: Int = totalPromptTokens,
    @SerialName("after_tokens") val afterTokens: Int = totalPromptTokens,
    @SerialName("trigger_tokens") val triggerTokens: Int = promptTokenLimit,
    @SerialName("target_tokens") val targetTokens: Int = promptTokenLimit,
    @SerialName("archive_message_count") val archiveMessageCount: Int = 0,
    @SerialName("transaction_count") val transactionCount: Int = 0,
)

@Serializable
data class AgentCoreContextComponentMetrics(
    val kind: String,
    @SerialName("used_tokens") val usedTokens: Int,
    @SerialName("dropped_tokens") val droppedTokens: Int,
    @SerialName("truncated_tokens") val truncatedTokens: Int,
    @SerialName("used_ids") val usedIds: List<String> = emptyList(),
    @SerialName("dropped_ids") val droppedIds: List<String> = emptyList(),
    @SerialName("truncated_ids") val truncatedIds: List<String> = emptyList(),
)

@Serializable
data class AgentCoreContextArchiveRequest(
    @SerialName("archive_id") val archiveId: String,
    @SerialName("archive_digest") val archiveDigest: String,
    @SerialName("message_indices") val messageIndices: List<Int>,
    @SerialName("message_ids") val messageIds: List<String>,
    val messages: List<JsonObject>,
    @SerialName("existing_summary") val existingSummary: String? = null,
    @SerialName("target_summary_tokens") val targetSummaryTokens: Int,
)

@Serializable
data class AgentCoreContextPrepareOutput(
    @SerialName("schema_version") val schemaVersion: Int,
    val action: String,
    val messages: List<JsonObject>,
    val tools: List<JsonElement> = emptyList(),
    @SerialName("payload_digest") val payloadDigest: String,
    val changed: Boolean = false,
    val reason: String = "",
    val metrics: AgentCoreContextMetrics,
    @SerialName("component_metrics") val componentMetrics: List<AgentCoreContextComponentMetrics> = emptyList(),
    @SerialName("archive_request") val archiveRequest: AgentCoreContextArchiveRequest? = null,
)

internal object AgentCoreContextNative {
    private val loaded: Boolean = runCatching {
        System.loadLibrary("nola_core_jni")
    }.isSuccess

    val isAvailable: Boolean
        get() = loaded && runCatching {
            nativeContextPrepareAbiVersion() == AGENT_CORE_CONTEXT_ABI_VERSION
        }.getOrDefault(false)

    external fun nativeContextPrepareAbiVersion(): Int
    external fun nativeContextPrepare(input: ByteArray): ByteArray?
}

object AgentCoreContextPreparation {
    private val json = Json {
        ignoreUnknownKeys = true
        explicitNulls = false
        encodeDefaults = true
    }

    val isAvailable: Boolean get() = AgentCoreContextNative.isAvailable

    fun encodeInput(input: AgentCoreContextPrepareInput): ByteArray =
        json.encodeToString(input).encodeToByteArray()

    fun decodeOutput(output: ByteArray): AgentCoreContextPrepareOutput =
        json.decodeFromString(AgentCoreContextPrepareOutput.serializer(), output.decodeToString())

    fun prepare(input: AgentCoreContextPrepareInput): AgentCoreContextPrepareOutput? {
        if (!AgentCoreContextNative.isAvailable) return null
        return AgentCoreContextNative.nativeContextPrepare(encodeInput(input))?.let(::decodeOutput)
    }
}
