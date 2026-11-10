package com.nola.data.local.datastore

import android.content.Context
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.AndroidModelConfiguration
import com.nola.domain.model.ChatMemoryProfile
import com.nola.domain.model.ModelExecutionCapabilities
import com.nola.domain.model.CustomDataSourceConfig
import com.nola.domain.model.DataSourceUsageRecord
import com.nola.domain.model.McpServerConfig
import com.nola.domain.model.StructuredMemoryRecord
import com.nola.domain.model.UploadedContent
import com.nola.data.local.security.SecureTokenStore
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.Serializable
import java.nio.charset.StandardCharsets
import java.util.UUID
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class MemoryProfileDataStore @Inject constructor(
    @ApplicationContext private val context: Context,
    private val secureTokenStore: SecureTokenStore,
    private val applicationRuntime: AndroidApplicationRuntime
) {
    private val json = Json { ignoreUnknownKeys = false; prettyPrint = false }
    private val revision = MutableStateFlow(0L)
    private val modelConfiguration = AndroidModelConfiguration(applicationRuntime)

    fun recallProfileMemories(query: String, limit: Int): List<StructuredMemoryRecord> =
        applicationRuntime.recallProfileMemories(query, limit)

    companion object {
        private const val MEMORY_ENABLED_SETTING_KEY = "assistant.memory.enabled"
        private const val UPLOADED_DOCUMENTS_SETTING_KEY = "assistant.uploaded_documents"
        private const val RESOURCE_PRINCIPAL_SETTING_KEY = "assistant.resource_principal_id"
        private const val RESPONSE_STYLE_SETTING_KEY = "assistant.response_style"
        private const val CHAT_PREFERENCES_SETTING_KEY = "chat.preferences"
        private const val CURRENT_SETTING_SCHEMA_VERSION = 1
    }

    val profile: Flow<ChatMemoryProfile> = revision.map {
        val memorySnapshot = applicationRuntime.memorySnapshot()
        val canonicalRecords = memorySnapshot.records
        val canonicalSources = applicationRuntime.listDataSources()
        val canonicalUsage = applicationRuntime.listDataSourceUsage()
        val canonicalSkills = applicationRuntime.listSkills()
        val canonicalMcpServers = applicationRuntime.listMcpServers()
        ChatMemoryProfile(
            canonicalMemorySnapshot = memorySnapshot.canonical,
            longTermMemory = memoryLines(canonicalRecords, "long_term"),
            memoryRecords = canonicalRecords,
            userPreferences = memoryLines(canonicalRecords, "user_preferences"),
            mcpServers = canonicalMcpServers,
            uploadedSkills = canonicalSkills,
            uploadedDocuments = productSetting(UPLOADED_DOCUMENTS_SETTING_KEY, emptyList()),
            customDataSources = canonicalSources,
            dataSourceUsageRecords = canonicalUsage
        )
    }.flowOn(Dispatchers.IO)

    val agentLlmEndpoint: Flow<String> = revision.map { modelConfiguration.load()?.endpoint.orEmpty() }

    val agentLlmApiKey: Flow<String> = revision.map {
        runCatching { configuredApiModelProfiles().let {all->all.firstOrNull {it.selected} ?: all.firstOrNull()}?.let {resolveApiProfile(it).second}.orEmpty() }.getOrDefault("")
    }

    val agentLlmModel: Flow<String> = revision.map { modelConfiguration.load()?.model.orEmpty() }

    fun agentLlmConfiguration(): AndroidModelConfiguration.Snapshot? = modelConfiguration.load()

    fun configuredApiModelProfiles(): List<com.nola.data.local.CanonicalModelProfile> =
        modelConfiguration.configuredProfiles()

    suspend fun updateModelExecutionCapabilities(profileId: String, expectedRevision: Long, capabilities: ModelExecutionCapabilities) =
        withContext(Dispatchers.IO) {
            modelConfiguration.updateExecutionCapabilities(profileId, expectedRevision, capabilities)
            revision.update { it + 1 }
        }

    fun resolveApiProfile(profile: com.nola.data.local.CanonicalModelProfile, parameters: com.nola.domain.model.ModelParameters? = null): Pair<kotlinx.serialization.json.JsonObject,String> {
        val response=applicationRuntime.execute(kotlinx.serialization.json.buildJsonObject {
            put("type","resolve_remote_model_profile"); put("profile_id",profile.id); put("expected_revision",profile.revision)
            put("request",kotlinx.serialization.json.buildJsonObject {
                if (parameters?.reasoningPreference != null) {
                    put("reasoning_selection", if (parameters.reasoningPreference == "automatic") kotlinx.serialization.json.JsonNull
                        else kotlinx.serialization.json.buildJsonObject { put("kind", "effort"); put("value", parameters.reasoningPreference) })
                } else parameters?.reasoningTier?.let { put("reasoning_tier",it) }
            })
        }).jsonObject
        val reference=response["credential_ref"]?.jsonPrimitive?.contentOrNull
        val secret=if (reference!=null) secureTokenStore.readModelAccountSecret(reference)
            ?: error("Model account credential is unavailable; reconnect this account")
        else { check(response["credential_required"]?.jsonPrimitive?.boolean==false) {"Model account needs reconnection"}; "" }
        return response.getValue("resolved").jsonObject to secret
    }

    val responseStyle: Flow<String> = revision.map { productSetting(RESPONSE_STYLE_SETTING_KEY, "casual") }

    val memoryEnabled: Flow<Boolean> = revision.map { productSetting(MEMORY_ENABLED_SETTING_KEY, true) }

    val autoCompactTriggerPercent: Flow<Int> = revision.map {
        val setting = applicationRuntime.productSetting(CHAT_PREFERENCES_SETTING_KEY) ?: return@map 80
        val preferences = json.decodeFromString<ChatPreferences>(setting.valueJson)
        require(preferences.schemaVersion == 1) {
            "Unsupported chat preferences schema version: ${preferences.schemaVersion}"
        }
        require(preferences.autoCompactTriggerPercent in 50..95) {
            "Invalid auto-compact trigger percentage: ${preferences.autoCompactTriggerPercent}"
        }
        preferences.autoCompactTriggerPercent
    }

    suspend fun skillEditor(id: String): AndroidApplicationRuntime.SkillEditorSnapshot = withContext(Dispatchers.IO) {
        requireNotNull(applicationRuntime.skillEditor(id)) { "Skill no longer exists" }
    }

    fun skillEditor(displayed: UploadedContent): AndroidApplicationRuntime.SkillEditorSnapshot {
        val record = requireNotNull(displayed.canonicalSkillRecord) { "Reload the Skill before editing" }
        val revision = requireNotNull(displayed.canonicalRevision) { "Reload the Skill before editing" }
        require(revision > 0 && record["id"] == kotlinx.serialization.json.JsonPrimitive(displayed.resourceId)) {
            "Skill editor identity or revision is invalid"
        }
        return AndroidApplicationRuntime.SkillEditorSnapshot(record, revision)
    }

    suspend fun skills(): List<UploadedContent> = withContext(Dispatchers.IO) { applicationRuntime.listSkills() }

    data class DocumentEditorSnapshot(val documents: List<UploadedContent>, val revision: Long?)

    suspend fun documentEditor(): DocumentEditorSnapshot = withContext(Dispatchers.IO) {
        val setting = applicationRuntime.productSetting(UPLOADED_DOCUMENTS_SETTING_KEY)
            ?: return@withContext DocumentEditorSnapshot(emptyList(), null)
        val decoded = json.decodeFromString<Setting<List<UploadedContent>>>(setting.valueJson)
        require(decoded.schemaVersion == CURRENT_SETTING_SCHEMA_VERSION)
        DocumentEditorSnapshot(decoded.value, setting.revision)
    }

    suspend fun saveDocuments(expected: DocumentEditorSnapshot, documents: List<UploadedContent>): DocumentEditorSnapshot = withContext(Dispatchers.IO) {
        val ids = documents.map { requireNotNull(it.resourceId) { "Document identity is required" } }
        require(ids.all(String::isNotBlank) && ids.toSet().size == ids.size) { "Document identities must be unique" }
        val saved = applicationRuntime.submitMutation(buildJsonObject {
            put("type", "put_product_setting_checked")
            put("key", UPLOADED_DOCUMENTS_SETTING_KEY)
            put("value_json", json.encodeToString(Setting(schemaVersion = 1, value = documents)))
            if (expected.revision == null) put("expected_revision", JsonNull)
            else put("expected_revision", expected.revision)
            put("updated_at_ms", System.currentTimeMillis())
        }).jsonObject
        revision.update { it + 1 }
        DocumentEditorSnapshot(documents, saved.getValue("revision").jsonPrimitive.content.toLong())
    }

    suspend fun saveSkill(original: AndroidApplicationRuntime.SkillEditorSnapshot?, edited: UploadedContent) = withContext(Dispatchers.IO) {
        if (original == null) {
            applicationRuntime.createUserSkill(edited.copy(resourceId = edited.resourceId ?: UUID.randomUUID().toString()))
        } else {
            applicationRuntime.configureUserSkill(original, edited.name, edited.content)
        }
        revision.update { it + 1 }
    }

    suspend fun setSkillEnabled(original: AndroidApplicationRuntime.SkillEditorSnapshot, enabled: Boolean) = withContext(Dispatchers.IO) {
        applicationRuntime.setSkillEnabled(original, enabled)
        revision.update { it + 1 }
    }

    suspend fun deleteSkill(original: AndroidApplicationRuntime.SkillEditorSnapshot) = withContext(Dispatchers.IO) {
        applicationRuntime.deleteUserSkill(original)
        revision.update { it + 1 }
    }

    suspend fun saveMcpServer(original: McpServerConfig?, edited: McpServerConfig) = withContext(Dispatchers.IO) {
        if (original == null) {
            applicationRuntime.createMcpServer(edited.copy(serverId = UUID.randomUUID().toString()))
        } else {
            applicationRuntime.configureMcpServer(
                AndroidApplicationRuntime.McpEditorSnapshot(
                    original, requireNotNull(original.canonicalRevision) { "Reload the MCP server before editing" }
                ), edited
            )
        }
        revision.update { it + 1 }
    }

    suspend fun deleteMcpServer(original: McpServerConfig) = withContext(Dispatchers.IO) {
        applicationRuntime.deleteMcpServer(AndroidApplicationRuntime.McpEditorSnapshot(
            original, requireNotNull(original.canonicalRevision) { "Reload the MCP server before deleting" }
        ))
        revision.update { it + 1 }
    }

    suspend fun saveMemoryText(profile: ChatMemoryProfile): ChatMemoryProfile = withContext(Dispatchers.IO) {
        val committed = applicationRuntime.replaceMemories(
            requireNotNull(profile.canonicalMemorySnapshot) { "Reload the memory profile before saving" },
            reconciledProfileMemories(profile)
        )
        revision.update { it + 1 }
        profile.copy(
            canonicalMemorySnapshot = committed.canonical,
            memoryRecords = committed.records,
            longTermMemory = memoryLines(committed.records, "long_term"),
            userPreferences = memoryLines(committed.records, "user_preferences")
        )
    }

    internal fun resolveDataSources(conversationId: String): List<com.nola.data.local.CanonicalDataSourceSelection> =
        com.nola.data.local.CanonicalDataSourceOverrides { command ->
            applicationRuntime.execute(command as kotlinx.serialization.json.JsonObject)
        }.resolve(conversationId, System.currentTimeMillis())

    fun recordDataSourceUsage(records: List<DataSourceUsageRecord>) {
        if (records.isEmpty()) return
        try {
            records.forEach(applicationRuntime::appendDataSourceUsage)
        } finally {
            // Earlier records may have committed even if a later command fails.
            revision.value += 1
        }
    }

    fun saveDataSource(source: CustomDataSourceConfig) {
        try { applicationRuntime.saveDataSource(source) } finally { revision.value += 1 }
    }

    suspend fun refreshDataSource(source: CustomDataSourceConfig, read: suspend () -> CustomDataSourceConfig): CustomDataSourceConfig {
        return try { applicationRuntime.refreshDataSource(source, read) } finally { revision.value += 1 }
    }

    fun deleteDataSource(id: String) {
        applicationRuntime.deleteDataSource(id)
        revision.value += 1
    }

    fun setDataSourceEnabled(id: String, enabled: Boolean) {
        applicationRuntime.setDataSourceEnabled(id, enabled)
        revision.value += 1
    }

    suspend fun saveAgentLlmConfig(endpoint: String, apiKey: String, model: String, wireProtocol: String = "chat_completions") {
        modelConfiguration.validateInput(endpoint, model)
        val reference=apiKey.trim().takeIf {it.isNotEmpty()}?.let {"android-model:"+java.util.UUID.randomUUID()}
        if (reference != null) check(secureTokenStore.saveModelAccountSecret(reference,apiKey.trim())) {
            "Unable to store model account credential"
        }
        modelConfiguration.save(endpoint,model,wireProtocol,reference)
        revision.value += 1
    }

    suspend fun saveResponseStyle(style: String) {
        putProductSetting(RESPONSE_STYLE_SETTING_KEY, style)
        revision.value += 1
    }

    suspend fun saveMemoryEnabled(enabled: Boolean) {
        applicationRuntime.putProductSetting(
            MEMORY_ENABLED_SETTING_KEY,
            json.encodeToString(Setting(schemaVersion = 1, value = enabled)),
            System.currentTimeMillis(),
        )
        revision.value += 1
    }

    suspend fun saveAutoCompactTriggerPercent(percent: Int) {
        require(percent in 50..95) { "Invalid auto-compact trigger percentage: $percent" }
        applicationRuntime.putProductSetting(
            CHAT_PREFERENCES_SETTING_KEY,
            json.encodeToString(ChatPreferences(autoCompactTriggerPercent = percent)),
            System.currentTimeMillis(),
        )
        revision.value += 1
    }

    suspend fun clearAll() {
        modelConfiguration.clearAll()
        secureTokenStore.deleteLocalModelApiKey()
        listOf(
            MEMORY_ENABLED_SETTING_KEY, UPLOADED_DOCUMENTS_SETTING_KEY,
            RESOURCE_PRINCIPAL_SETTING_KEY, RESPONSE_STYLE_SETTING_KEY,
            CHAT_PREFERENCES_SETTING_KEY,
        ).forEach { applicationRuntime.deleteProductSetting(it) }
        revision.value += 1
    }

    suspend fun getSnapshot(): ChatMemoryProfile = profile.first()

    private fun isPersonalMemory(record: StructuredMemoryRecord): Boolean =
        record.scope == "global" && record.owner == buildJsonObject {
            put("kind", "profile"); put("profile_id", "local")
        }

    private fun memoryLines(
        records: List<StructuredMemoryRecord>,
        namespace: String,
    ): List<String> = records.filter { isPersonalMemory(it) && it.namespace == namespace }.map { it.content }

    private fun reconciledProfileMemories(profile: ChatMemoryProfile): List<StructuredMemoryRecord> {
        // Display namespaces are lossy (custom("personal") and personal look
        // identical). Never deduplicate records outside this editor's scope.
        val untouched = profile.memoryRecords.filter {
            !isPersonalMemory(it) || (it.namespace != "long_term" && it.namespace != "user_preferences")
        }
        val byLogicalKey = profile.memoryRecords.filter { record ->
            isPersonalMemory(record) && when (record.namespace) {
                "long_term" -> profile.longTermMemory.any { it.trim() == record.content.trim() }
                "user_preferences" -> profile.userPreferences.any { it.trim() == record.content.trim() }
                else -> false
            }
        }.associateBy {
            "${it.namespace}|${it.kind}|${it.content.trim().lowercase()}"
        }.toMutableMap()
        fun append(lines: List<String>, namespace: String, kind: String) {
            lines.map(String::trim).filter(String::isNotEmpty).distinct().forEach { content ->
                val key = "$namespace|$kind|${content.lowercase()}"
                byLogicalKey.putIfAbsent(key, profileMemoryRecord(namespace, kind, content))
            }
        }
        append(profile.longTermMemory, "long_term", "reference")
        append(profile.userPreferences, "user_preferences", "preference")
        return untouched + byLogicalKey.values.toList()
    }

    suspend fun getOrCreateResourcePrincipalId(): String {
        val existing: String = productSetting(RESOURCE_PRINCIPAL_SETTING_KEY, "")
        if (existing.isNotBlank()) return existing
        val principalId = UUID.randomUUID().toString().lowercase()
        putProductSetting(RESOURCE_PRINCIPAL_SETTING_KEY, principalId)
        revision.value += 1
        return principalId
    }

    private inline fun <reified T> productSetting(key: String, default: T): T {
        val setting = applicationRuntime.productSetting(key) ?: return default
        val decoded = json.decodeFromString<Setting<T>>(setting.valueJson)
        require(decoded.schemaVersion == CURRENT_SETTING_SCHEMA_VERSION) {
            "Unsupported product setting schema version ${decoded.schemaVersion} for $key"
        }
        return decoded.value
    }

    private inline fun <reified T> putProductSetting(key: String, value: T) {
        applicationRuntime.putProductSetting(
            key,
            json.encodeToString(Setting(schemaVersion = 1, value = value)),
            System.currentTimeMillis(),
        )
    }

}

@Serializable
private data class Setting<T>(val schemaVersion: Int, val value: T)

@Serializable
private data class ChatPreferences(
    val schemaVersion: Int = 1,
    val autoCompactTriggerPercent: Int,
)

private fun profileMemoryRecord(
    namespace: String,
    kind: String,
    content: String,
    provenanceKind: String = "explicit_user",
): StructuredMemoryRecord {
    val stableId = UUID.nameUUIDFromBytes(
        "nola:android:profile:$namespace:$kind:${content.lowercase()}".toByteArray(StandardCharsets.UTF_8)
    ).toString()
    return StructuredMemoryRecord(
        id = stableId,
        owner = kotlinx.serialization.json.buildJsonObject { put("kind", kotlinx.serialization.json.JsonPrimitive("profile")); put("profile_id", kotlinx.serialization.json.JsonPrimitive("local")) },
        namespace = namespace,
        kind = kind,
        content = content,
        confidence = 0.8,
        provenanceKind = provenanceKind,
        sourceReference = "profile://android",
    )
}
