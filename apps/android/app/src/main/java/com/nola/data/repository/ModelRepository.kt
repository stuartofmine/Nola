package com.nola.data.repository

import com.nola.data.local.datastore.MemoryProfileDataStore
import com.nola.data.local.CanonicalModelProfile
import com.nola.data.remote.api.BackendApiService
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelParameters
import com.nola.domain.model.ModelExecutionCapabilities
import com.nola.domain.model.ModelWireProtocol
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.coroutines.flow.first
import javax.inject.Inject
import javax.inject.Singleton

interface ModelRepository {
    suspend fun listOfficialModels(): List<ModelInfo>
    suspend fun listConfiguredModels(): List<ModelInfo>
    suspend fun getApiModelConfig(): ApiModelConfig?
    suspend fun getApiModelConfigFor(model: ModelInfo, parameters: ModelParameters? = null): ApiModelConfig?
    suspend fun saveApiModelConfig(config: ApiModelConfig)
    suspend fun updateModelExecutionCapabilities(model: ModelInfo, capabilities: ModelExecutionCapabilities)
    suspend fun getResponseStyle(): String
    suspend fun saveResponseStyle(style: String)
}

data class ApiModelConfig(
    val endpoint: String,
    val apiKey: String,
    val model: String,
    val wireProtocol: ModelWireProtocol = ModelWireProtocol.ChatCompletions,
    val resolvedRuntime: kotlinx.serialization.json.JsonObject? = null,
)

@Singleton
class ModelRepositoryImpl @Inject constructor(
    private val api: BackendApiService,
    private val memoryProfileDataStore: MemoryProfileDataStore
) : ModelRepository {

    override suspend fun listConfiguredModels(): List<ModelInfo> =
        memoryProfileDataStore.configuredApiModelProfiles().map(CanonicalModelProfile::toConfiguredModelInfo)

    override suspend fun listOfficialModels(): List<ModelInfo> {
        return try {
            val response = api.listModels()
            (response.data?.map { ModelInfo(id = it.id, name = it.id) }
                ?: response.models?.map { ModelInfo(id = it, name = it) }
                ?: emptyList())
        } catch (_: Exception) {
            emptyList()
        }
    }

    override suspend fun getApiModelConfig(): ApiModelConfig? {
        val profile=memoryProfileDataStore.configuredApiModelProfiles().let {all->all.firstOrNull {it.selected} ?: all.firstOrNull()} ?: return null
        return resolve(profile)
    }

    private fun resolve(profile: CanonicalModelProfile, parameters: ModelParameters? = null): ApiModelConfig {
        val (resolved,secret)=memoryProfileDataStore.resolveApiProfile(profile, parameters)
        val runtime=resolved.getValue("runtime").jsonObject
        val source=Json.parseToJsonElement(profile.sourceJson).jsonObject
        val protocol=checkNotNull(ModelWireProtocol.fromWireValue(source.getValue("wireProtocol").jsonPrimitive.content)) {"Unsupported device model protocol"}
        return ApiModelConfig(resolved.getValue("endpoint").jsonPrimitive.content,secret,
            resolved.getValue("model").jsonPrimitive.content,protocol,runtime)
    }

    override suspend fun getApiModelConfigFor(model: ModelInfo, parameters: ModelParameters?): ApiModelConfig? {
        val profile=memoryProfileDataStore.configuredApiModelProfiles().singleOrNull {
            it.id==model.profileId && it.modelName==model.id
        } ?: return null
        require(model.profileRevision==null || profile.revision==model.profileRevision) {"Model profile revision changed"}
        return resolve(profile, parameters)
    }

    override suspend fun saveApiModelConfig(config: ApiModelConfig) {
        memoryProfileDataStore.saveAgentLlmConfig(config.endpoint, config.apiKey, config.model, config.wireProtocol.wireValue)
    }

    override suspend fun updateModelExecutionCapabilities(model: ModelInfo, capabilities: ModelExecutionCapabilities) {
        memoryProfileDataStore.updateModelExecutionCapabilities(
            requireNotNull(model.profileId) { "The model is not a configured profile" },
            requireNotNull(model.profileRevision) { "The model configuration revision is unavailable" },
            capabilities,
        )
    }

    override suspend fun getResponseStyle(): String {
        return memoryProfileDataStore.responseStyle.first()
    }

    override suspend fun saveResponseStyle(style: String) {
        memoryProfileDataStore.saveResponseStyle(style)
    }
}

internal fun CanonicalModelProfile.toConfiguredModelInfo(): ModelInfo = ModelInfo(
    id = modelName,
    name = displayName,
    source = ModelInfo.ModelSource.Cloud,
    profileId = id,
    profileRevision = revision,
    executionCapabilitiesDeclared = Json.parseToJsonElement(sourceJson).jsonObject["executionCapabilities"] is kotlinx.serialization.json.JsonObject,
    executionCapabilities = runCatching {
        val value = Json.parseToJsonElement(sourceJson).jsonObject["executionCapabilities"]
        if (value == null) ModelExecutionCapabilities(toolCalls = true)
        else Json { ignoreUnknownKeys = true }.decodeFromJsonElement(ModelExecutionCapabilities.serializer(), value)
    }.getOrDefault(ModelExecutionCapabilities(toolCalls = false)),
)
