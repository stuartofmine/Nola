package com.nola.llm.provider

import com.nola.domain.model.ModelInput
import com.nola.domain.model.ModelOutput
import com.nola.domain.model.ModelParameterLimits
import com.nola.domain.model.providerMessages
import com.nola.llm.LlmStreamEvent
import com.nola.llm.LlmStreamUsage
import io.ktor.client.HttpClient
import io.ktor.client.request.header
import com.nola.data.remote.applyStreamingTimeouts
import io.ktor.client.request.preparePost
import io.ktor.client.request.setBody
import io.ktor.client.statement.bodyAsChannel
import io.ktor.client.statement.bodyAsText
import io.ktor.http.ContentType
import io.ktor.http.contentType
import io.ktor.utils.io.readUTF8Line
import java.security.MessageDigest
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOn
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

internal fun openAiResponsesUrl(baseUrl: String): String {
    val normalized = baseUrl.trim().trimEnd('/')
    return when {
        normalized.endsWith("/responses", ignoreCase = true) -> normalized
        normalized.endsWith("/chat/completions", ignoreCase = true) ->
            normalized.dropLast("/chat/completions".length) + "/responses"
        normalized.endsWith("/v1", ignoreCase = true) -> "$normalized/responses"
        else -> "$normalized/v1/responses"
    }
}

private fun routeIdentity(config: OpenAiProvider.Config): String = MessageDigest.getInstance("SHA-256")
    .digest("${config.baseUrl.trim().trimEnd('/')}\n${config.model}\n${config.apiKey}".toByteArray())
    .joinToString("") { "%02x".format(it.toInt() and 0xff) }

private fun JsonElement?.string(): String? = (this as? JsonPrimitive)?.contentOrNull

private fun responseItemForStatelessReplay(item: JsonElement): JsonElement {
    val objectValue = item as? JsonObject ?: return item
    if (objectValue["type"].string() != "reasoning") return item
    val hasPlaintext = (objectValue["content"] as? JsonArray)?.any { part ->
        (part as? JsonObject)?.get("type").string() == "reasoning_text" &&
            !(part as? JsonObject)?.get("text").string().isNullOrEmpty()
    } == true
    if (!hasPlaintext) return item
    return JsonObject(objectValue.filterKeys { it != "encrypted_content" && it != "summary" })
}

/** Stateless replay is scoped to the exact route; opaque reasoning items are never synthesized. */
internal fun responsesRequestBody(input: ModelInput, config: OpenAiProvider.Config): JsonObject {
    val continuation = input.providerContinuation
    if (continuation != null) {
        require(continuation["provider_namespace"].string() == "openai.responses")
        require(continuation["model_id"].string() == config.model)
        require(continuation["state"]?.jsonObject?.get("route_identity").string() == routeIdentity(config)) {
            "Responses continuation belongs to a different model route"
        }
    }
    val responseItems = continuation?.get("state")?.jsonObject?.get("response_items") as? JsonArray
    val messages = input.providerMessages()
    val replayIndex = if (responseItems.isNullOrEmpty()) -1 else messages.indexOfLast {
        it["role"].string() == "assistant"
    }
    require(responseItems.isNullOrEmpty() || replayIndex >= 0) {
        "Responses continuation has no assistant message to replace"
    }
    val providerCallIds = mutableMapOf<String, String>()
    responseItems?.forEach { item ->
        val call = item as? JsonObject ?: return@forEach
        if (call["type"].string() == "function_call") {
            val id = call["call_id"].string() ?: return@forEach
            providerCallIds[id] = id
        }
    }
    val converted = buildJsonArray {
        messages.forEachIndexed { index, message ->
            if (index == replayIndex) {
                responseItems!!.forEach { add(responseItemForStatelessReplay(it)) }
                return@forEachIndexed
            }
            val role = message["role"].string() ?: error("Model message has no role")
            if (role == "tool") {
                val scoped = message["tool_call_id"].string() ?: error("Tool result has no call ID")
                val providerId = providerCallIds.keys.firstOrNull { scoped == it || scoped.endsWith("::$it") }
                    ?: scoped
                add(buildJsonObject {
                    put("type", "function_call_output")
                    put("call_id", providerId)
                    put("output", message["content"] ?: JsonPrimitive(""))
                })
                return@forEachIndexed
            }
            require(role in setOf("system", "developer", "user", "assistant")) { "Unsupported Responses role: $role" }
            val content = message["content"] ?: JsonPrimitive("")
            val convertedContent = if (content is JsonArray) buildJsonArray {
                content.forEach { block ->
                    val value = block.jsonObject
                    when (value["type"].string()) {
                        "text" -> add(buildJsonObject {
                            put("type", if (role == "assistant") "output_text" else "input_text")
                            put("text", value["text"] ?: JsonPrimitive(""))
                        })
                        "image_url" -> add(buildJsonObject {
                            put("type", "input_image")
                            put("image_url", value["image_url"]?.jsonObject?.get("url") ?: error("Image URL missing"))
                        })
                        else -> error("Unsupported Responses content block")
                    }
                }
            } else content
            add(buildJsonObject {
                put("role", role)
                put("content", convertedContent)
            })
            (message["tool_calls"] as? JsonArray)?.forEach { element ->
                val call = element.jsonObject
                val function = (call["function"] as? JsonObject) ?: call
                add(buildJsonObject {
                    put("type", "function_call")
                    put("call_id", call["id"] ?: call["call_id"] ?: error("Tool call has no ID"))
                    put("name", function["name"] ?: error("Tool call has no name"))
                    val arguments = function["arguments"] ?: JsonPrimitive("{}")
                    put("arguments", if (arguments is JsonPrimitive && arguments.isString) arguments else JsonPrimitive(arguments.toString()))
                })
            }
        }
    }
    val capabilities = config.executionCapabilities
    require(!capabilities.samplingRequiresNoReasoning || config.reasoningSelection?.get("value")==JsonPrimitive("none")) {
        "Conditional sampling requires an explicit Responses reasoning-effort selection"
    }
    require(capabilities.samplingParameters || (config.temperature == 0.7f && config.topP == 0.9f && !config.sendTopK)) {
        "This model does not declare sampling parameter support"
    }
    require(!config.sendTopK) { "Responses does not support this top-k parameter" }
    require(config.presencePenalty == 0.0 && config.frequencyPenalty == 0.0 &&
        config.repetitionPenalty == null && config.seed == null && config.stopSequences.isBlank()) {
        "Responses does not support the selected Chat Completions generation options"
    }
    return buildJsonObject {
        put("model", config.model)
        put("input", converted)
        put("stream", true)
        put("store", false)
        put("include", buildJsonArray { add(JsonPrimitive("reasoning.encrypted_content")) })
        put("max_output_tokens", ModelParameterLimits.validatedOutputTokens(config.maxCompletionTokens))
        val reasoning = buildJsonObject {
            config.reasoningSelection?.let { selection ->
                require(selection["kind"] == JsonPrimitive("effort")) { "Responses requires an effort selection" }
                put("effort", selection.getValue("value"))
            }
            if (capabilities.reasoning && input.userPrefs["enable_thinking"] != "false"
                && config.reasoningSelection?.get("value") != JsonPrimitive("none")
                && config.reasoningSummarySupported != false) {
                put("summary", "auto")
            }
        }
        if (reasoning.isNotEmpty()) put("reasoning", reasoning)
        if (capabilities.samplingParameters) {
            put("temperature", config.temperature.toDouble())
            put("top_p", config.topP.toDouble())
        }
        if (input.tools.isNotEmpty()) {
            require(capabilities.toolCalls) { "This model does not declare native tool-call support" }
            put("tools", buildJsonArray {
                input.tools.forEach { element ->
                    val source = element.jsonObject
                    val function = (source["function"] as? JsonObject) ?: source
                    add(buildJsonObject {
                        function.forEach { (key, value) -> put(key, value) }
                        put("type", "function")
                        if (function["strict"] == null) put("strict", false)
                    })
                }
            })
            input.userPrefs["required_tool_name"]?.takeIf(String::isNotBlank)?.let { required ->
                put("tool_choice", buildJsonObject { put("type", "function"); put("name", required) })
            }
            put("parallel_tool_calls", input.userPrefs["parallel_tool_calls"] == "true")
        }
        promptCacheKeyForRequest(input, config)?.let { put("prompt_cache_key", it) }
    }
}

internal class OpenAiResponsesAdapter(
    private val httpClient: HttpClient,
    private val config: OpenAiProvider.Config,
) {
    private val json = Json { ignoreUnknownKeys = true; isLenient = true }

    fun events(input: ModelInput): Flow<LlmStreamEvent> = flow {
        val body = responsesRequestBody(input, config)
        httpClient.preparePost(openAiResponsesUrl(config.baseUrl)) {
            applyStreamingTimeouts(httpClient)
            contentType(ContentType.Application.Json)
            header("Authorization", "Bearer ${config.apiKey}")
            setBody(body)
        }.execute { response ->
            if (response.status.value !in 200..299) {
                throw llmRequestException(response.status.value, response.bodyAsText())
            }
            var completed = false
            var streamedText = false
            // Per-output adapter progress, not UI state. The same reasoning can
            // arrive as live deltas and again in the terminal item snapshot.
            data class ReasoningProgress(val channel: String, val text: StringBuilder = StringBuilder())
            val reasoning = mutableMapOf<String, ReasoningProgress>()
            fun bindReasoningIdentity(id: String?, index: String?): ReasoningProgress? {
                val byId = id?.let(reasoning::get)
                val byIndex = index?.let(reasoning::get)
                require(byId == null || byIndex == null || byId === byIndex) {
                    "Responses reasoning item identities conflict"
                }
                val progress = byId ?: byIndex
                if (progress != null) { id?.let { reasoning[it] = progress }; index?.let { reasoning[it] = progress } }
                return progress
            }
            fun reasoningProgress(event: JsonObject, representation: String): ReasoningProgress {
                val id = event["item_id"].string()?.let { "id:$it" }
                val index = event["output_index"]?.jsonPrimitive?.intOrNull?.let { "index:$it" }
                val progress = bindReasoningIdentity(id, index)
                    ?: if (id == null && index == null) reasoning.getOrPut("unscoped") { ReasoningProgress(representation) }
                    else ReasoningProgress(representation)
                id?.let { reasoning[it] = progress }; index?.let { reasoning[it] = progress }
                return progress
            }
            fun visibleReasoning(item: JsonObject, channel: String): String =
                ((item[if (channel == "summary") "summary" else "content"] as? JsonArray).orEmpty())
                    .mapNotNull { it as? JsonObject }
                    .filter { it["type"].string() == if (channel == "summary") "summary_text" else "reasoning_text" }
                    .joinToString("") { it["text"].string().orEmpty() }
            val channel = response.bodyAsChannel()
            while (true) {
                val line = channel.readUTF8Line() ?: break
                if (!line.startsWith("data:")) continue
                val payload = line.removePrefix("data:").trimStart()
                if (payload == "[DONE]") break
                val event = runCatching { json.parseToJsonElement(payload).jsonObject }.getOrNull() ?: continue
                when (event["type"].string()) {
                    "response.output_text.delta", "response.refusal.delta" -> {
                        event["delta"].string()?.takeIf(String::isNotEmpty)?.let {
                            streamedText = true
                            emit(LlmStreamEvent.TextDelta(it))
                        }
                    }
                    "response.reasoning_summary_text.delta", "response.reasoning_text.delta" -> {
                        val representation = if (event["type"].string() == "response.reasoning_text.delta") "plain" else "summary"
                        val progress = reasoningProgress(event, representation)
                        // A provider may expose both a summary and plaintext for
                        // one output item. Display one representation once.
                        if (progress.channel == representation) event["delta"].string()
                            ?.takeIf(String::isNotEmpty)?.let { delta ->
                                progress.text.append(delta)
                                emit(LlmStreamEvent.ReasoningDelta(delta))
                            }
                    }
                    "response.failed", "error" -> error("Responses provider reported failure")
                    "response.completed", "response.incomplete" -> {
                        check(!completed) { "Responses stream returned duplicate terminal events" }
                        val result = event["response"]?.jsonObject ?: error("Responses terminal event has no response")
                        val output = result["output"] as? JsonArray ?: JsonArray(emptyList())
                        val reasoningItemCount = output.count { (it as? JsonObject)?.get("type").string() == "reasoning" }
                        require(reasoning["unscoped"] == null || reasoningItemCount <= 1) {
                            "Responses unscoped reasoning cannot identify multiple output items"
                        }
                        output.forEachIndexed { index, element ->
                            val item = element as? JsonObject ?: return@forEachIndexed
                            if (item["type"].string() != "reasoning") return@forEachIndexed
                            val key = item["id"].string()?.let { "id:$it" }
                            val progress = bindReasoningIdentity(key, "index:$index")
                                ?: reasoning["unscoped"].takeIf { reasoningItemCount == 1 }
                            val representation = progress?.channel ?: if (visibleReasoning(item, "summary").isNotEmpty()) "summary" else "plain"
                            val snapshot = visibleReasoning(item, representation)
                            val prior = progress?.text?.toString().orEmpty()
                            if (snapshot.isNotEmpty()) {
                                require(snapshot.startsWith(prior)) { "Responses reasoning snapshot contradicts its streamed output" }
                                snapshot.removePrefix(prior).takeIf(String::isNotEmpty)?.let { emit(LlmStreamEvent.ReasoningDelta(it)) }
                            }
                        }
                        if (!streamedText) output.forEach { item ->
                            if (item.jsonObject["type"].string() == "message") {
                                (item.jsonObject["content"] as? JsonArray)?.forEach { part ->
                                    part.jsonObject["text"].string()?.takeIf(String::isNotEmpty)
                                        ?.let { emit(LlmStreamEvent.TextDelta(it)) }
                                }
                            }
                        }
                        output.forEachIndexed { index, item ->
                            val call = item.jsonObject
                            if (call["type"].string() == "function_call") {
                                emit(LlmStreamEvent.ToolCallDelta(
                                    index = index,
                                    callId = call["call_id"].string() ?: error("Responses function call has no call_id"),
                                    capabilityId = call["name"].string() ?: error("Responses function call has no name"),
                                    argumentsDelta = call["arguments"].string().orEmpty().ifEmpty { "{}" },
                                ))
                            }
                        }
                        val usage = result["usage"] as? JsonObject
                        if (usage != null) {
                            val prompt = usage["input_tokens"]?.jsonPrimitive?.intOrNull
                            val completion = usage["output_tokens"]?.jsonPrimitive?.intOrNull
                            emit(LlmStreamEvent.Usage(LlmStreamUsage(
                                promptTokens = prompt,
                                cachedInputTokens = (usage["input_tokens_details"] as? JsonObject)
                                    ?.get("cached_tokens")?.jsonPrimitive?.intOrNull,
                                completionTokens = completion,
                                totalTokens = usage["total_tokens"]?.jsonPrimitive?.intOrNull,
                                reasoningTokens = (usage["output_tokens_details"] as? JsonObject)?.get("reasoning_tokens")?.jsonPrimitive?.intOrNull,
                            )))
                        }
                        emit(LlmStreamEvent.Continuation(buildJsonObject {
                            put("provider_namespace", "openai.responses")
                            put("model_id", config.model)
                            result["id"]?.let { put("response_id", it) }
                            put("state", buildJsonObject {
                                put("response_items", output)
                                put("route_identity", routeIdentity(config))
                            })
                            put("integrity_kind", "encrypted")
                            put("compaction_policy", "clear_on_compaction")
                        }))
                        val finish = if (event["type"].string() == "response.incomplete") {
                            result["incomplete_details"]?.jsonObject?.get("reason").string() ?: "incomplete"
                        } else "stop"
                        emit(LlmStreamEvent.Completed(finish))
                        completed = true
                    }
                }
            }
            check(completed) { "Responses stream ended before a terminal response event" }
        }
    }.flowOn(Dispatchers.IO)

    suspend fun summary(input: ModelInput): ModelOutput {
        val text = StringBuilder()
        val reasoning = StringBuilder()
        val calls = mutableListOf<JsonObject>()
        var continuation: JsonObject? = null
        var usage = LlmStreamUsage()
        var finishReason = "stop"
        events(input).collect { event ->
            when (event) {
                is LlmStreamEvent.TextDelta -> text.append(event.text)
                is LlmStreamEvent.ReasoningDelta -> reasoning.append(event.text)
                is LlmStreamEvent.ToolCallDelta -> calls += buildJsonObject {
                    put("id", event.callId)
                    put("name", event.capabilityId)
                    put("arguments", runCatching { json.parseToJsonElement(event.argumentsDelta) }.getOrDefault(JsonObject(emptyMap())))
                }
                is LlmStreamEvent.Usage -> usage = event.usage
                is LlmStreamEvent.Continuation -> continuation = event.value
                is LlmStreamEvent.Completed -> finishReason = event.finishReason
            }
        }
        return ModelOutput(text.toString(), usage.totalTokens, finishReason, reasoning.toString(), calls, continuation)
    }
}
