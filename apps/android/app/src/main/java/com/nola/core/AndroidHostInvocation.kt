package com.nola.core

import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelParameters
import com.nola.domain.model.ModelWireProtocol
import com.nola.llm.provider.OpenAiProvider
import java.net.URI
import java.security.MessageDigest
import kotlinx.serialization.json.*

internal const val ANDROID_HOST_INVOCATION_BINDING_VERSION = 1

internal fun invocationSha256(value: String): String = MessageDigest.getInstance("SHA-256")
    .digest(value.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it.toInt() and 255) }

internal fun canonicalInvocationJson(value: JsonElement): JsonElement = when (value) {
    is JsonObject -> JsonObject(value.toSortedMap().mapValues { canonicalInvocationJson(it.value) })
    is JsonArray -> JsonArray(value.map(::canonicalInvocationJson))
    else -> value
}

internal fun androidHttpModelRoute(config: OpenAiProvider.Config): JsonObject {
    val uri = URI(config.baseUrl)
    val scheme = uri.scheme?.lowercase()
    require(scheme in listOf("http", "https") && !uri.host.isNullOrBlank() &&
        uri.rawUserInfo == null && uri.rawFragment == null) { "Invalid model route" }
    val port = uri.port.takeUnless { it == -1 || scheme == "https" && it == 443 || scheme == "http" && it == 80 }
    val host = uri.host.lowercase().let { if (':' in it && !it.startsWith("[")) "[$it]" else it }
    val endpoint = "$scheme://$host${port?.let { ":$it" }.orEmpty()}${uri.rawPath.takeUnless { it.isNullOrEmpty() } ?: "/"}"
    fun source(present: Boolean) = buildJsonObject { put("kind", if (present) "ephemeral" else "none") }
    return buildJsonObject {
        put("kind", "http"); put("protocol", config.wireProtocol.wireValue)
        put("output_token_field", if (config.wireProtocol == ModelWireProtocol.Responses) "max_output_tokens"
            else config.executionCapabilities.chatOutputTokenField.requestKey(config.baseUrl))
        put("provider", JsonNull); put("endpoint", endpoint); put("model", config.model)
        put("siwc_account", JsonNull)
        put("endpoint_query_sha256", uri.rawQuery?.let { JsonPrimitive(invocationSha256(it)) } ?: JsonNull)
        put("endpoint_query_source", source(uri.rawQuery != null))
        put("credential_sha256", invocationSha256(config.apiKey)); put("credential_source", source(config.apiKey.isNotEmpty()))
    }
}

internal fun androidNativeModelRoute(modelId: String): JsonObject = buildJsonObject {
    put("kind", "native"); put("adapter", "llama_cpp"); put("adapter_contract_revision", 1)
    put("model_id", modelId); put("profile_ref", JsonNull)
}

internal fun AndroidTurnModelSnapshot.invocationGeneration(thinking: Boolean): JsonObject = buildJsonObject {
    put("max_retries", 0); put("parallel_tool_calls", JsonNull); put("reasoning_selection", resolvedRuntime?.get("reasoning_selection") ?: JsonNull)
    put("reasoning_tier", resolvedRuntime?.get("reasoning_tier") ?: JsonPrimitive("balanced"))
    put("sampling_override", model.source == ModelInfo.ModelSource.Local || model.executionCapabilities.samplingParameters)
    put("generation_options", buildJsonObject {
        parameters.topK?.let { put("top_k", it) }
        if (parameters.presencePenalty != 0.0) put("presence_penalty", parameters.presencePenalty)
        if (parameters.frequencyPenalty != 0.0) put("frequency_penalty", parameters.frequencyPenalty)
        parameters.repetitionPenalty?.let { put("repetition_penalty", it) }
        parameters.seed?.let { put("seed", it) }
        val stops = parameters.stopSequences.lines().map(String::trim).filter(String::isNotEmpty)
        if (stops.isNotEmpty()) put("stop", JsonArray(stops.map(::JsonPrimitive)))
    })
    put("temperature", parameters.temperature); put("top_p", parameters.topP)
    put("max_output_tokens", parameters.maxCompletionTokens); put("thinking_enabled", thinking)
    put("reasoning_summary_supported", resolvedRuntime?.get("model_capabilities")?.jsonObject
        ?.get("reasoning_summary_supported") ?: JsonNull)
    put("prompt_cache", false); put("prompt_cache_key_sha256", JsonNull)
    put("prompt_cache_key_prefix_sha256", JsonNull); put("prompt_cache_retention", JsonNull)
}

internal fun frozenAndroidParameters(binding: JsonObject, context: JsonObject): ModelParameters {
    val generation = binding.getValue("generation").jsonObject
    val options = generation.getValue("generation_options").jsonObject
    val output = generation.getValue("max_output_tokens").jsonPrimitive.int
    require(context.getValue("reserved_output_tokens").jsonPrimitive.int == output)
    return ModelParameters(
        temperature = generation.getValue("temperature").jsonPrimitive.float,
        topP = generation.getValue("top_p").jsonPrimitive.float,
        topK = options["top_k"]?.jsonPrimitive?.intOrNull,
        presencePenalty = options["presence_penalty"]?.jsonPrimitive?.doubleOrNull ?: 0.0,
        frequencyPenalty = options["frequency_penalty"]?.jsonPrimitive?.doubleOrNull ?: 0.0,
        repetitionPenalty = options["repetition_penalty"]?.jsonPrimitive?.doubleOrNull,
        seed = options["seed"]?.jsonPrimitive?.longOrNull,
        stopSequences = (options["stop"] as? JsonArray)?.joinToString("\n") { it.jsonPrimitive.content } ?: "",
        maxCompletionTokens = output,
        maxContextTokens = context.getValue("context_window_tokens").jsonPrimitive.int,
        enableThinking = generation.getValue("thinking_enabled").jsonPrimitive.boolean,
        reasoningPreference = (generation["reasoning_selection"] as? JsonObject)?.takeIf { it["kind"] == JsonPrimitive("effort") }
            ?.get("value")?.jsonPrimitive?.content,
        reasoningTier = (generation["reasoning_selection"] as? JsonObject)?.takeIf { it["kind"] == JsonPrimitive("budget") }
            ?.let { generation["reasoning_tier"]?.jsonPrimitive?.content },
    )
}

internal fun bindAndroidInvocation(model: JsonObject, config: JsonObject): JsonObject {
    val policy = buildJsonObject {
        for (key in listOf("host_catalog", "tools", "permission_rules", "context_window")) put(key, config.getValue(key))
        put("model_capabilities", model.getValue("model_capabilities"))
    }
    return buildJsonObject {
        put("version", ANDROID_HOST_INVOCATION_BINDING_VERSION); put("primary", model.getValue("primary")); put("fallbacks", JsonArray(emptyList()))
        put("generation", model.getValue("generation")); put("output_admission", model.getValue("output_admission"))
        put("execution_policy_sha256", invocationSha256(canonicalInvocationJson(policy).toString()))
    }
}
