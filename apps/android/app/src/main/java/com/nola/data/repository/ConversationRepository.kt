package com.nola.data.repository

import com.nola.data.local.AndroidApplicationRuntime
import com.nola.core.AndroidAgentPermissionMode
import com.nola.data.local.CanonicalDataSourceOverrides
import com.nola.data.local.CanonicalDataSourceOverride
import kotlinx.serialization.json.jsonObject
import com.nola.data.local.CanonicalConversation
import com.nola.data.local.CanonicalConversationMessage
import com.nola.data.remote.sse.SseEvent
import com.nola.domain.model.ChatRole
import com.nola.domain.model.CompactedMemoryEntry
import com.nola.domain.model.Conversation
import com.nola.domain.model.ConversationMessage
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ConversationGenerationOverride
import com.nola.domain.projection.ConversationProjectionV1
import com.nola.domain.projection.CanonicalConversationProjector
import com.nola.domain.projection.ConversationProjectionWindowV1
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonPrimitive
import java.util.UUID
import javax.inject.Inject
import javax.inject.Singleton

interface ConversationRepository {
    suspend fun createConversation(
        title: String? = null,
        language: String? = null,
        modelInfo: ModelInfo? = null,
        promptResponseStyle: String? = null,
        permissionMode: String? = null,
        parameters: com.nola.domain.model.ModelParameters? = null,
        hostBinding: com.nola.domain.model.ExecutionHostBinding? = null,
    ): Conversation
    suspend fun getConversation(id: String): Conversation?
    suspend fun getAllConversations(): List<Conversation>
    suspend fun getProjection(id: String): ConversationProjectionV1
    suspend fun getContextUsage(id: String): ConversationProjectionV1.ContextUsage = getProjection(id).contextUsage
    suspend fun getProjectionWindow(id: String, beforeOrdinal: Long?): ConversationProjectionWindowV1
    suspend fun prepareRemoteSubmission(
        sourceHostId: String,
        clientId: String,
        conversationId: String,
        conversationTitle: String,
        submissionId: String,
        userText: String,
        userMeta: Map<String, String>?,
        createdAtMs: Long,
        userMessageId: String = submissionId,
    ): com.nola.data.remote.api.RemoteSubmissionIntent
    suspend fun bindRemoteSubmissionRequest(identity: com.nola.data.remote.api.RemoteSubmissionIdentity, request: kotlinx.serialization.json.JsonObject): com.nola.data.remote.api.RemoteSubmissionIntent
    suspend fun readRemoteSubmissionIntent(conversationId: String): com.nola.data.remote.api.RemoteSubmissionIntent?
    suspend fun readRemoteSubmissionRequest(identity: com.nola.data.remote.api.RemoteSubmissionIdentity, requestDigest: String): kotlinx.serialization.json.JsonObject
    suspend fun acceptRemoteSubmissionReceipt(receipt: com.nola.data.remote.api.RemoteSubmissionReceipt): com.nola.data.remote.api.RemoteSubmissionIntent
    suspend fun abandonPreparedRemoteSubmission(identity: com.nola.data.remote.api.RemoteSubmissionIdentity): com.nola.data.remote.api.RemoteSubmissionIntent
    suspend fun adoptObservedRemoteRun(receipt: com.nola.data.remote.api.RemoteSubmissionReceipt)
    suspend fun getMessages(convId: String): List<ConversationMessage>
    suspend fun getActiveMessages(convId: String): List<ConversationMessage>
    suspend fun getArchivedMessages(convId: String): List<ConversationMessage>
    suspend fun deleteConversation(id: String)
    suspend fun updateTitle(id: String, title: String)
    suspend fun updateSessionMemory(id: String, memory: String)
    suspend fun updateKnowledge(id: String, knowledgePoints: String, wiki: String)
    suspend fun selectModel(id: String, model: ModelInfo, parameters: com.nola.domain.model.ModelParameters, expectedRevision: Long): Conversation
    suspend fun selectHost(id: String, binding: com.nola.domain.model.ExecutionHostBinding, parameters: com.nola.domain.model.ModelParameters, expectedRevision: Long): Conversation
    suspend fun modelParameters(id: String, model: ModelInfo): com.nola.domain.model.ModelParameters?
    suspend fun saveModelParameters(id: String, model: ModelInfo, value: com.nola.domain.model.ModelParameters, expectedBaseline: com.nola.domain.model.ModelParameters?, expectedRevision: Long): Conversation
    suspend fun updateGenerationOverride(id: String, model: ModelInfo, value: ConversationGenerationOverride)
    suspend fun updateResponseStyle(id: String, style: String?)
    suspend fun updatePermissionMode(id: String, mode: String, expectedRevision: Long,
        owner: com.nola.data.local.PermissionConfigurationOwner)
    suspend fun updateExecutionBackend(id: String, backend: String, sessions: Map<String, String>)
    suspend fun setDataSourceEnabled(id: String, sourceId: String, enabled: Boolean?)
    suspend fun touchLastActive(id: String)
}

@Singleton
class ConversationRepositoryImpl @Inject constructor(
    private val applicationRuntime: AndroidApplicationRuntime,
    private val assistantJournal: com.nola.data.local.AndroidAssistantJournal,
    private val deviceModelStore: com.nola.llm.local.DeviceModelStore,
) : ConversationRepository {
    private val json = Json { ignoreUnknownKeys = true }
    private val sourceOverrides = CanonicalDataSourceOverrides { applicationRuntime.execute(it.jsonObject) }

    override suspend fun createConversation(title: String?, language: String?, modelInfo: ModelInfo?, promptResponseStyle: String?, permissionMode: String?, parameters: com.nola.domain.model.ModelParameters?, hostBinding: com.nola.domain.model.ExecutionHostBinding?): Conversation {
        com.nola.domain.model.requireValidPromptResponseStyle(promptResponseStyle)
        permissionMode?.let(::requireValidConversationPermissionMode)
        val now = System.currentTimeMillis()
        val id = UUID.randomUUID().toString()
        val isHost = modelInfo?.source == ModelInfo.ModelSource.WebHost
        require(!isHost || hostBinding != null) { "Host connection binding is missing" }
        require(isHost || hostBinding == null)
        val identity = modelInfo?.takeUnless { isHost }?.let(::conversationModelIdentity)
        if (identity != null) requireNotNull(resolveModel(identity)) { "Selected model is unavailable" }
        val settings = AndroidConversationSettings(language = language, modelSelectionID = identity,
            executionHostBinding = hostBinding,
            promptResponseStyle = promptResponseStyle, permissionModeRawValue = permissionMode)
        val encoded = if (isHost) JsonObject(Json.parseToJsonElement(json.encodeToString(settings)).jsonObject +
            initialHostSettings(requireNotNull(hostBinding), requireNotNull(parameters))).toString()
            else json.encodeToString(settings)
        val initial = Json.parseToJsonElement(if (modelInfo != null && parameters != null && !isHost)
            writeConversationParameters(encoded, modelInfo, parameters) else encoded).jsonObject.toMutableMap()
        initial.remove("knowledgePoints"); initial.remove("wiki")
        initial["knowledge"] = JsonObject(mapOf("keyPointsMarkdown" to JsonPrimitive(settings.knowledgePoints),
            "wikiMarkdown" to JsonPrimitive(settings.wiki), "updatedAt" to kotlinx.serialization.json.JsonNull))
        applicationRuntime.createConversation(id, title.orEmpty(), now, now, JsonObject(initial).toString())
        return requireNotNull(getConversation(id))
    }

    override suspend fun getConversation(id: String): Conversation? = applicationRuntime
        .loadConversation(id)
        ?.let { it.toDomain(resolveModel(it.settings().modelSelectionID, it.settings().executionHostBinding)) }
        ?.let { conversation ->
            val selected = sourceOverrides.resolve(id, System.currentTimeMillis())
                .filter { it.enabled }.map { "data_source:${it.sourceId}" }
            conversation.copy(dataSourceIds = conversation.dataSourceIds
                .filterNot { it.startsWith("data_source:") } + selected)
        }

    override suspend fun getAllConversations(): List<Conversation> = applicationRuntime
        .listRootConversations()
        .mapNotNull { item -> applicationRuntime.loadConversation(item.id)?.let { it.toDomain(resolveModel(it.settings().modelSelectionID, it.settings().executionHostBinding)) } }

    private suspend fun resolveModel(identity: String?, binding: com.nola.domain.model.ExecutionHostBinding? = null): ModelInfo? {
        if (binding != null) return if (identity == null) ModelInfo("web-host", "Nola Web Host", ModelInfo.ModelSource.WebHost) else null
        return resolveConversationModel(identity,
        applicationRuntime::modelProfile,
        if (identity?.startsWith("local:") == true) deviceModelStore.installedModels().map {
            ModelInfo(it.id, it.displayName, ModelInfo.ModelSource.Local)
        } else emptyList())
    }

    override suspend fun getProjection(id: String): ConversationProjectionV1 =
        applicationRuntime.loadConversationProjectionV1(id)

    override suspend fun getContextUsage(id: String): ConversationProjectionV1.ContextUsage =
        com.nola.domain.projection.ConversationProjectionV1Decoder.decodeContextUsage(applicationRuntime.execute(buildJsonObject {
            put("type", "load_conversation_context_usage_v1"); put("conversation_id", id)
        }).jsonObject)

    override suspend fun getProjectionWindow(id: String, beforeOrdinal: Long?): ConversationProjectionWindowV1 =
        applicationRuntime.loadConversationProjectionWindowV1(
            id = id,
            beforeOrdinal = beforeOrdinal,
            limit = 80,
            contentByteLimit = 512_000,
        ).let { it.copy(projection = assistantJournal.overlay(it.projection, it.assistantJournalHeads)) }

    override suspend fun prepareRemoteSubmission(
        sourceHostId: String,
        clientId: String,
        conversationId: String,
        conversationTitle: String,
        submissionId: String,
        userText: String,
        userMeta: Map<String, String>?,
        createdAtMs: Long,
        userMessageId: String,
    ): com.nola.data.remote.api.RemoteSubmissionIntent = applicationRuntime.prepareRemoteSubmission(
            sourceHostId = sourceHostId,
            clientId = clientId,
            conversationId = conversationId,
            conversationTitle = conversationTitle,
            submissionId = submissionId,
            userText = userText,
            userContentJson = userMeta?.let { json.encodeToString(it) },
            createdAtMs = createdAtMs,
            userMessageId = userMessageId,
        )

    override suspend fun bindRemoteSubmissionRequest(identity: com.nola.data.remote.api.RemoteSubmissionIdentity, request: kotlinx.serialization.json.JsonObject) =
        applicationRuntime.bindRemoteSubmissionRequest(identity, request)
    override suspend fun readRemoteSubmissionIntent(conversationId: String) = applicationRuntime.readRemoteSubmissionIntent(conversationId)
    override suspend fun readRemoteSubmissionRequest(identity: com.nola.data.remote.api.RemoteSubmissionIdentity, requestDigest: String) =
        applicationRuntime.readRemoteSubmissionRequest(identity, requestDigest)
    override suspend fun acceptRemoteSubmissionReceipt(receipt: com.nola.data.remote.api.RemoteSubmissionReceipt) = applicationRuntime.acceptRemoteSubmissionReceipt(receipt)
    override suspend fun abandonPreparedRemoteSubmission(identity: com.nola.data.remote.api.RemoteSubmissionIdentity) = applicationRuntime.abandonPreparedRemoteSubmission(identity)
    override suspend fun adoptObservedRemoteRun(receipt: com.nola.data.remote.api.RemoteSubmissionReceipt) = applicationRuntime.adoptObservedRemoteRun(receipt)


    override suspend fun getMessages(convId: String): List<ConversationMessage> = canonicalMessages(convId)

    override suspend fun getActiveMessages(convId: String): List<ConversationMessage> {
        val archived = archivedMessageIDs(convId)
        return canonicalMessages(convId).filterNot { it.id in archived }
    }

    override suspend fun getArchivedMessages(convId: String): List<ConversationMessage> {
        val archived = archivedMessageIDs(convId)
        return canonicalMessages(convId).filter { it.id in archived }
    }

    override suspend fun deleteConversation(id: String) {
        applicationRuntime.deleteConversation(id)
    }

    override suspend fun updateTitle(id: String, title: String) {
        applicationRuntime.renameConversation(id, title, System.currentTimeMillis())
    }

    override suspend fun updateSessionMemory(id: String, memory: String) {
        applicationRuntime.replaceContextSummary(applicationRuntime.prepareContextCompaction(id), memory)
    }

    override suspend fun updateKnowledge(id: String, knowledgePoints: String, wiki: String) =
        updateSettings(id) { copy(knowledgePoints = knowledgePoints, wiki = wiki) }

    override suspend fun selectModel(id: String, model: ModelInfo, parameters: com.nola.domain.model.ModelParameters, expectedRevision: Long): Conversation {
        val identity = conversationModelIdentity(model)
        requireNotNull(resolveModel(identity)) { "Selected model is unavailable" }
        val stored = applicationRuntime.loadConversation(id) ?: error("Conversation is missing: $id")
        if (stored.configurationRevision != expectedRevision) throw ConcurrentModificationException("Conversation changed while selecting model")
        val selection = conversationModelSelection(stored.settingsJson, model, parameters)
        applicationRuntime.selectConversationExecution(id, expectedRevision, selection).requireCommitted()
        return requireNotNull(getConversation(id))
    }

    override suspend fun modelParameters(id: String, model: ModelInfo): com.nola.domain.model.ModelParameters? {
        val stored = applicationRuntime.loadConversation(id) ?: error("Conversation is missing: $id")
        if (model.source == ModelInfo.ModelSource.WebHost) return readHostParameters(stored.settingsJson).takeIf { stored.settings().executionHostBinding != null }
        return readConversationParameters(stored.settingsJson, model)
    }

    override suspend fun selectHost(id: String, binding: com.nola.domain.model.ExecutionHostBinding, parameters: com.nola.domain.model.ModelParameters, expectedRevision: Long): Conversation {
        val selection = conversationHostSelection(binding, parameters)
        applicationRuntime.selectConversationExecution(id, expectedRevision, selection).requireCommitted()
        return requireNotNull(getConversation(id))
    }

    override suspend fun saveModelParameters(id: String, model: ModelInfo, value: com.nola.domain.model.ModelParameters, expectedBaseline: com.nola.domain.model.ModelParameters?, expectedRevision: Long): Conversation {
        val stored = applicationRuntime.loadConversation(id) ?: error("Conversation is missing: $id")
        if (model.source == ModelInfo.ModelSource.WebHost) {
            if (stored.configurationRevision != expectedRevision || stored.settings().modelSelectionID != null ||
                (expectedBaseline != null && readHostParameters(stored.settingsJson) != expectedBaseline))
                throw ConcurrentModificationException("Host configuration changed while editing")
            return selectHost(id, requireNotNull(stored.settings().executionHostBinding), value, expectedRevision)
        }
        if (stored.configurationRevision != expectedRevision || stored.settings().modelSelectionID != conversationModelIdentity(model))
            throw ConcurrentModificationException("Conversation model changed while editing parameters")
        if (expectedBaseline != null && readConversationParameters(stored.settingsJson, model) != expectedBaseline) {
            throw ConcurrentModificationException("Conversation parameters changed while editing; reload and retry")
        }
        val updated = Json.parseToJsonElement(writeConversationParameters(stored.settingsJson, model, value)).jsonObject
        val patch = JsonObject(updated.filterKeys { it == "modelOverrides" || it == "clientModelPresentation" })
        applicationRuntime.patchConversationConfiguration(id, expectedRevision, patch.toString()).requireCommitted()
        return requireNotNull(getConversation(id))
    }

    override suspend fun updateGenerationOverride(id: String, model: ModelInfo, value: ConversationGenerationOverride) {
        value.validate()
        val key = ConversationGenerationOverride.modelKey(model)
        patchConfiguration(id, conversationGenerationPatch(key, value))
    }

    override suspend fun updateResponseStyle(id: String, style: String?) {
        com.nola.domain.model.requireValidPromptResponseStyle(style)
        updateSettings(id) { copy(promptResponseStyle = style) }
    }

    override suspend fun updatePermissionMode(id: String, mode: String, expectedRevision: Long,
        owner: com.nola.data.local.PermissionConfigurationOwner) {
        requireValidConversationPermissionMode(mode)
        val runId = when (owner) {
            com.nola.data.local.PermissionConfigurationOwner.Inactive -> null
            is com.nola.data.local.PermissionConfigurationOwner.Local -> owner.key.runId
            is com.nola.data.local.PermissionConfigurationOwner.Remote -> owner.identity.submissionId
        }
        val source = (owner as? com.nola.data.local.PermissionConfigurationOwner.Remote)?.identity?.sourceHostId
        applicationRuntime.setConversationPermission(id, expectedRevision, runId, source, mode).requireCommitted()
    }

    private fun patchConfiguration(id: String, patch: JsonObject) {
        val current = applicationRuntime.loadConversation(id) ?: error("Conversation is missing: $id")
        applicationRuntime.patchConversationConfiguration(id, requireNotNull(current.configurationRevision), patch.toString()).requireCommitted()
    }

    override suspend fun updateExecutionBackend(id: String, backend: String, sessions: Map<String, String>) =
        updateSettings(id) { copy(executionBackend = backend, externalExecutionSessions = sessions) }

    override suspend fun setDataSourceEnabled(id: String, sourceId: String, enabled: Boolean?) {
        if (sourceId.startsWith("data_source:")) {
            val canonicalId = sourceId.removePrefix("data_source:")
            require(sourceOverrides.resolve(id, System.currentTimeMillis()).any { it.sourceId == canonicalId }) {
                "Data source is missing: $canonicalId"
            }
            sourceOverrides.set(id, canonicalId, when (enabled) {
                true -> CanonicalDataSourceOverride.Enabled
                false -> CanonicalDataSourceOverride.Disabled
                null -> CanonicalDataSourceOverride.Inherit
            }, System.currentTimeMillis())
        } else {
            updateSettings(id) {
                val selected = dataSourceIds.filterNot { it.startsWith("data_source:") }.toMutableSet()
                if (enabled == true) selected.add(sourceId) else selected.remove(sourceId)
                copy(dataSourceIds = selected.toList())
            }
        }
    }

    override suspend fun touchLastActive(id: String) {
        updateSettings(id) { this }
    }

    private fun updateSettings(
        id: String,
        transform: AndroidConversationSettings.() -> AndroidConversationSettings,
    ) {
        val stored = applicationRuntime.loadConversation(id) ?: error("Conversation is missing: $id")
        val before = stored.settingsJson?.let { Json.parseToJsonElement(it).jsonObject } ?: JsonObject(emptyMap())
        val after = Json.parseToJsonElement(transformConversationSettings(stored.settingsJson, transform)).jsonObject
        val changes = (before.keys + after.keys).filter { before[it] != after[it] }
            .associateWith { after[it] ?: kotlinx.serialization.json.JsonNull }.toMutableMap()
        if (changes.containsKey("knowledgePoints") || changes.containsKey("wiki")) {
            changes.remove("knowledgePoints"); changes.remove("wiki")
            changes["knowledge"] = JsonObject(mapOf(
                "keyPointsMarkdown" to (after["knowledgePoints"] ?: JsonPrimitive("")),
                "wikiMarkdown" to (after["wiki"] ?: JsonPrimitive(""))))
        }
        changes.remove("version")
        changes.remove("permissionModeRawValue")?.let { changes["permissionProfile"] = it }
        if (changes.isNotEmpty()) applicationRuntime.patchConversationConfiguration(id,
            requireNotNull(stored.configurationRevision), JsonObject(changes).toString()).requireCommitted()
    }

    private fun canonicalMessages(id: String): List<ConversationMessage> {
        val projection = applicationRuntime.loadConversationProjectionV1(id)
        val heads = mutableListOf<JsonObject>()
        var before: Long? = null
        do {
            val window = applicationRuntime.loadConversationProjectionWindowV1(id, before, 128, 16384)
            require(window.projection.revision == projection.revision) { "Conversation changed during journal history read" }
            heads += window.assistantJournalHeads
            val next = if (window.hasMore) checkNotNull(window.beforeOrdinal) else null
            require(next == null || before == null || next < before)
            before = next
        } while (before != null)
        return CanonicalConversationProjector.project(assistantJournal.overlay(projection, heads))
    }

    private fun archivedMessageIDs(id: String): Set<String> = applicationRuntime
        .loadConversation(id)
        ?.settings()
        ?.compactedMemoryEntries
        .orEmpty()
        .flatMap(CompactedMemoryEntry::archivedMessageIds)
        .toSet()
}

@Serializable
internal data class AndroidConversationSettings(
    val version: Int = 1,
    val language: String? = null,
    val sessionMemory: String = "",
    val knowledgePoints: String = "",
    val wiki: String = "",
    @Serializable(with = com.nola.domain.model.CanonicalTimestampSerializer::class) val lastCompactedAt: Long? = null,
    @kotlinx.serialization.SerialName("selectedPromptTemplateID") val selectedPromptTemplateId: String? = null,
    val dataSourceIds: List<String> = emptyList(),
    val modelSelectionID: String? = null,
    val agentRoomOwner: com.nola.domain.model.AgentRoomContextOwner? = null,
    val executionHostBinding: com.nola.domain.model.ExecutionHostBinding? = null,
    val promptResponseStyle: String? = null,
    val permissionModeRawValue: String? = null,
    val executionBackend: String = "nola",
    val externalExecutionSessions: Map<String, String> = emptyMap(),
    val persistenceEnabled: Boolean = true,
    val compactedMemoryEntries: List<CompactedMemoryEntry> = emptyList(),
)

internal fun requireValidConversationPermissionMode(value: String): String {
    return AndroidAgentPermissionMode.fromWireValue(value).wireValue
}

// Patch only changed fields. Other clients own additional settings in the same object.
internal fun transformConversationSettings(
    stored: String?,
    transform: AndroidConversationSettings.() -> AndroidConversationSettings,
): String {
    val json = Json { ignoreUnknownKeys = true }
    val original = stored?.let { json.parseToJsonElement(it).jsonObject }
        ?: kotlinx.serialization.json.JsonObject(emptyMap())
    val current = json.decodeFromString<AndroidConversationSettings>(original.toString())
    val before = json.parseToJsonElement(json.encodeToString(current)).jsonObject
    val updated = current.transform()
    if (updated.promptResponseStyle != current.promptResponseStyle) {
        com.nola.domain.model.requireValidPromptResponseStyle(updated.promptResponseStyle)
    }
    if (updated.permissionModeRawValue != current.permissionModeRawValue) {
        updated.permissionModeRawValue?.let(::requireValidConversationPermissionMode)
    }
    val after = json.parseToJsonElement(json.encodeToString(updated)).jsonObject
    val merged = original.toMutableMap()
    (before.keys + after.keys).forEach { key ->
        if (before[key] != after[key]) {
            after[key]?.let { merged[key] = it } ?: merged.remove(key)
        }
    }
    return kotlinx.serialization.json.JsonObject(merged).toString()
}

/** A sparse Core patch never resubmits stale Mac-owned fields or other models. */
internal fun conversationGenerationPatch(modelKey: String, value: ConversationGenerationOverride): JsonObject {
    require(modelKey.isNotBlank())
    value.validate()
    val fields = buildMap<String, kotlinx.serialization.json.JsonElement> {
        value.maxContextLength?.let { put("maxContextLength", JsonPrimitive(it)) }
        value.maxOutputTokens?.let { put("maxOutputTokens", JsonPrimitive(it)) }
    }
    return JsonObject(mapOf("modelOverrides" to JsonObject(mapOf(modelKey to JsonObject(fields)))))
}

internal fun readConversationGenerationOverrides(stored: String?): Map<String, ConversationGenerationOverride> {
    val models = runCatching {
        stored?.let { Json.parseToJsonElement(it).jsonObject["modelOverrides"] as? JsonObject }
    }.getOrNull() ?: return emptyMap()
    return models.mapNotNull { (key, element) ->
        val fields = element as? JsonObject ?: return@mapNotNull null
        val context = fields["maxContextLength"]?.jsonPrimitive?.intOrNull
        val output = fields["maxOutputTokens"]?.jsonPrimitive?.intOrNull
        val value = ConversationGenerationOverride(context, output)
        if (runCatching { value.validate() }.isFailure) null else key to value
    }.toMap()
}

private fun CanonicalConversation.settings(): AndroidConversationSettings {
    val decoder = Json { ignoreUnknownKeys = true }
    val base = settingsJson
        ?.let { runCatching { decoder.decodeFromString<AndroidConversationSettings>(it) }.getOrNull() }
        ?: AndroidConversationSettings()
    val fields = settingsJson?.let { Json.parseToJsonElement(it).jsonObject } ?: return base
    val knowledge = fields["knowledge"] as? JsonObject
    return base.copy(
        agentRoomOwner = fields["agentRoomOwner"]?.let {
            Json.decodeFromJsonElement(com.nola.domain.model.AgentRoomContextOwner.serializer(), it)
        },
        knowledgePoints = knowledge?.get("keyPointsMarkdown")?.jsonPrimitive?.content ?: base.knowledgePoints,
        wiki = knowledge?.get("wikiMarkdown")?.jsonPrimitive?.content ?: base.wiki,
        selectedPromptTemplateId = fields["selectedPromptTemplateID"]?.takeUnless { it is kotlinx.serialization.json.JsonNull }
            ?.jsonPrimitive?.content ?: base.selectedPromptTemplateId,
    )
}

private fun CanonicalConversation.toDomain(model: ModelInfo?): Conversation {
    val settings = settings()
    return Conversation(
        id = id,
        title = title.ifEmpty { null },
        createdAt = createdAtMs,
        lastActiveAt = updatedAtMs,
        language = settings.language,
        sessionMemory = settings.sessionMemory,
        knowledgePoints = settings.knowledgePoints,
        wiki = settings.wiki,
        lastCompactedAt = settings.lastCompactedAt,
        selectedPromptTemplateId = settings.selectedPromptTemplateId,
        dataSourceIds = settings.dataSourceIds,
        modelSelectionID = settings.modelSelectionID,
        agentRoomOwner = settings.agentRoomOwner,
        configurationRevision = requireNotNull(configurationRevision) { "Canonical conversation configuration revision is missing" },
        executionHostBinding = settings.executionHostBinding,
        selectedModelInfo = model,
        generationOverrides = readConversationGenerationOverrides(settingsJson),
        promptResponseStyle = settings.promptResponseStyle,
        permissionMode = settings.permissionModeRawValue,
        executionBackend = settings.executionBackend,
        externalExecutionSessions = settings.externalExecutionSessions,
        persistenceEnabled = settings.persistenceEnabled,
        compactedMemoryEntries = settings.compactedMemoryEntries,
    )
}

private fun ConversationMessage.toCanonical(): CanonicalConversationMessage = CanonicalConversationMessage(
    id = id,
    role = role.name,
    text = text,
    metadataJson = meta?.let { Json.encodeToString(it) },
    createdAtMs = createdAt,
)

private fun CanonicalConversationMessage.toDomain(conversationId: String): ConversationMessage = ConversationMessage(
    id = id,
    conversationId = conversationId,
    role = runCatching { ChatRole.valueOf(role) }.getOrDefault(ChatRole.user),
    text = text,
    createdAt = createdAtMs,
    meta = metadataJson?.let { runCatching { Json.decodeFromString<Map<String, String>>(it) }.getOrNull() },
)
