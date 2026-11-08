package com.nola.ui.screen.chat

import com.nola.agent.trace.AgentTraceProjection
import com.nola.core.AgentCoreSnapshot
import com.nola.core.AgentCoreTaskProjection
import com.nola.core.AndroidAgentPermissionAction
import com.nola.core.AndroidAgentPermissionRequest
import com.nola.data.remote.dto.ExecutionBackendDto
import com.nola.domain.model.AnswerSource
import com.nola.domain.model.ChatRole
import com.nola.domain.model.ContextBreakdown
import com.nola.domain.model.ContextUsage
import com.nola.domain.model.Conversation
import com.nola.domain.model.ConversationGenerationOverride
import com.nola.domain.model.ConversationMessage
import com.nola.domain.model.CustomDataSourceConfig
import com.nola.domain.model.HostAgentPermissionRequest
import com.nola.domain.model.HostAgentToolEvent
import com.nola.domain.model.HostComputerUseStatus
import com.nola.domain.model.HostNativeBrowserStatus
import com.nola.domain.model.McpServerConfig
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelParameters
import com.nola.domain.model.ProcessingState
import com.nola.domain.model.UnifiedDataSource
import com.nola.domain.model.UploadedContent
import com.nola.domain.projection.asLegacyLongOrNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive

/** A rendered error may acknowledge only its original immutable snapshot. */
internal fun ChatUiState.dismissingDisplayedError(expected: ChatUiState): ChatUiState =
    if (this === expected) copy(error = null) else this

data class ChatUiState(
    val taskChildOwner: com.nola.domain.projection.ConversationProjectionV1.TaskChildOwner? = null,
    val rejectedQueuedUiAction: com.nola.data.remote.api.RemoteSubmissionReceipt? = null,
    val cancellingQueuedUiAction: Boolean = false,
    val messages: List<ConversationMessage> = emptyList(),
    val historyBeforeOrdinal: Long? = null,
    val hasMoreHistory: Boolean = false,
    val compactionSourceTargetId: String? = null,
    val compactionSourceRequest: Long = 0,
    val compactionSourceStatus: CompactionSourceStatus = CompactionSourceStatus.Idle,
    val conversations: List<Conversation> = emptyList(),
    val conversationId: String? = null,
    val unknownEffectRecovery: UnknownEffectRecoveryPresentation? = null,
    val resolvingUnknownEffects: Boolean = false,
    val processingState: ProcessingState = ProcessingState.Idle,
    val executionTeardownPending: Boolean = false,
    val currentResponse: String = "",
    val journalResponse: ConversationMessage? = null,
    val activeResponseId: String? = null,
    val revealResponseId: String? = null,
    val activeTurnId: String? = null,
    val activeResponseStartedAt: Long? = null,
    val thinking: String = "",
    val thinkingFinished: Boolean = false,
    val toolEvents: List<HostAgentToolEvent> = emptyList(),
    val permissionRequest: HostAgentPermissionRequest? = null,
    val contextUsage: Float = 0f,
    val contextTokens: Int = 0,
    val contextMaxTokens: Int = 32000,
    val agentMode: Boolean = false,
    val permissionMode: com.nola.core.AndroidAgentPermissionMode = com.nola.core.AndroidAgentPermissionMode.Cautious,
    val permissionSaving: Boolean = false,
    val permissionSaveFailed: Boolean = false,
    val permissionConfigurationOwner: com.nola.data.local.PermissionConfigurationOwner? = null,
    val permissionCommitRevision: Long = 0,
    val permissionPolicyRunId: String? = null,
    val permissionPolicyRevision: Long = 0,
    val remotePermissionApplicationState: String? = null,
    val remotePermissionPolicy: com.nola.core.AgentActivePermissionPolicy? = null,
    val hybridDeviceToolsEnabled: Boolean = false,
    val hybridDeviceCapabilities: Set<String> = emptySet(),
    val pairedHostIdentity: String = "",
    val currentModel: ModelInfo? = null,
    val configurationRevision: Long = 0,
    val executionHostBinding: com.nola.domain.model.ExecutionHostBinding? = null,
    val availableModels: List<ModelInfo> = emptyList(),
    val executionBackends: List<ExecutionBackendDto> = emptyList(),
    val currentExecutionBackend: String = "nola",
    val responseStyleOverride: String? = null,
    val sessionMemory: String = "",
    val knowledgePoints: String = "",
    val conversationWiki: String = "",
    val longTermMemory: String = "",
    val userPreferences: String = "",
    val uploadedSkills: List<UploadedContent> = emptyList(),
    val hostUploadedSkills: List<UploadedContent> = emptyList(),
    val hostUploadedDocuments: List<UploadedContent> = emptyList(),
    val hostMcpServers: List<McpServerConfig> = emptyList(),
    val hostComputerUseStatus: HostComputerUseStatus? = null,
    val hostNativeBrowserStatus: HostNativeBrowserStatus? = null,
    val uploadedDocuments: List<UploadedContent> = emptyList(),
    val mcpServers: List<McpServerConfig> = emptyList(),
    val dataSources: List<UnifiedDataSource> = emptyList(),
    val customDataSources: List<CustomDataSourceConfig> = emptyList(),
    val selectedDataSourceIds: List<String> = emptyList(),
    val autoCompactTriggerPercent: Int = 80,
    val currentParameters: ModelParameters = ModelParameters(),
    val generationOverride: ConversationGenerationOverride = ConversationGenerationOverride(),
    val contextUsageDetail: com.nola.domain.model.ContextUsage? = null,
    val agentCoreSnapshot: AgentCoreSnapshot? = null,
    val agentPermissionRequest: AndroidAgentPermissionRequest? = null,
    val agentPermissionAction: AndroidAgentPermissionAction? = null,
    val answerSources: List<AnswerSource> = emptyList(),
    val coreTaskProjection: AgentCoreTaskProjection = AgentCoreTaskProjection(),
    val coreSkillProjection: com.nola.core.AgentCoreSkillRuntimeProjection =
        com.nola.core.AgentCoreSkillRuntimeProjection(),
    val traceProjection: AgentTraceProjection = AgentTraceProjection(),
    val error: String? = null,
    val pendingCommandId: String? = null,
    val pendingExpertHostAdmission: com.nola.data.local.AndroidPendingExpertAdmission? = null,
    val expertHostConfirmationStatus: String? = null,
    val checkingExpertHostAdmission: Boolean = false,
) {
    // Updated only at admission/observation/save boundaries, never by a render-time query.
    val permissionApplicationStatus: String get() {
        if (permissionSaving) return "saving"
        if (permissionSaveFailed) return "failed"
        when (val owner = permissionConfigurationOwner) {
            null -> return "checking"
            com.nola.data.local.PermissionConfigurationOwner.Inactive -> return "saved"
            is com.nola.data.local.PermissionConfigurationOwner.Remote -> {
                if (permissionPolicyRunId != null && permissionPolicyRunId != owner.identity.submissionId) return "checking"
                return when (remotePermissionApplicationState) {
                    "inactive" -> "saved"
                    "pending" -> "pending"
                    "active" -> "active"
                    else -> "checking"
                }
            }
            is com.nola.data.local.PermissionConfigurationOwner.Local -> if (owner.key.conversationId != conversationId ||
                owner.key.runId != agentCoreSnapshot?.turnId) return "pending"
        }
        val snapshot = agentCoreSnapshot
        val policy = snapshot?.permissions?.activePolicy
        return if (snapshot?.sessionId == conversationId && policy != null && policy.runId == snapshot.turnId &&
            policy.profile == permissionMode.wireValue && policy.configurationRevision >= permissionCommitRevision) "active" else "pending"
    }
    val responseStyle: String get() = responseStyleOverride ?: currentParameters.responseStyle
    val effectiveGenerationParameters: ModelParameters get() = generationOverride.applyingTo(currentParameters)
}

enum class CompactionSourceStatus { Idle, Loading, Unavailable, Failed }

internal fun ChatUiState.consumeCompactionSourceTarget(request: Long): ChatUiState =
    if (compactionSourceRequest == request && compactionSourceTargetId != null) {
        copy(compactionSourceTargetId = null)
    } else {
        this
    }

private const val CONTEXT_USED_TOKENS_META_KEY = "nola.context.usedTokens"
internal const val AGENT_CORE_TURN_ID_META_KEY = "nola.agent.turnId"
private const val CONTEXT_LIMIT_TOKENS_META_KEY = "nola.context.limitTokens"
private const val CONTEXT_CACHE_READ_TOKENS_META_KEY = "nola.context.cacheReadTokens"
private const val CONTEXT_CACHE_WRITE_TOKENS_META_KEY = "nola.context.cacheWriteTokens"
private const val CONTEXT_CACHE_STATUS_META_KEY = "nola.context.cacheStatus"

internal data class RestoredConversationContextUsage(
    val usedTokens: Int,
    val limitTokens: Int,
    val cachedInputTokens: Int? = null,
    val cacheWriteInputTokens: Int? = null,
    val promptCacheStatus: String? = null,
) {
    val fraction: Float
        get() = (usedTokens.toFloat() / limitTokens.coerceAtLeast(1).toFloat())
            .coerceIn(0f, 1f)
}

internal fun contextUsageMessageMeta(state: ChatUiState): Map<String, String> {
    if (state.contextTokens <= 0 || state.contextMaxTokens <= 0) return emptyMap()
    return mutableMapOf(
        CONTEXT_USED_TOKENS_META_KEY to state.contextTokens.toString(),
        CONTEXT_LIMIT_TOKENS_META_KEY to state.contextMaxTokens.toString(),
    ).apply {
        state.contextUsageDetail?.cachedInputTokens?.let {
            put(CONTEXT_CACHE_READ_TOKENS_META_KEY, it.toString())
        }
        state.contextUsageDetail?.cacheWriteInputTokens?.let {
            put(CONTEXT_CACHE_WRITE_TOKENS_META_KEY, it.toString())
        }
        state.contextUsageDetail?.promptCacheStatus?.let {
            put(CONTEXT_CACHE_STATUS_META_KEY, it)
        }
    }
}

internal fun restoredConversationContextUsage(
    messages: List<ConversationMessage>,
    fallbackLimitTokens: Int,
): RestoredConversationContextUsage {
    val meta = messages
        .asReversed()
        .firstOrNull { message ->
            message.role == ChatRole.assistant &&
                message.meta?.containsKey(CONTEXT_USED_TOKENS_META_KEY) == true
        }
        ?.meta
    val usedTokens = meta?.get(CONTEXT_USED_TOKENS_META_KEY)?.toIntOrNull()?.coerceAtLeast(0) ?: 0
    val limitTokens = meta
        ?.get(CONTEXT_LIMIT_TOKENS_META_KEY)
        ?.toIntOrNull()
        ?.takeIf { it > 0 }
        ?: fallbackLimitTokens.coerceAtLeast(1)
    return RestoredConversationContextUsage(
        usedTokens = usedTokens,
        limitTokens = limitTokens,
        cachedInputTokens = meta?.get(CONTEXT_CACHE_READ_TOKENS_META_KEY)?.toIntOrNull(),
        cacheWriteInputTokens = meta?.get(CONTEXT_CACHE_WRITE_TOKENS_META_KEY)?.toIntOrNull(),
        promptCacheStatus = meta?.get(CONTEXT_CACHE_STATUS_META_KEY),
    )
}

internal fun ChatUiState.beginAssistantTurn(
    responseId: String,
    startedAt: Long,
): ChatUiState = copy(
    permissionCommitRevision = 0,
    permissionConfigurationOwner = null,
    permissionPolicyRunId = null,
    permissionPolicyRevision = 0,
    remotePermissionPolicy = null,
    remotePermissionApplicationState = null,
    processingState = ProcessingState.BuildingContext,
    journalResponse = null,
    currentResponse = "",
    activeResponseId = responseId,
    revealResponseId = null,
    activeTurnId = responseId.removeSuffix(":assistant"),
    activeResponseStartedAt = startedAt,
    thinking = "",
    thinkingFinished = false,
    toolEvents = emptyList(),
    permissionRequest = null,
    answerSources = emptyList(),
    coreTaskProjection = AgentCoreTaskProjection(),
    coreSkillProjection = com.nola.core.AgentCoreSkillRuntimeProjection(),
    traceProjection = AgentTraceProjection(),
    error = null,
    pendingCommandId = null,
    pendingExpertHostAdmission = null,
    expertHostConfirmationStatus = null,
    checkingExpertHostAdmission = false,
)

internal fun ChatUiState.activeAssistantMessage(): ConversationMessage? {
    val responseId = activeResponseId ?: return null
    journalResponse?.takeIf { it.id == responseId && it.conversationId == conversationId }?.let { return it }
    val hasVisibleActivity = processingState.isGenerating ||
        currentResponse.isNotEmpty() || thinking.isNotEmpty() || toolEvents.isNotEmpty()
    if (!hasVisibleActivity) return null
    return ConversationMessage(
        id = responseId,
        conversationId = conversationId.orEmpty(),
        role = ChatRole.assistant,
        text = currentResponse,
        createdAt = activeResponseStartedAt ?: System.currentTimeMillis(),
    )
}

internal fun ChatUiState.timelineMessages(): List<ConversationMessage> {
    val active = activeAssistantMessage() ?: return messages
    val existingIndex = messages.indexOfFirst { it.id == active.id }
    if (existingIndex < 0) return messages + active
    return messages.toMutableList().also { it[existingIndex] = active }
}

internal fun ChatUiState.completeAssistantTurn(message: ConversationMessage): ChatUiState {
    val existingIndex = messages.indexOfFirst { it.id == message.id }
    val completedMessages = if (existingIndex < 0) {
        messages + message
    } else {
        messages.toMutableList().also { it[existingIndex] = message }
    }
    return copy(
        messages = completedMessages,
        processingState = ProcessingState.Idle,
        journalResponse = null,
        currentResponse = "",
        activeResponseId = null,
        revealResponseId = message.id,
        activeTurnId = null,
        activeResponseStartedAt = null,
        thinking = "",
        thinkingFinished = false,
        toolEvents = emptyList(),
        permissionRequest = null,
    )
}

internal fun ChatUiState.applyTraceProjection(
    projection: AgentTraceProjection,
): ChatUiState {
    // Request-scoped diagnostics and the cumulative physical ledger are read
    // from canonical usage. A trace carries no authority to replace either.
    return copy(
        traceProjection = projection,
        processingState = when {
            projection.isTerminal -> processingState
            else -> ProcessingState.Streaming
        },
    )
}

internal fun ChatUiState.applyCanonicalContextUsage(
    usage: com.nola.domain.projection.ConversationProjectionV1.ContextUsage,
): ChatUiState {
    val used = usage.usedTokens.coerceIn(0, Int.MAX_VALUE.toLong()).toInt()
    val limit = usage.windowTokens.coerceIn(0, Int.MAX_VALUE.toLong()).toInt()
    val fraction = (used.toFloat() / limit.coerceAtLeast(1)).coerceIn(0f, 1f)
    val diagnostic = usage.promptCacheDiagnostic.takeIf { usage.promptCacheMeasurement != null }
    fun kotlinx.serialization.json.JsonObject?.text(key: String): String? =
        (this?.get(key) as? kotlinx.serialization.json.JsonPrimitive)?.content?.takeUnless { it == "null" }
    val totals = usage.conversationCacheUsage
    return copy(contextTokens = used, contextMaxTokens = limit, contextUsage = fraction,
        contextUsageDetail = (contextUsageDetail ?: com.nola.domain.model.ContextUsage(
            totalTokens = used, maxTokens = limit, usagePercent = fraction * 100f,
            breakdown = com.nola.domain.model.ContextBreakdown(),
        )).copy(totalTokens = used, maxTokens = limit, usagePercent = fraction * 100f,
            contextIsEstimated = usage.estimated, measuredRequestId = usage.measurement.text("request_id"),
            cacheDiagnosticRequestId = usage.promptCacheMeasurement.text("request_id"),
            cacheDiagnosticAttemptId = usage.promptCacheMeasurement.text("attempt_id"),
            promptCacheStatus = diagnostic.text("status"), cachedInputTokens = diagnostic.text("cacheReadInputTokens")?.toIntOrNull(),
            cacheWriteInputTokens = diagnostic.text("cacheWriteInputTokens")?.toIntOrNull(),
            cacheHitRateBps = if (totals != null) totals.text("hit_rate_bps")?.toIntOrNull() else contextUsageDetail?.cacheHitRateBps,
            cacheReportedRequests = if (totals != null) totals.text("reported_requests")?.toLongOrNull() else contextUsageDetail?.cacheReportedRequests,
            cacheUnreportedRequests = if (totals != null) totals.text("unreported_requests")?.toLongOrNull() else contextUsageDetail?.cacheUnreportedRequests))
}

/** Absence of a Run does not erase a committed rejection of its submission. */
internal fun ChatUiState.withRemoteOwnerAbsent(preserveSubmissionOutcome: Boolean): ChatUiState = copy(
    processingState = ProcessingState.Idle, permissionRequest = null,
    activeResponseId = null, activeResponseStartedAt = null,
    error = if (preserveSubmissionOutcome) error else null,
    permissionConfigurationOwner = com.nola.data.local.PermissionConfigurationOwner.Inactive,
)
