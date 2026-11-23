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
        val reasoning = source["reasoning"] as? JsonObject
        val control = (reasoning?.get("control") as? JsonPrimitive)?.contentOrNull
        val efforts = if (profile.runtime == "remote" &&
            (source["executionCapabilities"] as? JsonObject)?.get("reasoning") == JsonPrimitive(true)) {
            when (control) {
                "effort" -> (reasoning["efforts"] as? JsonArray)
                    ?.mapNotNull { (it as? JsonPrimitive)?.contentOrNull }.orEmpty()
                "budget" -> {
                    val budgets = (reasoning["budgets"] as? JsonArray).orEmpty()
                    if (budgets.isNotEmpty()) listOf("concise", "balanced", "detailed") else emptyList()
                }
                else -> emptyList()
            }
        } else emptyList()
        return ParameterSnapshot(parameters, profile, efforts)
    }

    suspend fun saveParameters(snapshot: ParameterSnapshot, params: ModelParameters): ParameterSnapshot {
        // configure_parameters preserves the stored selection. A true flag in the
        // command requests a selection change and is rejected by Core.
        val input = snapshot.profile.copy(parametersJson = json.encodeToString(params),
            selected = false, enabled = true, deletedAtMs = null, updatedAtMs = System.currentTimeMillis())
        val committed = applicationRuntime.configureModelParameters(input, snapshot.profile.revision)
        revision.value += 1
        return snapshot.copy(parameters = params, profile = snapshot.profile.copy(parametersJson = input.parametersJson,
            revision = committed, updatedAtMs = input.updatedAtMs))
    }

    suspend fun loadParameters(modelID: String): ModelParameters = loadParametersNow(modelID)

    suspend fun loadLocalParameters(modelID: String, contextWindow: Int): ModelParameters {
        val profile = applicationRuntime.modelProfile(modelID)
        val raw = profile?.let { json.decodeFromString<ModelParameters>(it.parametersJson) }
            ?: defaultParameters.copy(maxContextTokens = contextWindow.coerceIn(2048, 8192))
        val maxContext = contextWindow.coerceAtLeast(512)
        return raw.copy(maxContextTokens = raw.maxContextTokens.coerceIn(512, maxContext))
    }

    fun localParameterSnapshot(modelID: String, modelName: String, contextWindow: Int): ParameterSnapshot {
        val existing = applicationRuntime.modelProfile(modelID)
        val clampedContext = contextWindow.coerceAtLeast(512)
        if (existing != null) {
            val base = parameterSnapshot(modelID)
            return base.copy(
                parameters = base.parameters.copy(
                    maxContextTokens = base.parameters.maxContextTokens.coerceIn(512, clampedContext),
                ),
            )
        }
        val params = defaultParameters.copy(maxContextTokens = contextWindow.coerceIn(2048, 8192))
        val now = System.currentTimeMillis()
        val synthetic = CanonicalModelProfile(
            id = modelID,
            displayName = modelName,
            provider = "llama_cpp",
            modelName = modelID,
            runtime = "local",
            sourceJson = "{}",
            parametersJson = json.encodeToString(params),
            selected = false,
            enabled = true,
            revision = 0L,
            createdAtMs = now,
            updatedAtMs = now,
        )
        return ParameterSnapshot(params, synthetic, emptyList())
    }

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
