package com.nola.core

import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.buffer
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.produceIn
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.collect
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

data class AgentModelInvocationPlan(
    val nativeToolCalls: Boolean = false,
    val parallelToolCalls: Boolean = false,
    val reasoning: Boolean = false,
    val promptCache: Boolean = false,
    val streamText: Boolean = false,
    val streamReasoning: Boolean = false,
    val streamToolCalls: Boolean = false,
)

data class AgentModelRequest(
    val sessionId: String,
    val turnId: String,
    val iteration: Int,
    val messages: List<AgentCoreMessage>,
    val tools: List<JsonElement>,
    val reservedOutputTokens: Int,
    val policyFragments: List<AgentPromptFragment> = emptyList(),
    val stablePromptDigest: String? = null,
    val invocationPlan: AgentModelInvocationPlan = AgentModelInvocationPlan(),
    val requiredToolName: String? = null,
    val providerContinuation: JsonObject? = null,
)

data class AgentModelCompletion(
    val content: String,
    val reasoning: String = "",
    val toolCalls: List<JsonObject> = emptyList(),
    val finishReason: String = "stop",
    val usage: AgentCoreTokenUsage = AgentCoreTokenUsage(),
    val providerContinuation: JsonObject? = null,
)

sealed interface AgentModelEvent {
    data class TextDelta(val text: String) : AgentModelEvent
    data class ReasoningDelta(val text: String) : AgentModelEvent
    data class ToolCallDelta(
        val partId: String,
        val callId: String,
        val capabilityId: String,
        val argumentsDelta: String,
    ) : AgentModelEvent
    data class Completed(val completion: AgentModelCompletion) : AgentModelEvent
}

interface AgentModelClient {
    fun generateAgentTurn(request: AgentModelRequest): Flow<AgentModelEvent>
}

object AgentStructuredOutputExecutor {
    suspend fun execute(
        sessionId: String,
        turnId: String,
        systemInstructions: String,
        modelInput: JsonElement,
        responseSchema: JsonElement,
        submissionToolName: String,
        submissionToolDescription: String,
        client: AgentModelClient,
        reservedOutputTokens: Int = 4_096,
    ): JsonElement {
        val normalizedToolName = submissionToolName.trim()
        require(normalizedToolName.isNotEmpty()) {
            "Structured output requires a submission tool name"
        }
        val tool = buildJsonObject {
            put("type", "function")
            put("function", buildJsonObject {
                put("name", normalizedToolName)
                put("description", submissionToolDescription)
                put("parameters", responseSchema)
            })
        }
        val request = AgentModelRequest(
            sessionId = sessionId,
            turnId = turnId,
            iteration = 1,
            messages = listOf(
                AgentCoreMessage(role = "system", content = systemInstructions),
                AgentCoreMessage(
                    role = "user",
                    content = Json.encodeToString(JsonElement.serializer(), modelInput),
                ),
            ),
            tools = listOf(tool),
            reservedOutputTokens = reservedOutputTokens.coerceIn(256, 8_192),
            invocationPlan = AgentModelInvocationPlan(
                nativeToolCalls = true,
                parallelToolCalls = false,
                reasoning = false,
                promptCache = false,
                streamText = false,
                streamReasoning = false,
                streamToolCalls = true,
            ),
            requiredToolName = normalizedToolName,
        )
        var completion: AgentModelCompletion? = null
        client.generateAgentTurn(request).collect { event ->
            if (event is AgentModelEvent.Completed) completion = event.completion
        }
        val matchingCalls = completion?.toolCalls.orEmpty().filter {
            it["name"]?.jsonPrimitive?.contentOrNull == normalizedToolName
        }
        check(matchingCalls.size == 1) {
            "Model did not submit exactly one required structured output"
        }
        return matchingCalls.single()["arguments"]
            ?: error("Structured output tool call did not contain arguments")
    }
}

object AgentMemoryWorkerModelExecutor {
    const val submissionToolName = "submit_memory_worker_output"

    suspend fun execute(
        plan: JsonObject,
        client: AgentModelClient,
        reservedOutputTokens: Int = 6_144,
    ): JsonElement {
        val jobId = plan["job_id"]?.jsonPrimitive?.contentOrNull
            ?: error("Core returned a Memory Worker plan without job_id")
        val systemInstructions = plan["system_instructions"]?.jsonPrimitive?.contentOrNull
            ?: error("Core returned a Memory Worker plan without system_instructions")
        val modelInput = plan["model_input"]
            ?: error("Core returned a Memory Worker plan without model_input")
        val responseSchema = plan["response_schema"]
            ?: error("Core returned a Memory Worker plan without response_schema")
        return AgentStructuredOutputExecutor.execute(
            sessionId = "memory-worker:$jobId",
            turnId = "memory-worker:$jobId",
            systemInstructions = systemInstructions,
            modelInput = modelInput,
            responseSchema = responseSchema,
            submissionToolName = submissionToolName,
            submissionToolDescription = "Submit semantic memory candidates for deterministic Core materialization and validation.",
            client = client,
            reservedOutputTokens = reservedOutputTokens.coerceIn(256, 6_144),
        )
    }
}


/** Coalesce only text already queued during a Core commit; never delay a fresh delta. */
internal fun Flow<AgentModelEvent>.coalesceQueuedTextDeltas(): Flow<AgentModelEvent> = flow {
    coroutineScope {
        val events = this@coalesceQueuedTextDeltas.buffer(64).produceIn(this)
        var pending: AgentModelEvent? = null
        while (true) {
            val result = if (pending == null) events.receiveCatching() else null
            result?.exceptionOrNull()?.let { throw it }
            val first = pending ?: result?.getOrNull() ?: break
            pending = null
            if (first !is AgentModelEvent.TextDelta && first !is AgentModelEvent.ReasoningDelta) {
                emit(first)
                continue
            }
            val text = StringBuilder(when (first) {
                is AgentModelEvent.TextDelta -> first.text
                is AgentModelEvent.ReasoningDelta -> first.text
                else -> error("Unreachable text channel")
            })
            for (index in 0 until 63) {
                val next = events.tryReceive().getOrNull() ?: break
                when {
                    first is AgentModelEvent.TextDelta && next is AgentModelEvent.TextDelta -> text.append(next.text)
                    first is AgentModelEvent.ReasoningDelta && next is AgentModelEvent.ReasoningDelta -> text.append(next.text)
                    else -> { pending = next; break }
                }
            }
            emit(if (first is AgentModelEvent.TextDelta) AgentModelEvent.TextDelta(text.toString())
                else AgentModelEvent.ReasoningDelta(text.toString()))
        }
    }
}
