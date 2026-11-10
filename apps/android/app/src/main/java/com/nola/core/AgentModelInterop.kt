@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)

package com.nola.core

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

data class AgentParsedModelOutput(
    val content: String,
    val reasoning: String,
    val toolCalls: List<JsonObject>,
    val finishReason: String,
    val warnings: List<String>
)

object AgentModelInterop {
    private val json = Json { ignoreUnknownKeys = true; explicitNulls = false }

    fun stitchContinuation(accumulated: String, continuation: String): String? {
        val input = buildJsonObject {
            put("accumulated", accumulated)
            put("continuation", continuation)
        }
        val bytes = AgentCoreNative.nativeStitchAgentModelContinuation(
            input.toString().encodeToByteArray()
        ) ?: return null
        return runCatching {
            json.parseToJsonElement(bytes.decodeToString())
                .jsonObject["append"]
                ?.jsonPrimitive
                ?.content
        }.getOrNull()
    }

    fun parse(
        content: String,
        reasoning: String,
        nativeToolCalls: List<JsonObject>,
        allowedToolNames: List<String>,
    ): AgentParsedModelOutput? {
        val input = buildJsonObject {
            put("content", content)
            put("reasoning", reasoning)
            put("native_tool_calls", JsonArray(nativeToolCalls))
            put("allowed_tool_names", JsonArray(allowedToolNames.map { JsonPrimitive(it) }))
        }
        val bytes = AgentCoreNative.nativeParseAgentModelOutput(input.toString().encodeToByteArray()) ?: return null
        val root = runCatching { json.parseToJsonElement(bytes.decodeToString()).jsonObject }.getOrNull() ?: return null
        return AgentParsedModelOutput(
            content = root["content"]?.jsonPrimitive?.content.orEmpty(),
            reasoning = root["reasoning"]?.jsonPrimitive?.content.orEmpty(),
            toolCalls = root["tool_calls"]?.jsonArray?.mapNotNull { it as? JsonObject }.orEmpty(),
            finishReason = root["finish_reason"]?.jsonPrimitive?.content ?: "stop",
            warnings = root["warnings"]?.jsonArray?.map { it.jsonPrimitive.content }.orEmpty()
        )
    }
}
