package com.nola.ui.screen.chat

import com.nola.agent.trace.AgentTraceProjection
import com.nola.core.AgentCoreSkillRuntimeProjection
import com.nola.core.AgentCoreTaskProjection
import com.nola.core.AndroidAgentPermissionMode
import com.nola.domain.model.ConversationGenerationOverride
import com.nola.domain.model.ProcessingState

/** Preserve application/model defaults, retire every previous conversation field.
 * Null usage detail means unmeasured, not a measured zero cache-hit rate. */
internal fun ChatUiState.newConversationPresentation(defaultPermission: AndroidAgentPermissionMode): ChatUiState = copy(
    conversationId = null,
    taskChildOwner = null,
    configurationRevision = 0,
    executionHostBinding = null,
    messages = emptyList(),
    historyBeforeOrdinal = null,
    hasMoreHistory = false,
    compactionSourceTargetId = null,
    compactionSourceRequest = compactionSourceRequest + 1,
    compactionSourceStatus = CompactionSourceStatus.Idle,
    unknownEffectRecovery = null,
    resolvingUnknownEffects = false,
    processingState = ProcessingState.Idle,
    pendingExpertHostAdmission = null,
    expertHostConfirmationStatus = null,
    checkingExpertHostAdmission = false,
    currentResponse = "",
    journalResponse = null,
    activeResponseId = null,
    revealResponseId = null,
    activeTurnId = null,
    activeResponseStartedAt = null,
    thinking = "",
    thinkingFinished = false,
    toolEvents = emptyList(),
    permissionRequest = null,
    contextUsage = 0f,
    contextTokens = 0,
    contextMaxTokens = currentParameters.maxContextTokens,
    contextUsageDetail = null,
    permissionMode = defaultPermission,
    permissionSaving = false,
    permissionSaveFailed = false,
    permissionCommitRevision = 0,
    permissionPolicyRunId = null,
    permissionPolicyRevision = 0,
    remotePermissionPolicy = null,
    remotePermissionApplicationState = null,
    permissionConfigurationOwner = null,
    currentExecutionBackend = "nola",
    responseStyleOverride = null,
    generationOverride = ConversationGenerationOverride(),
    sessionMemory = "",
    knowledgePoints = "",
    conversationWiki = "",
    selectedDataSourceIds = emptyList(),
    agentCoreSnapshot = null,
    agentPermissionRequest = null,
    agentPermissionAction = null,
    answerSources = emptyList(),
    coreTaskProjection = AgentCoreTaskProjection(),
    coreSkillProjection = AgentCoreSkillRuntimeProjection(),
    traceProjection = AgentTraceProjection(),
    error = null,
)
