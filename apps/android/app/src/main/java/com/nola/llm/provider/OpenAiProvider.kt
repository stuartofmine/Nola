package com.nola.llm.provider

import com.nola.data.remote.applyStreamingTimeouts

import com.nola.domain.model.ModelInput
import com.nola.domain.model.ModelOutput
import com.nola.domain.model.ModelParameterLimits
import com.nola.domain.model.ModelExecutionCapabilities
import com.nola.domain.model.ModelWireProtocol
import com.nola.domain.model.providerMessages
import com.nola.llm.LlmClient
import com.nola.llm.LlmRequestException
import com.nola.llm.LlmStreamEvent
import com.nola.llm.LlmStreamUsage
import io.ktor.client.HttpClient
import io.ktor.client.call.body
import io.ktor.client.request.get
import io.ktor.client.request.header
import io.ktor.client.request.preparePost
import io.ktor.client.request.setBody
import io.ktor.client.statement.bodyAsChannel
import io.ktor.client.statement.bodyAsText
import io.ktor.http.ContentType
import io.ktor.http.contentType
import io.ktor.utils.io.readUTF8Line
import java.net.URI
import java.security.MessageDigest
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.emitAll
import kotlinx.coroutines.flow.flowOn
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonObjectBuilder
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.putJsonObject
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import kotlin.time.Duration

private fun ModelInput.requiredToolChoice(): JsonObject? =
    userPrefs["required_tool_name"]
        ?.trim()
        ?.takeIf(String::isNotEmpty)
        ?.let { requiredToolName ->
            buildJsonObject {
                put("type", "function")
                putJsonObject("function") {
                    put("name", requiredToolName)
                }
            }
        }

/** Shared by one-shot, text-stream and event-stream requests. Never silently clamp user settings. */
internal fun JsonObjectBuilder.putGenerationParameters(config: OpenAiProvider.Config) {
    require(config.temperature.isFinite() && config.temperature in 0f..2f) { "Temperature must be in 0..2" }
    require(config.topP.isFinite() && config.topP in 0f..1f) { "Top P must be in 0..1" }
    require(config.topK == null || config.topK >= 1) { "Top K must be positive" }
    require(!config.executionCapabilities.samplingRequiresNoReasoning || config.reasoningSelection?.get("value")==JsonPrimitive("none")) {
        "Conditional sampling requires an explicit reasoning-effort contract unavailable in direct Chat Completions"
    }
    require(config.executionCapabilities.samplingParameters ||
        (config.temperature == 0.7f && config.topP == 0.9f && !config.sendTopK)) {
        "This model does not declare sampling parameter support"
    }
    if (config.executionCapabilities.samplingParameters) {
        put("temperature", config.temperature.toDouble())
        put("top_p", config.topP.toDouble())
    }
    config.reasoningSelection?.let {selection ->
        require(selection["kind"]==JsonPrimitive("effort")) {"Chat Completions requires an effort selection"}
        put("reasoning_effort",selection.getValue("value"))
    }
    if (config.sendTopK) config.topK?.let { put("top_k", it) }
    require(config.presencePenalty.isFinite() && config.presencePenalty in -2.0..2.0)
    require(config.frequencyPenalty.isFinite() && config.frequencyPenalty in -2.0..2.0)
    require(config.repetitionPenalty == null || config.repetitionPenalty.isFinite() && config.repetitionPenalty > 0)
    if (config.presencePenalty != 0.0) put("presence_penalty", config.presencePenalty)
    if (config.frequencyPenalty != 0.0) put("frequency_penalty", config.frequencyPenalty)
    config.repetitionPenalty?.let { put("repetition_penalty", it) }
    config.seed?.let { put("seed", it) }
    val stops = config.stopSequences.lines().map(String::trim).filter(String::isNotEmpty)
    if (stops.isNotEmpty()) put("stop", JsonArray(stops.map(::JsonPrimitive)))
    put(config.executionCapabilities.chatOutputTokenField.requestKey(config.baseUrl),
        ModelParameterLimits.validatedOutputTokens(config.maxCompletionTokens))
}

private fun validateExecution(input: ModelInput, config: OpenAiProvider.Config) {
    val capabilities = config.executionCapabilities
    require(capabilities.imageInput || input.attachments.isEmpty()) {
        "This model does not declare image-input support"
    }
    if (!capabilities.toolCalls) {
        require(input.tools.isEmpty() && input.providerMessages().none { message ->
            message["role"]?.jsonPrimitive?.contentOrNull == "tool" || message["tool_calls"] != null
        }) { "This model does not declare native tool-call support" }
    }
}

@Singleton
class OpenAiProvider @Inject constructor(
    private val httpClient: HttpClient
) : LlmClient {

    data class Config(
        val baseUrl: String,
        val apiKey: String,
        val model: String,
        val wireProtocol: ModelWireProtocol = ModelWireProtocol.ChatCompletions,
        val executionCapabilities: ModelExecutionCapabilities = ModelExecutionCapabilities(),
        val temperature: Float = 0.7f,
        val topP: Float = 0.9f,
        val topK: Int? = null,
        val sendTopK: Boolean = false,
        val presencePenalty: Double = 0.0,
        val frequencyPenalty: Double = 0.0,
        val repetitionPenalty: Double? = null,
        val seed: Long? = null,
        val stopSequences: String = "",
        val maxCompletionTokens: Int = 16_384,
        val supportsPromptCacheKey: Boolean = false,
        val promptCacheKey: String? = null,
        val promptCacheKeyPrefix: String? = null,
        val reasoningSelection: JsonObject? = null,
        val reasoningSummarySupported: Boolean? = null,
    ) {
        fun withResolvedRuntime(runtime: JsonObject?): Config = if (runtime==null) this else copy(
            reasoningSelection=runtime["reasoning_selection"]?.takeUnless {it is kotlinx.serialization.json.JsonNull}?.jsonObject,
            reasoningSummarySupported=runtime["model_capabilities"]?.jsonObject?.get("reasoning_summary_supported")
                ?.let { if (it == JsonNull) null else it.jsonPrimitive.boolean })
        fun withParameters(value: com.nola.domain.model.ModelParameters): Config = copy(
            temperature = value.temperature, topP = value.topP, topK = value.topK,
            sendTopK = value.topK != null, maxCompletionTokens = value.maxCompletionTokens,
            presencePenalty = value.presencePenalty, frequencyPenalty = value.frequencyPenalty,
            repetitionPenalty = value.repetitionPenalty, seed = value.seed, stopSequences = value.stopSequences,
        )
    }

    var config: Config? = null

    // The returned instance is not the mutable application-wide provider.
    internal fun clientForTurn(configuration: Config): LlmClient =
        OpenAiProvider(httpClient).apply { config = configuration }
    private val json = Json { ignoreUnknownKeys = true; isLenient = true }
    override val supportsNativeEventStream: Boolean get() = true

    suspend fun testConnection(baseUrl: String, apiKey: String): Int {
        val responseBody = httpClient.get(openAiModelsUrl(baseUrl)) {
            if (apiKey.isNotBlank()) {
                header("Authorization", "Bearer $apiKey")
            }
        }.body<String>()
        val obj = json.decodeFromString<JsonObject>(responseBody)
        return obj["data"]?.jsonArray?.size
            ?: obj["models"]?.jsonArray?.size
            ?: 0
    }

    override suspend fun generateSummary(input: ModelInput, timeout: Duration?): ModelOutput {
        val cfg = config ?: return ModelOutput("No model configured")
        validateExecution(input, cfg)
        if (cfg.wireProtocol == ModelWireProtocol.Responses) {
            return OpenAiResponsesAdapter(httpClient, cfg).summary(input)
        }
        val messages = input.providerMessages()
        val enableThinking = input.userPrefs["enable_thinking"] != "false"

        val body = buildJsonObject {
            put("model", cfg.model)
            putJsonArray("messages") {
                messages.forEach { add(it) }
            }
            if (input.tools.isNotEmpty()) {
                put("tools", kotlinx.serialization.json.JsonArray(input.tools))
                put("parallel_tool_calls", input.userPrefs["parallel_tool_calls"] == "true")
                input.requiredToolChoice()?.let { put("tool_choice", it) }
            }
            put("stream", false)
            putGenerationParameters(cfg)
            promptCacheKeyForRequest(input, cfg)?.let { put("prompt_cache_key", it) }
            if (!enableThinking && shouldSendDashScopeThinkingControl(cfg.baseUrl, cfg.model)) {
                put("enable_thinking", false)
            }
        }

        val response = httpClient.preparePost(openAiChatCompletionsUrl(cfg.baseUrl)) {
            contentType(ContentType.Application.Json)
            header("Authorization", "Bearer ${cfg.apiKey}")
            setBody(body)
        }.execute()

        val responseBody = response.body<String>()
        if (response.status.value !in 200..299) {
            throw llmRequestException(response.status.value, responseBody)
        }
        val obj = json.decodeFromString<JsonObject>(responseBody)
        val content = obj["choices"]?.jsonArray?.get(0)?.jsonObject
            ?.get("message")?.jsonObject?.get("content")
            ?.jsonPrimitive?.contentOrNull ?: ""
        val responseMessage = obj["choices"]?.jsonArray?.get(0)?.jsonObject
            ?.get("message")?.jsonObject
        val reasoning = responseMessage?.get("reasoning_content")?.jsonPrimitive?.contentOrNull
            ?: responseMessage?.get("reasoning")?.jsonPrimitive?.contentOrNull
            ?: ""
        val toolCalls = responseMessage?.get("tool_calls")?.jsonArray?.mapNotNull { element ->
            val call = element as? JsonObject ?: return@mapNotNull null
            val function = call["function"] as? JsonObject ?: return@mapNotNull null
            val name = function["name"]?.jsonPrimitive?.contentOrNull ?: return@mapNotNull null
            val argumentsText = function["arguments"]?.jsonPrimitive?.contentOrNull ?: "{}"
            val arguments = runCatching { json.parseToJsonElement(argumentsText) as? JsonObject }
                .getOrNull() ?: JsonObject(emptyMap())
            buildJsonObject {
                put("id", call["id"]?.jsonPrimitive?.contentOrNull.orEmpty())
                put("name", name)
                put("arguments", arguments)
            }
        }.orEmpty()
        val finishReason = obj["choices"]?.jsonArray?.get(0)?.jsonObject
            ?.get("finish_reason")?.jsonPrimitive?.contentOrNull ?: "unknown"
        val usage = (obj["usage"] as? JsonObject)?.get("total_tokens")?.toString()?.toIntOrNull()

        return ModelOutput(
            text = content,
            tokensUsed = usage,
            finishReason = finishReason,
            reasoning = reasoning,
            toolCalls = toolCalls,
        )
    }

    override fun generateSummaryStream(input: ModelInput): Flow<String> = flow<String> {
        val cfg = config ?: return@flow
        validateExecution(input, cfg)
        if (cfg.wireProtocol == ModelWireProtocol.Responses) {
            OpenAiResponsesAdapter(httpClient, cfg).events(input).collect { event ->
                when (event) {
                    is LlmStreamEvent.TextDelta -> emit(event.text)
                    is LlmStreamEvent.ReasoningDelta -> if (input.userPrefs["enable_thinking"] != "false") {
                        emit("<think>${event.text}</think>\n")
                    }
                    else -> Unit
                }
            }
            return@flow
        }

        val messages = input.providerMessages()
        val enableThinking = input.userPrefs["enable_thinking"] != "false"

        val body = buildJsonObject {
            put("model", cfg.model)
            putJsonArray("messages") {
                messages.forEach { add(it) }
            }
            if (input.tools.isNotEmpty()) {
                put("tools", kotlinx.serialization.json.JsonArray(input.tools))
                put("parallel_tool_calls", input.userPrefs["parallel_tool_calls"] == "true")
                input.requiredToolChoice()?.let { put("tool_choice", it) }
            }
            put("stream", true)
            putGenerationParameters(cfg)
            promptCacheKeyForRequest(input, cfg)?.let { put("prompt_cache_key", it) }
            if (!enableThinking && shouldSendDashScopeThinkingControl(cfg.baseUrl, cfg.model)) {
                put("enable_thinking", false)
            }
        }

        httpClient.preparePost(openAiChatCompletionsUrl(cfg.baseUrl)) {
            applyStreamingTimeouts(httpClient)
            contentType(ContentType.Application.Json)
            header("Authorization", "Bearer ${cfg.apiKey}")
            setBody(body)
        }.execute { response ->
            if (response.status.value !in 200..299) {
                val errorBody = response.bodyAsText()
                throw llmRequestException(response.status.value, errorBody)
            }
            val channel = response.bodyAsChannel()
            var thinkingOpen = false
            var sawExplicitTerminal = false
            while (true) {
                val line = channel.readUTF8Line() ?: break
                if (line.startsWith("data: ")) {
                    val payload = line.removePrefix("data: ")
                    if (payload == "[DONE]") {
                        sawExplicitTerminal = true
                        break
                    }
                    try {
                        val obj = json.decodeFromString<JsonObject>(payload)
                        val choices = runCatching { obj["choices"]?.jsonArray }.getOrNull().orEmpty()
                        if (choices.any { choice ->
                                (choice as? JsonObject)
                                    ?.get("finish_reason")
                                    ?.jsonPrimitive
                                    ?.contentOrNull
                                    ?.isNotEmpty() == true
                            }) {
                            sawExplicitTerminal = true
                        }
                        val delta = obj["choices"]?.jsonArray?.get(0)?.jsonObject
                            ?.get("delta")?.jsonObject
                        val reasoning = delta?.get("reasoning_content")?.jsonPrimitive?.contentOrNull
                            ?: delta?.get("reasoning")?.jsonPrimitive?.contentOrNull
                        if (enableThinking && !reasoning.isNullOrEmpty()) {
                            if (!thinkingOpen) {
                                emit("<think>")
                                thinkingOpen = true
                            }
                            emit(reasoning)
                        }
                        val content = delta?.get("content")?.jsonPrimitive?.contentOrNull
                        if (!content.isNullOrEmpty() && content != "null") {
                            if (thinkingOpen) {
                                emit("</think>\n")
                                thinkingOpen = false
                            }
                            emit(content)
                        }
                    } catch (_: Exception) { }
                }
            }
            if (!sawExplicitTerminal) {
                throw java.io.IOException("Model stream ended before an explicit completion event")
            }
            if (thinkingOpen) emit("</think>\n")
        }
    }.flowOn(Dispatchers.IO)

    override fun generateEventStream(input: ModelInput): Flow<LlmStreamEvent> = flow {
        val cfg = config ?: error("No model configured")
        validateExecution(input, cfg)
        if (cfg.wireProtocol == ModelWireProtocol.Responses) {
            emitAll(OpenAiResponsesAdapter(httpClient, cfg).events(input))
            return@flow
        }
        val enableThinking = input.userPrefs["enable_thinking"] != "false"
        val body = buildJsonObject {
            put("model", cfg.model)
            putJsonArray("messages") { input.providerMessages().forEach { add(it) } }
            if (input.tools.isNotEmpty()) {
                put("tools", kotlinx.serialization.json.JsonArray(input.tools))
                put("parallel_tool_calls", input.userPrefs["parallel_tool_calls"] == "true")
                input.requiredToolChoice()?.let { put("tool_choice", it) }
            }
            put("stream", true)
            putJsonObject("stream_options") { put("include_usage", true) }
            putGenerationParameters(cfg)
            promptCacheKeyForRequest(input, cfg)?.let { put("prompt_cache_key", it) }
            if (!enableThinking && shouldSendDashScopeThinkingControl(cfg.baseUrl, cfg.model)) {
                put("enable_thinking", false)
            }
        }

        httpClient.preparePost(openAiChatCompletionsUrl(cfg.baseUrl)) {
            applyStreamingTimeouts(httpClient)
            contentType(ContentType.Application.Json)
            header("Authorization", "Bearer ${cfg.apiKey}")
            setBody(body)
        }.execute { response ->
            if (response.status.value !in 200..299) {
                throw llmRequestException(response.status.value, response.bodyAsText())
            }
            val states = mutableMapOf<Int, NativeToolCallState>()
            var finishReason = "stop"
            var sawExplicitTerminal = false
            val channel = response.bodyAsChannel()
            while (true) {
                val line = channel.readUTF8Line() ?: break
                if (!line.startsWith("data:")) continue
                val payload = line.removePrefix("data:").trimStart()
                if (payload == "[DONE]") {
                    sawExplicitTerminal = true
                    break
                }
                val envelope = runCatching { json.decodeFromString<JsonObject>(payload) }.getOrNull() ?: continue
                envelope["error"]?.let { throw IllegalStateException(it.toString()) }
                (envelope["usage"] as? JsonObject)?.let { usage ->
                    val promptTokens = usage["prompt_tokens"]?.jsonPrimitive?.intOrNull
                    val promptTokenDetails = usage["prompt_tokens_details"] as? JsonObject
                    val cachedInputTokens = promptTokenDetails?.get("cached_tokens")?.jsonPrimitive?.intOrNull
                        ?: usage["cache_read_input_tokens"]?.jsonPrimitive?.intOrNull
                    val cacheWriteInputTokens = usage["cache_creation_input_tokens"]?.jsonPrimitive?.intOrNull
                    val completionTokens = usage["completion_tokens"]?.jsonPrimitive?.intOrNull
                    val totalTokens = usage["total_tokens"]?.jsonPrimitive?.intOrNull
                    emit(LlmStreamEvent.Usage(LlmStreamUsage(
                        promptTokens = promptTokens,
                        cachedInputTokens = cachedInputTokens,
                        cacheWriteInputTokens = cacheWriteInputTokens,
                        completionTokens = completionTokens,
                        totalTokens = totalTokens,
                        reasoningTokens = (usage["completion_tokens_details"] as? JsonObject)?.get("reasoning_tokens")?.jsonPrimitive?.intOrNull,
                    )))
                }
                val choices = runCatching { envelope["choices"]?.jsonArray }.getOrNull().orEmpty()
                choices.forEach { choiceElement ->
                    val choice = choiceElement as? JsonObject ?: return@forEach
                    choice["finish_reason"]?.jsonPrimitive?.contentOrNull
                        ?.takeIf(String::isNotEmpty)?.let {
                            finishReason = it
                            sawExplicitTerminal = true
                        }
                    val delta = choice["delta"] as? JsonObject ?: return@forEach
                    val reasoning = delta["reasoning_content"]?.jsonPrimitive?.contentOrNull
                        ?: delta["reasoning"]?.jsonPrimitive?.contentOrNull
                    // This is provider-emitted content, not an instruction to generate reasoning.
                    // Core invocation policy decides whether it is projected to the user.
                    if (!reasoning.isNullOrEmpty()) {
                        emit(LlmStreamEvent.ReasoningDelta(reasoning))
                    }
                    delta["content"]?.jsonPrimitive?.contentOrNull
                        ?.takeIf { it.isNotEmpty() && it != "null" }
                        ?.let { emit(LlmStreamEvent.TextDelta(it)) }
                    val calls = runCatching { delta["tool_calls"]?.jsonArray }.getOrNull().orEmpty()
                    calls.forEach callsLoop@{ callElement ->
                        val call = callElement as? JsonObject ?: return@callsLoop
                        val index = call["index"]?.jsonPrimitive?.intOrNull ?: 0
                        val upstreamId = call["id"]?.jsonPrimitive?.contentOrNull?.trim().orEmpty()
                        val state = states.getOrPut(index) {
                            NativeToolCallState(callId = upstreamId.ifEmpty { "${UUID.randomUUID()}-$index" })
                        }
                        val function = call["function"] as? JsonObject
                        function?.get("name")?.jsonPrimitive?.contentOrNull
                            ?.takeIf(String::isNotEmpty)?.let { name ->
                                state.capabilityId = when {
                                    name.startsWith(state.capabilityId) -> name
                                    state.capabilityId.endsWith(name) -> state.capabilityId
                                    else -> state.capabilityId + name
                                }
                            }
                        function?.get("arguments")?.jsonPrimitive?.contentOrNull
                            ?.takeIf(String::isNotEmpty)?.let(state.pendingArguments::append)
                        if (state.capabilityId.isNotEmpty() && state.pendingArguments.isNotEmpty()) {
                            emit(LlmStreamEvent.ToolCallDelta(
                                index = index,
                                callId = state.callId,
                                capabilityId = state.capabilityId,
                                argumentsDelta = state.pendingArguments.toString(),
                            ))
                            state.pendingArguments.clear()
                            state.didEmitArguments = true
                        }
                    }
                }
            }
            if (!sawExplicitTerminal) {
                throw java.io.IOException("Model stream ended before an explicit completion event")
            }
            states.toSortedMap().forEach { (index, state) ->
                check(state.capabilityId.isNotEmpty()) { "A streamed tool call had no capability name" }
                if (state.pendingArguments.isNotEmpty() || !state.didEmitArguments) {
                    emit(LlmStreamEvent.ToolCallDelta(
                        index = index,
                        callId = state.callId,
                        capabilityId = state.capabilityId,
                        argumentsDelta = state.pendingArguments.toString().ifEmpty { "{}" },
                    ))
                }
            }
            emit(LlmStreamEvent.Completed(finishReason))
        }
    }.flowOn(Dispatchers.IO)

    override suspend fun chat(
        message: String,
        systemPrompt: String?,
        enableThinking: Boolean,
        timeout: Duration?
    ): String {
        val result = generateSummary(ModelInput(
            messages = listOf(mapOf("role" to "user", "content" to message)),
            systemPrompt = systemPrompt,
            userPrefs = mapOf("enable_thinking" to enableThinking.toString()),
        ))
        return result.text
    }
}

internal fun llmRequestException(statusCode: Int, body: String): LlmRequestException =
    LlmRequestException(
        statusCode = statusCode,
        retryable = statusCode == 408 || statusCode == 425 || statusCode == 429 || statusCode >= 500,
        providerErrorCode = openAiProviderErrorCode(body),
        message = "LLM request failed ($statusCode): ${body.take(200)}",
    )

/**
 * Reads only the provider's typed error envelope. Request echoes and localized
 * prose are deliberately ignored so routing remains a protocol decision.
 */
internal fun openAiProviderErrorCode(body: String): String? {
    val root = runCatching { Json.parseToJsonElement(body).jsonObject }.getOrNull() ?: return null
    val errors = listOfNotNull(root["error"], root["errors"]).flatMap { envelope ->
        runCatching { listOf(envelope.jsonObject) }.getOrElse {
            runCatching {
                envelope.jsonArray.mapNotNull { item ->
                    runCatching { item.jsonObject }.getOrNull()
                }
            }.getOrDefault(emptyList())
        }
    }
    for (error in errors) {
        for (key in listOf("code", "type", "error_code")) {
            val providerCode = runCatching {
                error[key]?.jsonPrimitive?.contentOrNull
            }.getOrNull()
            when (providerCode?.trim()?.lowercase()) {
                "context_length_exceeded",
                "context_window_exceeded",
                "context_length_error",
                "prompt_too_long",
                "input_too_long",
                "request_too_large" -> return "context_length_exceeded"
            }
        }
    }
    return null
}

private class NativeToolCallState(
    val callId: String,
    var capabilityId: String = "",
    val pendingArguments: StringBuilder = StringBuilder(),
    var didEmitArguments: Boolean = false,
)

internal fun promptCacheKeyForRequest(
    input: ModelInput,
    config: OpenAiProvider.Config,
): String? {
    if (!config.supportsPromptCacheKey) return null
    val stableDigest = input.stablePromptDigest
        ?.trim()
        ?.lowercase()
        ?.takeIf { it.length == 64 && it.all(Char::isHexDigit) }
        ?: return null
    config.promptCacheKey?.trim()?.takeIf(String::isNotEmpty)?.let { return it }

    val provider = runCatching { URI(config.baseUrl.trim()).host.orEmpty().lowercase() }
        .getOrDefault("")
        .ifBlank { "openai-compatible" }
    val namespace = listOfNotNull(
        config.promptCacheKeyPrefix?.trim()?.takeIf(String::isNotEmpty),
        provider,
        config.model.trim(),
        stableDigest,
    ).joinToString("\u0000")
    return MessageDigest.getInstance("SHA-256")
        .digest(namespace.encodeToByteArray())
        .joinToString("") { byte -> "%02x".format(byte.toInt() and 0xff) }
}

private fun Char.isHexDigit(): Boolean =
    this in '0'..'9' || this in 'a'..'f' || this in 'A'..'F'

internal fun shouldSendDashScopeThinkingControl(baseUrl: String, model: String): Boolean {
    val host = runCatching { java.net.URI(baseUrl.trim()).host.orEmpty().lowercase() }
        .getOrDefault("")
    val normalizedModel = model.lowercase()
    return host.contains("dashscope.aliyuncs.com") && (
        normalizedModel.contains("qwen3") ||
            normalizedModel.contains("qwen-3") ||
            normalizedModel.contains("qwq")
        )
}

internal fun openAiChatCompletionsUrl(configuredUrl: String): String {
    val normalized = configuredUrl.trim().trimEnd('/')
    require(normalized.isNotEmpty()) { "OpenAI endpoint must not be empty" }
    return when {
        normalized.endsWith("/chat/completions", ignoreCase = true) -> normalized
        normalized.endsWith("/v1", ignoreCase = true) -> "$normalized/chat/completions"
        else -> "$normalized/v1/chat/completions"
    }
}

internal fun openAiModelsUrl(configuredUrl: String): String {
    val completionsUrl = openAiChatCompletionsUrl(configuredUrl)
    return completionsUrl.removeSuffix("/chat/completions") + "/models"
}
