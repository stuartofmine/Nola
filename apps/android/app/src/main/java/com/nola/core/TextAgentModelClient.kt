package com.nola.core

import com.nola.llm.LlmClient
import com.nola.domain.model.ModelAttachment
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.takeWhile
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

internal fun isModelOutputLimitFinishReason(finishReason: String): Boolean =
    when (finishReason.trim().lowercase()) {
        "length", "max_tokens", "max_output_tokens", "token_limit" -> true
        else -> false
    }

internal suspend fun Flow<com.nola.llm.LlmStreamEvent>.collectThroughProtocolCompletion(
    action: suspend (com.nola.llm.LlmStreamEvent) -> Unit,
) {
    takeWhile { event ->
        action(event)
        event !is com.nola.llm.LlmStreamEvent.Completed
    }.collect { }
}

internal class AgentNativeToolCallAccumulator(
    private val fallbackCallIdPrefix: String = "tool-call",
) {
    data class StreamDelta(
        val callId: String,
        val capabilityId: String,
        val argumentsDelta: String,
    )

    private data class Buffer(
        val callId: String,
        var capabilityId: String,
        val arguments: StringBuilder = StringBuilder(),
        var emittedArgumentCount: Int = 0,
    )

    private val buffers = linkedMapOf<Int, Buffer>()
    val hasCalls: Boolean get() = buffers.isNotEmpty()

    fun append(
        index: Int,
        callId: String,
        capabilityId: String,
        argumentsDelta: String,
    ): StreamDelta? {
        val buffer = buffers.getOrPut(index) {
            Buffer(
                callId = callId.ifEmpty { "$fallbackCallIdPrefix-$index" },
                capabilityId = capabilityId,
            )
        }
        if (capabilityId.isNotEmpty()) buffer.capabilityId = capabilityId
        buffer.arguments.append(argumentsDelta)
        if (buffer.capabilityId.isEmpty()) return null
        val pending = buffer.arguments.substring(buffer.emittedArgumentCount)
        buffer.emittedArgumentCount = buffer.arguments.length
        return StreamDelta(
            callId = buffer.callId,
            capabilityId = buffer.capabilityId,
            argumentsDelta = pending,
        )
    }

    fun append(callId: String, capabilityId: String, argumentsDelta: String) {
        val existingIndex = buffers.entries.firstOrNull { it.value.callId == callId }?.key
        val index = existingIndex ?: ((buffers.keys.maxOrNull() ?: -1) + 1)
        append(index, callId, capabilityId, argumentsDelta)
    }

    fun completedCalls(): List<JsonObject> = buffers.values.map { buffer ->
        require(buffer.capabilityId.isNotEmpty()) { "A streamed tool call has no capability ID" }
        val rawArguments = buffer.arguments.toString().ifBlank { "{}" }
        buildJsonObject {
            put("call_id", buffer.callId)
            put("name", buffer.capabilityId)
            put("arguments", Json.parseToJsonElement(rawArguments))
        }
    }
}

class TextAgentModelClient(
    private val client: LlmClient,
    private val enableThinking: Boolean = true,
    private val requestBuilder: AgentRequestModelBuilder = AgentRequestModelBuilder(),
    private val attachments: List<ModelAttachment> = emptyList(),
) : AgentModelClient {
    override fun generateAgentTurn(request: AgentModelRequest): Flow<AgentModelEvent> = flow {
        val builtEnvelope = requestBuilder.build(request, enableThinking)
        val envelope = builtEnvelope.copy(
            modelInput = builtEnvelope.modelInput.copy(
                attachments = if (request.iteration == 0) attachments else emptyList(),
            ),
        )
        val invocation = request.invocationPlan
        val hasNativeTools = invocation.nativeToolCalls && request.tools.isNotEmpty()
        val canStreamNativeTools = !hasNativeTools || invocation.streamToolCalls
        val hasStreamingChannel = invocation.streamText || invocation.streamReasoning || invocation.streamToolCalls || request.requiredToolName != null
        if (client.supportsNativeEventStream && canStreamNativeTools && hasStreamingChannel) {
            val content = StringBuilder()
            val reasoning = StringBuilder()
            var finishReason = "stop"
            var sawExplicitCompletion = false
            var usage = AgentCoreTokenUsage()
            var providerContinuation: JsonObject? = null
            val nativeToolCalls = AgentNativeToolCallAccumulator(
                fallbackCallIdPrefix = "${request.turnId}-${request.iteration}-tool-call",
            )
            suspend fun applyEvent(event: com.nola.llm.LlmStreamEvent, includeTextChannels: Boolean) {
                when (event) {
                    is com.nola.llm.LlmStreamEvent.TextDelta -> {
                        if (!includeTextChannels) return
                        content.append(event.text)
                        if (invocation.streamText) emit(AgentModelEvent.TextDelta(event.text))
                    }
                    is com.nola.llm.LlmStreamEvent.ReasoningDelta -> {
                        if (!includeTextChannels) return
                        reasoning.append(event.text)
                        if (invocation.streamReasoning) emit(AgentModelEvent.ReasoningDelta(event.text))
                    }
                    is com.nola.llm.LlmStreamEvent.ToolCallDelta -> {
                        val normalized = nativeToolCalls.append(
                            index = event.index,
                            callId = event.callId,
                            capabilityId = event.capabilityId,
                            argumentsDelta = event.argumentsDelta,
                        ) ?: return
                        if (invocation.streamToolCalls) {
                            emit(AgentModelEvent.ToolCallDelta(
                                partId = "tool:${normalized.callId}",
                                callId = normalized.callId,
                                capabilityId = normalized.capabilityId,
                                argumentsDelta = normalized.argumentsDelta,
                            ))
                        }
                    }
                    is com.nola.llm.LlmStreamEvent.Usage -> {
                        usage = AgentCoreTokenUsage(
                            promptTokens = event.usage.promptTokens,
                            cachedInputTokens = event.usage.cachedInputTokens,
                            cacheWriteInputTokens = event.usage.cacheWriteInputTokens,
                            completionTokens = event.usage.completionTokens,
                            totalTokens = event.usage.totalTokens,
                            reasoningTokens = event.usage.reasoningTokens,
                        )
                    }
                    is com.nola.llm.LlmStreamEvent.Continuation -> {
                        providerContinuation = event.value
                    }
                    is com.nola.llm.LlmStreamEvent.Completed -> {
                        finishReason = event.finishReason
                        sawExplicitCompletion = true
                    }
                }
            }
            client.generateEventStream(envelope.modelInput)
                .collectThroughProtocolCompletion { event ->
                    applyEvent(event, includeTextChannels = true)
                }
            if (!sawExplicitCompletion) {
                throw java.io.IOException("Model event stream ended before completion")
            }
            val completion = if (nativeToolCalls.hasCalls) {
                val completedCalls = nativeToolCalls.completedCalls()
                request.requiredToolName?.let { requiredToolName ->
                    require(completedCalls.any { call -> call["name"]?.jsonPrimitive?.contentOrNull == requiredToolName }) {
                        "The model did not call the required tool"
                    }
                }
                AgentModelCompletion(
                    content = content.toString(),
                    reasoning = reasoning.toString(),
                    toolCalls = completedCalls,
                    finishReason = finishReason,
                    usage = usage,
                    providerContinuation = providerContinuation,
                )
            } else {
                val parsed = AgentModelInterop.parse(
                    content = content.toString(),
                    reasoning = reasoning.toString(),
                    nativeToolCalls = emptyList(),
                    allowedToolNames = envelope.allowedToolNames,
                ) ?: error("Nola core did not parse the streamed model output")
                AgentModelCompletion(
                    content = parsed.content,
                    reasoning = parsed.reasoning,
                    toolCalls = parsed.toolCalls,
                    finishReason = finishReason,
                    usage = usage,
                    providerContinuation = providerContinuation,
                )
            }
            emit(AgentModelEvent.Completed(completion))
            return@flow
        }
        val output = client.generateSummary(envelope.modelInput, timeout = null)
        val parsed = AgentModelInterop.parse(
            content = output.text,
            reasoning = output.reasoning,
            nativeToolCalls = output.toolCalls,
            allowedToolNames = envelope.allowedToolNames,
        )
            ?: error("Nola core did not parse the local model output")
        request.requiredToolName?.let { requiredToolName ->
            require(parsed.toolCalls.any { call -> call["name"]?.jsonPrimitive?.contentOrNull == requiredToolName }) {
                "The model did not call the required tool"
            }
        }
        emit(AgentModelEvent.Completed(
            AgentModelCompletion(
                content = parsed.content,
                reasoning = parsed.reasoning,
                toolCalls = parsed.toolCalls,
                finishReason = output.finishReason,
                usage = AgentCoreTokenUsage(totalTokens = output.tokensUsed),
                providerContinuation = output.providerContinuation,
            )
        ))
    }
}
