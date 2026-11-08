package com.nola.llm

import com.nola.domain.model.ModelInput
import com.nola.domain.model.ModelOutput
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.serialization.json.JsonObject
import kotlin.time.Duration

interface LlmClient {
    suspend fun generateSummary(input: ModelInput, timeout: Duration? = null): ModelOutput
    fun generateSummaryStream(input: ModelInput): Flow<String>
    val supportsNativeEventStream: Boolean get() = false
    fun generateEventStream(input: ModelInput): Flow<LlmStreamEvent> = flow {
        throw UnsupportedOperationException("Typed Agent streaming is not supported by this provider")
    }
    suspend fun chat(
        message: String,
        systemPrompt: String? = null,
        enableThinking: Boolean = true,
        timeout: Duration? = null
    ): String
}

class LlmRequestException(
    val statusCode: Int,
    val retryable: Boolean,
    providerErrorCode: String? = null,
    message: String,
) : IllegalStateException(message) {
    val code: String = providerErrorCode ?: when (statusCode) {
        408 -> "model_timeout"
        429 -> "model_rate_limited"
        in 500..599 -> "model_server_unavailable"
        else -> "model_request_failed"
    }
}

data class LlmStreamUsage(
    val promptTokens: Int? = null,
    val cachedInputTokens: Int? = null,
    val cacheWriteInputTokens: Int? = null,
    val completionTokens: Int? = null,
    val totalTokens: Int? = null,
    val reasoningTokens: Int? = null,
)

sealed interface LlmStreamEvent {
    data class TextDelta(val text: String) : LlmStreamEvent
    data class ReasoningDelta(val text: String) : LlmStreamEvent
    data class ToolCallDelta(
        val index: Int,
        val callId: String,
        val capabilityId: String,
        val argumentsDelta: String,
    ) : LlmStreamEvent
    data class Usage(val usage: LlmStreamUsage) : LlmStreamEvent
    data class Continuation(val value: JsonObject) : LlmStreamEvent
    data class Completed(val finishReason: String = "stop") : LlmStreamEvent
}
