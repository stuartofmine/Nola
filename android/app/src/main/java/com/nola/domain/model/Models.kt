package com.nola.domain.model

import kotlinx.serialization.Serializable
import kotlinx.serialization.SerialName
import kotlinx.serialization.encodeToString
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.json.Json
import com.nola.util.hostToolContainsSensitiveData
import com.nola.util.redactHostToolText
import java.util.UUID

const val HOST_TOOL_EVENTS_META_KEY = "hostToolEvents"
const val AGENT_THINKING_META_KEY = "agentThinkingText"
const val AGENT_TRACE_PHASE_META_KEY = "agentTracePhase"
const val ANSWER_SOURCES_META_KEY = "agentAnswerSources"

private val hostToolEventsJson = Json {
    ignoreUnknownKeys = true
    prettyPrint = false
}

enum class ChatRole { user, assistant, system }

@Serializable
data class CompactedMemoryEntry(
    val id: String = UUID.randomUUID().toString(),
    @Serializable(with = CanonicalTimestampSerializer::class) val createdAt: Long = System.currentTimeMillis(),
    val summary: String,
    val rawExcerpt: String,
    @SerialName("archivedMessageIDs") val archivedMessageIds: List<String>
)

@Serializable
data class ExecutionHostBinding(val baseURL: String, val tlsCertificateSHA256: String? = null)

@Serializable
data class Conversation(
    val id: String = UUID.randomUUID().toString(),
    val title: String? = null,
    val createdAt: Long = System.currentTimeMillis(),
    val lastActiveAt: Long = System.currentTimeMillis(),
    val language: String? = null,
    val sessionMemory: String = "",
    val knowledgePoints: String = "",
    val wiki: String = "",
    val lastCompactedAt: Long? = null,
    val selectedPromptTemplateId: String? = null,
    val dataSourceIds: List<String> = emptyList(),
    val modelSelectionID: String? = null,
    val agentRoomOwner: AgentRoomContextOwner? = null,
    val configurationRevision: Long = 0,
    val executionHostBinding: ExecutionHostBinding? = null,
    @kotlinx.serialization.Transient val selectedModelInfo: ModelInfo? = null,
    val generationOverrides: Map<String, ConversationGenerationOverride> = emptyMap(),
    val promptResponseStyle: String? = null,
    val permissionMode: String? = null,
    val executionBackend: String = "nola",
    val externalExecutionSessions: Map<String, String> = emptyMap(),
    val persistenceEnabled: Boolean = true,
    val compactedMemoryEntries: List<CompactedMemoryEntry> = emptyList()
)

@Serializable
data class AgentRoomContextOwner(
    @kotlinx.serialization.SerialName("room_id") val roomId: String,
    @kotlinx.serialization.SerialName("participant_id") val participantId: String,
    @kotlinx.serialization.SerialName("agent_id") val agentId: String,
)

@Serializable
data class ConversationMessage(
    val id: String = UUID.randomUUID().toString(),
    val conversationId: String,
    val role: ChatRole,
    val text: String,
    val createdAt: Long = System.currentTimeMillis(),
    val meta: Map<String, String>? = null,
    @kotlinx.serialization.Transient
    val historicalParts: List<com.nola.domain.projection.ConversationProjectionV1.Part.Historical> = emptyList(),
    @kotlinx.serialization.Transient
    val canonicalParts: List<com.nola.domain.projection.ConversationProjectionV1.Part> = emptyList(),
)

@Serializable
enum class ModelWireProtocol(val wireValue: String) {
    @SerialName("chat_completions") ChatCompletions("chat_completions"),
    @SerialName("responses") Responses("responses");

    companion object {
        fun fromWireValue(value: String): ModelWireProtocol? = entries.firstOrNull { it.wireValue == value }
    }
}

@Serializable
enum class ChatOutputTokenField {
    @SerialName("auto") Auto,
    @SerialName("max_tokens") MaxTokens,
    @SerialName("max_completion_tokens") MaxCompletionTokens;

    fun requestKey(baseUrl: String): String = when (this) {
        MaxTokens -> "max_tokens"
        MaxCompletionTokens -> "max_completion_tokens"
        Auto -> if (runCatching { java.net.URI(baseUrl).host?.equals("api.openai.com", ignoreCase = true) }.getOrNull() == true)
            "max_completion_tokens" else "max_tokens"
    }
}

@Serializable
data class ModelExecutionCapabilities(
    val toolCalls: Boolean = true,
    val imageInput: Boolean = false,
    val reasoning: Boolean = false,
    val samplingParameters: Boolean = false,
    val samplingRequiresNoReasoning: Boolean = false,
    val chatOutputTokenField: ChatOutputTokenField = ChatOutputTokenField.Auto,
)

@Serializable
data class ModelInfo(
    val id: String,
    val name: String,
    val source: ModelSource = ModelSource.Cloud,
    val isVisionModel: Boolean = false,
    /** Canonical profile identity; `id` remains the provider wire model name. */
    val profileId: String? = null,
    val profileRevision: Long? = null,
    val executionCapabilities: ModelExecutionCapabilities = ModelExecutionCapabilities(toolCalls = true),
    val executionCapabilitiesDeclared: Boolean = true,
) {
    enum class ModelSource { Local, Cloud, WebHost }
    val selectionId: String get() = profileId ?: id
    val parameterProfileId: String get() = profileId ?: id
}

@Serializable
data class ModelParameters(
    val temperature: Float = 0.7f,
    val topP: Float = 0.9f,
    val topK: Int? = null,
    @kotlinx.serialization.SerialName("maxOutputTokens") val maxCompletionTokens: Int = 16_384,
    @kotlinx.serialization.SerialName("maxContextLength") val maxContextTokens: Int = 65536,
    val presencePenalty: Double = 0.0,
    val frequencyPenalty: Double = 0.0,
    val repetitionPenalty: Double? = null,
    val seed: Long? = null,
    val stopSequences: String = "",
    val responseStyle: String = "casual",
    @kotlinx.serialization.SerialName("showThinking") val enableThinking: Boolean = true,
    val enableRetrieval: Boolean = false,
    val streamOutput: Boolean = true,
    val customSystemPrompt: String = "",
    val reasoningTier: String? = null,
    val reasoningPreference: String? = null
)

@Serializable
data class ChatMemoryProfile(
    @kotlinx.serialization.Transient
    val canonicalMemorySnapshot: kotlinx.serialization.json.JsonArray? = null,
    val longTermMemory: List<String> = emptyList(),
    val memoryRecords: List<StructuredMemoryRecord> = emptyList(),
    val userPreferences: List<String> = emptyList(),
    val mcpServers: List<McpServerConfig> = emptyList(),
    val uploadedSkills: List<UploadedContent> = emptyList(),
    val uploadedDocuments: List<UploadedContent> = emptyList(),
    val customDataSources: List<CustomDataSourceConfig> = emptyList(),
    val dataSourceUsageRecords: List<DataSourceUsageRecord> = emptyList()
)

@Serializable
data class StructuredMemoryRecord(
    val id: String,
    val owner: kotlinx.serialization.json.JsonObject,
    val namespace: String = "personal",
    val scope: String = "global",
    val kind: String = "reference",
    val content: String,
    val confidence: Double = 0.6,
    val sensitivity: String? = null,
    val provenanceKind: String = "imported",
    val sourceReference: String? = null,
    val observedAt: Long? = null,
    val createdAt: Long = System.currentTimeMillis(),
    val updatedAt: Long = createdAt,
    val retention: String = "permanent",
    val expiresAt: Long? = null,
    val supersedes: List<String> = emptyList(),
    val tags: List<String> = emptyList(),
    val contentHash: String? = null,
    val previousVersionId: String? = null
)

@Serializable
data class McpServerConfig(
    @kotlinx.serialization.Transient
    val canonicalRevision: Long? = null,
    @kotlinx.serialization.Transient
    val canonicalConfiguration: kotlinx.serialization.json.JsonObject? = null,
    val name: String,
    val url: String,
    val enabled: Boolean = true,
    val serverId: String = name,
    val availability: String? = null,
    val protocolVersion: String? = null,
    val tools: List<String> = emptyList(),
    val resources: List<String> = emptyList(),
    val resourceTemplates: List<String> = emptyList(),
    val prompts: List<String> = emptyList(),
    val resourceId: String? = null,
    val accessOwnerId: String? = null,
    val accessRevision: Long = 1,
    val accessGrants: List<ResourceAccessGrantRecord> = emptyList(),
)

@Serializable
data class ResourceAccessGrantRecord(
    val grantId: String,
    val principalKind: String,
    val principalId: String,
    val permissions: List<String>,
    val hostId: String? = null,
    val resourceRevision: Long? = null,
    val contentDigest: String? = null,
    val expiresAtEpochMs: Long? = null,
)

@Serializable
data class UploadedContent(
    @kotlinx.serialization.Transient
    val canonicalRevision: Long? = null,
    @kotlinx.serialization.Transient
    val canonicalSkillRecord: kotlinx.serialization.json.JsonObject? = null,
    val name: String,
    val content: String,
    val kind: String = "text",
    val enabled: Boolean = true,
    val description: String = "",
    val version: String? = null,
    val requiredTools: List<String> = emptyList(),
    val validationSteps: List<String> = emptyList(),
    val resourceId: String? = null,
    val accessOwnerId: String? = null,
    val accessRevision: Long = 1,
    val accessGrants: List<ResourceAccessGrantRecord> = emptyList(),
)

data class HostComputerUseHost(
    val hostId: String,
    val capabilities: List<String> = emptyList(),
    val lastSeenAgeSec: Double? = null
)

data class HostComputerUseStatus(
    val featureEnabled: Boolean = false,
    val tokenConfigured: Boolean = false,
    val connected: Boolean = false,
    val hosts: List<HostComputerUseHost> = emptyList(),
    val tools: List<String> = emptyList()
)

data class HostNativeBrowserHost(
    val hostId: String,
    val capabilities: List<String> = emptyList(),
    val lastSeenAgeSec: Double? = null
)

data class HostNativeBrowserStatus(
    val tokenConfigured: Boolean = false,
    val connected: Boolean = false,
    val structuredSearch: Boolean = false,
    val hosts: List<HostNativeBrowserHost> = emptyList(),
    val pendingCommands: Int = 0,
    val inFlightCommands: Int = 0
)

@Serializable
data class ContextUsage(
    val totalTokens: Int,
    val maxTokens: Int,
    val usagePercent: Float,
    val breakdown: ContextBreakdown,
    val suggestions: List<ContextSuggestion> = emptyList(),
    val cachedInputTokens: Int? = null,
    val cacheWriteInputTokens: Int? = null,
    val promptCacheStatus: String? = null,
    val contextIsEstimated: Boolean = false,
    val measuredRequestId: String? = null,
    val cacheDiagnosticRequestId: String? = null,
    val cacheDiagnosticAttemptId: String? = null,
    val cacheHitRateBps: Int? = null,
    val cacheReportedRequests: Long? = null,
    val cacheUnreportedRequests: Long? = null,
)

@Serializable
data class ContextBreakdown(
    val messages: Int = 0,
    val sessionMemory: Int = 0,
    val longTermMemory: Int = 0,
    val userPreferences: Int = 0,
    val mcpTools: Int = 0,
    val uploadedSkills: Int = 0,
    val uploadedDocuments: Int = 0,
    val dataSources: Int = 0,
    val scaffold: Int = 0
)

@Serializable
data class ContextSuggestion(
    val title: String,
    val detail: String,
    val severity: String = "info",
    val estimatedSavingsTokens: Int = 0
)

data class MemorySearchHit(
    val source: String,
    val preview: String,
    val score: Float,
    val timestamp: Long? = null
)

data class HostAgentToolEvent(
    val id: String,
    val toolName: String,
    val args: Map<String, Any> = emptyMap(),
    val argsPreview: String = "",
    val status: String = "running",
    val resultSummary: String? = null,
    val detailPreview: String? = null,
    val fullOutputRef: String? = null,
    val recovery: String? = null,
    val nextAction: String? = null,
    val screenshotUrl: String? = null,
    val finalUrl: String? = null,
    val pageTitle: String? = null,
    val downloadUrl: String? = null,
    val downloadFilename: String? = null,
    val screenshotWarning: String? = null,
    val sensitive: Boolean = false,
    val error: Boolean = false
)

@Serializable
data class AnswerSource(
    val id: String,
    val sourceId: String,
    val title: String,
    val uri: String,
    val mimeType: String? = null,
    val page: Long? = null,
    val lineStart: Long? = null,
    val lineEnd: Long? = null,
    val section: String? = null,
    val memoryScope: String? = null,
    val memoryNamespace: String? = null,
    val memorySource: String? = null,
    val memoryObservedAt: String? = null,
    val memoryUsageCount: Long = 0,
    val memorySupersedesCount: Int = 0,
    val memoryRequiresVerification: Boolean = false,
    val memoryTruncated: Boolean = false,
)

fun encodeAnswerSourcesForMeta(sources: List<AnswerSource>): Map<String, String> =
    if (sources.isEmpty()) emptyMap()
    else mapOf(ANSWER_SOURCES_META_KEY to hostToolEventsJson.encodeToString(sources))

fun decodeAnswerSourcesFromMeta(meta: Map<String, String>?): List<AnswerSource> =
    meta?.get(ANSWER_SOURCES_META_KEY)?.takeIf(String::isNotBlank)?.let { raw ->
        runCatching { hostToolEventsJson.decodeFromString<List<AnswerSource>>(raw) }
            .getOrDefault(emptyList())
    }.orEmpty()

@Serializable
private data class HostAgentToolEventSnapshot(
    val id: String,
    val toolName: String,
    val argsPreview: String = "",
    val status: String,
    val resultSummary: String? = null,
    val detailPreview: String? = null,
    val fullOutputRef: String? = null,
    val recovery: String? = null,
    val nextAction: String? = null,
    val screenshotUrl: String? = null,
    val finalUrl: String? = null,
    val pageTitle: String? = null,
    val downloadUrl: String? = null,
    val downloadFilename: String? = null,
    val screenshotWarning: String? = null,
    val sensitive: Boolean = false,
    val error: Boolean = false
)

fun encodeHostToolEventsForMeta(events: List<HostAgentToolEvent>): Map<String, String>? {
    if (events.isEmpty()) return null
    val snapshots = events.map { event ->
        val sensitive = event.sensitive || hostToolContainsSensitiveData(
            event.args.toString(),
            event.resultSummary,
            event.detailPreview,
            event.fullOutputRef,
            event.nextAction,
            event.screenshotUrl,
            event.finalUrl,
            event.pageTitle,
            event.downloadUrl,
            event.downloadFilename,
            event.screenshotWarning
        )
        HostAgentToolEventSnapshot(
            id = event.id,
            toolName = event.toolName,
            argsPreview = if (sensitive) "" else redactHostToolText(
                event.argsPreview.ifBlank { event.args.toString() }
            ),
            status = event.status,
            resultSummary = event.resultSummary?.let(::redactHostToolText).takeUnless { sensitive },
            detailPreview = event.detailPreview?.let(::redactHostToolText).takeUnless { sensitive },
            fullOutputRef = event.fullOutputRef?.let(::redactHostToolText).takeUnless { sensitive },
            recovery = event.recovery,
            nextAction = event.nextAction?.let(::redactHostToolText).takeUnless { sensitive },
            screenshotUrl = event.screenshotUrl.takeUnless { sensitive },
            finalUrl = event.finalUrl.takeUnless { sensitive },
            pageTitle = event.pageTitle?.let(::redactHostToolText).takeUnless { sensitive },
            downloadUrl = event.downloadUrl.takeUnless { sensitive },
            downloadFilename = event.downloadFilename?.let(::redactHostToolText).takeUnless { sensitive },
            screenshotWarning = event.screenshotWarning?.let(::redactHostToolText),
            sensitive = sensitive,
            error = event.error
        )
    }
    return mapOf(HOST_TOOL_EVENTS_META_KEY to hostToolEventsJson.encodeToString(snapshots))
}

fun decodeHostToolEventsFromMeta(meta: Map<String, String>?): List<HostAgentToolEvent> {
    val raw = meta?.get(HOST_TOOL_EVENTS_META_KEY)?.takeIf { it.isNotBlank() } ?: return emptyList()
    return try {
        hostToolEventsJson.decodeFromString<List<HostAgentToolEventSnapshot>>(raw).map { snapshot ->
            HostAgentToolEvent(
                id = snapshot.id,
                toolName = snapshot.toolName,
                argsPreview = snapshot.argsPreview,
                status = snapshot.status,
                resultSummary = snapshot.resultSummary,
                detailPreview = snapshot.detailPreview,
                fullOutputRef = snapshot.fullOutputRef,
                recovery = snapshot.recovery,
                nextAction = snapshot.nextAction,
                screenshotUrl = snapshot.screenshotUrl,
                finalUrl = snapshot.finalUrl,
                pageTitle = snapshot.pageTitle,
                downloadUrl = snapshot.downloadUrl,
                downloadFilename = snapshot.downloadFilename,
                screenshotWarning = snapshot.screenshotWarning,
                sensitive = snapshot.sensitive,
                error = snapshot.error
            )
        }
    } catch (_: Exception) {
        emptyList()
    }
}

fun encodeAgentTraceProjectionForMeta(
    thinking: String,
    phase: String?,
): Map<String, String> = buildMap {
    thinking.trim().takeIf(String::isNotEmpty)?.let { put(AGENT_THINKING_META_KEY, it) }
    phase?.trim()?.takeIf(String::isNotEmpty)?.let { put(AGENT_TRACE_PHASE_META_KEY, it) }
}

fun decodeAgentThinkingFromMeta(meta: Map<String, String>?): String =
    meta?.get(AGENT_THINKING_META_KEY).orEmpty()

fun decodeAgentTracePhaseFromMeta(meta: Map<String, String>?): String? =
    meta?.get(AGENT_TRACE_PHASE_META_KEY)?.trim()?.takeIf(String::isNotEmpty)

data class HostAgentPermissionRequest(
    val allowConversation: Boolean,
    val id: String,
    val toolName: String,
    val args: Map<String, Any> = emptyMap(),
    val match: String = "",
    val reason: String = "",
    val message: String = "",
    val nonce: String? = null,
    val hostId: String? = null,
    val clientId: String? = null,
    val sessionId: String? = null,
    val expiresAt: Long? = null,
    val protocolVersion: Int? = null,
    val deviceBridge: Boolean = false,
    val permissionInstructions: List<com.nola.core.AgentPermissionInstruction> = emptyList(),
)

/** Current Core timestamp contract. Numeric historical payloads are rejected. */
object CanonicalTimestampSerializer : kotlinx.serialization.KSerializer<Long> {
    override val descriptor = kotlinx.serialization.descriptors.PrimitiveSerialDescriptor("CanonicalTimestamp", kotlinx.serialization.descriptors.PrimitiveKind.STRING)
    override fun serialize(encoder: kotlinx.serialization.encoding.Encoder, value: Long) = encoder.encodeString(java.time.Instant.ofEpochMilli(value).toString())
    override fun deserialize(decoder: kotlinx.serialization.encoding.Decoder): Long = java.time.Instant.parse(decoder.decodeString()).toEpochMilli()
}
