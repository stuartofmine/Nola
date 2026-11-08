package com.nola.data.local.datastore

import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.CanonicalModelProfile
import com.nola.domain.model.ModelParameters
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.map
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.*

@Singleton
class ModelParametersStore @Inject constructor(
    private val applicationRuntime: AndroidApplicationRuntime,
) {
    private val json = Json { ignoreUnknownKeys = false; prettyPrint = false }
    private val revision = MutableStateFlow(0L)

    companion object {
        val defaultParameters = ModelParameters(
            temperature = 0.7f,
            topP = 0.9f,
            topK = null,
            maxCompletionTokens = 16_384,
            maxContextTokens = 65536,
            responseStyle = "casual",
            enableThinking = true,
            enableRetrieval = false,
            streamOutput = true,
            customSystemPrompt = "",
        )
    }

    fun getParameters(modelID: String): Flow<ModelParameters> = revision.map {
        loadParametersNow(modelID)
    }

    suspend fun saveParameters(modelID: String, params: ModelParameters) {
        saveParameters(parameterSnapshot(modelID), params)
    }

    data class ParameterSnapshot(val parameters: ModelParameters, internal val profile: CanonicalModelProfile, val reasoningEfforts: List<String> = emptyList())

    fun parameterSnapshot(modelID: String): ParameterSnapshot {
        val profile = checkNotNull(applicationRuntime.modelProfile(modelID)) { "Model profile is unavailable: $modelID" }
        check(profile.revision >= 0) { "Invalid model revision" }
        val parameters = json.decodeFromString<ModelParameters>(profile.parametersJson)
        val source = json.parseToJsonElement(profile.sourceJson).jsonObject
        val efforts = if (profile.runtime == "remote" &&
            (source["executionCapabilities"] as? JsonObject)?.get("reasoning") == JsonPrimitive(true) &&
            (source["reasoning"] as? JsonObject)?.get("control") == JsonPrimitive("effort")) {
            (source.getValue("reasoning").jsonObject["efforts"] as? JsonArray)
                ?.map { it.jsonPrimitive.content }.orEmpty()
        } else emptyList()
        return ParameterSnapshot(parameters, profile, efforts)
    }

    suspend fun saveParameters(snapshot: ParameterSnapshot, params: ModelParameters): ParameterSnapshot {
        val input = snapshot.profile.copy(parametersJson = json.encodeToString(params),
            selected = false, enabled = true, deletedAtMs = null, updatedAtMs = System.currentTimeMillis())
        val committed = applicationRuntime.configureModelParameters(input, snapshot.profile.revision)
        revision.value += 1
        return snapshot.copy(parameters = params, profile = snapshot.profile.copy(parametersJson = input.parametersJson,
            revision = committed, updatedAtMs = input.updatedAtMs))
    }

    suspend fun loadParameters(modelID: String): ModelParameters = loadParametersNow(modelID)

    suspend fun clearAll() {
        applicationRuntime.resetModelParameters(
            profileIds = applicationRuntime.listModelProfiles().map { it.id },
            parametersJson = json.encodeToString(defaultParameters),
            updatedAtMs = System.currentTimeMillis(),
        )
        revision.value += 1
    }

    suspend fun exportSnapshot(): Map<String, ModelParameters> = applicationRuntime.listModelProfiles()
        .associate { profile -> profile.id to json.decodeFromString<ModelParameters>(profile.parametersJson) }

    private fun loadParametersNow(modelID: String): ModelParameters = applicationRuntime
        .modelProfile(modelID)
        ?.let { json.decodeFromString<ModelParameters>(it.parametersJson) }
        ?: defaultParameters

}
