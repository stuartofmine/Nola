package com.nola.data.repository

import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelParameters
import kotlinx.serialization.json.*

internal fun conversationParameterKey(model: ModelInfo): String = conversationModelIdentity(model)

internal fun conversationModelSelection(stored: String?, model: ModelInfo, value: ModelParameters): JsonObject {
    val key = conversationModelIdentity(model)
    val fields = Json.parseToJsonElement(writeConversationParameters(stored, model, value)).jsonObject
    return buildJsonObject {
        put("kind", "model")
        put("model_id", key)
        put("parameters", fields.getValue("modelOverrides").jsonObject.getValue(key))
        put("presentation", fields.getValue("clientModelPresentation").jsonObject.getValue(key))
    }
}

internal fun conversationHostSelection(binding: com.nola.domain.model.ExecutionHostBinding, value: ModelParameters): JsonObject {
    validateExecutionHostBinding(binding)
    return buildJsonObject {
        put("kind", "host")
        put("binding", Json.encodeToJsonElement(com.nola.domain.model.ExecutionHostBinding.serializer(), binding))
        put("parameters", parameterFields(value))
        put("presentation", buildJsonObject {
            put("responseStyle", value.responseStyle); put("showThinking", value.enableThinking)
            put("enableRetrieval", value.enableRetrieval); put("streamOutput", value.streamOutput)
            put("customSystemPrompt", value.customSystemPrompt)
        })
    }
}

internal fun readConversationParameters(stored: String?, model: ModelInfo): ModelParameters? {
    val root = stored?.let { Json.parseToJsonElement(it).jsonObject } ?: return null
    val key = conversationParameterKey(model)
    val fields = root["modelOverrides"]?.jsonObject?.get(key)?.jsonObject ?: return null
    return readParameterFields(fields, root["clientModelPresentation"]?.jsonObject?.get(key)?.jsonObject)
}

internal fun readHostParameters(stored: String?): ModelParameters? {
    val root = stored?.let { Json.parseToJsonElement(it).jsonObject } ?: return null
    return readParameterFields(root["clientHostParameters"] as? JsonObject ?: return null,
        (root["clientModelPresentation"] as? JsonObject)?.get("execution-host") as? JsonObject)
}

private fun readParameterFields(fields: JsonObject, options: JsonObject?): ModelParameters? {
    val context = fields["maxContextLength"]?.jsonPrimitive?.intOrNull ?: return null
    val output = fields["maxOutputTokens"]?.jsonPrimitive?.intOrNull ?: return null
    val temperature = fields["temperature"]?.jsonPrimitive?.floatOrNull ?: return null
    val topP = fields["topP"]?.jsonPrimitive?.floatOrNull ?: return null
    return applyModelPresentation(ModelParameters(temperature = temperature, topP = topP,
        topK = fields["topK"]?.jsonPrimitive?.intOrNull,
        presencePenalty = fields["presencePenalty"]?.jsonPrimitive?.doubleOrNull ?: 0.0,
        frequencyPenalty = fields["frequencyPenalty"]?.jsonPrimitive?.doubleOrNull ?: 0.0,
        repetitionPenalty = fields["repetitionPenalty"]?.jsonPrimitive?.doubleOrNull,
        seed = (fields["seed"] as? JsonObject)?.takeIf { it["kind"]?.jsonPrimitive?.content == "value" }
            ?.get("value")?.jsonPrimitive?.longOrNull,
        stopSequences = fields["stopSequences"]?.jsonPrimitive?.contentOrNull ?: "",
        reasoningTier = fields["reasoningTier"]?.jsonPrimitive?.contentOrNull,
        reasoningPreference = fields["reasoningPreference"]?.jsonPrimitive?.contentOrNull,
        maxContextTokens = context, maxCompletionTokens = output), options)
}

/** Presentation is native Host guidance; numeric generation inheritance remains Core-owned. */
internal fun applyModelPresentation(base: ModelParameters, options: JsonObject?): ModelParameters = base.copy(
    responseStyle = options?.get("responseStyle")?.jsonPrimitive?.contentOrNull ?: base.responseStyle,
    enableThinking = options?.get("showThinking")?.jsonPrimitive?.booleanOrNull ?: base.enableThinking,
    enableRetrieval = options?.get("enableRetrieval")?.jsonPrimitive?.booleanOrNull ?: base.enableRetrieval,
    streamOutput = options?.get("streamOutput")?.jsonPrimitive?.booleanOrNull ?: base.streamOutput,
    customSystemPrompt = options?.get("customSystemPrompt")?.jsonPrimitive?.contentOrNull ?: base.customSystemPrompt,
)

internal fun initialHostSettings(binding: com.nola.domain.model.ExecutionHostBinding, value: ModelParameters): JsonObject = buildJsonObject {
    validateExecutionHostBinding(binding)
    put("executionHostBinding", Json.encodeToJsonElement(com.nola.domain.model.ExecutionHostBinding.serializer(), binding))
    put("clientHostParameters", parameterFields(value))
    put("clientModelPresentation", buildJsonObject { put("execution-host", buildJsonObject {
        put("responseStyle", value.responseStyle); put("showThinking", value.enableThinking)
        put("enableRetrieval", value.enableRetrieval); put("streamOutput", value.streamOutput)
        put("customSystemPrompt", value.customSystemPrompt)
    }) })
}

internal fun writeConversationParameters(stored: String?, model: ModelInfo, value: ModelParameters): String {
    val root = (stored?.let { Json.parseToJsonElement(it).jsonObject } ?: JsonObject(emptyMap())).toMutableMap()
    val key = conversationParameterKey(model)
    val models = root["modelOverrides"]?.jsonObject?.toMutableMap() ?: mutableMapOf()
    val fields = models[key]?.jsonObject?.toMutableMap() ?: mutableMapOf()
    fields.putAll(parameterFields(value))
    models[key] = JsonObject(fields); root["modelOverrides"] = JsonObject(models)
    val clients = root["clientModelPresentation"]?.jsonObject?.toMutableMap() ?: mutableMapOf()
    val options = clients[key]?.jsonObject?.toMutableMap() ?: mutableMapOf()
    options.putAll(mapOf("responseStyle" to JsonPrimitive(value.responseStyle),
        "showThinking" to JsonPrimitive(value.enableThinking), "enableRetrieval" to JsonPrimitive(value.enableRetrieval),
        "streamOutput" to JsonPrimitive(value.streamOutput), "customSystemPrompt" to JsonPrimitive(value.customSystemPrompt)))
    clients[key] = JsonObject(options); root["clientModelPresentation"] = JsonObject(clients)
    return JsonObject(root).toString()
}

private fun parameterFields(value: ModelParameters): JsonObject = buildJsonObject {
    put("temperature", value.temperature); put("topP", value.topP); put("topK", value.topK)
    put("maxContextLength", value.maxContextTokens); put("maxOutputTokens", value.maxCompletionTokens)
    put("presencePenalty", value.presencePenalty); put("frequencyPenalty", value.frequencyPenalty)
    put("repetitionPenalty", value.repetitionPenalty); put("stopSequences", value.stopSequences)
    put("reasoningPreference", value.reasoningPreference?.let(::JsonPrimitive) ?: JsonNull)
    put("reasoningTier", value.reasoningTier?.let(::JsonPrimitive) ?: JsonNull)
    put("seed", buildJsonObject {
        put("kind", if (value.seed == null) "clear" else "value")
        value.seed?.let { put("value", it.toString()) }
    })
}
