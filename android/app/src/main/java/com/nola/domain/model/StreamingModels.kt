package com.nola.domain.model

import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonObject

data class StreamState(
    val messages: List<ConversationMessage> = emptyList(),
    val processingState: ProcessingState = ProcessingState.Idle,
    val response: String = "",
    val thinking: String = "",
    val thinkingFinished: Boolean = false,
    val thinkingShowing: Boolean = true,
    val toolEvents: List<HostAgentToolEvent> = emptyList(),
    val permissionRequest: HostAgentPermissionRequest? = null,
    val tokenCount: Int = 0,
    val conversationId: String? = null,
    val agentMode: Boolean = false
)

enum class ProcessingState {
    Idle,
    BuildingContext,
    Streaming,
    ParsingThinking,
    AwaitingRecovery,
    Error;

    val isStreaming: Boolean get() = this == Streaming || this == ParsingThinking
    val isGenerating: Boolean get() = this == BuildingContext || isStreaming
    val blocksInput: Boolean get() = isGenerating || this == AwaitingRecovery
}

data class ModelInput(
    val messages: List<Map<String, String>> = emptyList(),
    val systemPrompt: String? = null,
    val userPrefs: Map<String, String> = emptyMap(),
    val canonicalMessages: List<JsonObject> = emptyList(),
    val tools: List<JsonElement> = emptyList(),
    val contextPayloadDigest: String? = null,
    /**
     * Core-owned identity of the stable prompt prefix for a main Agent turn.
     *
     * This is deliberately absent from auxiliary requests such as title generation,
     * classification, and compaction so they cannot inherit the main-turn cache identity.
     */
    val stablePromptDigest: String? = null,
    val attachments: List<ModelAttachment> = emptyList(),
    val providerContinuation: JsonObject? = null,
)

data class ModelAttachment(
    val name: String,
    val mimeType: String,
    val base64Data: String,
)

fun ModelInput.providerMessages(): List<JsonObject> {
    val baseMessages = if (canonicalMessages.isNotEmpty()) canonicalMessages else buildList {
        systemPrompt?.takeIf(String::isNotBlank)?.let { prompt ->
            add(buildJsonObject {
                put("role", "system")
                put("content", prompt)
            })
        }
        messages.forEach { message ->
            add(buildJsonObject {
                message.forEach { (key, value) -> put(key, JsonPrimitive(value)) }
            })
        }
    }
    if (attachments.isEmpty()) return baseMessages
    val lastUserIndex = baseMessages.indexOfLast {
        it["role"]?.let { role -> (role as? JsonPrimitive)?.content } == "user"
    }
    if (lastUserIndex < 0) return baseMessages
    return baseMessages.mapIndexed { index, message ->
        if (index != lastUserIndex) return@mapIndexed message
        val originalContent = message["content"]
        buildJsonObject {
            message.forEach { (key, value) ->
                if (key != "content") put(key, value)
            }
            put("content", buildJsonArray {
                when (originalContent) {
                    is JsonArray -> originalContent.forEach(::add)
                    is JsonPrimitive -> if (originalContent.content.isNotEmpty()) {
                        add(buildJsonObject {
                            put("type", "text")
                            put("text", originalContent.content)
                        })
                    }
                    else -> Unit
                }
                attachments.forEach { attachment ->
                    add(buildJsonObject {
                        put("type", "image_url")
                        putJsonObject("image_url") {
                            put("url", "data:${attachment.mimeType};base64,${attachment.base64Data}")
                        }
                    })
                }
            })
        }
    }
}

data class ModelOutput(
    val text: String,
    val tokensUsed: Int? = null,
    val finishReason: String = "stop",
    val reasoning: String = "",
    val toolCalls: List<JsonObject> = emptyList(),
    val providerContinuation: JsonObject? = null,
)
